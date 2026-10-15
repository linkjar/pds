//! The MCP endpoint: JSON-RPC 2.0 over Streamable HTTP
//! (`docs/sidecars.md` §2.4).
//!
//! Each request is one POST with one JSON-RPC message, and the answer is
//! one JSON body. The server starts no stream of its own, so a GET is
//! answered with 405.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_sidecar::client::ClientError;
use pds_sidecar::http::json_response;
use serde_json::{Map, Value, json};

use crate::Mcp;
use crate::pds::Xrpc;
use crate::store::{Session, is_nsid};

/// The revisions of the protocol the server speaks, the newest first.
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

/// The largest JSON-RPC message.
const BODY_LIMIT: usize = 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// Why a call did not give a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A JSON-RPC error: the method, the tool or its input is wrong.
    Protocol(i64, String),
    /// The session lacks a scope. The HTTP answer is 403 with a challenge.
    Scope(String),
    /// The session made too many calls in this minute.
    Busy,
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::Protocol(INVALID_PARAMS, message.into())
}

/// A tool of the server.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// The scope a session needs. A write tool also takes
    /// `repo:write:<collection>` for its collection.
    scope: &'static str,
    write: bool,
    /// The properties of the input, each with its JSON type and whether
    /// it is required.
    input: &'static [(&'static str, &'static str, bool, &'static str)],
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "whoami",
        description: "The account of this session: its DID and its handle.",
        scope: "account:read",
        write: false,
        input: &[],
    },
    Tool {
        name: "describe_repo",
        description: "The repository of the account: its handle, its DID document and its collections.",
        scope: "repo:read",
        write: false,
        input: &[],
    },
    Tool {
        name: "list_collections",
        description: "The collections of the repository, as NSIDs.",
        scope: "repo:read",
        write: false,
        input: &[],
    },
    Tool {
        name: "list_records",
        description: "A page of the records of one collection, with a cursor for the next page.",
        scope: "repo:read",
        write: false,
        input: &[
            ("collection", "string", true, "The NSID of the collection."),
            (
                "limit",
                "integer",
                false,
                "At most this many records, 1 to 100. Default 50.",
            ),
            ("cursor", "string", false, "The cursor of the page before."),
            ("reverse", "boolean", false, "The oldest record first."),
        ],
    },
    Tool {
        name: "get_record",
        description: "One record of the repository.",
        scope: "repo:read",
        write: false,
        input: &[
            ("collection", "string", true, "The NSID of the collection."),
            ("rkey", "string", true, "The record key."),
        ],
    },
    Tool {
        name: "create_record",
        description: "Creates a record in a collection. The PDS validates it against its lexicon.",
        scope: "repo:write",
        write: true,
        input: &[
            ("collection", "string", true, "The NSID of the collection."),
            (
                "record",
                "object",
                true,
                "The record. Its $type is the collection.",
            ),
            (
                "rkey",
                "string",
                false,
                "The record key. Without one the PDS makes a TID.",
            ),
        ],
    },
    Tool {
        name: "put_record",
        description: "Creates a record or replaces the one with this key.",
        scope: "repo:write",
        write: true,
        input: &[
            ("collection", "string", true, "The NSID of the collection."),
            ("rkey", "string", true, "The record key."),
            (
                "record",
                "object",
                true,
                "The record. Its $type is the collection.",
            ),
        ],
    },
    Tool {
        name: "delete_record",
        description: "Deletes one record.",
        scope: "repo:write",
        write: true,
        input: &[
            ("collection", "string", true, "The NSID of the collection."),
            ("rkey", "string", true, "The record key."),
        ],
    },
];

fn tool_json(tool: &Tool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, kind, needed, description) in tool.input {
        properties.insert(
            (*name).to_owned(),
            json!({ "type": kind, "description": description }),
        );
        if *needed {
            required.push(*name);
        }
    }
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
        "annotations": {
            "readOnlyHint": !tool.write,
            "destructiveHint": tool.name == "delete_record" || tool.name == "put_record",
        },
    })
}

const PROMPTS: &[(&str, &str, &str)] = &[
    (
        "summarize_collection",
        "Summarize the newest records of one collection.",
        "Use the list_records tool on the collection $collection and summarize the newest records. Name each record by its key.",
    ),
    (
        "explain_record",
        "Explain one record in plain words.",
        "Use the get_record tool for the collection $collection and the key $rkey. Explain what the record holds and what its fields mean.",
    ),
];

