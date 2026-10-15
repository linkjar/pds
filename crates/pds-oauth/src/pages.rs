//! The authorization pages (`docs/SPEC.md` §11.4), rendered on the server.
//!
//! A page is one HTML document with its style inline under a nonce. It
//! works without scripts. Every page wears the LinkJar journey
//! ([`crate::theme`]): the brand at the top, one centred column, the links
//! at the foot. The page of one account takes the app's account page
//! instead: the sections at the side, a hero card, three tiles, then
//! grouped rows. `parity/README.md` states what a page must keep for the
//! Harness: the field names, the button names and the `alert` role of an
//! error.

use std::fmt::Write;

use crate::theme::{FONT_FACE, RULES, TOKENS};

/// Escapes text for HTML content and for a quoted attribute.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The operator's brand on every page: the header, the fine print and
/// the footer. A link that is `None` is not drawn.
#[derive(Debug, Clone, Copy, Default)]
pub struct Brand<'a> {
    /// The name of the service, in the header and in the title of the
    /// tab.
    pub name: &'a str,
    /// The mark in the header.
    pub logo: Option<&'a str>,
    /// Where the header and the "Home" link lead.
    pub home: Option<&'a str>,
    /// The terms of service, in the fine print and the footer.
    pub terms: Option<&'a str>,
    /// The privacy policy, in the fine print and the footer.
    pub privacy: Option<&'a str>,
    /// The "Support" link of the footer.
    pub support: Option<&'a str>,
}

/// What every page is wrapped in.
#[derive(Debug, Clone, Copy)]
pub struct Shell<'a> {
    /// The nonce of the style element, which the CSP header names.
    pub nonce: &'a str,
    /// The brand.
    pub brand: Brand<'a>,
}

/// What every page of one authorization carries.
#[derive(Debug, Clone, Copy)]
pub struct Flow<'a> {
    /// The brand.
    pub brand: Brand<'a>,
    /// The nonce of the style element, which the CSP header names.
    pub nonce: &'a str,
    /// The client.
    pub client_id: &'a str,
    /// The pushed request.
    pub request_uri: &'a str,
    /// The token that ties a form to the device and the request.
    pub csrf: &'a str,
    /// The identity providers a person can continue with: the name in the
    /// path and the name on the button.
    pub providers: &'a [(&'a str, &'a str)],
    /// The hCaptcha site key, when a sign-up needs a challenge.
    pub captcha: Option<&'a str>,
}

impl<'a> Flow<'a> {
    fn shell(&self) -> Shell<'a> {
        Shell {
            nonce: self.nonce,
            brand: self.brand,
        }
    }
}

/// Reads an invite code from the fragment into the form and into the
/// provider links.
const INVITE_HANDOFF: &str = "(function(){var m=/(?:^#|&)invite=([^&]+)/.exec(location.hash);if(!m)return;\
var c=decodeURIComponent(m[1]),f=document.getElementById('inviteCode'),w=document.getElementById('invite-field');\
if(f){f.value=c;if(w)w.hidden=true;}\
var a=document.querySelectorAll('a[data-provider]');\
for(var i=0;i<a.length;i++){a[i].href+=(a[i].href.indexOf('?')<0?'?':'&')+'invite='+encodeURIComponent(c);}})();";

/// Marks the section in view in the side list of the account page. The
/// page reads the same without it.
const SECTION_IN_VIEW: &str = "(function(){if(!('IntersectionObserver' in window))return;\
var l=document.querySelectorAll('.lj-side a');\
document.querySelectorAll('.lj-content section[id]').forEach(function(s){\
new IntersectionObserver(function(e){if(!e[0].isIntersecting)return;\
l.forEach(function(a){if(a.hash==='#'+s.id)a.setAttribute('aria-current','true');else a.removeAttribute('aria-current')})},\
{rootMargin:'-25% 0px -65% 0px'}).observe(s)})})();";

/// The arrow in the chip of the main action.
const ARROW: &str = "<span class=\"lj-chip\" aria-hidden=\"true\">\
<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"M5 12h14m-6-6 6 6-6 6\"/></svg></span>";

/// A line icon of the account page, from the app's prototype.
fn icon(paths: &str) -> String {
    format!(
        "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{paths}</svg>"
    )
}

const ICON_HOME: &str =
    "<path d=\"M4 11.5 12 5l8 6.5V19a1 1 0 0 1-1 1h-4v-5h-6v5H5a1 1 0 0 1-1-1z\"/>";
const ICON_USER: &str = "<circle cx=\"12\" cy=\"8.5\" r=\"3.5\"/><path d=\"M5 19.5c1.2-3 3.8-4.5 7-4.5s5.8 1.5 7 4.5\"/>";
const ICON_KEY: &str =
    "<circle cx=\"8\" cy=\"15\" r=\"4\"/><path d=\"m11 12 8.5-8.5M16 7l2.5 2.5M19.5 3.5 21 5\"/>";
const ICON_DEVICES: &str = "<rect x=\"3\" y=\"5\" width=\"13\" height=\"10\" rx=\"2\"/><path d=\"M7 19h6m-3-4v4\"/><rect x=\"17\" y=\"10\" width=\"4.5\" height=\"9\" rx=\"1.2\"/>";
const ICON_APPS: &str = "<rect x=\"4\" y=\"4\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"13\" y=\"4\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"4\" y=\"13\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"13\" y=\"13\" width=\"7\" height=\"7\" rx=\"2\"/>";
const ICON_INFO: &str = "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M12 11v5m0-8.5v.5\"/>";
const ICON_LAPTOP: &str =
    "<rect x=\"5\" y=\"5\" width=\"14\" height=\"10\" rx=\"2\"/><path d=\"M3 19h18\"/>";
const ICON_PHONE: &str =
    "<rect x=\"7\" y=\"3\" width=\"10\" height=\"18\" rx=\"2.5\"/><path d=\"M11 18h2\"/>";

/// The marks of the identity providers, from the patched build.
fn provider_mark(id: &str) -> &'static str {
    match id {
        "apple" => {
            "<svg viewBox=\"0 0 24 24\" fill=\"currentColor\" aria-hidden=\"true\"><path d=\"M16.37 12.63c.02 2.6 2.28 3.47 2.3 3.48-.02.06-.36 1.24-1.19 2.45-.72 1.05-1.46 2.1-2.63 2.12-1.15.02-1.52-.68-2.84-.68s-1.73.66-2.82.7c-1.13.04-1.99-1.13-2.71-2.18-1.48-2.14-2.61-6.04-1.09-8.67.75-1.31 2.1-2.14 3.56-2.16 1.11-.02 2.16.75 2.84.75s1.95-.92 3.29-.79c.56.02 2.13.23 3.14 1.7-.08.05-1.87 1.09-1.85 3.28zM14.2 6.25c.6-.73 1-1.74.89-2.75-.86.03-1.91.57-2.53 1.3-.55.64-1.04 1.67-.91 2.66.96.07 1.95-.49 2.55-1.21z\"/></svg>"
        }
        "google" => {
            "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path fill=\"#4285F4\" d=\"M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.27-4.74 3.27-8.1z\"/><path fill=\"#34A853\" d=\"M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84A11 11 0 0 0 12 23z\"/><path fill=\"#FBBC05\" d=\"M5.84 14.09a6.6 6.6 0 0 1 0-4.18V7.07H2.18a11 11 0 0 0 0 9.86l3.66-2.84z\"/><path fill=\"#EA4335\" d=\"M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15A11 11 0 0 0 2.18 7.07l3.66 2.84C6.71 7.31 9.14 5.38 12 5.38z\"/></svg>"
        }
        "github" => {
            "<svg viewBox=\"0 0 16 16\" fill=\"currentColor\" aria-hidden=\"true\"><path d=\"M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z\"/></svg>"
        }
        _ => "",
    }
}

