//! The pages of the console, rendered on the server. Every value that
//! comes from the PDS is escaped.

use std::fmt::Write as _;

use serde_json::Value;

use crate::session::Session;

pub use pds_oauth::pages::escape;

/// A value of the PDS as text: a string as it is, `null` as a dash.
fn show(value: &Value) -> String {
    match value {
        Value::Null => "–".to_owned(),
        Value::String(text) => escape(text),
        other => escape(&other.to_string()),
    }
}

/// Percent-encodes a value for a path segment or a query.
pub fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// The actions the console knows, with the text of each for the banner
/// that follows it.
pub const ACTIONS: &[(&str, &str)] = &[
    ("takedown", "The account is taken down."),
    ("reinstate", "The takedown is lifted."),
    ("revoke-sessions", "The sessions of the account are ended."),
    ("sync", "The server sent #sync for the repository."),
    ("disconnect", "The subscriber is disconnected."),
    ("block-domain", "The email domain is blocked."),
    ("unblock-domain", "The block of the email domain is lifted."),
    ("checkpoint", "The audit log has a new checkpoint."),
    ("purge", "Expired sessions and requests are removed."),
];

fn banner(done: Option<&str>) -> String {
    done.and_then(|done| ACTIONS.iter().find(|(name, _)| *name == done))
        .map(|(_, text)| format!(r#"<p class="banner" role="status">{text}</p>"#))
        .unwrap_or_default()
}

/// The frame of a page for an operator who is signed in.
fn layout(session: &Session, title: &str, attributes: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · PDS console</title>
<link rel="stylesheet" href="/console.css">
</head><body{attributes}>
<header><strong>PDS console</strong>
<nav><a href="/">Overview</a><a href="/accounts">Accounts</a><a href="/audit">Audit log</a><a href="/domains">Email domains</a><a href="/roles">Roles</a></nav>
<span class="dim"><code>{did}</code> · {role}</span>
<form method="post" action="/sign-out"><input type="hidden" name="csrf" value="{csrf}"><button>Sign out</button></form>
</header>
<main><h1>{title}</h1>
{body}
</main>
<script src="/console.js"></script>
</body></html>"#,
        title = escape(title),
        did = escape(&session.did),
        role = escape(&session.role),
        csrf = escape(&session.csrf),
    )
}

/// A page for a browser without a session.
pub fn plain(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · PDS console</title>
<link rel="stylesheet" href="/console.css">
</head><body><main><h1>{title}</h1>
{body}
</main></body></html>"#,
        title = escape(title),
    )
}

/// The page that offers the sign-in.
pub fn sign_in(pds: &str) -> String {
    plain(
        "Sign in",
        &format!(
            r#"<p>The console signs you in on the pages of <code>{}</code>. Your account needs an operator role on that server.</p>
<p><a href="/login">Sign in with your account</a></p>"#,
            escape(pds)
        ),
    )
}

/// A page that reports a failure.
pub fn failure(session: Option<&Session>, message: &str) -> String {
    let body = format!(
        r#"<p class="banner error" role="alert">{}</p><p><a href="/">Back to the overview</a></p>"#,
        escape(message)
    );
    match session {
        Some(session) => layout(session, "The action did not happen", "", &body),
        None => plain("The console could not continue", &body),
    }
}

/// A form that takes an action. Each has a field for the reason.
/// `confirm` is the text a person types before a dangerous action.
fn action(
    session: &Session,
    name: &str,
    label: &str,
    hidden: &[(&str, &str)],
    confirm: Option<&str>,
) -> String {
    let mut fields = String::new();
    for (key, value) in hidden {
        let _ = write!(
            fields,
            r#"<input type="hidden" name="{}" value="{}">"#,
            escape(key),
            escape(value)
        );
    }
    let (confirm, class) = match confirm {
        Some(text) => (
            format!(
                r#"<input type="text" name="confirm" required autocomplete="off" aria-label="Type {text} to confirm" placeholder="Type {text} to confirm">"#,
                text = escape(text)
            ),
            r#" class="danger""#,
        ),
        None => (String::new(), ""),
    };
    format!(
        r#"<form class="action" method="post" action="/act"><input type="hidden" name="csrf" value="{csrf}"><input type="hidden" name="action" value="{name}">{fields}<input type="text" name="reason" required maxlength="500" autocomplete="off" aria-label="Reason" placeholder="Reason">{confirm}<button{class}>{label}</button></form>"#,
        csrf = escape(&session.csrf),
        name = escape(name),
        label = escape(label),
    )
}

