//! The pages of the console. Each is a component that renders the whole
//! document for an operator; every value that comes from the PDS is
//! escaped by Topcoat.

// `#[component]` makes a struct of the parameters of each public
// component; the parameters are documented at the function.
#![allow(missing_docs)]

use std::sync::Arc;

use serde_json::Value;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::StatusCode,
    runtime::connected,
    view::{View, component, emit, live, view},
};

use crate::{
    Console,
    live::{Feed, Frame, Live, Update},
    ui::{self, Dialog, Section, Tone, Who, icons, show, status_tone, text},
};

/// The actions the console knows, with the text of each for the toast
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

/// Percent-encodes a value for a path segment or a query.
pub fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// The path of the page of an account.
pub fn account_path(did: &str) -> String {
    format!("/accounts/{}", encode(did))
}

/// The overview: the state of the server, its connections, and the
/// housekeeping. The numbers follow the stream of the ops listener once
/// the browser connects.
#[component]
pub async fn overview(
    who: Who,
    first: Frame,
    feed: Option<Feed>,
    #[default] done: Option<String>,
) -> Result<impl View> {
    let first = Arc::new(first);
    let host = text(&first.overview["hostname"]).unwrap_or_else(|| "the server".to_owned());
    let operator = who.operator;
    let csrf = who.csrf.clone();
    let numbers_first = Arc::clone(&first);
    let numbers_feed = feed.clone();
    Ok(view! {
        ui::document(
            title: "Overview",
            who: Some(who.clone()),
            current: Some(Section::Overview),
            ui::toast(done: done)
            ui::page_head(title: "Overview", lede: format!("What {host} is doing right now."))
            live_numbers(who: who, first: numbers_first, feed: numbers_feed)
            if operator {
                housekeeping(csrf: csrf.clone())
                live_dialogs(csrf: csrf, first: first, feed: feed)
            }
        )
    })
}

/// The live region of the numbers, the diagram and the subscribers: the
/// frame of the page load, then each frame of the stream once the browser
/// connects, with the change since the frame before.
#[component]
async fn live_numbers(
    cx: &Cx,
    who: Who,
    first: Arc<Frame>,
    feed: Option<Feed>,
) -> Result<impl View> {
    Ok(live! {
        let mut shown = first;
        let mut previous: Option<Arc<Frame>> = None;
        let mut state = Live::Snapshot;
        let mut updates = feed.as_ref().map(Feed::watch);
        loop {
            let token = emit! {
                ui::overview_state(
                    who: who.clone(),
                    frame: Arc::clone(&shown),
                    previous: previous.clone(),
                    live: state.clone(),
                )
            }?;
            if !connected(cx) || matches!(state, Live::Ended(_)) {
                break Ok(token);
            }
            let Some(updates) = updates.as_mut() else {
                break Ok(token);
            };
            if updates.changed().await.is_err() {
                state = Live::Ended("The live view of the console ended.".to_owned());
                continue;
            }
            let update = updates.borrow_and_update().clone();
            match update {
                Update::Waiting => {}
                Update::Frame(frame) => {
                    previous = Some(std::mem::replace(&mut shown, frame));
                    state = Live::Streaming;
                }
                Update::Ended(why) => state = Live::Ended(why),
            }
        }
    })
}

/// The live region of the dialogs that disconnect a subscriber. It emits
/// again only when a subscriber comes or goes, so an open dialog is not
/// replaced under the operator.
#[component]
async fn live_dialogs(
    cx: &Cx,
    csrf: String,
    first: Arc<Frame>,
    feed: Option<Feed>,
) -> Result<impl View> {
    Ok(live! {
        let mut shown = first;
        let mut updates = feed.as_ref().map(Feed::watch);
        loop {
            let token = emit! {
                ui::disconnect_dialogs(csrf: csrf.clone(), subscribers: shown.subscribers.clone())
            }?;
            if !connected(cx) {
                break Ok(token);
            }
            let Some(updates) = updates.as_mut() else {
                break Ok(token);
            };
            loop {
                if updates.changed().await.is_err() {
                    break;
                }
                let update = updates.borrow_and_update().clone();
                match update {
                    Update::Frame(frame) if frame.ids() != shown.ids() => {
                        shown = frame;
                        break;
                    }
                    Update::Ended(_) => break,
                    Update::Frame(_) | Update::Waiting => {}
                }
            }
            if updates.has_changed().is_err() {
                break Ok(token);
            }
        }
    })
}