/// The three kinds of button: the light pill of the main action, the
/// translucent pill, and the quiet text of a way back.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Primary,
    Secondary,
    Quiet,
}

/// A submit button. `attrs` are more attributes, each with its leading
/// space, already escaped.
fn button(kind: Kind, attrs: &str, label: &str) -> String {
    match kind {
        Kind::Primary => format!(
            "<button type=\"submit\" class=\"lj-primary\"{attrs}>{}{ARROW}</button>",
            escape(label)
        ),
        Kind::Secondary => format!(
            "<button type=\"submit\" class=\"lj-secondary\"{attrs}>{}</button>",
            escape(label)
        ),
        Kind::Quiet => format!(
            "<button type=\"submit\" class=\"lj-quiet\"{attrs}>{}</button>",
            escape(label)
        ),
    }
}

/// The whole document around `main`, which holds its own `<main>`.
fn document(shell: Shell<'_>, title: &str, main: &str, script: Option<&str>) -> String {
    let brand = shell.brand;
    let mark = brand.logo.map_or_else(String::new, |logo| {
        format!("<img src=\"{}\" alt=\"\">", escape(logo))
    });
    let header = brand.home.map_or_else(
        || {
            format!(
                "<header><div class=\"lj-brand\">{mark}{}</div></header>",
                escape(brand.name)
            )
        },
        |home| {
            format!(
                "<header><a class=\"lj-brand\" href=\"{}\">{mark}{}</a></header>",
                escape(home),
                escape(brand.name)
            )
        },
    );
    let mut links = String::new();
    for (href, label) in [
        (brand.home, "Home"),
        (brand.terms, "Terms of Service"),
        (brand.privacy, "Privacy Policy"),
        (brand.support, "Support"),
    ] {
        if let Some(href) = href {
            let _ = write!(links, "<a href=\"{}\">{label}</a>", escape(href));
        }
    }
    let nav = if links.is_empty() {
        String::new()
    } else {
        format!("<nav>{links}</nav>")
    };
    let script = script.map_or_else(String::new, |script| {
        format!(
            "<script nonce=\"{}\">{script}</script>",
            escape(shell.nonce)
        )
    });
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"dark\">\
<title>{} · {}</title>\
<style nonce=\"{}\">{FONT_FACE}{TOKENS}{RULES}</style></head><body>{header}{main}\
<footer class=\"lj-footer\"><span class=\"lj-locale\" lang=\"en\">English</span>{nav}</footer>\
{script}</body></html>",
        escape(title),
        escape(brand.name),
        escape(shell.nonce),
    )
}

/// The journey: one centred column on the stage.
fn stage(panel: &str) -> String {
    format!("<main class=\"lj-stage\"><div class=\"lj-panel\">{panel}</div></main>")
}

/// The heading of a page, and the lede under it. The lede is HTML.
fn heading(title: &str, lede: Option<&str>) -> String {
    let lede = lede.map_or_else(String::new, |lede| {
        format!("<p class=\"lj-lede\">{lede}</p>")
    });
    format!("<h1 class=\"lj-title\">{}</h1>{lede}", escape(title))
}

/// The step track of the sign-up: a short centred bar under the heading.
fn steps(step: usize, of: usize) -> String {
    let mut track = String::new();
    for done in 1..=of {
        track.push_str(if done <= step {
            "<i class=\"done\"></i>"
        } else {
            "<i></i>"
        });
    }
    format!(
        "<div class=\"lj-steps\"><p>Step {step} of {of}</p><div class=\"lj-track\">{track}</div></div>"
    )
}

/// A labelled field. `input` is the `<input>`; `hint` is HTML under it.
fn field(id: &str, label: &str, input: &str, hint: Option<&str>) -> String {
    let hint = hint.map_or_else(String::new, |hint| {
        format!("<p class=\"lj-hint\">{hint}</p>")
    });
    format!(
        "<div class=\"lj-field\"><label for=\"{}\">{}</label>{input}{hint}</div>",
        escape(id),
        escape(label)
    )
}

fn hidden(flow: &Flow<'_>) -> String {
    format!(
        "<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
<input type=\"hidden\" name=\"request_uri\" value=\"{}\">\
<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        escape(flow.client_id),
        escape(flow.request_uri),
        escape(flow.csrf),
    )
}

/// The same request again with another view: a link in the form of a
/// button.
fn view_form(flow: &Flow<'_>, buttons: &[(&str, &str, Kind)]) -> String {
    let mut out = format!(
        "<form method=\"get\" action=\"/oauth/authorize\">\
<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
<input type=\"hidden\" name=\"request_uri\" value=\"{}\"><div class=\"lj-actions\">",
        escape(flow.client_id),
        escape(flow.request_uri),
    );
    for (view, label, kind) in buttons {
        let attrs = format!(" name=\"view\" value=\"{}\"", escape(view));
        out.push_str(&button(*kind, &attrs, label));
    }
    out.push_str("</div></form>");
    out
}

/// The rows of the identity providers: each a link that starts the
/// sign-in. `query` is the query of the start, already escaped;
/// `handoff` marks the links for the invite hand-off script.
fn provider_rows(providers: &[(&str, &str)], query: &str, handoff: bool) -> String {
    let mut out = String::new();
    let marker = if handoff { " data-provider" } else { "" };
    for (id, label) in providers {
        let _ = write!(
            out,
            "<a class=\"lj-provider\"{marker} href=\"/oauth/external/{}/start{query}\">{}<span>Continue with {}</span></a>",
            escape(id),
            provider_mark(id),
            escape(label)
        );
    }
    if out.is_empty() {
        out
    } else {
        format!("<div class=\"lj-providers\">{out}</div>")
    }
}

/// The providers of an authorization: the start carries its request.
fn flow_providers(flow: &Flow<'_>) -> String {
    let query = format!(
        "?request_uri={}",
        escape(&flow.request_uri.replace(':', "%3A"))
    );
    provider_rows(flow.providers, &query, true)
}

fn alert(error: Option<&str>) -> String {
    error.map_or_else(String::new, |error| {
        format!("<div role=\"alert\">{}</div>", escape(error))
    })
}

/// A link of the fine print, or its words alone when the brand has no
/// such page.
fn fine_link(href: Option<&str>, words: &str) -> String {
    href.map_or_else(
        || words.to_owned(),
        |href| format!("<a href=\"{}\">{words}</a>", escape(href)),
    )
}