fn tile(label: &str, path: &str, value: &Value) -> String {
    format!(
        r#"<div class="tile"><span>{label}</span><b data-live="{path}">{}</b></div>"#,
        show(value)
    )
}

/// The diagram of the connections: the subscribers on the left, the
/// server in the middle, each line with the lag of the subscriber.
fn topology(hostname: &str, subscribers: &[Value]) -> String {
    let rows = subscribers.len().max(1);
    let height = 40 + rows * 44;
    let middle = height / 2;
    let mut svg = format!(
        r#"<svg class="topology" viewBox="0 0 640 {height}" role="img" aria-label="Connections of the server"><rect x="420" y="{top}" width="200" height="48" rx="8" fill="none" stroke="currentColor"/><text x="520" y="{label}" text-anchor="middle" fill="currentColor" font-size="14">{host}</text>"#,
        top = middle - 24,
        label = middle + 5,
        host = escape(hostname),
    );
    if subscribers.is_empty() {
        let _ = write!(
            svg,
            r#"<text x="20" y="{}" fill="currentColor" font-size="13">No subscriber is connected.</text>"#,
            middle + 5
        );
    }
    for (index, subscriber) in subscribers.iter().enumerate() {
        let y = 42 + index * 44;
        let name = subscriber["userAgent"]
            .as_str()
            .or_else(|| subscriber["remote"].as_str())
            .unwrap_or("subscriber");
        let name: String = name.chars().take(28).collect();
        let lag = subscriber["lag"].as_i64().unwrap_or(0);
        let colour = if lag > 1000 { "#c0392b" } else { "#2e8b57" };
        let _ = write!(
            svg,
            r#"<rect x="10" y="{top}" width="220" height="32" rx="6" fill="none" stroke="currentColor"/><text x="20" y="{text}" fill="currentColor" font-size="12">{name}</text><line x1="230" y1="{y}" x2="420" y2="{middle}" stroke="{colour}" stroke-width="2"/><text x="250" y="{above}" fill="currentColor" font-size="11">lag {lag}</text>"#,
            top = y - 16,
            text = y + 4,
            above = y - 6,
            name = escape(&name),
        );
    }
    svg.push_str("</svg>");
    svg
}

fn subscriber_rows(session: &Session, subscribers: &[Value]) -> String {
    let mut rows = String::new();
    for subscriber in subscribers {
        let id = subscriber["id"].to_string();
        let cell = |field: &str| {
            format!(
                r#"<td data-subscriber="{id}" data-field="{field}">{}</td>"#,
                show(&subscriber[field])
            )
        };
        let act = if session.is_operator() {
            action(session, "disconnect", "Disconnect", &[("id", &id)], None)
        } else {
            String::new()
        };
        let _ = write!(
            rows,
            "<tr><td>{}</td><td>{}</td>{}{}{}{}{}<td>{}</td><td>{act}</td></tr>",
            show(&subscriber["remote"]),
            show(&subscriber["userAgent"]),
            cell("cursor"),
            cell("lag"),
            cell("frames"),
            cell("bytes"),
            cell("lastFrameAt"),
            show(&subscriber["connectedAt"]),
        );
    }
    rows
}

