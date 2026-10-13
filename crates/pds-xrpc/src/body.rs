//! Request bodies, read and checked as the Reference reads and checks them
//! (`docs/SPEC.md` §4.2).
//!
//! The order is the Reference's (`createSchemaInputVerifier` in
//! `packages/xrpc-server/src/util.ts` at the pin): the body must be there,
//! then its encoding must be the method's, then it must fit the limit and
//! parse, then each property is checked in the order the lexicon declares
//! it. The messages are those of `@atproto/lex-schema`.
//!
//! One answer differs: for a body that is not JSON the Reference repeats
//! the message of its JSON parser, which names the engine's own position
//! and token. This server says that the body is not valid JSON.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use cid::Cid;
use pds_lexicon::wording::{
    invalid_format, invalid_type, json_path_segment, required_key, stringify_str,
};
use serde_json::{Map, Value};

use crate::error::XrpcError;
use crate::params::Format;

/// 413, as the Reference's body parser answers a body over the limit.
pub fn payload_too_large() -> XrpcError {
    XrpcError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        error: "PayloadTooLarge".to_owned(),
        message: "request entity too large".to_owned(),
    }
}

/// The media type of the request without its parameters, in lower case.
pub fn request_encoding(headers: &HeaderMap) -> Result<String, XrpcError> {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XrpcError::invalid_request("Request encoding (Content-Type) required but not provided")
        })
}

/// The `Content-Length` of the request, when it states one.
pub fn content_length(headers: &HeaderMap) -> Result<Option<u64>, XrpcError> {
    headers
        .get(header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .ok_or_else(|| XrpcError::invalid_request("invalid content-length"))
        })
        .transpose()
}

/// Fails for a request that carries no body at all: neither a length nor a
/// transfer encoding.
pub fn expect_body(headers: &HeaderMap) -> Result<(), XrpcError> {
    if headers.contains_key(header::TRANSFER_ENCODING)
        || headers.contains_key(header::CONTENT_LENGTH)
    {
        Ok(())
    } else {
        Err(XrpcError::invalid_request(
            "A request body is expected but none was provided",
        ))
    }
}

/// Reads the JSON object of a procedure's input. `limit` is the most bytes
/// the method reads.
pub async fn read_json(
    headers: &HeaderMap,
    body: Body,
    limit: usize,
) -> Result<Map<String, Value>, XrpcError> {
    expect_body(headers)?;
    let encoding = request_encoding(headers)?;
    if encoding != "application/json" {
        return Err(XrpcError::invalid_request(format!(
            "Wrong request encoding (Content-Type): {encoding}"
        )));
    }
    if content_length(headers)?.is_some_and(|length| length > limit as u64) {
        return Err(payload_too_large());
    }
    let bytes = axum::body::to_bytes(body, limit)
        .await
        .map_err(|_| payload_too_large())?;
    if bytes.is_empty() {
        // Express gives an empty body as an empty object.
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(other) => Err(XrpcError::invalid_request(format!(
            "{} at $",
            invalid_type(&["object"], &stringify(&other))
        ))),
        Err(_) => Err(XrpcError::invalid_request(
            "The request body is not valid JSON",
        )),
    }
}

/// A JSON value as a message shows it.
pub fn stringify(value: &Value) -> String {
    fn list<T>(items: impl Iterator<Item = T>, total: usize, show: impl Fn(T) -> String) -> String {
        let mut text = items.take(2).map(show).collect::<Vec<_>>().join(", ");
        if total > 2 {
            text.push_str(", ...");
        }
        text
    }
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(text) => stringify_str(text),
        Value::Array(items) => format!("[{}]", list(items.iter(), items.len(), stringify)),
        Value::Object(fields) => format!(
            "{{{}}}",
            list(fields.keys(), fields.len(), |key| format!(
                "{}: ...",
                stringify_str(key)
            ))
        ),
    }
}

/// The JSON path of the property `key` of the value at `at`.
pub fn child(at: &str, key: &str) -> String {
    format!("{at}{}", json_path_segment(key))
}

fn refuse(message: &str, at: &str) -> XrpcError {
    XrpcError::invalid_request(format!("{message} at {at}"))
}

/// The property `key` of the object at `at`, or the error of a required
/// property that is absent.
fn property<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    required: bool,
    at: &str,
) -> Result<Option<&'a Value>, XrpcError> {
    match object.get(key) {
        Some(value) => Ok(Some(value)),
        None if required => Err(refuse(&required_key(key), at)),
        None => Ok(None),
    }
}