/// The first page of an authorization that names no account: sign in or
/// sign up.
pub fn welcome(flow: &Flow<'_>) -> String {
    let panel = format!(
        "{}{}",
        heading("Welcome.", Some("Please authenticate to continue")),
        view_form(
            flow,
            &[
                ("sign-in", "Sign in", Kind::Primary),
                ("sign-up", "Create a new account", Kind::Secondary),
            ]
        ),
    );
    document(flow.shell(), "Authenticate", &stage(&panel), None)
}

/// The password form of a sign-in. `action` is where it posts, `hidden`
/// the hidden fields.
fn password_form(
    action: &str,
    hidden: &str,
    username: &str,
    locked: bool,
    remember: bool,
) -> String {
    let readonly = if locked { " readonly" } else { " autofocus" };
    let focus = if locked { " autofocus" } else { "" };
    let checked = if remember { " checked" } else { "" };
    format!(
        "<form method=\"post\" action=\"{}\">{hidden}{}{}\
<label class=\"lj-check\"><input type=\"checkbox\" name=\"remember\" value=\"true\"{checked}><span>Remember this device</span></label>\
<div class=\"lj-actions\">{}</div></form>",
        escape(action),
        field(
            "username",
            "Handle or email",
            &format!(
                "<input id=\"username\" type=\"text\" name=\"username\" value=\"{}\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" required{readonly}>",
                escape(username)
            ),
            None
        ),
        field(
            "password",
            "Password",
            &format!(
                "<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"current-password\" required{focus}>"
            ),
            None
        ),
        button(Kind::Primary, "", "Sign in"),
    )
}

/// The providers first, the password form folded under them. Without
/// providers, the form alone. An error opens the fold.
fn fold(providers: &str, form: &str, error: Option<&str>) -> String {
    if providers.is_empty() {
        form.to_owned()
    } else {
        let open = if error.is_some() { " open" } else { "" };
        format!(
            "{providers}<details class=\"lj-email-fold\"{open}><summary>Or use email</summary>{form}</details>"
        )
    }
}

/// The sign-in form. `locked` holds the account the request names, which
/// the form does not let the user change.
pub fn sign_in(flow: &Flow<'_>, username: &str, locked: bool, error: Option<&str>) -> String {
    let form = password_form(
        "/oauth/authorize/sign-in",
        &hidden(flow),
        username,
        locked,
        false,
    );
    let lede = format!("Sign in to your {} account.", escape(flow.brand.name));
    let panel = format!(
        "{}{}{}{}",
        heading("Welcome back.", Some(&lede)),
        alert(error),
        fold(&flow_providers(flow), &form, error),
        view_form(flow, &[("welcome", "Back", Kind::Quiet)]),
    );
    document(flow.shell(), "Sign in", &stage(&panel), None)
}

/// The accounts the device remembers, each as a button.
pub fn accounts(flow: &Flow<'_>, accounts: &[(String, String)]) -> String {
    let mut panel = heading(
        "Choose an account.",
        Some("Continue to the application with one of your accounts."),
    );
    let _ = write!(
        panel,
        "<form method=\"get\" action=\"/oauth/authorize\">\
<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
<input type=\"hidden\" name=\"request_uri\" value=\"{}\"><div class=\"lj-actions\">",
        escape(flow.client_id),
        escape(flow.request_uri),
    );
    for (did, handle) in accounts {
        let attrs = format!(" name=\"did\" value=\"{}\"", escape(did));
        panel.push_str(&button(Kind::Secondary, &attrs, handle));
    }
    panel.push_str("</div></form>");
    panel.push_str(&view_form(
        flow,
        &[
            ("sign-in", "Use another account", Kind::Quiet),
            ("sign-up", "Create a new account", Kind::Quiet),
        ],
    ));
    document(flow.shell(), "Choose an account", &stage(&panel), None)
}

/// What the consent page says of the client and of the request.
#[derive(Debug, Clone, Copy)]
pub struct Consent<'a> {
    /// The account that is asked.
    pub did: &'a str,
    /// Its handle.
    pub handle: &'a str,
    /// The client's own name, shown for a trusted client only.
    pub client_name: Option<&'a str>,
    /// The scopes of the request.
    pub scopes: &'a [&'a str],
    /// The proof of a sign-in that the device does not remember.
    pub ephemeral: Option<&'a str>,
}

fn describe_scope(scope: &str) -> String {
    match scope {
        "atproto" => "Know which account you use".to_owned(),
        "transition:generic" => {
            "Read and change your posts, likes, follows and other records, and upload files"
                .to_owned()
        }
        "transition:email" => "Read your email address".to_owned(),
        "transition:chat.bsky" => "Read and send your chat messages".to_owned(),
        other => format!("Permission {other}"),
    }
}

/// The consent page: who asks, as which account, for what.
pub fn consent(flow: &Flow<'_>, consent: &Consent<'_>) -> String {
    // A client the operator does not trust is shown by its identifier, never by the name it gives itself.
    let client = consent.client_name.map_or_else(
        || format!("<code>{}</code>", escape(flow.client_id)),
        |name| format!("<strong>{}</strong>", escape(name)),
    );
    let mut scopes = String::new();
    for scope in consent.scopes {
        let _ = write!(scopes, "<li>{}</li>", escape(&describe_scope(scope)));
    }
    let ephemeral = consent.ephemeral.map_or_else(String::new, |token| {
        format!(
            "<input type=\"hidden\" name=\"ephemeral\" value=\"{}\">",
            escape(token)
        )
    });
    let lede = format!(
        "{client} asks for access to your account <strong>{}</strong>.",
        escape(consent.handle)
    );
    let panel = format!(
        "{}<p class=\"lj-note\">The application will be able to:</p><ul class=\"lj-scopes\">{scopes}</ul>\
<form method=\"post\" action=\"/oauth/authorize/accept\">{}\
<input type=\"hidden\" name=\"did\" value=\"{}\">{ephemeral}\
<div class=\"lj-actions\">{}{}</div></form>",
        heading("Authorize access.", Some(&lede)),
        hidden(flow),
        escape(consent.did),
        button(Kind::Primary, "", "Authorize"),
        button(
            Kind::Secondary,
            " formaction=\"/oauth/authorize/reject\"",
            "Deny access"
        ),
    );
    document(flow.shell(), "Authorize access", &stage(&panel), None)
}

