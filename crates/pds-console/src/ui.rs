//! The components of the console, written after the anatomy of Base UI.
//!
//! Every part is an element with a `data-slot` name, and its state is a
//! data attribute the stylesheet reads. Topcoat escapes every value that
//! comes from the PDS.

// `#[component]` makes a struct of the parameters of each public
// component; the parameters are documented at the function.
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pds_oauth::theme::{FONT_PATH, LINKJAR_MARK_PATH};
use serde_json::Value;
use topcoat::{
    Result,
    context::{Cx, app_context},
    runtime::Event,
    view::{Child, View, component, view},
};

use crate::{
    Console,
    live::{Frame, Live},
    pages::ACTIONS,
    session::Session,
};

/// The path of the tokens of the LinkJar journey and the typeface, which
/// the console shares with the pages of the PDS (`pds_oauth::theme`).
pub const THEME_PATH: &str = "/theme.css";

/// The operator a page is rendered for.
#[derive(Debug, Clone)]
pub struct Who {
    /// The account.
    pub did: String,
    /// The handle of the account, when the console could read it.
    pub handle: Option<String>,
    /// `operator` or `moderator`.
    pub role: String,
    /// The token of the forms.
    pub csrf: String,
    /// Whether the role may take the actions of an operator.
    pub operator: bool,
}

impl Who {
    /// The operator of a session.
    pub fn of(session: &Session) -> Self {
        Self {
            did: session.did.clone(),
            handle: session.handle.clone(),
            role: session.role.clone(),
            csrf: session.csrf.clone(),
            operator: session.is_operator(),
        }
    }

    /// The name to show: the handle, or the DID.
    pub fn display(&self) -> &str {
        self.handle.as_deref().unwrap_or(&self.did)
    }
}

/// The sections of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The state of the server.
    Overview,
    /// The accounts.
    Accounts,
    /// The audit log.
    Audit,
    /// The blocked email domains.
    Domains,
    /// The operator roles.
    Roles,
}

impl Section {
    /// Every section, in the order of the sidebar.
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Accounts,
        Self::Audit,
        Self::Domains,
        Self::Roles,
    ];

    /// The label in the sidebar.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Accounts => "Accounts",
            Self::Audit => "Audit log",
            Self::Domains => "Email domains",
            Self::Roles => "Roles",
        }
    }

    /// The path of the page.
    pub const fn href(self) -> &'static str {
        match self {
            Self::Overview => "/",
            Self::Accounts => "/accounts",
            Self::Audit => "/audit",
            Self::Domains => "/domains",
            Self::Roles => "/roles",
        }
    }

    /// The path of the icon.
    const fn icon(self) -> &'static str {
        match self {
            Self::Overview => "M4 12.5 12 6l8 6.5V19a1 1 0 0 1-1 1h-4v-5h-6v5H5a1 1 0 0 1-1-1z",
            Self::Accounts => {
                "M12 12a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Zm-7 7.5c1.2-3 3.8-4.5 7-4.5s5.8 1.5 7 4.5"
            }
            Self::Audit => "M6 4h9l3 3v13H6zM9 12h6M9 16h6M15 4v3h3",
            Self::Domains => "M4 6h16v12H4zM4 9l8 5 8-5",
            Self::Roles => "M12 3l7 3v5c0 4.5-3 8.3-7 9.5C8 19.3 5 15.5 5 11V6z",
        }
    }
}

/// The paths of the small icons.
pub mod icons {
    /// An arrow to the right.
    pub const ARROW: &str = "M5 12h14m-6-6 6 6-6 6";
    /// A tick.
    pub const CHECK: &str = "m5 12 5 5 9-10";
    /// Two sheets.
    pub const COPY: &str = "M9 9h10v11H9zM5 15V4h10";
    /// A key.
    pub const KEY: &str = "M8 15a4 4 0 1 0 0-8 4 4 0 0 0 0 8Zm3-3 8.5-8.5M16 7l2.5 2.5";
    /// Four tiles.
    pub const APPS: &str = "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z";
    /// A laptop.
    pub const DEVICE: &str = "M5 5h14v10H5zM3 19h18";
    /// A warning sign.
    pub const WARN: &str = "M12 4 3 20h18zM12 10v4m0 3v.5";
}