/// A string property with a format and, when given, a longest length in
/// UTF-8 bytes.
pub fn string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    required: bool,
    format: Format,
    max_len: Option<usize>,
    at: &str,
) -> Result<Option<&'a str>, XrpcError> {
    let Some(value) = property(object, key, required, at)? else {
        return Ok(None);
    };
    let at = child(at, key);
    let Value::String(text) = value else {
        return Err(refuse(&invalid_type(&["string"], &stringify(value)), &at));
    };
    if let Some(max) = max_len
        && text.len() > max
    {
        return Err(refuse(
            &format!("string too big (maximum {max}, got {})", text.len()),
            &at,
        ));
    }
    if let Err(name) = format.check(text) {
        // `check` names the format as the message does; `invalid_format`
        // takes the lexicon's name.
        let lexicon_name = match format {
            Format::Any => "string",
            Format::Did => "did",
            Format::Handle => "handle",
            Format::AtIdentifier => "at-identifier",
            Format::Nsid => "nsid",
            Format::RecordKey => "record-key",
            Format::Tid => "tid",
            Format::Cid => "cid",
        };
        debug_assert!(!name.is_empty());
        return Err(refuse(
            &invalid_format(lexicon_name, &stringify_str(text)),
            &at,
        ));
    }
    Ok(Some(text))
}

/// A boolean property.
pub fn boolean(
    object: &Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<Option<bool>, XrpcError> {
    match property(object, key, false, at)? {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(other) => Err(refuse(
            &invalid_type(&["boolean"], &stringify(other)),
            &child(at, key),
        )),
    }
}

/// An optional property that holds a CID in its string form.
pub fn cid(object: &Map<String, Value>, key: &str, at: &str) -> Result<Option<Cid>, XrpcError> {
    let text = string(object, key, false, Format::Cid, None, at)?;
    text.map(|text| text.parse::<Cid>().map_err(XrpcError::internal))
        .transpose()
}

/// Checks that every value under `value` is one the data model has
/// (SPEC §6.1). JSON has one more: a number that is not an integer.
fn lex_values(value: &Value, at: &str) -> Result<(), XrpcError> {
    match value {
        Value::Number(number) if !number.is_i64() && !number.is_u64() => Err(refuse(
            &invalid_type(
                &[
                    "null", "boolean", "integer", "string", "cid", "bytes", "array", "object",
                ],
                &number.to_string(),
            ),
            at,
        )),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(index, item)| lex_values(item, &format!("{at}[{index}]"))),
        Value::Object(fields) => fields
            .iter()
            .try_for_each(|(key, field)| lex_values(field, &child(at, key))),
        _ => Ok(()),
    }
}

/// A required property that holds a record: an object of data model
/// values.
pub fn record<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<&'a Map<String, Value>, XrpcError> {
    let value = property(object, key, true, at)?.unwrap_or(&Value::Null);
    let at = child(at, key);
    let Value::Object(fields) = value else {
        return Err(refuse(&invalid_type(&["object"], &stringify(value)), &at));
    };
    lex_values(value, &at)?;
    Ok(fields)
}

/// A required property that holds an array.
pub fn array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<&'a [Value], XrpcError> {
    match property(object, key, true, at)? {
        Some(Value::Array(items)) => Ok(items),
        other => Err(refuse(
            &invalid_type(&["array"], &stringify(other.unwrap_or(&Value::Null))),
            &child(at, key),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(json: &str) -> Map<String, Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn properties_are_refused_with_the_reference_wording() {
        let body = object(r#"{"rkey":".","long":"kkkk","validate":"yes","n":5}"#);
        let message = |result: Result<Option<&str>, XrpcError>| result.unwrap_err().message;
        assert_eq!(
            message(string(&body, "repo", true, Format::AtIdentifier, None, "$")),
            r#"Missing required key "repo" at $"#
        );
        assert_eq!(
            message(string(
                &body,
                "rkey",
                false,
                Format::RecordKey,
                Some(512),
                "$"
            )),
            r#"Invalid record key (got ".") at $.rkey"#
        );
        assert_eq!(
            message(string(&body, "long", false, Format::Any, Some(3), "$")),
            "string too big (maximum 3, got 4) at $.long"
        );
        assert_eq!(
            message(string(&body, "n", false, Format::Any, None, "$")),
            "Expected string value type (got 5) at $.n"
        );
        assert_eq!(
            boolean(&body, "validate", "$").unwrap_err().message,
            r#"Expected boolean value type (got "yes") at $.validate"#
        );
        assert_eq!(
            string(&body, "absent", false, Format::Any, None, "$"),
            Ok(None)
        );
    }

    #[test]
    fn a_float_in_a_record_is_refused_where_it_is() {
        let body = object(r#"{"record":{"$type":"a.b.c","list":[1,{"ratio":1.5}]}}"#);
        assert_eq!(
            record(&body, "record", "$").unwrap_err().message,
            "Expected one of null, boolean, integer, string, cid, bytes, array or object value \
             type (got 1.5) at $.record.list[1].ratio"
        );
        let body = object(r#"{"record":[1]}"#);
        assert_eq!(
            record(&body, "record", "$").unwrap_err().message,
            "Expected object value type (got [1]) at $.record"
        );
        assert_eq!(
            record(&object("{}"), "record", "$").unwrap_err().message,
            r#"Missing required key "record" at $"#
        );
    }
}