/// The housekeeping of an operator: a checkpoint of the audit log, and
/// the removal of what expired.
#[component]
async fn housekeeping(csrf: String) -> Result<impl View> {
    Ok(view! {
        <section class="section" data-slot="housekeeping">
            <h2>"Housekeeping"</h2>
            <p class="lede">"Each action asks for a reason, which the server writes into the audit log."</p>
            <div class="group">
                <div class="row">
                    <span class="row-icon">ui::icon(path: icons::KEY)</span>
                    <span class="row-text">
                        <strong>"Sign a checkpoint now"</strong>
                        <small>"The audit log gets a signed checkpoint over the entries so far."</small>
                    </span>
                    ui::action(csrf: csrf.clone(), spec: Dialog {
                        id: "checkpoint".to_owned(),
                        action: "checkpoint",
                        label: "Sign".to_owned(),
                        title: "Sign a checkpoint".to_owned(),
                        lede: "The server signs the audit log up to its last entry.".to_owned(),
                        hidden: Vec::new(),
                        confirm: None,
                        danger: false,
                    })
                </div>
                <div class="row">
                    <span class="row-icon">ui::icon(path: icons::APPS)</span>
                    <span class="row-text">
                        <strong>"Remove expired sessions and requests"</strong>
                        <small>"Sessions past their end and authorization requests nobody finished are removed."</small>
                    </span>
                    ui::action(csrf: csrf, spec: Dialog {
                        id: "purge".to_owned(),
                        action: "purge",
                        label: "Remove".to_owned(),
                        title: "Remove what expired".to_owned(),
                        lede: "Nothing that is still in use is touched.".to_owned(),
                        hidden: Vec::new(),
                        confirm: None,
                        danger: false,
                    })
                </div>
            </div>
        </section>
    })
}

/// A page of accounts: the listing, or the matches of a search.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// The accounts, as `internal.ops.listAccounts` lists them.
    pub accounts: Vec<Value>,
    /// The cursor of the next page of the listing.
    pub cursor: Option<String>,
    /// The search, when the page is one.
    pub query: Option<String>,
    /// A note about the search.
    pub note: Option<String>,
}

/// A row of the account table.
struct AccountRow {
    did: String,
    handle: String,
    status: String,
    created: Option<String>,
    rev: Option<String>,
}

/// The accounts, with the search.
#[component]
pub async fn accounts(who: Who, listing: Listing) -> Result<impl View> {
    let Listing {
        accounts,
        cursor,
        query,
        note,
    } = listing;
    let searching = query.is_some();
    let rows: Vec<AccountRow> = accounts
        .iter()
        .map(|entry| AccountRow {
            did: entry["did"].as_str().unwrap_or_default().to_owned(),
            handle: show(&entry["handle"]),
            status: show(&entry["status"]),
            created: text(&entry["createdAt"]),
            rev: text(&entry["rev"]),
        })
        .collect();
    let next = cursor.map(|cursor| format!("/accounts?cursor={}", encode(&cursor)));
    Ok(view! {
        ui::document(
            title: "Accounts",
            who: Some(who),
            current: Some(Section::Accounts),
            ui::page_head(
                title: "Accounts",
                lede: "Every account the server hosts. Search by handle, DID, or email address.",
                <form class="search" method="get" action="/accounts" role="search" data-slot="search">
                    <input
                        type="search"
                        name="q"
                        value=(query.clone())
                        placeholder="Handle, DID or email"
                        aria-label="Search accounts"
                        autocomplete="off"
                    >
                    <button class="pill" type="submit" data-variant="secondary">"Search"</button>
                </form>
            )
            match note {
                Some(note) => ui::alert(text: note),
                None => "",
            }
            if rows.is_empty() {
                ui::empty(text: if searching { "No account matches." } else { "The server hosts no account." })
            } else {
                ui::table(
                    columns: vec!["Handle", "DID", "Status", "Created", "Revision"],
                    #[key(row.did)]
                    for row in rows {
                        <tr>
                            <td data-cell="text"><a href=(account_path(&row.did))>(row.handle)</a></td>
                            <td>ui::mono(text: row.did, wide: true)</td>
                            <td>ui::chip(text: row.status.clone(), tone: status_tone(&row.status))</td>
                            <td>ui::when(iso: row.created)</td>
                            <td data-cell="number">(row.rev.unwrap_or_else(|| "–".to_owned()))</td>
                        </tr>
                    }
                )
            }
            match next {
                Some(next) if !searching => {
                    <p class="pager" data-slot="pager">
                        <a class="pill" href=(next) data-variant="secondary">"Next page"</a>
                    </p>
                }
                _ => "",
            }
        )
    })
}