/// The first step of a sign-up: the email address and the password, with
/// the providers under them as the alternative.
pub fn sign_up_credentials(
    flow: &Flow<'_>,
    invite_required: bool,
    email: &str,
    invite_code: &str,
    error: Option<&str>,
) -> String {
    let invite = if invite_required {
        format!(
            "<div id=\"invite-field\" class=\"lj-field\"><label for=\"inviteCode\">Invite code</label>\
<input id=\"inviteCode\" type=\"text\" name=\"inviteCode\" value=\"{}\" autocapitalize=\"none\" spellcheck=\"false\" required></div>",
            escape(invite_code)
        )
    } else {
        String::new()
    };
    let focus = if invite_required { "" } else { " autofocus" };
    let providers = flow_providers(flow);
    let alternative = if providers.is_empty() {
        String::new()
    } else {
        format!("<p class=\"lj-divider\">or</p>{providers}")
    };
    let panel = format!(
        "{}{}{}<form method=\"post\" action=\"/oauth/authorize/sign-up\">{}{invite}{}{}\
<div class=\"lj-actions\">{}</div></form>{alternative}\
<p class=\"lj-fine\">By continuing you agree to the {} and the {}.</p>{}",
        heading("Create your account.", None),
        steps(1, 2),
        alert(error),
        hidden(flow),
        field(
            "email",
            "Email",
            &format!(
                "<input id=\"email\" type=\"email\" name=\"email\" value=\"{}\" autocomplete=\"email\" required{focus}>",
                escape(email)
            ),
            None
        ),
        field(
            "password",
            "Password",
            "<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"new-password\" minlength=\"8\" required>",
            Some("At least 8 characters.")
        ),
        button(Kind::Primary, "", "Continue"),
        fine_link(flow.brand.terms, "Terms of Service"),
        fine_link(flow.brand.privacy, "Privacy Policy"),
        view_form(flow, &[("welcome", "Back", Kind::Quiet)]),
    );
    // The app hands an invite code over in the fragment `#invite=<code>`
    // (patch 103). The page then submits it in place of the field, and the
    // provider links carry it. A page that was sent back with an error
    // has no fragment, so the field shows again with the server's answer.
    document(
        flow.shell(),
        "Create a new account",
        &stage(&panel),
        Some(INVITE_HANDOFF),
    )
}

/// The second step of a sign-up: the handle, and the challenge when one
/// is configured. `pending` names the first step, which the server keeps.
pub fn sign_up_handle(
    flow: &Flow<'_>,
    pending: &str,
    handle: &str,
    domain: &str,
    error: Option<&str>,
) -> String {
    // The challenge is a script of hCaptcha. The form sends its answer in
    // `h-captcha-response`.
    let captcha = flow.captcha.map_or_else(String::new, |site_key| {
        format!(
            "<h2>Verify you are human</h2><div class=\"h-captcha\" data-sitekey=\"{}\" data-theme=\"dark\"></div>\
<script src=\"https://js.hcaptcha.com/1/api.js\" async defer nonce=\"{}\"></script>",
            escape(site_key),
            escape(flow.nonce)
        )
    });
    // The hosted domain rides in the field, after what the person types.
    let input = format!(
        "<div class=\"lj-affix\"><input id=\"handle\" type=\"text\" name=\"handle\" value=\"{}\" autocapitalize=\"none\" spellcheck=\"false\" required autofocus><span aria-hidden=\"true\">{}</span></div>",
        escape(handle),
        escape(domain)
    );
    let hint = format!("Your handle ends with <strong>{}</strong>.", escape(domain));
    let panel = format!(
        "{}{}{}<form method=\"post\" action=\"/oauth/authorize/sign-up\">{}\
<input type=\"hidden\" name=\"pending\" value=\"{}\">{}{captcha}\
<div class=\"lj-actions\">{}</div></form>",
        heading(
            "Choose a username.",
            Some("Your handle is your name on the network.")
        ),
        steps(2, 2),
        alert(error),
        hidden(flow),
        escape(pending),
        field("handle", "Username", &input, Some(&hint)),
        button(Kind::Primary, "", "Create account"),
    );
    document(flow.shell(), "Choose your handle", &stage(&panel), None)
}

/// An OAuth session on the account page.
#[derive(Debug, Clone)]
pub struct SessionLine {
    /// The `jti` of the session's access token, which the "Remove" button
    /// sends.
    pub token_id: String,
    /// The client.
    pub client_id: String,
    /// When the session began.
    pub created_at: String,
    /// When it was last refreshed.
    pub updated_at: String,
}

/// A device on the account page.
#[derive(Debug, Clone)]
pub struct DeviceLine {
    /// The device, which the "Sign out" button sends.
    pub device_id: String,
    /// The browser it last showed.
    pub user_agent: String,
    /// The address it last came from.
    pub ip_address: String,
    /// When the server last saw it.
    pub last_seen_at: String,
    /// Whether it is the device that asks.
    pub current: bool,
}

/// A form of one button on the account page. `class` is the class of the
/// button.
fn account_form(
    action: &str,
    csrf: &str,
    fields: &[(&str, &str)],
    class: &str,
    label: &str,
) -> String {
    let mut out = format!(
        "<form method=\"post\" action=\"{}\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        escape(action),
        escape(csrf)
    );
    for (name, value) in fields {
        let _ = write!(
            out,
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        );
    }
    let _ = write!(
        out,
        "<button type=\"submit\" class=\"{class}\">{}</button></form>",
        escape(label)
    );
    out
}

/// The first page of the account pages for a device that no account is
/// signed in on.
pub fn account_welcome(shell: Shell<'_>) -> String {
    let panel = format!(
        "{}<form method=\"get\" action=\"/account\"><div class=\"lj-actions\">{}</div></form>",
        heading(
            "Your account.",
            Some("Sign in to manage your sign-in methods, your devices and the apps you allowed.")
        ),
        button(Kind::Primary, " name=\"view\" value=\"sign-in\"", "Sign in"),
    );
    document(shell, "Your account", &stage(&panel), None)
}

/// The sign-in form of the account pages. `providers` are the identity
/// providers a person can continue with: the name in the path and the
/// name on the button.
pub fn account_sign_in(
    shell: Shell<'_>,
    csrf: &str,
    username: &str,
    error: Option<&str>,
    providers: &[(&str, &str)],
) -> String {
    let hidden = format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        escape(csrf)
    );
    let form = password_form("/account/sign-in", &hidden, username, false, true);
    let lede = format!("Sign in to your {} account.", escape(shell.brand.name));
    let panel = format!(
        "{}{}{}",
        heading("Welcome back.", Some(&lede)),
        alert(error),
        fold(&provider_rows(providers, "", false), &form, error),
    );
    document(shell, "Sign in", &stage(&panel), None)
}

/// The accounts that are signed in on the device, each with a link to its
/// page.
pub fn account_list(shell: Shell<'_>, accounts: &[(String, String)]) -> String {
    let mut panel = heading("Your accounts.", None);
    for (did, handle) in accounts {
        let _ = write!(
            panel,
            "<section class=\"lj-card\"><p><strong>{}</strong></p>\
<a class=\"lj-secondary lj-small\" href=\"/account/{}/manage\">Manage this account</a></section>",
            escape(handle),
            escape(did)
        );
    }
    let _ = write!(
        panel,
        "<form method=\"get\" action=\"/account\"><div class=\"lj-actions\">{}</div></form>",
        button(
            Kind::Quiet,
            " name=\"view\" value=\"sign-in\"",
            "Sign in to another account"
        )
    );
    document(shell, "Your accounts", &stage(&panel), None)
}