/// The tone of a chip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    /// A fact.
    #[default]
    Neutral,
    /// A fact with no dot.
    Quiet,
    /// Healthy, done, in place.
    Good,
    /// Behind, refused, taken down.
    Warn,
    /// The operator's own account.
    Accent,
}

impl Tone {
    /// The tone as a data attribute.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Quiet => "quiet",
            Self::Good => "good",
            Self::Warn => "warn",
            Self::Accent => "accent",
        }
    }
}

/// A value of the PDS as text: a string as it is, `null` as a dash.
pub fn show(value: &Value) -> String {
    match value {
        Value::Null => "–".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// A string of the PDS, when it is one.
pub fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// The tone of a hosting status.
pub fn status_tone(status: &str) -> Tone {
    match status {
        "active" => Tone::Good,
        "takendown" | "suspended" => Tone::Warn,
        _ => Tone::Neutral,
    }
}

/// The lag of a subscriber as a chip.
pub fn lag_tone(lag: i64) -> Tone {
    if lag > 1000 { Tone::Warn } else { Tone::Good }
}

/// The time at `iso` relative to `now`: `just now`, `3 min ago`, `in 2 h`,
/// or the date when it is more than a month away.
pub fn relative(iso: &str, now: DateTime<Utc>) -> Option<String> {
    let then = DateTime::parse_from_rfc3339(iso).ok()?.with_timezone(&Utc);
    let seconds = (now - then).num_seconds();
    let (past, span) = (seconds >= 0, seconds.abs());
    let text = if span < 5 {
        return Some("just now".to_owned());
    } else if span < 60 {
        format!("{span} s")
    } else if span < 3600 {
        format!("{} min", span / 60)
    } else if span < 86_400 {
        format!("{} h", span / 3600)
    } else if span < 30 * 86_400 {
        format!("{} d", span / 86_400)
    } else {
        return Some(then.format("%-d %b %Y").to_string());
    };
    Some(if past {
        format!("{text} ago")
    } else {
        format!("in {text}")
    })
}

/// The DID of an account as a regular expression for a `pattern`
/// attribute: the characters of the syntax are escaped, nothing else,
/// which the `v` flag of the browser requires.
pub fn pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        if "^$\\.*+?()[]{}|/".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

/// A small line icon.
#[component]
pub async fn icon(path: &'static str) -> Result<impl View> {
    Ok(view! {
        <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d=(path)></path>
        </svg>
    })
}

/// The document: the head with the stylesheet, the font and the browser
/// runtime, and the body with the sidebar when an operator is signed in.
#[component]
pub async fn document(
    cx: &Cx,
    #[into] title: String,
    who: Option<Who>,
    current: Option<Section>,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    let host = app_context::<Console>(cx).hostname().to_owned();
    let page_title = format!("{title} · {host}");
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <title>(page_title)</title>
                <link rel="preload" href=(FONT_PATH) as="font" type="font/woff2" crossorigin="">
                <link rel="stylesheet" href=(THEME_PATH)>
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                topcoat::runtime::script()
            </head>
            <body>
                match who {
                    Some(who) => {
                        <div class="shell" data-slot="shell">
                            sidebar(who: who.clone(), current: current, host: host.clone())
                            <main class="content" data-slot="content">
                                (child)
                                mobile_foot(who: who)
                            </main>
                        </div>
                    }
                    None => <main class="plain" data-slot="plain">(child)</main>,
                }
            </body>
        </html>
    })
}