/// A session of an account, as a row.
struct SessionRow {
    icon: &'static str,
    kind: &'static str,
    client: String,
    detail: String,
    last: Option<String>,
    ends: Option<String>,
}

fn sessions_of(card: &Value) -> Vec<SessionRow> {
    let list = |key: &str| card[key].as_array().map_or(&[][..], Vec::as_slice);
    let mut rows = Vec::new();
    for session in list("legacySessions") {
        rows.push(SessionRow {
            icon: icons::KEY,
            kind: "Password",
            client: show(&session["appPassword"]),
            detail: "A session with the password, or an app password.".to_owned(),
            last: None,
            ends: text(&session["expiresAt"]),
        });
    }
    for session in list("oauthSessions") {
        rows.push(SessionRow {
            icon: icons::APPS,
            kind: "OAuth",
            client: show(&session["clientId"]),
            detail: format!("Scope {}", show(&session["scope"])),
            last: text(&session["updatedAt"]),
            ends: text(&session["expiresAt"]),
        });
    }
    for device in list("devices") {
        rows.push(SessionRow {
            icon: icons::DEVICE,
            kind: "Device",
            client: show(&device["userAgent"]),
            detail: format!("From {}", show(&device["ipAddress"])),
            last: text(&device["lastSeenAt"]),
            ends: None,
        });
    }
    rows
}

/// One account: the card, the facts, the sessions and the actions.
#[component]
pub async fn account(who: Who, card: Value, #[default] done: Option<String>) -> Result<impl View> {
    let did = card["did"].as_str().unwrap_or_default().to_owned();
    let title = text(&card["handle"]).unwrap_or_else(|| did.clone());
    let initial: String = title
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let status = show(&card["status"]);
    let role = text(&card["role"]);
    let taken_down = !card["takedownRef"].is_null();
    let takedown_ref = text(&card["takedownRef"]);
    let own = who.did == did;
    let operator = who.operator;
    let csrf = who.csrf.clone();
    let tiles: Vec<(&'static str, String)> = vec![
        ("Records", show(&card["repo"]["records"])),
        ("Blobs", show(&card["repo"]["blobs"])),
        ("Blocks", show(&card["repo"]["blocks"])),
        ("Revision", show(&card["rev"])),
    ];
    let email = show(&card["email"]);
    let confirmed = text(&card["emailConfirmedAt"]);
    let created = text(&card["createdAt"]);
    let deactivated = text(&card["deactivatedAt"]);
    let head = text(&card["head"]);
    let sessions = sessions_of(&card);
    Ok(view! {
        ui::document(
            title: title.clone(),
            who: Some(who),
            current: Some(Section::Accounts),
            ui::toast(done: done)
            <section class="hero" data-slot="hero">
                <div class="hero-mark" aria-hidden="true">(initial)</div>
                <div class="hero-text">
                    <p class="eyebrow">"Account"</p>
                    <h1>(title.clone())</h1>
                    ui::mono(text: did.clone(), wide: true)
                    <p class="chips">
                        ui::chip(text: status.clone(), tone: status_tone(&status))
                        match role {
                            Some(role) => ui::chip(text: role, tone: Tone::Accent),
                            None => "",
                        }
                        if own {
                            ui::chip(text: "This is you", tone: Tone::Quiet)
                        }
                    </p>
                </div>
            </section>
            <div class="tiles mt-2.5" data-slot="tiles">
                #[key(stat.0)]
                for stat in tiles {
                    ui::tile(label: stat.0, value: stat.1)
                }
            </div>
            <section class="section" data-slot="facts">
                <h2>"Facts"</h2>
                <div class="facts mt-3.5">
                    <div><span>"Email"</span><strong>(email)</strong></div>
                    <div><span>"Email confirmed"</span><strong>ui::when(iso: confirmed)</strong></div>
                    <div><span>"Created"</span><strong>ui::when(iso: created)</strong></div>
                    <div><span>"Head"</span><strong>
                        match head {
                            Some(head) => ui::mono(text: head),
                            None => "–",
                        }
                    </strong></div>
                    <div><span>"Takedown reference"</span><strong>(takedown_ref.unwrap_or_else(|| "–".to_owned()))</strong></div>
                    <div><span>"Deactivated"</span><strong>ui::when(iso: deactivated)</strong></div>
                </div>
            </section>
            sessions_group(rows: sessions)
            account_actions(csrf: csrf, did: did, title: title, taken_down: taken_down, operator: operator)
        )
    })
}