fn prompt_json(name: &str, description: &str) -> Value {
    let mut arguments = vec![
        json!({"name": "collection", "description": "The NSID of the collection.", "required": true}),
    ];
    if name == "explain_record" {
        arguments.push(json!({"name": "rkey", "description": "The record key.", "required": true}));
    }
    json!({ "name": name, "description": description, "arguments": arguments })
}

fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str, Failure> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("the input needs the string \"{key}\"")))
}

fn collection_of(input: &Map<String, Value>) -> Result<&str, Failure> {
    let collection = text(input, "collection")?;
    if !is_nsid(collection) {
        return Err(invalid("\"collection\" is not an NSID"));
    }
    Ok(collection)
}

/// The call of one request: the session and the name of its AI client.
struct Caller<'a> {
    mcp: &'a Mcp,
    session: &'a Session,
}

impl Caller<'_> {
    async fn xrpc(
        &self,
        nsid: &str,
        params: &[(&str, &str)],
        input: Option<&Value>,
    ) -> Result<Value, ClientError> {
        let call = Xrpc {
            nsid,
            params,
            input,
            ai_client: &self.session.client_id,
        };
        self.mcp.pds().xrpc(&self.session.pds, call).await
    }

    async fn read(
        &self,
        name: &str,
        input: &Map<String, Value>,
    ) -> Result<Result<Value, ClientError>, Failure> {
        let did = self.session.pds.did.as_str();
        let describe = "com.atproto.repo.describeRepo";
        Ok(match name {
            "whoami" => self.xrpc(describe, &[("repo", did)], None).await.map(|repo| {
                json!({ "did": repo["did"], "handle": repo["handle"], "pds": self.mcp.config().pds_public_url })
            }),
            "describe_repo" => self.xrpc(describe, &[("repo", did)], None).await,
            "list_collections" => self
                .xrpc(describe, &[("repo", did)], None)
                .await
                .map(|repo| json!({ "collections": repo["collections"] })),
            "list_records" => {
                let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100);
                let limit = limit.to_string();
                let mut params = vec![
                    ("repo", did),
                    ("collection", collection_of(input)?),
                    ("limit", limit.as_str()),
                ];
                if let Some(cursor) = input.get("cursor").and_then(Value::as_str) {
                    params.push(("cursor", cursor));
                }
                if input.get("reverse") == Some(&Value::Bool(true)) {
                    params.push(("reverse", "true"));
                }
                self.xrpc("com.atproto.repo.listRecords", &params, None).await
            }
            "get_record" => {
                let params = [
                    ("repo", did),
                    ("collection", collection_of(input)?),
                    ("rkey", text(input, "rkey")?),
                ];
                self.xrpc("com.atproto.repo.getRecord", &params, None).await
            }
            _ => return Err(Failure::Protocol(INVALID_PARAMS, format!("Unknown tool: {name}"))),
        })
    }

    async fn write(
        &self,
        name: &str,
        input: &Map<String, Value>,
    ) -> Result<Result<Value, ClientError>, Failure> {
        let did = self.session.pds.did.as_str();
        let collection = collection_of(input)?;
        if !self.session.may_write(collection) {
            return Err(Failure::Scope(format!("repo:write:{collection}")));
        }
        let record = || {
            input
                .get("record")
                .filter(|record| record.is_object())
                .ok_or_else(|| invalid("the input needs the object \"record\""))
        };
        let mut body = json!({ "repo": did, "collection": collection });
        let nsid = match name {
            "create_record" => {
                body["record"] = record()?.clone();
                if let Some(rkey) = input.get("rkey").and_then(Value::as_str) {
                    body["rkey"] = json!(rkey);
                }
                "com.atproto.repo.createRecord"
            }
            "put_record" => {
                body["record"] = record()?.clone();
                body["rkey"] = json!(text(input, "rkey")?);
                "com.atproto.repo.putRecord"
            }
            _ => {
                body["rkey"] = json!(text(input, "rkey")?);
                "com.atproto.repo.deleteRecord"
            }
        };
        Ok(self.xrpc(nsid, &[], Some(&body)).await)
    }

    /// `tools/call`. A refusal of the PDS is a result with `isError`, so
    /// that the model reads it.
    async fn call_tool(&self, params: &Value) -> Result<Value, Failure> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
            return Err(Failure::Protocol(
                INVALID_PARAMS,
                format!("Unknown tool: {name}"),
            ));
        };
        let empty = Map::new();
        let input = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(input)) => input,
            Some(_) => return Err(invalid("\"arguments\" must be an object")),
        };
        if !tool.write && !self.session.has(tool.scope) {
            return Err(Failure::Scope(tool.scope.to_owned()));
        }
        if !self.session.spend(tool.write) {
            return Err(Failure::Busy);
        }
        let answer = if tool.write {
            self.write(name, input).await?
        } else {
            self.read(name, input).await?
        };
        Ok(match answer {
            Ok(value) => json!({
                "content": [{ "type": "text", "text": value.to_string() }],
                "structuredContent": value,
                "isError": false,
            }),
            Err(error) => json!({
                "content": [{ "type": "text", "text": error.to_string() }],
                "isError": true,
            }),
        })
    }

    /// `resources/read` of `at://<did>/<collection>/<rkey>` in the
    /// repository of the session.
    async fn read_resource(&self, params: &Value) -> Result<Value, Failure> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let missing =
            || Failure::Protocol(RESOURCE_NOT_FOUND, format!("Resource not found: {uri}"));
        let parts: Vec<&str> = uri
            .strip_prefix("at://")
            .ok_or_else(missing)?
            .split('/')
            .collect();
        let [did, collection, rkey] = parts.as_slice() else {
            return Err(missing());
        };
        if *did != self.session.pds.did || !is_nsid(collection) || rkey.is_empty() {
            return Err(missing());
        }
        if !self.session.has("repo:read") {
            return Err(Failure::Scope("repo:read".to_owned()));
        }
        if !self.session.spend(false) {
            return Err(Failure::Busy);
        }
        let params = [("repo", *did), ("collection", *collection), ("rkey", *rkey)];
        let record = self
            .xrpc("com.atproto.repo.getRecord", &params, None)
            .await
            .map_err(|_| missing())?;
        Ok(json!({
            "contents": [{ "uri": uri, "mimeType": "application/json", "text": record["value"].to_string() }],
        }))
    }
}

