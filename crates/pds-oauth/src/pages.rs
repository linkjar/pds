//! The authorization pages (`docs/SPEC.md` §11.4), rendered on the server.
//!
//! A page is one HTML document with its style inline under a nonce. It
//! works without scripts. `parity/README.md` states what a page must keep
//! for the Harness: the field names, the button names and the `alert` role
//! of an error.

use std::fmt::Write;

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

/// What every page of one authorization carries.
#[derive(Debug, Clone, Copy)]
pub struct Flow<'a> {
    /// The name of the service, shown in the heading.
    pub service: &'a str,
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

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fff;--fg:#111827;--muted:#4b5563;--line:#d1d5db;--brand:#1d4ed8;--brand-fg:#fff;--bad:#b91c1c;--bad-bg:#fef2f2}\
@media(prefers-color-scheme:dark){:root{--bg:#0b1220;--fg:#f3f4f6;--muted:#cbd5e1;--line:#475569;--brand:#93c5fd;--brand-fg:#0b1220;--bad:#fecaca;--bad-bg:#450a0a}}\
*{box-sizing:border-box}\
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
main{max-width:26rem;margin:0 auto;padding:2rem 1rem}\
h1{font-size:1.4rem;margin:0 0 .25rem}\
h2{font-size:1.1rem;margin:1.5rem 0 .25rem}\
section{border-top:1px solid var(--line);padding:.75rem 0}\
p{margin:.5rem 0;color:var(--muted)}\
label{display:block;margin:1rem 0 .25rem;font-weight:600}\
input[type=text],input[type=email],input[type=password]{width:100%;padding:.65rem .75rem;border:1px solid var(--line);border-radius:.5rem;background:var(--bg);color:var(--fg);font:inherit}\
input[readonly]{color:var(--muted)}\
.check{display:flex;gap:.5rem;align-items:center;margin:1rem 0;font-weight:400}\
.row{display:flex;flex-direction:column;gap:.5rem;margin-top:1.25rem}\
button{width:100%;padding:.7rem 1rem;border-radius:.5rem;border:1px solid var(--line);background:var(--bg);color:var(--fg);font:inherit;font-weight:600;cursor:pointer}\
button.primary{background:var(--brand);border-color:var(--brand);color:var(--brand-fg)}\
a.button{display:block;padding:.7rem 1rem;border-radius:.5rem;border:1px solid var(--line);color:var(--fg);font-weight:600;text-align:center;text-decoration:none}\
summary{margin:1.25rem 0 .25rem;font-weight:600;cursor:pointer}\
button:focus-visible,input:focus-visible{outline:3px solid var(--brand);outline-offset:2px}\
[role=alert]{margin:1rem 0;padding:.75rem;border:1px solid var(--bad);border-radius:.5rem;background:var(--bad-bg);color:var(--bad)}\
code{word-break:break-all;font-size:.9rem}\
ul{padding-left:1.25rem;color:var(--muted)}";

fn document(nonce: &str, title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<meta name=\"robots\" content=\"noindex\"><title>{}</title>\
<style nonce=\"{}\">{STYLE}</style></head><body><main>{body}</main></body></html>",
        escape(title),
        escape(nonce),
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
fn view_form(flow: &Flow<'_>, buttons: &[(&str, &str, bool)]) -> String {
    let mut out = format!(
        "<form method=\"get\" action=\"/oauth/authorize\">\
<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
<input type=\"hidden\" name=\"request_uri\" value=\"{}\"><div class=\"row\">",
        escape(flow.client_id),
        escape(flow.request_uri),
    );
    for (view, label, primary) in buttons {
        let class = if *primary { " class=\"primary\"" } else { "" };
        let _ = write!(
            out,
            "<button type=\"submit\" name=\"view\" value=\"{}\"{class}>{}</button>",
            escape(view),
            escape(label)
        );
    }
    out.push_str("</div></form>");
    out
}

/// The buttons of the identity providers: each a link that starts the
/// sign-in for this request.
fn provider_buttons(flow: &Flow<'_>) -> String {
    let mut out = String::new();
    for (id, label) in flow.providers {
        let query = format!(
            "request_uri={}",
            escape(&flow.request_uri.replace(':', "%3A"))
        );
        let _ = write!(
            out,
            "<a class=\"button\" href=\"/oauth/external/{}/start?{query}\">Continue with {}</a>",
            escape(id),
            escape(label)
        );
    }
    if out.is_empty() {
        out
    } else {
        format!("<div class=\"row\">{out}</div>")
    }
}

fn alert(error: Option<&str>) -> String {
    error.map_or_else(String::new, |error| {
        format!("<div role=\"alert\">{}</div>", escape(error))
    })
}

/// The first page of an authorization that names no account: sign in or
/// sign up.
pub fn welcome(flow: &Flow<'_>) -> String {
    let body = format!(
        "<h1>{}</h1><p>Please authenticate to continue</p>{}",
        escape(flow.service),
        view_form(
            flow,
            &[
                ("sign-in", "Sign in", true),
                ("sign-up", "Create a new account", false)
            ]
        ),
    );
    document(flow.nonce, "Authenticate", &body)
}

/// The sign-in form. `locked` holds the account the request names, which
/// the form does not let the user change.
pub fn sign_in(flow: &Flow<'_>, username: &str, locked: bool, error: Option<&str>) -> String {
    let readonly = if locked { " readonly" } else { " autofocus" };
    let focus = if locked { " autofocus" } else { "" };
    let form = format!(
        "<form method=\"post\" action=\"/oauth/authorize/sign-in\">{}\
<label for=\"username\">Handle or email address</label>\
<input id=\"username\" type=\"text\" name=\"username\" value=\"{}\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" required{readonly}>\
<label for=\"password\">Password</label>\
<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"current-password\" required{focus}>\
<label class=\"check\"><input type=\"checkbox\" name=\"remember\" value=\"true\"> Remember this account on this device</label>\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Sign in</button></div></form>",
        hidden(flow),
        escape(username),
    );
    // With identity providers the password form waits behind a disclosure.
    // It is open when the form has something to say.
    let providers = provider_buttons(flow);
    let form = if providers.is_empty() {
        form
    } else {
        let open = if error.is_some() { " open" } else { "" };
        format!("{providers}<details{open}><summary>Or use email</summary>{form}</details>")
    };
    let body = format!(
        "<h1>Sign in</h1><p>Sign in to your {} account.</p>{}{form}{}",
        escape(flow.service),
        alert(error),
        view_form(flow, &[("welcome", "Back", false)]),
    );
    document(flow.nonce, "Sign in", &body)
}

/// The accounts the device remembers, each as a button.
pub fn accounts(flow: &Flow<'_>, accounts: &[(String, String)]) -> String {
    let mut body = format!(
        "<h1>Choose an account</h1><p>Continue to the application with one of your accounts.</p>\
<form method=\"get\" action=\"/oauth/authorize\">\
<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
<input type=\"hidden\" name=\"request_uri\" value=\"{}\"><div class=\"row\">",
        escape(flow.client_id),
        escape(flow.request_uri),
    );
    for (did, handle) in accounts {
        let _ = write!(
            body,
            "<button type=\"submit\" name=\"did\" value=\"{}\">{}</button>",
            escape(did),
            escape(handle)
        );
    }
    body.push_str("</div></form>");
    body.push_str(&view_form(
        flow,
        &[
            ("sign-in", "Use another account", false),
            ("sign-up", "Create a new account", false),
        ],
    ));
    document(flow.nonce, "Choose an account", &body)
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
    let body = format!(
        "<h1>Authorize access</h1><p>{client} asks for access to your account <strong>{}</strong>.</p>\
<p>The application will be able to:</p><ul>{scopes}</ul>\
<form method=\"post\" action=\"/oauth/authorize/accept\">{}\
<input type=\"hidden\" name=\"did\" value=\"{}\">{ephemeral}\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Authorize</button>\
<button type=\"submit\" formaction=\"/oauth/authorize/reject\">Deny access</button></div></form>",
        escape(consent.handle),
        hidden(flow),
        escape(consent.did),
    );
    document(flow.nonce, "Authorize access", &body)
}

/// The first step of a sign-up: the email address and the password.
pub fn sign_up_credentials(
    flow: &Flow<'_>,
    invite_required: bool,
    email: &str,
    invite_code: &str,
    error: Option<&str>,
) -> String {
    let invite = if invite_required {
        format!(
            "<label for=\"inviteCode\">Invite code</label>\
<input id=\"inviteCode\" type=\"text\" name=\"inviteCode\" value=\"{}\" autocapitalize=\"none\" spellcheck=\"false\" required>",
            escape(invite_code)
        )
    } else {
        String::new()
    };
    let body = format!(
        "<h1>Create a new account</h1><p>Your account lives on {}.</p>{}\
<form method=\"post\" action=\"/oauth/authorize/sign-up\">{}{invite}\
<label for=\"email\">Email address</label>\
<input id=\"email\" type=\"email\" name=\"email\" value=\"{}\" autocomplete=\"email\" required>\
<label for=\"password\">Password</label>\
<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"new-password\" minlength=\"8\" required>\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Next</button></div></form>{}{}",
        escape(flow.service),
        alert(error),
        hidden(flow),
        escape(email),
        provider_buttons(flow),
        view_form(flow, &[("welcome", "Back", false)]),
    );
    document(flow.nonce, "Create a new account", &body)
}

/// The second step of a sign-up: the handle. `pending` names the first
/// step, which the server keeps.
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
            "<div class=\"h-captcha\" data-sitekey=\"{}\"></div>\
<script src=\"https://js.hcaptcha.com/1/api.js\" async defer nonce=\"{}\"></script>",
            escape(site_key),
            escape(flow.nonce)
        )
    });
    let body = format!(
        "<h1>Choose your handle</h1><p>Your handle is your name on the network. It ends with <strong>{}</strong>.</p>{}\
<form method=\"post\" action=\"/oauth/authorize/sign-up\">{}\
<input type=\"hidden\" name=\"pending\" value=\"{}\">\
<label for=\"handle\">Handle</label>\
<input id=\"handle\" type=\"text\" name=\"handle\" value=\"{}\" autocapitalize=\"none\" spellcheck=\"false\" required autofocus>{captcha}\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Create account</button></div></form>",
        escape(domain),
        alert(error),
        hidden(flow),
        escape(pending),
        escape(handle),
    );
    document(flow.nonce, "Choose your handle", &body)
}