/// A sign-in method of the account page.
#[derive(Debug, Clone)]
pub struct MethodLine {
    /// The provider's name in a path.
    pub provider: String,
    /// The provider's name on a button.
    pub label: String,
    /// The provider's identifier of the person, which "Unlink" sends.
    pub subject: String,
    /// The address the provider asserted.
    pub email: Option<String>,
}

/// The sign-in methods of an account.
#[derive(Debug, Clone, Default)]
pub struct Methods {
    /// Whether the account has a password.
    pub has_password: bool,
    /// The identities that are linked.
    pub linked: Vec<MethodLine>,
    /// The providers the account can link: the name in the path and the
    /// name on the button.
    pub offered: Vec<(String, String)>,
    /// Whether the account may remove a method: it keeps at least one.
    pub can_unlink: bool,
}

/// The page of one account.
#[derive(Debug, Clone, Copy)]
pub struct Account<'a> {
    /// The token that ties a form to the device.
    pub csrf: &'a str,
    /// The account.
    pub did: &'a str,
    /// Its handle.
    pub handle: &'a str,
    /// The host name of this server, which keeps the account.
    pub host: &'a str,
    /// Its sign-in methods.
    pub methods: &'a Methods,
    /// The applications with a session.
    pub sessions: &'a [SessionLine],
    /// The devices that remember the account.
    pub devices: &'a [DeviceLine],
    /// What the last action had to say.
    pub error: Option<&'a str>,
}

/// A row of a group: an icon or none, the text, and an action or none.
fn row(icon: &str, text: &str, action: &str) -> String {
    let icon = if icon.is_empty() {
        String::new()
    } else {
        format!("<span class=\"lj-row-icon\">{icon}</span>")
    };
    format!("<div class=\"lj-row\">{icon}<span class=\"lj-row-text\">{text}</span>{action}</div>")
}

/// A time of the account database, `2026-10-15T12:34:56.789Z`, as a
/// person reads it.
fn when(iso: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) if iso.as_bytes().get(10) == Some(&b'T') => {
            let zone = if iso.ends_with('Z') { " UTC" } else { "" };
            format!("{date} {time}{zone}")
        }
        _ => iso.to_owned(),
    }
}

/// A browser and a device out of a user agent string, such as "Safari on
/// Mac". A string that names neither is shown as it is.
fn describe_agent(agent: &str) -> String {
    let browser = if agent.contains("Edg/") {
        "Edge"
    } else if agent.contains("OPR/") {
        "Opera"
    } else if agent.contains("Firefox/") || agent.contains("FxiOS/") {
        "Firefox"
    } else if agent.contains("Chrome/") || agent.contains("CriOS/") {
        "Chrome"
    } else if agent.contains("Safari/") {
        "Safari"
    } else {
        return agent.to_owned();
    };
    let device = if agent.contains("iPhone") {
        "iPhone"
    } else if agent.contains("iPad") {
        "iPad"
    } else if agent.contains("Android") {
        "Android"
    } else if agent.contains("Macintosh") {
        "Mac"
    } else if agent.contains("Windows") {
        "Windows"
    } else if agent.contains("Linux") {
        "Linux"
    } else {
        return browser.to_owned();
    };
    format!("{browser} on {device}")
}