/// The sidebar: the mark and the server, the sections, and the operator
/// with the sign-out.
#[component]
async fn sidebar(who: Who, current: Option<Section>, host: String) -> Result<impl View> {
    Ok(view! {
        <nav class="side" aria-label="Console sections" data-slot="sidebar">
            <a class="brand" href="/" data-slot="brand">
                <img src=(LINKJAR_MARK_PATH) alt="" width="30" height="30">
                <span>"PDS console"<small>(host)</small></span>
            </a>
            <div class="side-list" data-slot="side-list">
                #[key(section.label())]
                for section in Section::ALL {
                    let active = current == Some(section);
                    <a href=(section.href()) aria-current=(active.then_some("page")) data-slot="side-item">
                        icon(path: section.icon())
                        <span>(section.label())</span>
                    </a>
                }
            </div>
            <div class="side-foot" data-slot="operator">
                <div>
                    <strong title=(who.did.clone())>(who.display().to_owned())</strong>
                    <small>(who.did.clone())</small>
                </div>
                <form method="post" action="/sign-out">
                    <input type="hidden" name="csrf" value=(who.csrf.clone())>
                    chip(text: who.role.clone(), tone: Tone::Accent)
                    <button class="row-action" type="submit">"Sign out"</button>
                </form>
            </div>
        </nav>
    })
}

/// The operator and the sign-out under the content, for a phone.
#[component]
async fn mobile_foot(who: Who) -> Result<impl View> {
    Ok(view! {
        <form class="mobile-foot" method="post" action="/sign-out" data-slot="mobile-foot">
            <input type="hidden" name="csrf" value=(who.csrf.clone())>
            <span class="min-w-0">
                <strong title=(who.did.clone())>(who.display().to_owned())</strong>
                chip(text: who.role.clone(), tone: Tone::Accent)
            </span>
            <button class="row-action" type="submit">"Sign out"</button>
        </form>
    })
}

/// The title of a page, with its lede and a toolbar beside.
#[component]
pub async fn page_head(
    #[into] title: String,
    #[into] lede: String,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <header class="page-head" data-slot="page-head">
            <div>
                <h1>(title)</h1>
                <p class="lede">(lede)</p>
            </div>
            <div class="toolbar" data-slot="toolbar">(child)</div>
        </header>
    })
}

/// A stat tile: a label, a value with its unit, and the change since the
/// frame before.
#[component]
pub async fn tile(
    #[into] label: String,
    #[into] value: String,
    #[default] unit: Option<&'static str>,
    #[default] delta: Option<i64>,
) -> Result<impl View> {
    let delta = delta.filter(|delta| *delta != 0);
    Ok(view! {
        <div class="tile" data-slot="tile">
            <span data-slot="tile-label">(label)</span>
            <strong data-slot="tile-value">
                (value)
                match unit {
                    Some(unit) => <small data-slot="tile-unit">(unit)</small>,
                    None => "",
                }
                match delta {
                    Some(delta) => {
                        <em data-slot="tile-delta" data-sign=(if delta > 0 { "up" } else { "down" })>
                            (format!("{delta:+}"))
                        </em>
                    }
                    None => "",
                }
            </strong>
        </div>
    })
}

/// A chip: a short fact with a tone.
#[component]
pub async fn chip(#[into] text: String, #[default] tone: Tone) -> Result<impl View> {
    Ok(view! {
        <span class="chip" data-slot="chip" data-tone=(tone.name())>(text)</span>
    })
}

/// An identifier in monospace: one line that is cut, not wrapped, the
/// whole value on hover, and a button that copies it.
#[component]
pub async fn mono(
    #[into] text: String,
    #[default] href: Option<String>,
    #[default] wide: bool,
) -> Result<impl View> {
    Ok(view! {
        <span class="mono" data-slot="mono" data-size=(wide.then_some("wide")) title=(text.clone())>
            match href {
                Some(href) => <a href=(href)><code>(text.clone())</code></a>,
                None => <code>(text.clone())</code>,
            }
            <button
                class="copy"
                type="button"
                aria-label="Copy"
                value=(text.clone())
                @click=$(|e: Event| {
                    let _copied = e.current_target.value;
                    raw!("navigator.clipboard.writeText(${_copied})")
                })
            >
                icon(path: icons::COPY)
            </button>
        </span>
    })
}