/// The overview: the state of the server, its connections, and the
/// housekeeping.
pub fn overview(
    session: &Session,
    state: &Value,
    subscribers: &[Value],
    done: Option<&str>,
) -> String {
    let mut body = banner(done);
    body.push_str(r#"<div class="tiles">"#);
    for (label, path) in [
        ("Accounts", "accounts"),
        ("Firehose head", "sequencer.head"),
        ("Subscribers", "subscribers"),
        ("Commits in flight", "commitsInFlight"),
        ("Notices waiting", "noticeOutboxDepth"),
        ("Audit entries", "audit.entries"),
        ("Entries not signed", "audit.unsigned"),
        ("Checkpoint age (s)", "audit.checkpointAgeSeconds"),
    ] {
        let value = path.split('.').fold(state, |at, key| &at[key]);
        body.push_str(&tile(label, path, value));
    }
    let _ = write!(
        body,
        r#"</div><p class="dim">Version {} · profile {} · server time <span data-live="time">{}</span></p><h2>Connections</h2>{}"#,
        show(&state["version"]),
        show(&state["profile"]),
        show(&state["time"]),
        topology(state["hostname"].as_str().unwrap_or("PDS"), subscribers),
    );
    if !subscribers.is_empty() {
        let _ = write!(
            body,
            "<table><thead><tr><th>Address</th><th>User agent</th><th>Cursor</th><th>Lag</th><th>Frames</th><th>Bytes</th><th>Last frame</th><th>Connected</th><th></th></tr></thead><tbody>{}</tbody></table>",
            subscriber_rows(session, subscribers)
        );
    }
    if session.is_operator() {
        let _ = write!(
            body,
            "<h2>Housekeeping</h2>{}{}",
            action(session, "checkpoint", "Sign a checkpoint now", &[], None),
            action(
                session,
                "purge",
                "Remove expired sessions and requests",
                &[],
                None
            ),
        );
    }
    let attributes = format!(r#" data-subscribers="{}""#, subscribers.len());
    layout(session, "Overview", &attributes, &body)
}

/// A page of the accounts.
pub fn accounts(session: &Session, page: &Value) -> String {
    let mut rows = String::new();
    for account in page["accounts"].as_array().map_or(&[][..], Vec::as_slice) {
        let did = account["did"].as_str().unwrap_or_default();
        let _ = write!(
            rows,
            r#"<tr><td><a href="/accounts/{}">{}</a></td><td><code>{}</code></td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>"#,
            encode(did),
            show(&account["handle"]),
            escape(did),
            show(&account["status"]),
            show(&account["createdAt"]),
            show(&account["rev"]),
        );
    }
    let mut body = format!(
        "<table><thead><tr><th>Handle</th><th>DID</th><th>Status</th><th>Created</th><th>Revision</th></tr></thead><tbody>{rows}</tbody></table>"
    );
    if let Some(cursor) = page["cursor"].as_str() {
        let _ = write!(
            body,
            r#"<p><a href="/accounts?cursor={}">Next page</a></p>"#,
            encode(cursor)
        );
    }
    layout(session, "Accounts", "", &body)
}

fn sessions_of(card: &Value) -> String {
    let list = |key: &str| card[key].as_array().map_or(&[][..], Vec::as_slice);
    let mut body = String::from("<h2>Sessions</h2>");
    let (legacy, oauth, devices) = (
        list("legacySessions"),
        list("oauthSessions"),
        list("devices"),
    );
    if legacy.is_empty() && oauth.is_empty() && devices.is_empty() {
        body.push_str(r#"<p class="dim">The account has no session.</p>"#);
        return body;
    }
    body.push_str("<table><thead><tr><th>Kind</th><th>Client or device</th><th>Last use</th><th>Ends</th></tr></thead><tbody>");
    for session in legacy {
        let _ = write!(
            body,
            "<tr><td>Password</td><td>{}</td><td>–</td><td>{}</td></tr>",
            show(&session["appPassword"]),
            show(&session["expiresAt"]),
        );
    }
    for session in oauth {
        let _ = write!(
            body,
            "<tr><td>OAuth</td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            show(&session["clientId"]),
            show(&session["updatedAt"]),
            show(&session["expiresAt"]),
        );
    }
    for device in devices {
        let _ = write!(
            body,
            "<tr><td>Device</td><td>{} · {}</td><td>{}</td><td>–</td></tr>",
            show(&device["userAgent"]),
            show(&device["ipAddress"]),
            show(&device["lastSeenAt"]),
        );
    }
    body.push_str("</tbody></table>");
    body
}

/// One account: the repository card, the sessions and the actions.
pub fn account(session: &Session, card: &Value, done: Option<&str>) -> String {
    let did = card["did"].as_str().unwrap_or_default();
    let mut body = banner(done);
    body.push_str("<dl>");
    for (label, value) in [
        ("DID", &card["did"]),
        ("Handle", &card["handle"]),
        ("Email", &card["email"]),
        ("Email confirmed", &card["emailConfirmedAt"]),
        ("Created", &card["createdAt"]),
        ("Status", &card["status"]),
        ("Takedown reference", &card["takedownRef"]),
        ("Role", &card["role"]),
        ("Head", &card["head"]),
        ("Revision", &card["rev"]),
        ("Records", &card["repo"]["records"]),
        ("Blobs", &card["repo"]["blobs"]),
        ("Blocks", &card["repo"]["blocks"]),
    ] {
        let _ = write!(body, "<dt>{label}</dt><dd>{}</dd>", show(value));
    }
    body.push_str("</dl>");
    body.push_str(&sessions_of(card));
    body.push_str("<h2>Actions</h2>");
    let hidden = [("did", did)];
    body.push_str(&action(
        session,
        "revoke-sessions",
        "End every session",
        &hidden,
        None,
    ));
    if card["takedownRef"].is_null() {
        body.push_str(&action(
            session,
            "takedown",
            "Take the account down",
            &hidden,
            Some(did),
        ));
    } else {
        body.push_str(&action(
            session,
            "reinstate",
            "Lift the takedown",
            &hidden,
            None,
        ));
    }
    if session.is_operator() {
        body.push_str(&action(session, "sync", "Send #sync", &hidden, None));
    }
    let title = card["handle"].as_str().unwrap_or(did);
    layout(session, title, "", &body)
}

/// A page of the audit log, the newest entry first.
pub fn audit(session: &Session, page: &Value) -> String {
    let mut rows = String::new();
    for entry in page["entries"].as_array().map_or(&[][..], Vec::as_slice) {
        let signed = if entry["signed"] == true {
            "yes"
        } else {
            "not yet"
        };
        let _ = write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{signed}</td></tr>",
            show(&entry["seq"]),
            show(&entry["time"]),
            show(&entry["actor"]),
            show(&entry["action"]),
            show(&entry["subject"]),
            show(&entry["outcome"]),
            show(&entry["reason"]),
        );
    }
    let mut body = format!(
        "<table><thead><tr><th>No.</th><th>Time</th><th>Actor</th><th>Action</th><th>Subject</th><th>Outcome</th><th>Reason</th><th>In a checkpoint</th></tr></thead><tbody>{rows}</tbody></table>"
    );
    if let Some(before) = page["before"].as_u64() {
        let _ = write!(
            body,
            r#"<p><a href="/audit?before={before}">Older entries</a></p>"#
        );
    }
    layout(session, "Audit log", "", &body)
}

/// The email domains that may not sign up.
pub fn domains(session: &Session, blocked: &Value, done: Option<&str>) -> String {
    let mut body = banner(done);
    let mut rows = String::new();
    for block in blocked["domains"].as_array().map_or(&[][..], Vec::as_slice) {
        let domain = block["domain"].as_str().unwrap_or_default();
        let lift = if session.is_operator() {
            action(
                session,
                "unblock-domain",
                "Lift",
                &[("domain", domain)],
                None,
            )
        } else {
            String::new()
        };
        let _ = write!(
            rows,
            "<tr><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{lift}</td></tr>",
            escape(domain),
            show(&block["reason"]),
            show(&block["createdBy"]),
            show(&block["createdAt"]),
        );
    }
    if rows.is_empty() {
        body.push_str(r#"<p class="dim">No email domain is blocked.</p>"#);
    } else {
        let _ = write!(
            body,
            "<table><thead><tr><th>Domain</th><th>Reason</th><th>Blocked by</th><th>When</th><th></th></tr></thead><tbody>{rows}</tbody></table>"
        );
    }
    if session.is_operator() {
        let _ = write!(
            body,
            r#"<h2>Block a domain</h2><form class="action" method="post" action="/act"><input type="hidden" name="csrf" value="{}"><input type="hidden" name="action" value="block-domain"><input type="text" name="domain" required aria-label="Domain" placeholder="example.com"><input type="text" name="reason" required maxlength="500" aria-label="Reason" placeholder="Reason"><button>Block</button></form>"#,
            escape(&session.csrf)
        );
    }
    layout(session, "Email domains", "", &body)
}

/// The accounts with a role. Only the admin credential grants one.
pub fn roles(session: &Session, list: &Value) -> String {
    let mut rows = String::new();
    for role in list["roles"].as_array().map_or(&[][..], Vec::as_slice) {
        let did = role["did"].as_str().unwrap_or_default();
        let _ = write!(
            rows,
            r#"<tr><td><a href="/accounts/{}"><code>{}</code></a></td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>"#,
            encode(did),
            escape(did),
            show(&role["role"]),
            show(&role["grantedBy"]),
            show(&role["grantedAt"]),
        );
    }
    let body = format!(
        r#"<table><thead><tr><th>Account</th><th>Role</th><th>Granted by</th><th>When</th></tr></thead><tbody>{rows}</tbody></table><p class="dim">A role is granted and taken away with the admin credential of the server, on its ops listener.</p>"#
    );
    layout(session, "Roles", "", &body)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::client::{Tokens, new_key};
    use crate::session::Sessions;

    fn session(role: &str) -> std::sync::Arc<Session> {
        let tokens = Tokens {
            access: "a".to_owned(),
            refresh: None,
            did: "did:plc:op".to_owned(),
        };
        Sessions::default().start(new_key(), tokens, role.to_owned())
    }

    #[test]
    fn a_value_of_the_server_is_escaped() {
        let session = session("operator");
        let subscribers = [json!({
            "id": 1, "userAgent": "<script>alert(1)</script>", "remote": "10.0.0.1",
            "cursor": 5, "lag": 2000, "frames": 1, "bytes": 2, "lastFrameAt": null,
            "connectedAt": "2026-10-13T00:00:00.000Z",
        })];
        let state = json!({"hostname": "pds\"><b>", "accounts": 3});
        let page = overview(&session, &state, &subscribers, Some("disconnect"));
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!page.contains("pds\"><b>"));
        assert!(page.contains("The subscriber is disconnected."));
        assert!(page.contains(r##"stroke="#c0392b""##));
        assert!(page.contains(r#"data-subscribers="1""#));
        // A banner is one of the known texts.
        assert!(!overview(&session, &state, &[], Some("<b>")).contains("banner"));
    }

    #[test]
    fn a_moderator_sees_the_actions_of_a_moderator() {
        let card = json!({"did": "did:plc:x", "handle": "x.test", "takedownRef": null});
        let moderator = account(&session("moderator"), &card, None);
        assert!(moderator.contains(r#"value="revoke-sessions""#));
        assert!(moderator.contains(r#"value="takedown""#));
        assert!(moderator.contains("Type did:plc:x to confirm"));
        assert!(!moderator.contains(r#"value="sync""#));
        let operator = account(&session("operator"), &card, None);
        assert!(operator.contains(r#"value="sync""#));
        let down = json!({"did": "did:plc:x", "takedownRef": "ops-1"});
        let page = account(&session("moderator"), &down, None);
        assert!(page.contains(r#"value="reinstate""#));
        assert!(!page.contains(r#"value="takedown""#));
    }
}