/// The sessions of an account as grouped rows.
#[component]
async fn sessions_group(rows: Vec<SessionRow>) -> Result<impl View> {
    Ok(view! {
        <section class="section" data-slot="sessions">
            <h2>"Sessions"</h2>
            <p class="lede">"Where the account is signed in."</p>
            if rows.is_empty() {
                ui::empty(text: "The account has no session.")
            } else {
                <div class="group">
                    #[key((index, row.kind))]
                    for (index, row) in rows.into_iter().enumerate() {
                        <div class="row">
                            <span class="row-icon">ui::icon(path: row.icon)</span>
                            <span class="row-text">
                                <small>(row.kind)</small>
                                <strong>(row.client)</strong>
                                <small>(row.detail)</small>
                            </span>
                            <span class="row-text fine">
                                "Last use " ui::when(iso: row.last)
                                <br>
                                "Ends " ui::when(iso: row.ends)
                            </span>
                        </div>
                    }
                </div>
            }
        </section>
    })
}

/// The actions on an account: a moderator ends its sessions and changes
/// its status; an operator also sends `#sync`.
#[component]
async fn account_actions(
    csrf: String,
    did: String,
    title: String,
    taken_down: bool,
    operator: bool,
) -> Result<impl View> {
    let hidden = vec![("did", did.clone())];
    let revoke_lede = format!("Every session of {title} ends. The account can sign in again.");
    let reinstate_lede = format!("{title} is served again, with its repository and its blobs.");
    let takedown_title = format!("Take {title} down");
    let sync_lede = format!("Every subscriber sees a #sync for {title}.");
    Ok(view! {
        <section class="section" data-slot="actions">
            <h2>"Actions"</h2>
            <p class="lede">"Each action asks for a reason, which the server writes into the audit log."</p>
            <div class="group">
                <div class="row">
                    <span class="row-text">
                        <strong>"End every session"</strong>
                        <small>"Every password session, OAuth session and device of the account is signed out."</small>
                    </span>
                    ui::action(csrf: csrf.clone(), spec: Dialog {
                        id: "revoke-sessions".to_owned(),
                        action: "revoke-sessions",
                        label: "End sessions".to_owned(),
                        title: "End every session".to_owned(),
                        lede: revoke_lede,
                        hidden: hidden.clone(),
                        confirm: None,
                        danger: false,
                    })
                </div>
                if taken_down {
                    <div class="row">
                        <span class="row-text">
                            <strong>"Lift the takedown"</strong>
                            <small>"The account is served again."</small>
                        </span>
                        ui::action(csrf: csrf.clone(), spec: Dialog {
                            id: "reinstate".to_owned(),
                            action: "reinstate",
                            label: "Lift".to_owned(),
                            title: "Lift the takedown".to_owned(),
                            lede: reinstate_lede,
                            hidden: hidden.clone(),
                            confirm: None,
                            danger: false,
                        })
                    </div>
                } else {
                    <div class="row">
                        <span class="row-text">
                            <strong>"Take the account down"</strong>
                            <small>"The account, its repository and its blobs are no longer served. Type the DID to confirm."</small>
                        </span>
                        ui::action(csrf: csrf.clone(), spec: Dialog {
                            id: "takedown".to_owned(),
                            action: "takedown",
                            label: "Take down".to_owned(),
                            title: takedown_title,
                            lede: "The account is no longer served and cannot sign in. The takedown can be lifted here later.".to_owned(),
                            hidden: hidden.clone(),
                            confirm: Some(did),
                            danger: true,
                        })
                    </div>
                }
                if operator {
                    <div class="row">
                        <span class="row-text">
                            <strong>"Send #sync"</strong>
                            <small>"The firehose carries a #sync event for the repository, so a relay fetches it again."</small>
                        </span>
                        ui::action(csrf: csrf, spec: Dialog {
                            id: "sync".to_owned(),
                            action: "sync",
                            label: "Send".to_owned(),
                            title: "Send #sync".to_owned(),
                            lede: sync_lede,
                            hidden,
                            confirm: None,
                            danger: false,
                        })
                    </div>
                }
            </div>
        </section>
    })
}