/// A time of the PDS, relative to now, the exact time on hover.
#[component]
pub async fn when(#[default] iso: Option<String>) -> Result<impl View> {
    let shown = iso.as_deref().and_then(|iso| relative(iso, Utc::now()));
    Ok(view! {
        match (iso, shown) {
            (Some(iso), Some(shown)) => <time datetime=(iso.clone()) title=(iso)>(shown)</time>,
            (Some(iso), None) => (iso),
            (None, _) => "–",
        }
    })
}

/// A data table in a card. The rows are the children.
#[component]
pub async fn table(columns: Vec<&'static str>, #[default] child: Child<'_>) -> Result<impl View> {
    Ok(view! {
        <div class="table-wrap" data-slot="table">
            <table>
                <thead>
                    <tr>
                        for column in columns {
                            <th scope="col">(column)</th>
                        }
                    </tr>
                </thead>
                <tbody>(child)</tbody>
            </table>
        </div>
    })
}

/// The empty state of a list.
#[component]
pub async fn empty(#[into] text: String) -> Result<impl View> {
    Ok(view! { <p class="empty" data-slot="empty">(text)</p> })
}

/// An inline alert.
#[component]
pub async fn alert(#[into] text: String) -> Result<impl View> {
    Ok(view! {
        <div class="alert" role="alert" data-slot="alert" data-tone="warn">
            icon(path: icons::WARN)
            <span>(text)</span>
        </div>
    })
}

/// The toast after an action: one of the known texts, or nothing.
#[component]
pub async fn toast(#[default] done: Option<String>) -> Result<impl View> {
    let text = done.and_then(|done| {
        ACTIONS
            .iter()
            .find(|(name, _)| *name == done)
            .map(|(_, text)| *text)
    });
    Ok(view! {
        match text {
            Some(text) => {
                <p class="toast" role="status" data-slot="toast">
                    icon(path: icons::CHECK)
                    <span>(text)</span>
                </p>
            }
            None => "",
        }
    })
}

/// The button that opens a dialog.
#[component]
pub async fn trigger(
    #[into] id: String,
    #[into] label: String,
    #[default] danger: bool,
) -> Result<impl View> {
    Ok(view! {
        <button
            class="row-action"
            type="button"
            command="show-modal"
            commandfor=(id)
            data-slot="trigger"
            data-tone=(danger.then_some("danger"))
        >
            (label)
        </button>
    })
}