/// An OAuth session on the account page.
#[derive(Debug, Clone)]
pub struct SessionLine {
    /// The `jti` of the session's access token, which the "Revoke" button
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

fn account_form(action: &str, csrf: &str, fields: &[(&str, &str)], label: &str) -> String {
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
        "<button type=\"submit\">{}</button></form>",
        escape(label)
    );
    out
}

/// The first page of the account pages for a device that no account is
/// signed in on.
pub fn account_welcome(nonce: &str, service: &str) -> String {
    let body = format!(
        "<h1>Your {} account</h1><p>Sign in to manage your sign-in methods, your sessions and your devices.</p>\
<form method=\"get\" action=\"/account\"><div class=\"row\">\
<button type=\"submit\" name=\"view\" value=\"sign-in\" class=\"primary\">Sign in</button></div></form>",
        escape(service)
    );
    document(nonce, "Your account", &body)
}

/// The sign-in form of the account pages. `providers` are the identity
/// providers a person can continue with: the name in the path and the
/// name on the button.
pub fn account_sign_in(
    nonce: &str,
    service: &str,
    csrf: &str,
    username: &str,
    error: Option<&str>,
    providers: &[(&str, &str)],
) -> String {
    let form = format!(
        "<form method=\"post\" action=\"/account/sign-in\">\
<input type=\"hidden\" name=\"csrf\" value=\"{}\">\
<label for=\"username\">Handle or email address</label>\
<input id=\"username\" type=\"text\" name=\"username\" value=\"{}\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" required>\
<label for=\"password\">Password</label>\
<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"current-password\" required>\
<label class=\"check\"><input type=\"checkbox\" name=\"remember\" value=\"true\" checked> Remember this account on this device</label>\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Sign in</button></div></form>",
        escape(csrf),
        escape(username),
    );
    let mut buttons = String::new();
    for (id, label) in providers {
        let _ = write!(
            buttons,
            "<a class=\"button\" href=\"/oauth/external/{}/start\">Continue with {}</a>",
            escape(id),
            escape(label)
        );
    }
    let form = if buttons.is_empty() {
        form
    } else {
        let open = if error.is_some() { " open" } else { "" };
        format!(
            "<div class=\"row\">{buttons}</div><details{open}><summary>Or use email</summary>{form}</details>"
        )
    };
    let body = format!(
        "<h1>Sign in</h1><p>Sign in to your {} account.</p>{}{form}",
        escape(service),
        alert(error),
    );
    document(nonce, "Sign in", &body)
}