fn get_prompt(params: &Value) -> Result<Value, Failure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some((_, description, template)) = PROMPTS.iter().find(|(known, _, _)| *known == name)
    else {
        return Err(invalid(format!("Unknown prompt: {name}")));
    };
    let argument = |key: &str| {
        params
            .get("arguments")
            .and_then(|arguments| arguments.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let collection = argument("collection");
    if !is_nsid(collection) {
        return Err(invalid(
            "the prompt needs the argument \"collection\", an NSID",
        ));
    }
    // A record key without the sign of a placeholder: the second
    // replacement puts in no placeholder of its own.
    let rkey: String = argument("rkey").chars().filter(|c| *c != '$').collect();
    let text = template
        .replace("$collection", collection)
        .replace("$rkey", &rkey);
    Ok(json!({
        "description": description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": text } }],
    }))
}

fn initialize(params: &Value) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = VERSIONS
        .iter()
        .find(|known| **known == asked)
        .unwrap_or(&VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
        "serverInfo": { "name": "linkjar-pds-mcp", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Tools read and write the atproto repository of the signed-in account. A write tool needs the scope repo:write or repo:write:<collection>.",
    })
}

async fn dispatch(caller: &Caller<'_>, method: &str, params: &Value) -> Result<Value, Failure> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": TOOLS.iter().map(tool_json).collect::<Vec<_>>() })),
        "tools/call" => caller.call_tool(params).await,
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({
            "resourceTemplates": [{
                "uriTemplate": format!("at://{}/{{collection}}/{{rkey}}", caller.session.pds.did),
                "name": "record",
                "description": "One record of the repository of the account.",
                "mimeType": "application/json",
            }],
        })),
        "resources/read" => caller.read_resource(params).await,
        "prompts/list" => Ok(json!({
            "prompts": PROMPTS.iter().map(|(name, description, _)| prompt_json(name, description)).collect::<Vec<_>>(),
        })),
        "prompts/get" => get_prompt(params),
        _ => Err(Failure::Protocol(
            METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )),
    }
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A 401 or a 403 with the challenge that names the metadata of the
/// resource (RFC 9728 §5.1) and, for a missing scope, the scope.
fn challenge(mcp: &Mcp, status: StatusCode, error: Option<(&str, &str)>) -> Response {
    let metadata = format!(
        "{}/.well-known/oauth-protected-resource",
        mcp.config().public_url
    );
    let mut value = format!(r#"Bearer resource_metadata="{metadata}""#);
    if let Some((error, scope)) = error {
        let _ = write!(value, r#", error="{error}""#);
        if !scope.is_empty() {
            let _ = write!(value, r#", scope="{scope}""#);
        }
    }
    let mut response = json_response(
        status,
        &json!({ "error": error.map_or("unauthorized", |(error, _)| error) }),
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

fn session_of(mcp: &Mcp, headers: &HeaderMap) -> Option<Arc<Session>> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| mcp.store().session(token.trim()))
        .flatten()
}

/// `POST /mcp`.
pub async fn post(State(mcp): State<Mcp>, headers: HeaderMap, body: Body) -> Response {
    let Some(session) = session_of(&mcp, &headers) else {
        let error = headers
            .contains_key(header::AUTHORIZATION)
            .then_some(("invalid_token", ""));
        return challenge(&mcp, StatusCode::UNAUTHORIZED, error);
    };
    let Ok(bytes) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return json_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rpc_error(&Value::Null, INVALID_REQUEST, "The message is too large"),
        );
    };
    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(&Value::Null, PARSE_ERROR, "Parse error"),
        );
    };
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").cloned();
    let (Some(method), Some("2.0")) = (method, message.get("jsonrpc").and_then(Value::as_str))
    else {
        // An answer of the client to a request the server never made, or
        // a message that is not JSON-RPC.
        if message.get("result").is_some() || message.get("error").is_some() {
            return StatusCode::ACCEPTED.into_response();
        }
        let id = id.unwrap_or(Value::Null);
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(&id, INVALID_REQUEST, "Invalid Request"),
        );
    };
    // A notification has no id and gets no answer.
    let Some(id) = id else {
        return StatusCode::ACCEPTED.into_response();
    };
    let caller = Caller {
        mcp: &mcp,
        session: &session,
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match dispatch(&caller, method, &params).await {
        Ok(result) => json_response(
            StatusCode::OK,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        ),
        Err(Failure::Protocol(code, text)) => {
            json_response(StatusCode::OK, &rpc_error(&id, code, &text))
        }
        Err(Failure::Scope(scope)) => challenge(
            &mcp,
            StatusCode::FORBIDDEN,
            Some(("insufficient_scope", &scope)),
        ),
        Err(Failure::Busy) => {
            let mut response = json_response(
                StatusCode::TOO_MANY_REQUESTS,
                &rpc_error(
                    &id,
                    INVALID_REQUEST,
                    "The session made too many calls in this minute",
                ),
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
            response
        }
    }
}

/// `GET /mcp`: the server starts no stream.
pub async fn get() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_has_a_schema_and_a_read_tool_says_so() {
        let tools: Vec<Value> = TOOLS.iter().map(tool_json).collect();
        assert_eq!(tools.len(), 8);
        let create = tools
            .iter()
            .find(|tool| tool["name"] == "create_record")
            .unwrap();
        assert_eq!(
            create["inputSchema"]["required"],
            json!(["collection", "record"])
        );
        assert_eq!(
            create["inputSchema"]["properties"]["record"]["type"],
            "object"
        );
        assert_eq!(create["annotations"]["readOnlyHint"], false);
        let list = tools
            .iter()
            .find(|tool| tool["name"] == "list_records")
            .unwrap();
        assert_eq!(list["annotations"]["readOnlyHint"], true);
        assert_eq!(list["inputSchema"]["additionalProperties"], false);
    }

    #[test]
    fn the_server_answers_with_a_revision_it_speaks() {
        let known = initialize(&json!({"protocolVersion": "2025-06-18"}));
        assert_eq!(known["protocolVersion"], "2025-06-18");
        let unknown = initialize(&json!({"protocolVersion": "1999-01-01"}));
        assert_eq!(unknown["protocolVersion"], VERSIONS[0]);
        assert!(known["capabilities"]["tools"].is_object());
    }

    #[test]
    fn a_prompt_takes_an_nsid_and_no_template_text() {
        let good = get_prompt(&json!({"name": "explain_record", "arguments": {"collection": "com.example.note", "rkey": "$collection"}})).unwrap();
        let text = good["messages"][0]["content"]["text"].as_str().unwrap();
        assert!(text.contains("com.example.note"));
        assert!(text.contains("the key collection."));
        assert!(
            get_prompt(
                &json!({"name": "explain_record", "arguments": {"collection": "ignore previous"}})
            )
            .is_err()
        );
        assert!(get_prompt(&json!({"name": "nope"})).is_err());
    }
}