/// What a dialog asks: an action of the ops listener, with its reason and,
/// for a destructive one, the DID typed again.
#[derive(Debug, Clone)]
pub struct Dialog {
    /// The id of the dialog element, which its trigger names.
    pub id: String,
    /// The action, one of [`ACTIONS`].
    pub action: &'static str,
    /// The text of the trigger and of the submit button.
    pub label: String,
    /// The title of the dialog.
    pub title: String,
    /// What the action does.
    pub lede: String,
    /// The hidden fields of the form.
    pub hidden: Vec<(&'static str, String)>,
    /// The text to type before a destructive action.
    pub confirm: Option<String>,
    /// Whether the action is destructive.
    pub danger: bool,
}

/// A dialog that takes an action: a sheet for an ordinary one, a modal
/// with a typed confirmation for a destructive one.
#[component]
pub async fn action_dialog(csrf: String, spec: Dialog) -> Result<impl View> {
    let Dialog {
        id,
        action: action_name,
        label,
        title,
        lede,
        hidden,
        confirm,
        danger,
    } = spec;
    let title_id = format!("{id}-title");
    Ok(view! {
        <dialog
            id=(id.clone())
            class="dialog"
            data-slot="dialog"
            data-kind=(if danger { "modal" } else { "sheet" })
            data-tone=(if danger { "danger" } else { "neutral" })
            closedby="any"
            aria-labelledby=(title_id.clone())
        >
            <form method="post" action="/act" data-slot="dialog-form">
                <input type="hidden" name="csrf" value=(csrf)>
                <input type="hidden" name="action" value=(action_name)>
                for (name, value) in hidden {
                    <input type="hidden" name=(name) value=(value)>
                }
                <h2 id=(title_id) data-slot="dialog-title">(title)</h2>
                <p data-slot="dialog-lede">(lede)</p>
                <label class="field" data-slot="field">
                    <span>"Reason"</span>
                    <input
                        type="text"
                        name="reason"
                        required=""
                        maxlength="500"
                        autocomplete="off"
                        placeholder="One line for the audit log"
                    >
                </label>
                match confirm {
                    Some(text) => {
                        <label class="field" data-slot="field">
                            <span>"Type " <code>(text.clone())</code> " to confirm"</span>
                            <input
                                type="text"
                                name="confirm"
                                required=""
                                autocomplete="off"
                                autocapitalize="off"
                                spellcheck="false"
                                pattern=(pattern(&text))
                                placeholder=(text)
                            >
                        </label>
                    }
                    None => "",
                }
                <div data-slot="dialog-actions">
                    <button type="button" class="pill" data-variant="secondary" command="close" commandfor=(id)>
                        "Cancel"
                    </button>
                    <button type="submit" class="pill" data-variant=(if danger { "danger" } else { "primary" })>
                        (label)
                        if !danger {
                            <i aria-hidden="true">icon(path: icons::ARROW)</i>
                        }
                    </button>
                </div>
            </form>
        </dialog>
    })
}

/// A trigger with its dialog.
#[component]
pub async fn action(csrf: String, spec: Dialog) -> Result<impl View> {
    let id = spec.id.clone();
    let label = spec.label.clone();
    let danger = spec.danger;
    Ok(view! {
        trigger(id: id, label: label, danger: danger)
        action_dialog(csrf: csrf, spec: spec)
    })
}

/// A subscriber in the diagram.
#[derive(Debug, Clone)]
pub struct Peer {
    /// The id of the subscriber.
    pub id: u64,
    /// Its user agent, or its address.
    pub name: String,
    /// Its lag in events.
    pub lag: i64,
}

impl Peer {
    /// The subscribers of a frame.
    pub fn of(subscribers: &[Value]) -> Vec<Self> {
        subscribers
            .iter()
            .map(|subscriber| {
                let name = subscriber["userAgent"]
                    .as_str()
                    .or_else(|| subscriber["remote"].as_str())
                    .unwrap_or("subscriber");
                Self {
                    id: subscriber["id"].as_u64().unwrap_or_default(),
                    name: name.chars().take(28).collect(),
                    lag: subscriber["lag"].as_i64().unwrap_or_default(),
                }
            })
            .collect()
    }
}

/// The diagram of the connections: the subscribers on the left, the
/// server on the right, each line with the lag of the subscriber and the
/// tone of its state.
#[component]
pub async fn topology(#[into] host: String, peers: Vec<Peer>) -> Result<impl View> {
    let rows = peers.len().max(1);
    let height = 40 + rows * 44;
    let middle = height / 2;
    let none = peers.is_empty();
    Ok(view! {
        <svg
            class="topology"
            viewBox=(format!("0 0 640 {height}"))
            role="img"
            aria-label="Connections of the server"
            data-slot="topology"
        >
            <rect x="420" y=(middle - 24) width="200" height="48" rx="12" fill="var(--lj-fill-2)" stroke="var(--lj-line)"></rect>
            <text x="520" y=(middle + 5) text-anchor="middle" fill="var(--lj-ink)" font-size="14" font-weight="500">(host)</text>
            if none {
                <text x="20" y=(middle + 5) fill="var(--lj-ink-3)" font-size="13">"No subscriber is connected."</text>
            }
            #[key(peer.id)]
            for (index, peer) in peers.into_iter().enumerate() {
                let y = 42 + index * 44;
                let lag = if peer.lag > 1000 { "behind" } else { "ok" };
                <g data-subscriber=(peer.id) data-lag=(lag)>
                    <rect x="10" y=(y - 16) width="220" height="32" rx="10" fill="var(--lj-fill-2)" stroke="var(--lj-line)"></rect>
                    <text x="20" y=(y + 4) fill="var(--lj-ink)" font-size="12">(peer.name)</text>
                    <line x1="230" y1=(y) x2="420" y2=(middle) stroke-width="2" data-lag=(lag)></line>
                    <text x="250" y=(y - 6) fill="var(--lj-ink-3)" font-size="11">"lag " (peer.lag)</text>
                </g>
            }
        </svg>
    })
}

/// A stat of the overview.
struct Stat {
    label: &'static str,
    path: &'static str,
    unit: Option<&'static str>,
}

const STATS: [Stat; 8] = [
    Stat {
        label: "Accounts",
        path: "accounts",
        unit: None,
    },
    Stat {
        label: "Firehose head",
        path: "sequencer.head",
        unit: None,
    },
    Stat {
        label: "Subscribers",
        path: "subscribers",
        unit: None,
    },
    Stat {
        label: "Commits in flight",
        path: "commitsInFlight",
        unit: None,
    },
    Stat {
        label: "Notices waiting",
        path: "noticeOutboxDepth",
        unit: None,
    },
    Stat {
        label: "Audit entries",
        path: "audit.entries",
        unit: None,
    },
    Stat {
        label: "Entries not signed",
        path: "audit.unsigned",
        unit: None,
    },
    Stat {
        label: "Checkpoint age",
        path: "audit.checkpointAgeSeconds",
        unit: Some("s"),
    },
];

/// A row of the subscriber table.
#[derive(Debug, Clone)]
struct SubscriberRow {
    id: u64,
    remote: String,
    agent: String,
    cursor: String,
    lag: i64,
    frames: String,
    bytes: String,
    last: Option<String>,
    connected: Option<String>,
}

impl SubscriberRow {
    fn of(subscriber: &Value) -> Self {
        Self {
            id: subscriber["id"].as_u64().unwrap_or_default(),
            remote: show(&subscriber["remote"]),
            agent: show(&subscriber["userAgent"]),
            cursor: show(&subscriber["cursor"]),
            lag: subscriber["lag"].as_i64().unwrap_or_default(),
            frames: show(&subscriber["frames"]),
            bytes: show(&subscriber["bytes"]),
            last: text(&subscriber["lastFrameAt"]),
            connected: text(&subscriber["connectedAt"]),
        }
    }
}

/// The state of the server: the tiles, the diagram and the subscribers.
/// The live region of the overview emits it for each frame.
#[component]
pub async fn overview_state(
    who: Who,
    frame: Arc<Frame>,
    previous: Option<Arc<Frame>>,
    live: Live,
) -> Result<impl View> {
    let delta = |path: &str| -> Option<i64> {
        let before = previous.as_ref()?.number(path)?;
        Some(frame.number(path)? - before)
    };
    let stats: Vec<(&'static str, String, Option<&'static str>, Option<i64>)> = STATS
        .iter()
        .map(|stat| {
            let value = stat
                .path
                .split('.')
                .fold(&frame.overview, |at, key| &at[key]);
            (stat.label, show(value), stat.unit, delta(stat.path))
        })
        .collect();
    let host = text(&frame.overview["hostname"]).unwrap_or_else(|| "PDS".to_owned());
    let version = show(&frame.overview["version"]);
    let profile = show(&frame.overview["profile"]);
    let time = text(&frame.overview["time"]);
    let peers = Peer::of(&frame.subscribers);
    let rows: Vec<SubscriberRow> = frame.subscribers.iter().map(SubscriberRow::of).collect();
    let (live_text, live_tone) = match &live {
        Live::Snapshot => ("Snapshot", Tone::Neutral),
        Live::Streaming => ("Live", Tone::Good),
        Live::Ended(_) => ("Live view ended", Tone::Warn),
    };
    let note = match &live {
        Live::Ended(why) => Some(why.clone()),
        _ => None,
    };
    let operator = who.operator;
    Ok(view! {
        <section class="section mt-0" data-slot="live" data-state=(live.name())>
            <div class="toolbar mb-3.5" data-slot="toolbar">
                chip(text: live_text, tone: live_tone)
                <span class="fine">
                    "Version " (version) " · profile " (profile) " · server time "
                    when(iso: time)
                </span>
            </div>
            match note {
                Some(note) => alert(text: note),
                None => "",
            }
            <div class="tiles" data-slot="tiles">
                #[key(stat.0)]
                for stat in stats {
                    tile(label: stat.0, value: stat.1, unit: stat.2, delta: stat.3)
                }
            </div>
        </section>
        <section class="section" data-slot="connections">
            <h2>"Connections"</h2>
            <p class="lede">"The firehose subscribers of the server, and how far behind each one is."</p>
            topology(host: host, peers: peers)
            if !rows.is_empty() {
                table(
                    columns: vec!["Address", "User agent", "Cursor", "Lag", "Frames", "Bytes", "Last frame", "Connected", ""],
                    #[key(row.id)]
                    for row in rows {
                        <tr id=(format!("subscriber-{}", row.id)) data-subscriber=(row.id)>
                            <td data-cell="text">(row.remote)</td>
                            <td data-cell="text" title=(row.agent.clone())>(row.agent)</td>
                            <td data-cell="number">(row.cursor)</td>
                            <td>chip(text: format!("{} behind", row.lag), tone: lag_tone(row.lag))</td>
                            <td data-cell="number">(row.frames)</td>
                            <td data-cell="number">(row.bytes)</td>
                            <td>when(iso: row.last)</td>
                            <td>when(iso: row.connected)</td>
                            <td data-cell="actions">
                                if operator {
                                    trigger(id: format!("disconnect-{}", row.id), label: "Disconnect", danger: true)
                                }
                            </td>
                        </tr>
                    }
                )
            }
        </section>
    })
}

/// The dialogs that disconnect the subscribers. They stay outside the
/// region of the numbers so that an open one survives the next frame.
#[component]
pub async fn disconnect_dialogs(csrf: String, subscribers: Vec<Value>) -> Result<impl View> {
    let peers = Peer::of(&subscribers);
    Ok(view! {
        <div data-slot="dialogs">
            #[key(peer.id)]
            for peer in peers {
                action_dialog(
                    csrf: csrf.clone(),
                    spec: Dialog {
                        id: format!("disconnect-{}", peer.id),
                        action: "disconnect",
                        label: "Disconnect".to_owned(),
                        title: format!("Disconnect {}", peer.name),
                        lede: "The server closes the firehose connection of this subscriber. It may connect again.".to_owned(),
                        hidden: vec![("id", peer.id.to_string())],
                        confirm: None,
                        danger: true,
                    },
                )
            }
        </div>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_shown_relative_to_now() {
        let now = DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let at = |iso: &str| relative(iso, now).unwrap();
        assert_eq!(at("2026-10-15T11:59:58Z"), "just now");
        assert_eq!(at("2026-10-15T11:59:20Z"), "40 s ago");
        assert_eq!(at("2026-10-15T11:45:00Z"), "15 min ago");
        assert_eq!(at("2026-10-15T08:00:00.000Z"), "4 h ago");
        assert_eq!(at("2026-10-12T12:00:00+00:00"), "3 d ago");
        assert_eq!(at("2026-10-15T14:00:00Z"), "in 2 h");
        assert_eq!(at("2026-07-01T12:00:00Z"), "1 Jul 2026");
        assert!(relative("yesterday", now).is_none());
    }

    #[test]
    fn a_pattern_escapes_the_syntax_of_a_regular_expression() {
        assert_eq!(pattern("did:plc:abc-123_x"), "did:plc:abc-123_x");
        assert_eq!(
            pattern("a.b(c)[d]{e}|f/g^$*+?\\"),
            r"a\.b\(c\)\[d\]\{e\}\|f\/g\^\$\*\+\?\\"
        );
    }
}