/// The sign-in methods in a few words: "Google and a password".
fn methods_summary(methods: &Methods) -> String {
    let mut parts: Vec<&str> = methods
        .linked
        .iter()
        .map(|method| method.label.as_str())
        .collect();
    if methods.has_password {
        parts.push("a password");
    }
    let mut words = match parts.as_slice() {
        [] => "No sign-in method".to_owned(),
        [one] => (*one).to_owned(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    };
    if let Some(first) = words.get(..1) {
        words.replace_range(..1, &first.to_ascii_uppercase());
    }
    escape(&words)
}

fn count(n: usize, one: &str, many: &str) -> String {
    match n {
        0 => "None yet".to_owned(),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// The hero card and the three tiles.
fn account_overview(account: &Account<'_>, name: &str) -> String {
    format!(
        "<section id=\"overview\" class=\"lj-hero\"><div class=\"lj-hero-text\"><p class=\"lj-eyebrow\">Your account</p><h1>{}</h1>\
<p class=\"lj-badges\"><span class=\"lj-badge\">Hosted by {name}</span></p>\
<p class=\"lj-fine\">One account for {name} and every app on the open social web. <a href=\"#about\">What does this mean?</a></p></div></section>\
<div class=\"lj-tiles\"><a class=\"lj-tile\" href=\"#methods\"><span>Signing in</span><strong>{}</strong></a>\
<a class=\"lj-tile\" href=\"#devices\"><span>Devices</span><strong>{}</strong></a>\
<a class=\"lj-tile\" href=\"#apps\"><span>Apps</span><strong>{}</strong></a></div>\
<section id=\"account\"><h2>Account</h2><p class=\"lj-lede\">Who you are on the network.</p><div class=\"lj-group\">{}{}</div></section>",
        escape(account.handle),
        methods_summary(account.methods),
        count(account.devices.len(), "signed in", "signed in"),
        count(account.sessions.len(), "allowed", "allowed"),
        row(
            "",
            &format!(
                "<small>Handle</small><strong>{}</strong><small>Your name on the network. Apps show it with an @ in front.</small>",
                escape(account.handle)
            ),
            ""
        ),
        row(
            "",
            &format!(
                "<small>Permanent ID</small><strong><code>{}</code></strong><small>It stays the same when your handle or host changes.</small>",
                escape(account.did)
            ),
            ""
        ),
    )
}

/// The "Sign-in methods" section: the password, the identities that are
/// linked, and the providers the account can link.
fn account_methods(account: &Account<'_>) -> String {
    let methods = account.methods;
    let key = icon(ICON_KEY);
    let mut rows = row(
        &key,
        &format!(
            "<strong>Password</strong><small>{}</small>",
            if methods.has_password {
                "A password is set."
            } else {
                "No password is set."
            }
        ),
        "",
    );
    for method in &methods.linked {
        let disabled = if methods.can_unlink { "" } else { " disabled" };
        let mark = provider_mark(&method.provider);
        let connected = method.email.as_deref().map_or_else(
            || "Connected.".to_owned(),
            |email| format!("Connected · {}", escape(email)),
        );
        rows.push_str(&row(
            if mark.is_empty() { &key } else { mark },
            &format!(
                "<strong>{}</strong><small>{connected}</small>",
                escape(&method.label)
            ),
            &format!(
                "<form method=\"post\" action=\"/account/unlink\">\
<input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"did\" value=\"{}\">\
<input type=\"hidden\" name=\"provider\" value=\"{}\"><input type=\"hidden\" name=\"subject\" value=\"{}\">\
<button type=\"submit\" class=\"lj-row-action\"{disabled}>Unlink {}</button></form>",
                escape(account.csrf),
                escape(account.did),
                escape(&method.provider),
                escape(&method.subject),
                escape(&method.label),
            ),
        ));
    }
    let link = escape(&account.did.replace(':', "%3A"));
    for (id, label) in &methods.offered {
        let mark = provider_mark(id);
        rows.push_str(&row(
            if mark.is_empty() { &key } else { mark },
            &format!(
                "<strong>{}</strong><small>Not connected.</small>",
                escape(label)
            ),
            &format!(
                "<a class=\"lj-row-action\" href=\"/oauth/external/{}/start?link={link}\">Link {}</a>",
                escape(id),
                escape(label)
            ),
        ));
    }
    let note = if !methods.linked.is_empty() && !methods.can_unlink {
        "<p class=\"lj-note\">This is your only sign-in method. Add another one before you remove it.</p>"
    } else {
        ""
    };
    format!(
        "<section id=\"methods\"><h2>Sign-in methods</h2><p class=\"lj-lede\">The ways into your account. Keep at least one.</p>\
<div class=\"lj-group\">{rows}</div>{note}</section>"
    )
}

/// The "Devices" and "Apps" sections.
fn account_sessions(account: &Account<'_>) -> String {
    let mut devices = String::new();
    for device in account.devices {
        let agent = device.user_agent.as_str();
        let phone = ["Mobile", "iPhone", "iPad", "Android"]
            .iter()
            .any(|word| agent.contains(word));
        let agent = describe_agent(agent);
        let tag = if device.current {
            " <em class=\"lj-tag\">This device</em>"
        } else {
            ""
        };
        // This device signs out from the end of the page.
        let action = if device.current {
            String::new()
        } else {
            account_form(
                "/account/sign-out",
                account.csrf,
                &[("did", account.did), ("device_id", &device.device_id)],
                "lj-row-action",
                "Sign out",
            )
        };
        devices.push_str(&row(
            &icon(if phone { ICON_PHONE } else { ICON_LAPTOP }),
            &format!(
                "<strong>{}{tag}</strong><small>Address {} · Last seen {}</small>",
                escape(&agent),
                escape(&device.ip_address),
                escape(&when(&device.last_seen_at))
            ),
            &action,
        ));
    }
    if devices.is_empty() {
        devices = row(
            &icon(ICON_DEVICES),
            "<strong>No device remembers this account.</strong>",
            "",
        );
    }
    let mut apps = String::new();
    for session in account.sessions {
        apps.push_str(&row(
            &icon(ICON_APPS),
            &format!(
                "<strong><code>{}</code></strong><small>Since {} · Last used {}</small>",
                escape(&session.client_id),
                escape(&when(&session.created_at)),
                escape(&when(&session.updated_at))
            ),
            &account_form(
                "/account/revoke",
                account.csrf,
                &[("did", account.did), ("token_id", &session.token_id)],
                "lj-row-action",
                "Remove",
            ),
        ));
    }
    if apps.is_empty() {
        apps = row(
            &icon(ICON_APPS),
            "<strong>No app has access.</strong><small>Apps you sign in to with this account appear here.</small>",
            "",
        );
    }
    format!(
        "<section id=\"devices\"><h2>Devices</h2><p class=\"lj-lede\">Where you are signed in.</p><div class=\"lj-group\">{devices}</div></section>\
<section id=\"apps\"><h2>Apps</h2><p class=\"lj-lede\">Apps you allowed to use your account. Remove any of them at any time.</p><div class=\"lj-group\">{apps}</div></section>"
    )
}

/// The "About your account" section, with the technical details and the
/// way out.
fn account_about(account: &Account<'_>, name: &str) -> String {
    let sign_out = account
        .devices
        .iter()
        .find(|device| device.current)
        .map_or_else(String::new, |device| {
            account_form(
                "/account/sign-out",
                account.csrf,
                &[("did", account.did), ("device_id", &device.device_id)],
                "lj-secondary",
                "Sign out",
            )
        });
    format!(
        "<section id=\"about\"><h2>About your account</h2>\
<p class=\"lj-lede\">Your {name} account is an AT Protocol account: one identity for every app built on the open network.</p>\
<div class=\"lj-facts\"><div><strong>It is yours.</strong><p>Your handle, your follows and your public data belong to the account, not to one app.</p></div>\
<div><strong>{name} hosts it.</strong><p>We keep it online and safe. We do not own it.</p></div>\
<div><strong>You can leave.</strong><p>Move to another host at any time and keep your identity and your data.</p></div></div>\
<details class=\"lj-advanced\"><summary>Technical details</summary>\
<p><strong>Permanent ID (DID).</strong> <code>{}</code></p>\
<p><strong>Personal data server.</strong> <code>{}</code></p>\
<p><strong>Handle lookup.</strong> Your handle resolves to your permanent ID through DNS or your host.</p></details>\
<div class=\"lj-actions\">{sign_out}<a class=\"lj-secondary\" href=\"/account\">All accounts</a></div></section>",
        escape(account.did),
        escape(account.host),
    )
}

/// The page of one account: its sign-in methods, the devices that
/// remember it, and the applications with a session.
pub fn account_home(shell: Shell<'_>, account: &Account<'_>) -> String {
    let name = escape(shell.brand.name);
    let mut side = String::from("<nav class=\"lj-side\" aria-label=\"Account sections\">");
    for (id, label, glyph) in [
        ("overview", "Overview", ICON_HOME),
        ("account", "Account", ICON_USER),
        ("methods", "Sign-in methods", ICON_KEY),
        ("devices", "Devices", ICON_DEVICES),
        ("apps", "Apps", ICON_APPS),
        ("about", "About", ICON_INFO),
    ] {
        let current = if id == "overview" {
            " aria-current=\"true\""
        } else {
            ""
        };
        let _ = write!(
            side,
            "<a href=\"#{id}\"{current}>{}<span>{label}</span></a>",
            icon(glyph)
        );
    }
    side.push_str("</nav>");
    // What the last action had to say floats at the foot of the page.
    let toast = account.error.map_or_else(String::new, |error| {
        format!("<p class=\"lj-toast\" role=\"alert\">{}</p>", escape(error))
    });
    let main = format!(
        "<main class=\"lj-account\">{side}<div class=\"lj-content\">{}{}{}{}</div></main>{toast}",
        account_overview(account, &name),
        account_methods(account),
        account_sessions(account),
        account_about(account, &name),
    );
    document(shell, "Your account", &main, Some(SECTION_IN_VIEW))
}

/// A page that ends a flow the server cannot return to the client.
pub fn error(shell: Shell<'_>, title: &str, message: &str) -> String {
    let panel = format!(
        "{}<div role=\"alert\">{}</div>",
        heading(title, None),
        escape(message)
    );
    document(shell, title, &stage(&panel), None)
}

/// The page of the `form_post` response mode: a form that carries the
/// answer to the client.
pub fn form_post(shell: Shell<'_>, action: &str, fields: &[(&str, &str)]) -> String {
    let mut inputs = String::new();
    for (name, value) in fields {
        let _ = write!(
            inputs,
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        );
    }
    let panel = format!(
        "{}<form method=\"post\" action=\"{}\">{inputs}<div class=\"lj-actions\">{}</div></form>",
        heading(
            "Return to the application.",
            Some("Continue to finish the sign-in.")
        ),
        escape(action),
        button(Kind::Primary, "", "Continue"),
    );
    document(shell, "Return to the application", &stage(&panel), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRAND: Brand<'static> = Brand {
        name: "pds.test",
        logo: Some("/oauth/assets/stock-mark.svg"),
        home: None,
        terms: None,
        privacy: None,
        support: None,
    };

    const SHELL: Shell<'static> = Shell {
        nonce: "n",
        brand: BRAND,
    };

    const FLOW: Flow<'static> = Flow {
        brand: BRAND,
        nonce: "n0nce",
        client_id: "https://app.example.org/client-metadata.json?a=1&b=\"2\"",
        request_uri: "urn:ietf:params:oauth:request_uri:req-00",
        csrf: "token",
        providers: &[],
        captcha: None,
    };

    fn account<'a>(methods: &'a Methods, error: Option<&'a str>) -> Account<'a> {
        Account {
            csrf: "csrf",
            did: "did:plc:abc",
            handle: "a.test",
            host: "pds.test",
            methods,
            sessions: &[],
            devices: &[],
            error,
        }
    }

    #[test]
    fn pages_keep_the_contract_of_the_harness() {
        let welcome = welcome(&FLOW);
        assert!(welcome.contains("Please authenticate to continue"));
        assert!(welcome.contains("value=\"sign-in\">Sign in<span class=\"lj-chip\""));
        assert!(welcome.contains(">Create a new account</button>"));

        let sign_in = sign_in(
            &FLOW,
            "alice.test",
            true,
            Some("Invalid identifier or password"),
        );
        assert!(!sign_in.contains("Please authenticate to continue"));
        assert!(sign_in.contains("name=\"username\" value=\"alice.test\""));
        assert!(sign_in.contains("type=\"password\" name=\"password\""));
        assert!(sign_in.contains("type=\"checkbox\" name=\"remember\""));
        assert!(sign_in.contains("<div role=\"alert\">Invalid identifier or password</div>"));
        // The submit button of the form comes before any other submit button.
        let submit = sign_in.find("<button type=\"submit\"").unwrap_or_default();
        assert!(
            sign_in[submit..].starts_with("<button type=\"submit\" class=\"lj-primary\">Sign in")
        );

        let sign_up = sign_up_credentials(&FLOW, true, "", "", None);
        assert!(sign_up.contains("name=\"email\"") && sign_up.contains("name=\"inviteCode\""));
        assert!(sign_up.contains("<div id=\"invite-field\""));
        assert!(sign_up.contains("<script nonce=\"n0nce\">(function(){"));
        assert!(!sign_up_credentials(&FLOW, false, "", "", None).contains("name=\"inviteCode\""));
        let handle = sign_up_handle(&FLOW, "p", "alice", ".test", None);
        assert!(handle.contains("name=\"handle\" value=\"alice\""));
        assert!(handle.contains("autofocus><span aria-hidden=\"true\">.test</span></div>"));
    }

    #[test]
    fn an_untrusted_client_is_shown_by_its_identifier_and_text_is_escaped() {
        let scopes = ["atproto", "transition:generic"];
        let mut consent_of = Consent {
            did: "did:plc:abc",
            handle: "alice.test",
            client_name: None,
            scopes: &scopes,
            ephemeral: Some("proof"),
        };
        let untrusted = consent(&FLOW, &consent_of);
        assert!(untrusted.contains(
            "<code>https://app.example.org/client-metadata.json?a=1&amp;b=&quot;2&quot;</code>"
        ));
        assert!(
            untrusted.contains(">Authorize<span class=\"lj-chip\"")
                && untrusted.contains(">Deny access</button>")
        );
        assert!(untrusted.contains("name=\"ephemeral\" value=\"proof\""));
        consent_of.client_name = Some("<b>App</b>");
        let trusted = consent(&FLOW, &consent_of);
        assert!(trusted.contains("<strong>&lt;b&gt;App&lt;/b&gt;</strong>"));
        assert!(!trusted.contains("<code>"));
        assert!(error(SHELL, "Error", "<script>").contains("&lt;script&gt;"));
    }

    #[test]
    fn providers_come_first_and_the_password_form_waits_behind_a_disclosure() {
        let flow = Flow {
            providers: &[("google", "Google"), ("github", "GitHub")],
            ..FLOW
        };
        let page = sign_in(&flow, "", false, None);
        let link = "<a class=\"lj-provider\" data-provider href=\"/oauth/external/google/start?request_uri=urn%3Aietf%3Aparams%3Aoauth%3Arequest_uri%3Areq-00\"><svg";
        assert!(page.contains(link), "{page}");
        assert!(page.contains("<span>Continue with Google</span></a>"));
        assert!(
            page.contains("<details class=\"lj-email-fold\"><summary>Or use email</summary><form")
        );
        assert!(
            page.find("Continue with GitHub").unwrap() < page.find("name=\"password\"").unwrap()
        );
        // An error opens the form it is about.
        assert!(
            sign_in(&flow, "", false, Some("x")).contains("<details class=\"lj-email-fold\" open>")
        );
        // The sign-up page opens on the email form, with the providers under
        // an "or" divider, and the fine print under those.
        let page = sign_up_credentials(&flow, false, "", "", None);
        let email = page.find("name=\"email\"").unwrap();
        let divider = page.find("<p class=\"lj-divider\">or</p>").unwrap();
        let google = page.find("Continue with Google").unwrap();
        let fine = page.find("<p class=\"lj-fine\">By continuing you agree to the Terms of Service and the Privacy Policy.</p>").unwrap();
        assert!(email < divider && divider < google && google < fine);
        assert!(page.contains("<p class=\"lj-hint\">At least 8 characters.</p>"));
        assert!(page.contains(
            "<p>Step 1 of 2</p><div class=\"lj-track\"><i class=\"done\"></i><i></i></div>"
        ));
        let without = sign_in(&FLOW, "", false, None);
        assert!(!without.contains("<details") && !without.contains("class=\"lj-provider\""));
        assert!(!sign_up_credentials(&FLOW, false, "", "", None).contains("class=\"lj-divider\""));
    }

    #[test]
    fn the_brand_draws_the_header_the_fine_print_and_the_footer() {
        let brand = Brand {
            name: "Link<Jar>",
            logo: Some("https://cdn.example/mark.png"),
            home: Some("https://example.org/?a=1&b=2"),
            terms: Some("https://example.org/terms"),
            privacy: None,
            support: Some("https://example.org/help"),
        };
        let flow = Flow { brand, ..FLOW };
        let page = sign_up_credentials(&flow, false, "", "", None);
        assert!(page.contains("<title>Create a new account · Link&lt;Jar&gt;</title>"));
        assert!(page.contains(
            "<header><a class=\"lj-brand\" href=\"https://example.org/?a=1&amp;b=2\"><img src=\"https://cdn.example/mark.png\" alt=\"\">Link&lt;Jar&gt;</a></header>"
        ));
        assert!(page.contains(
            "By continuing you agree to the <a href=\"https://example.org/terms\">Terms of Service</a> and the Privacy Policy.</p>"
        ));
        assert!(page.contains(
            "<nav><a href=\"https://example.org/?a=1&amp;b=2\">Home</a><a href=\"https://example.org/terms\">Terms of Service</a><a href=\"https://example.org/help\">Support</a></nav>"
        ));
        assert!(page.contains("<style nonce=\"n0nce\">@font-face{font-family:Geist;"));
        // Without links there is no header link and no footer navigation.
        let bare = welcome(&FLOW);
        assert!(bare.contains("<header><div class=\"lj-brand\"><img src=\"/oauth/assets/stock-mark.svg\" alt=\"\">pds.test</div></header>"));
        assert!(!bare.contains("<nav>"));
        assert!(bare.contains("<footer class=\"lj-footer\"><span class=\"lj-locale\" lang=\"en\">English</span></footer>"));
    }

    #[test]
    fn the_challenge_sits_on_the_handle_step_in_the_dark_theme() {
        let flow = Flow {
            captcha: Some("site-key"),
            ..FLOW
        };
        let page = sign_up_handle(&flow, "p", "", ".test", None);
        assert!(page.contains("<p>Step 2 of 2</p>"));
        assert!(page.contains(
            "<h2>Verify you are human</h2><div class=\"h-captcha\" data-sitekey=\"site-key\" data-theme=\"dark\"></div><script src=\"https://js.hcaptcha.com/1/api.js\" async defer nonce=\"n0nce\"></script>"
        ));
        assert!(page.contains("Your handle ends with <strong>.test</strong>."));
        assert!(!sign_up_handle(&FLOW, "p", "", ".test", None).contains("class=\"h-captcha\""));
    }

    #[test]
    fn the_account_page_offers_links_and_keeps_the_last_method() {
        let mut methods = Methods {
            has_password: false,
            linked: vec![MethodLine {
                provider: "google".to_owned(),
                label: "Google".to_owned(),
                subject: "g-1".to_owned(),
                email: Some("a@example.com".to_owned()),
            }],
            offered: vec![("github".to_owned(), "GitHub".to_owned())],
            can_unlink: false,
        };
        let page = account_home(SHELL, &account(&methods, None));
        assert!(page.contains("<h2>Sign-in methods</h2>"));
        assert!(page.contains(
            "<button type=\"submit\" class=\"lj-row-action\" disabled>Unlink Google</button>"
        ));
        assert!(page.contains("<small>Connected · a@example.com</small>"));
        assert!(page.contains(
            "<a class=\"lj-row-action\" href=\"/oauth/external/github/start?link=did%3Aplc%3Aabc\">Link GitHub</a>"
        ));
        assert!(page.contains("This is your only sign-in method."));
        assert!(page.contains("<span>Signing in</span><strong>Google</strong>"));
        assert!(page.contains("<small>No password is set.</small>"));
        assert!(page.contains("<script nonce=\"n\">(function(){if(!('IntersectionObserver'"));
        assert!(!page.contains("role=\"alert\""));
        methods.can_unlink = true;
        methods.has_password = true;
        let page = account_home(SHELL, &account(&methods, Some("x")));
        assert!(
            page.contains("<button type=\"submit\" class=\"lj-row-action\">Unlink Google</button>")
        );
        assert!(page.contains("<p class=\"lj-toast\" role=\"alert\">x</p>"));
        assert!(page.contains("<strong>Google and a password</strong>"));
        assert!(!page.contains("This is your only sign-in method."));
        let list = account_list(SHELL, &[("did:plc:abc".to_owned(), "a.test".to_owned())]);
        assert!(list.contains("href=\"/account/did:plc:abc/manage\""));
        assert!(
            account_welcome(SHELL).contains("value=\"sign-in\">Sign in<span class=\"lj-chip\"")
        );
        let form = account_sign_in(SHELL, "c", "", None, &[("apple", "Apple")]);
        assert!(form.contains("name=\"remember\" value=\"true\" checked>"));
        assert!(
            form.contains("<a class=\"lj-provider\" href=\"/oauth/external/apple/start\"><svg")
        );
        assert!(form.contains("<span>Continue with Apple</span></a>"));
        assert!(form.contains("<summary>Or use email</summary>"));
    }

    #[test]
    fn the_account_page_lists_devices_and_apps_with_their_buttons() {
        let methods = Methods {
            has_password: true,
            ..Methods::default()
        };
        let devices = [
            DeviceLine {
                device_id: "dev-1".to_owned(),
                user_agent: "Safari on Mac".to_owned(),
                ip_address: "203.0.113.9".to_owned(),
                last_seen_at: "2026-10-15T12:34:56.789Z".to_owned(),
                current: true,
            },
            DeviceLine {
                device_id: "dev-2".to_owned(),
                user_agent: "Mobile Safari <iPhone>".to_owned(),
                ip_address: "198.51.100.4".to_owned(),
                last_seen_at: "yesterday".to_owned(),
                current: false,
            },
        ];
        let sessions = [SessionLine {
            token_id: "tok-1".to_owned(),
            client_id: "https://app.example.org/client?x=1&y=2".to_owned(),
            created_at: "2026-10-01T08:00:00.000Z".to_owned(),
            updated_at: "2026-10-15T09:30:00.000Z".to_owned(),
        }];
        let page = account_home(
            SHELL,
            &Account {
                sessions: &sessions,
                devices: &devices,
                ..account(&methods, None)
            },
        );
        assert!(
            page.contains("<strong>Safari on Mac <em class=\"lj-tag\">This device</em></strong>")
        );
        assert!(page.contains("<strong>Mobile Safari &lt;iPhone&gt;</strong><small>Address 198.51.100.4 · Last seen yesterday</small>"));
        assert!(page.contains("Last seen 2026-10-15 12:34 UTC"));
        assert!(page.contains("<strong>A password</strong>"));
        assert!(
            page.contains("<strong>2 signed in</strong>")
                && page.contains("<strong>1 allowed</strong>")
        );
        // The other device signs out from its row, this one from the end of the page.
        assert!(page.contains("name=\"device_id\" value=\"dev-2\"><button type=\"submit\" class=\"lj-row-action\">Sign out</button>"));
        assert!(page.contains("name=\"device_id\" value=\"dev-1\"><button type=\"submit\" class=\"lj-secondary\">Sign out</button>"));
        assert!(page.contains("<strong><code>https://app.example.org/client?x=1&amp;y=2</code></strong><small>Since 2026-10-01 08:00 UTC · Last used 2026-10-15 09:30 UTC</small>"));
        assert!(page.contains("name=\"token_id\" value=\"tok-1\"><button type=\"submit\" class=\"lj-row-action\">Remove</button>"));
        assert!(
            page.contains("<p><strong>Personal data server.</strong> <code>pds.test</code></p>")
        );
        let empty = account_home(SHELL, &account(&methods, None));
        assert!(
            empty.contains("No device remembers this account.")
                && empty.contains("No app has access.")
        );
        assert!(empty.contains("<strong>None yet</strong>"));
    }
}