/// A row of the audit table.
struct AuditRow {
    seq: u64,
    time: Option<String>,
    actor: String,
    action: String,
    subject: String,
    outcome: String,
    reason: String,
    signed: bool,
}

/// A page of the audit log, the newest entry first.
#[component]
pub async fn audit(who: Who, page: Value) -> Result<impl View> {
    let rows: Vec<AuditRow> = page["entries"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|entry| AuditRow {
            seq: entry["seq"].as_u64().unwrap_or_default(),
            time: text(&entry["time"]),
            actor: show(&entry["actor"]),
            action: show(&entry["action"]),
            subject: show(&entry["subject"]),
            outcome: show(&entry["outcome"]),
            reason: show(&entry["reason"]),
            signed: entry["signed"] == true,
        })
        .collect();
    let older = page["before"]
        .as_u64()
        .map(|before| format!("/audit?before={before}"));
    Ok(view! {
        ui::document(
            title: "Audit log",
            who: Some(who),
            current: Some(Section::Audit),
            ui::page_head(title: "Audit log", lede: "Every action on the server, newest first. A checkpoint signs the entries before it.")
            if rows.is_empty() {
                ui::empty(text: "The audit log has no entry.")
            } else {
                ui::table(
                    columns: vec!["No.", "When", "Actor", "Action", "Subject", "Outcome", "Reason", "Checkpoint"],
                    #[key(row.seq)]
                    for row in rows {
                        <tr>
                            <td data-cell="number">(row.seq)</td>
                            <td>ui::when(iso: row.time)</td>
                            <td>ui::mono(text: row.actor)</td>
                            <td data-cell="text">(row.action)</td>
                            <td>ui::mono(text: row.subject)</td>
                            <td>ui::chip(text: row.outcome.clone(), tone: if row.outcome == "ok" { Tone::Good } else { Tone::Warn })</td>
                            <td data-cell="text" title=(row.reason.clone())>(row.reason)</td>
                            <td>
                                if row.signed {
                                    ui::chip(text: "signed", tone: Tone::Good)
                                } else {
                                    ui::chip(text: "not yet", tone: Tone::Neutral)
                                }
                            </td>
                        </tr>
                    }
                )
            }
            match older {
                Some(older) => {
                    <p class="pager" data-slot="pager">
                        <a class="pill" href=(older) data-variant="secondary">"Older entries"</a>
                    </p>
                }
                None => "",
            }
        )
    })
}