/// The accounts that are signed in on the device, each with a link to its
/// page.
pub fn account_list(nonce: &str, accounts: &[(String, String)]) -> String {
    let mut body = String::from("<h1>Your accounts</h1>");
    for (did, handle) in accounts {
        let _ = write!(
            body,
            "<section><p><strong>{}</strong></p><a class=\"button\" href=\"/account/{}/manage\">Manage this account</a></section>",
            escape(handle),
            escape(did)
        );
    }
    body.push_str(
        "<form method=\"get\" action=\"/account\"><div class=\"row\">\
<button type=\"submit\" name=\"view\" value=\"sign-in\">Sign in to another account</button></div></form>",
    );
    document(nonce, "Your accounts", &body)
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

fn methods_section(csrf: &str, did: &str, methods: &Methods) -> String {
    let mut out = String::from("<h2>Sign-in methods</h2>");
    let _ = write!(
        out,
        "<p>{}</p>",
        if methods.has_password {
            "A password is set."
        } else {
            "No password is set."
        }
    );
    for method in &methods.linked {
        let disabled = if methods.can_unlink { "" } else { " disabled" };
        let _ = write!(
            out,
            "<section><p>{}{}</p><form method=\"post\" action=\"/account/unlink\">\
<input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"did\" value=\"{}\">\
<input type=\"hidden\" name=\"provider\" value=\"{}\"><input type=\"hidden\" name=\"subject\" value=\"{}\">\
<button type=\"submit\"{disabled}>Unlink {}</button></form></section>",
            escape(&method.label),
            method
                .email
                .as_deref()
                .map_or_else(String::new, |email| format!("<br>{}", escape(email))),
            escape(csrf),
            escape(did),
            escape(&method.provider),
            escape(&method.subject),
            escape(&method.label),
        );
    }
    if !methods.linked.is_empty() && !methods.can_unlink {
        out.push_str(
            "<p>This is your only sign-in method. Add another one before you remove it.</p>",
        );
    }
    if !methods.offered.is_empty() {
        out.push_str("<div class=\"row\">");
        for (id, label) in &methods.offered {
            let _ = write!(
                out,
                "<a class=\"button\" href=\"/oauth/external/{}/start?link={}\">Link {}</a>",
                escape(id),
                escape(&did.replace(':', "%3A")),
                escape(label)
            );
        }
        out.push_str("</div>");
    }
    out
}

/// The page of one account: its sign-in methods, the applications with a
/// session, and the devices that remember the account.
#[allow(clippy::too_many_arguments)] // one argument for each part of the page
pub fn account_home(
    nonce: &str,
    csrf: &str,
    did: &str,
    handle: &str,
    methods: &Methods,
    sessions: &[SessionLine],
    devices: &[DeviceLine],
    error: Option<&str>,
) -> String {
    let mut body = format!(
        "<h1>{}</h1><p><code>{}</code></p>{}{}<h2>Applications</h2>",
        escape(handle),
        escape(did),
        alert(error),
        methods_section(csrf, did, methods),
    );
    if sessions.is_empty() {
        body.push_str("<p>No application has a session.</p>");
    }
    for session in sessions {
        let _ = write!(
            body,
            "<section><p><code>{}</code><br>Since {}. Last used {}.</p>{}</section>",
            escape(&session.client_id),
            escape(&session.created_at),
            escape(&session.updated_at),
            account_form(
                "/account/revoke",
                csrf,
                &[("did", did), ("token_id", &session.token_id)],
                "Revoke access"
            ),
        );
    }
    body.push_str("<h2>Devices</h2>");
    for device in devices {
        let this = if device.current { " (this device)" } else { "" };
        let _ = write!(
            body,
            "<section><p>{}{this}<br>Address {}. Last seen {}.</p>{}</section>",
            escape(&device.user_agent),
            escape(&device.ip_address),
            escape(&device.last_seen_at),
            account_form(
                "/account/sign-out",
                csrf,
                &[("did", did), ("device_id", &device.device_id)],
                "Sign out"
            ),
        );
    }
    body.push_str("<p><a href=\"/account\">All accounts</a></p>");
    document(nonce, "Your account", &body)
}

/// A page that ends a flow the server cannot return to the client.
pub fn error(nonce: &str, title: &str, message: &str) -> String {
    let body = format!(
        "<h1>{}</h1><div role=\"alert\">{}</div>",
        escape(title),
        escape(message)
    );
    document(nonce, title, &body)
}

/// The page of the `form_post` response mode: a form that carries the
/// answer to the client.
pub fn form_post(nonce: &str, action: &str, fields: &[(&str, &str)]) -> String {
    let mut inputs = String::new();
    for (name, value) in fields {
        let _ = write!(
            inputs,
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        );
    }
    let body = format!(
        "<h1>Return to the application</h1><form method=\"post\" action=\"{}\">{inputs}\
<div class=\"row\"><button type=\"submit\" class=\"primary\">Continue</button></div></form>",
        escape(action)
    );
    document(nonce, "Return to the application", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: Flow<'static> = Flow {
        service: "pds.test",
        nonce: "n0nce",
        client_id: "https://app.example.org/client-metadata.json?a=1&b=\"2\"",
        request_uri: "urn:ietf:params:oauth:request_uri:req-00",
        csrf: "token",
        providers: &[],
        captcha: None,
    };

    #[test]
    fn pages_keep_the_contract_of_the_harness() {
        let welcome = welcome(&FLOW);
        assert!(welcome.contains("Please authenticate to continue"));
        assert!(welcome.contains(">Sign in</button>"));
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
        assert!(sign_in[submit..].starts_with("<button type=\"submit\" class=\"primary\">Sign in"));

        let sign_up = sign_up_credentials(&FLOW, true, "", "", None);
        assert!(sign_up.contains("name=\"email\"") && sign_up.contains("name=\"inviteCode\""));
        assert!(!sign_up_credentials(&FLOW, false, "", "", None).contains("inviteCode"));
        assert!(
            sign_up_handle(&FLOW, "p", "alice", ".test", None)
                .contains("name=\"handle\" value=\"alice\"")
        );
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
            untrusted.contains(">Authorize</button>")
                && untrusted.contains(">Deny access</button>")
        );
        assert!(untrusted.contains("name=\"ephemeral\" value=\"proof\""));
        consent_of.client_name = Some("<b>App</b>");
        let trusted = consent(&FLOW, &consent_of);
        assert!(trusted.contains("<strong>&lt;b&gt;App&lt;/b&gt;</strong>"));
        assert!(!trusted.contains("<code>"));
        assert!(error("n", "Error", "<script>").contains("&lt;script&gt;"));
    }

    #[test]
    fn providers_come_first_and_the_password_form_waits_behind_a_disclosure() {
        let flow = Flow {
            providers: &[("google", "Google"), ("github", "GitHub")],
            ..FLOW
        };
        let page = sign_in(&flow, "", false, None);
        let link = "href=\"/oauth/external/google/start?request_uri=urn%3Aietf%3Aparams%3Aoauth%3Arequest_uri%3Areq-00\">Continue with Google</a>";
        assert!(page.contains(link), "{page}");
        assert!(page.contains("<details><summary>Or use email</summary><form"));
        assert!(
            page.find("Continue with GitHub").unwrap() < page.find("name=\"password\"").unwrap()
        );
        // An error opens the form it is about.
        assert!(sign_in(&flow, "", false, Some("x")).contains("<details open>"));
        // The sign-up page opens on the email form, with the providers under it.
        let page = sign_up_credentials(&flow, false, "", "", None);
        assert!(page.find("name=\"email\"").unwrap() < page.find("Continue with Google").unwrap());
        assert!(!sign_in(&FLOW, "", false, None).contains("<details"));
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
        let page = account_home(
            "n",
            "csrf",
            "did:plc:abc",
            "a.test",
            &methods,
            &[],
            &[],
            None,
        );
        assert!(page.contains("<h2>Sign-in methods</h2>"));
        assert!(page.contains("<button type=\"submit\" disabled>Unlink Google</button>"));
        assert!(page.contains(
            "href=\"/oauth/external/github/start?link=did%3Aplc%3Aabc\">Link GitHub</a>"
        ));
        methods.can_unlink = true;
        let page = account_home(
            "n",
            "csrf",
            "did:plc:abc",
            "a.test",
            &methods,
            &[],
            &[],
            Some("x"),
        );
        assert!(page.contains("<button type=\"submit\">Unlink Google</button>"));
        assert!(page.contains("<div role=\"alert\">x</div>"));
        let list = account_list("n", &[("did:plc:abc".to_owned(), "a.test".to_owned())]);
        assert!(list.contains("href=\"/account/did:plc:abc/manage\""));
        assert!(account_welcome("n", "pds.test").contains(">Sign in</button>"));
        let form = account_sign_in("n", "pds.test", "c", "", None, &[("apple", "Apple")]);
        assert!(form.contains("name=\"remember\""));
        assert!(form.contains("href=\"/oauth/external/apple/start\">Continue with Apple</a>"));
    }
}