/// The email domains that may not sign up.
#[component]
pub async fn domains(
    who: Who,
    blocked: Value,
    #[default] done: Option<String>,
) -> Result<impl View> {
    let operator = who.operator;
    let csrf = who.csrf.clone();
    let rows: Vec<(usize, String, String, String, Option<String>)> = blocked["domains"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .enumerate()
        .map(|(index, block)| {
            (
                index,
                block["domain"].as_str().unwrap_or_default().to_owned(),
                show(&block["reason"]),
                show(&block["createdBy"]),
                text(&block["createdAt"]),
            )
        })
        .collect();
    Ok(view! {
        ui::document(
            title: "Email domains",
            who: Some(who),
            current: Some(Section::Domains),
            ui::toast(done: done)
            ui::page_head(title: "Email domains", lede: "An address on a blocked domain, or a domain below it, cannot sign up.")
            if rows.is_empty() {
                ui::empty(text: "No email domain is blocked.")
            } else {
                <div class="group" data-slot="blocked">
                    #[key(row.0)]
                    for row in rows {
                        <div class="row">
                            <span class="row-text">
                                <strong><code>(row.1.clone())</code></strong>
                                <small>(row.2)</small>
                                <small>"Blocked by " (row.3) ", " ui::when(iso: row.4)</small>
                            </span>
                            if operator {
                                ui::action(csrf: csrf.clone(), spec: Dialog {
                                    id: format!("unblock-{}", row.0),
                                    action: "unblock-domain",
                                    label: "Lift".to_owned(),
                                    title: format!("Lift the block of {}", row.1),
                                    lede: "Addresses on the domain can sign up again.".to_owned(),
                                    hidden: vec![("domain", row.1)],
                                    confirm: None,
                                    danger: false,
                                })
                            }
                        </div>
                    }
                </div>
            }
            if operator {
                <section class="section" data-slot="block">
                    <h2>"Block a domain"</h2>
                    <p class="lede">"The domain and every domain below it."</p>
                    <form class="inline-form" method="post" action="/act">
                        <input type="hidden" name="csrf" value=(csrf.clone())>
                        <input type="hidden" name="action" value="block-domain">
                        <label class="field">
                            <span>"Domain"</span>
                            <input type="text" name="domain" required="" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="example.com">
                        </label>
                        <label class="field">
                            <span>"Reason"</span>
                            <input type="text" name="reason" required="" maxlength="500" autocomplete="off" placeholder="One line for the audit log">
                        </label>
                        <button class="pill" type="submit" data-variant="primary">
                            "Block"
                            <i aria-hidden="true">ui::icon(path: icons::ARROW)</i>
                        </button>
                    </form>
                </section>
            }
        )
    })
}

/// The accounts with a role. Only the admin credential grants one.
#[component]
pub async fn roles(who: Who, list: Value) -> Result<impl View> {
    let me = who.did.clone();
    let rows: Vec<(String, String, String, Option<String>)> = list["roles"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|role| {
            (
                role["did"].as_str().unwrap_or_default().to_owned(),
                show(&role["role"]),
                show(&role["grantedBy"]),
                text(&role["grantedAt"]),
            )
        })
        .collect();
    Ok(view! {
        ui::document(
            title: "Roles",
            who: Some(who),
            current: Some(Section::Roles),
            ui::page_head(title: "Roles", lede: "Who may use this console. A moderator reads and changes the status of accounts; an operator takes every action.")
            if rows.is_empty() {
                ui::empty(text: "No account holds a role.")
            } else {
                ui::table(
                    columns: vec!["Account", "Role", "Granted by", "When"],
                    #[key(row.0)]
                    for row in rows {
                        <tr>
                            <td>ui::mono(text: row.0.clone(), href: Some(account_path(&row.0)), wide: true)</td>
                            <td>ui::chip(text: row.1, tone: if row.0 == me { Tone::Accent } else { Tone::Neutral })</td>
                            <td>ui::mono(text: row.2)</td>
                            <td>ui::when(iso: row.3)</td>
                        </tr>
                    }
                )
            }
            <p class="fine mt-3.5">"A role is granted and taken away with the admin credential of the server, on its ops listener."</p>
        )
    })
}

/// The page that offers the sign-in.
#[component]
pub async fn sign_in(cx: &Cx) -> Result<impl View> {
    let pds = app_context::<Console>(cx).config().pds_public_url.clone();
    Ok(view! {
        ui::document(
            title: "Sign in",
            who: None,
            current: None,
            <div class="panel" data-slot="panel">
                <p class="brand"><img src=(pds_oauth::theme::LINKJAR_MARK_PATH) alt="" width="32" height="32">"PDS console"</p>
                <h1>"Sign in to the console."</h1>
                <p class="lede">
                    "The console signs you in on the pages of " <code>(pds)</code>
                    ". Your account needs an operator role on that server."
                </p>
                <a class="pill" href="/login" data-variant="primary" data-size="large">
                    "Sign in with your account"
                    <i aria-hidden="true">ui::icon(path: icons::ARROW)</i>
                </a>
            </div>
        )
    })
}

/// A page that reports a failure, with its status.
#[component]
pub async fn failure(
    who: Option<Who>,
    status: StatusCode,
    #[into] message: String,
) -> Result<impl View> {
    let signed_in = who.is_some();
    Ok(view! {
        (status)
        ui::document(
            title: if signed_in { "The action did not happen" } else { "The console could not continue" },
            who: who,
            current: None,
            if signed_in {
                ui::page_head(title: "The action did not happen", lede: "")
                ui::alert(text: message)
                <p><a class="pill" href="/" data-variant="secondary">"Back to the overview"</a></p>
            } else {
                <div class="panel" data-slot="panel">
                    <p class="brand"><img src=(pds_oauth::theme::LINKJAR_MARK_PATH) alt="" width="32" height="32">"PDS console"</p>
                    <h1>"The console could not continue."</h1>
                    ui::alert(text: message)
                    <a class="pill" href="/sign-in" data-variant="secondary">"Back to the sign-in"</a>
                </div>
            }
        )
    })
}
