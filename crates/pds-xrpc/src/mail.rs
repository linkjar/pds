//! Mail (`docs/SPEC.md` §10.3).
//!
//! The five token mails use the templates of the Reference
//! (`mailer/templates/*.hbs` at the pin), filled with the branding of the
//! server. A mail that gates a request is sent before the request is
//! answered, and its failure is the request's failure. Without
//! `PDS_EMAIL_SMTP_URL` nothing is sent and the request succeeds, as at the
//! pin.

use lettre::message::{Mailbox, MultiPart};
use lettre::{AsyncSmtpTransport, AsyncTransport as _, Message, Tokio1Executor};

use crate::error::XrpcError;

/// How the server sends mail and what its mails look like.
#[derive(Clone, Default)]
pub struct MailConfig {
    /// `PDS_EMAIL_SMTP_URL`.
    pub smtp_url: Option<String>,
    /// `PDS_EMAIL_FROM_ADDRESS`.
    pub from_address: Option<String>,
    /// `PDS_SERVICE_NAME`.
    pub service_name: Option<String>,
    /// `PDS_HOME_URL`.
    pub home_url: Option<String>,
    /// `PDS_LOGO_URL`.
    pub logo_url: Option<String>,
    /// `PDS_PRIMARY_COLOR`.
    pub primary_color: Option<String>,
    /// `PDS_EMAIL_DISABLE_CONFIRMATION_LINK`.
    pub disable_confirmation_link: bool,
    /// `PDS_MODERATION_EMAIL_SMTP_URL`.
    pub moderation_smtp_url: Option<String>,
    /// `PDS_MODERATION_EMAIL_ADDRESS`.
    pub moderation_address: Option<String>,
    /// Whether the mails use LinkJar's templates in place of the
    /// Reference's (SPEC §12.7). The server sets it from the profile.
    pub linkjar_templates: bool,
}

impl std::fmt::Debug for MailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The SMTP URL holds a password.
        f.debug_struct("MailConfig")
            .field("configured", &self.smtp_url.is_some())
            .field("from_address", &self.from_address)
            .finish_non_exhaustive()
    }
}

/// One of the token mails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// Confirm an email address.
    ConfirmEmail,
    /// Change an email address.
    UpdateEmail,
    /// Reset a password.
    ResetPassword,
    /// Delete an account.
    DeleteAccount,
    /// Sign a PLC operation.
    PlcOperation,
    /// A sign-in method of the account changed. Only the `linkjar`
    /// profile sends it.
    SignInMethodChange,
}

impl Template {
    /// The template of the Reference, or LinkJar's (SPEC §12.7, patch 101).
    const fn source(self, linkjar: bool) -> &'static str {
        match (self, linkjar) {
            (Self::ConfirmEmail, false) => include_str!("../templates/confirm-email.hbs"),
            (Self::UpdateEmail, false) => include_str!("../templates/update-email.hbs"),
            (Self::ResetPassword, false) => include_str!("../templates/reset-password.hbs"),
            (Self::DeleteAccount, false) => include_str!("../templates/delete-account.hbs"),
            (Self::PlcOperation, false) => include_str!("../templates/plc-operation.hbs"),
            (Self::ConfirmEmail, true) => include_str!("../templates/linkjar/confirm-email.hbs"),
            (Self::UpdateEmail, true) => include_str!("../templates/linkjar/update-email.hbs"),
            (Self::ResetPassword, true) => {
                include_str!("../templates/linkjar/reset-password.hbs")
            }
            (Self::DeleteAccount, true) => {
                include_str!("../templates/linkjar/delete-account.hbs")
            }
            (Self::PlcOperation, true) => include_str!("../templates/linkjar/plc-operation.hbs"),
            (Self::SignInMethodChange, _) => {
                include_str!("../templates/linkjar/sign-in-method-change.hbs")
            }
        }
    }

    /// The subject of the mail.
    pub const fn subject(self) -> &'static str {
        match self {
            Self::ConfirmEmail => "Email Confirmation",
            Self::UpdateEmail => "Email Update Requested",
            Self::ResetPassword => "Password Reset Requested",
            Self::DeleteAccount => "Account Deletion Requested",
            Self::PlcOperation => "PLC Update Operation Requested",
            Self::SignInMethodChange => "sign-in method changed",
        }
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '=' => out.push_str("&#x3D;"),
            other => out.push(other),
        }
    }
    out
}

impl MailConfig {
    fn value(
        &self,
        name: &str,
        token: &str,
        handle: Option<&str>,
        more: &[(&str, &str)],
    ) -> Option<String> {
        const LOGO: &str = "https://bsky.social/about/images/email/email_logo_default.png";
        const MARK: &str = "https://bsky.social/about/images/email/email_mark_dark.png";
        if let Some((_, value)) = more.iter().find(|(key, _)| *key == name) {
            return Some((*value).to_owned());
        }
        Some(match name {
            "token" => token.to_owned(),
            "handle" => handle.unwrap_or_default().to_owned(),
            "config.serviceName" => self
                .service_name
                .clone()
                .unwrap_or_else(|| "Bluesky".to_owned()),
            "config.homeUrl" => self
                .home_url
                .clone()
                .unwrap_or_else(|| "https://bsky.app".to_owned()),
            "config.logoUrl" => self.logo_url.clone().unwrap_or_else(|| LOGO.to_owned()),
            "config.markUrl" => self.logo_url.clone().unwrap_or_else(|| MARK.to_owned()),
            "config.primaryColor" => self
                .primary_color
                .clone()
                .unwrap_or_else(|| "#067df7".to_owned()),
            _ => return None,
        })
    }

    /// The HTML of a mail: the template with its variables filled in and
    /// escaped, and its one condition decided.
    pub fn render(&self, template: Template, token: &str, handle: Option<&str>) -> String {
        self.render_with(template, token, handle, &[])
    }

    /// [`MailConfig::render`] with the values of a template that has more
    /// than a token and a handle.
    pub fn render_with(
        &self,
        template: Template,
        token: &str,
        handle: Option<&str>,
        more: &[(&str, &str)],
    ) -> String {
        let mut out = String::new();
        let mut rest = template.source(self.linkjar_templates);
        let mut hidden = false;
        while let Some(open) = rest.find("{{") {
            let (before, tag) = rest.split_at(open);
            if !hidden {
                out.push_str(before);
            }
            let Some(close) = tag.find("}}") else {
                rest = tag;
                break;
            };
            let name = tag[2..close].trim();
            if let Some(condition) = name.strip_prefix("#if") {
                let shown = condition.trim() != "config.showBskyAppEmailConfirmationLink"
                    || !self.disable_confirmation_link;
                hidden = !shown;
            } else if name == "/if" {
                hidden = false;
            } else if !hidden && let Some(value) = self.value(name, token, handle, more) {
                out.push_str(&escape(&value));
            }
            rest = &tag[close + 2..];
        }
        if !hidden {
            out.push_str(rest);
        }
        out
    }

    /// Sends a mail of the operator to `to`, through the moderation
    /// mailer. `content` is HTML.
    pub async fn send_moderation(
        &self,
        to: &str,
        subject: &str,
        content: &str,
    ) -> Result<(), XrpcError> {
        let Some(url) = &self.moderation_smtp_url else {
            tracing::debug!(subject, "Moderation email auth is not configured");
            return Ok(());
        };
        deliver(
            url,
            self.moderation_address.as_deref().unwrap_or_default(),
            to,
            subject,
            content.to_owned(),
            None,
        )
        .await
    }

    /// Sends a token mail to `to`.
    pub async fn send(
        &self,
        template: Template,
        to: &str,
        token: &str,
        handle: Option<&str>,
    ) -> Result<(), XrpcError> {
        let Some(url) = &self.smtp_url else {
            tracing::debug!(
                subject = template.subject(),
                "No SMTP URL has been configured"
            );
            return Ok(());
        };
        let html = self.render(template, token, handle);
        deliver(
            url,
            self.from_address.as_deref().unwrap_or_default(),
            to,
            template.subject(),
            html,
            None,
        )
        .await
    }

    /// Whether the server can send mail.
    pub const fn can_send(&self) -> bool {
        self.smtp_url.is_some()
    }

    /// Sends the notice of a changed sign-in method (SPEC §12.6). `id`
    /// names the notice: a retry carries the same `Message-ID`, so a
    /// mailbox shows one mail for one change.
    pub async fn send_notice(
        &self,
        id: &str,
        to: &str,
        change: &str,
        did: &str,
        time: &str,
    ) -> Result<(), XrpcError> {
        let Some(url) = &self.smtp_url else {
            return Err(XrpcError::internal("No SMTP URL has been configured"));
        };
        let service = self.service_name.as_deref().unwrap_or("Bluesky");
        let host = self
            .home_url
            .as_deref()
            .and_then(|home| home.split("://").nth(1))
            .and_then(|rest| rest.split(['/', ':']).next())
            .unwrap_or("localhost");
        let html = self.render_with(
            Template::SignInMethodChange,
            "",
            None,
            &[("change", change), ("did", did), ("time", time)],
        );
        deliver(
            url,
            self.from_address.as_deref().unwrap_or_default(),
            to,
            &format!("{service}: {}", Template::SignInMethodChange.subject()),
            html,
            Some(format!("<{id}@{host}>")),
        )
        .await
    }
}

/// Sends one mail with an HTML part and its text.
async fn deliver(
    url: &str,
    from: &str,
    to: &str,
    subject: &str,
    html: String,
    message_id: Option<String>,
) -> Result<(), XrpcError> {
    let text = plain_text(&html);
    let from: Mailbox = from.parse().map_err(XrpcError::internal)?;
    let to: Mailbox = to.parse().map_err(XrpcError::internal)?;
    let mut builder = Message::builder();
    if message_id.is_some() {
        builder = builder.message_id(message_id);
    }
    let message = builder
        .from(from)
        .to(to)
        .subject(subject)
        .multipart(MultiPart::alternative_plain_html(text, html))
        .map_err(XrpcError::internal)?;
    let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
        .map_err(XrpcError::internal)?
        .build();
    transport
        .send(message)
        .await
        .map(drop)
        .map_err(XrpcError::internal)
}

/// The text of an HTML mail, for readers that show no HTML: the text
/// between the tags, with the address of each link after it.
pub fn plain_text(html: &str) -> String {
    let body = html.find("<body").map_or(html, |at| &html[at..]);
    let mut out = String::new();
    let mut rest = body;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        let name = tag
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "p" | "/p" | "br" | "br/" | "/tr" | "/div" | "/h1" | "/h2"
        ) {
            out.push('\n');
        }
        if name == "style" || name == "head" {
            let end = format!("</{name}>");
            rest = rest[open..]
                .find(&end)
                .map_or("", |at| &rest[open + at + end.len()..]);
            continue;
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    let unescaped = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'");
    let lines: Vec<String> = unescaped
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let mut text = String::new();
    let mut blank = true;
    for line in lines {
        if line.is_empty() {
            if !blank {
                text.push('\n');
            }
            blank = true;
        } else {
            text.push_str(&line);
            text.push('\n');
            blank = false;
        }
    }
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_filled_and_its_text_names_the_token() {
        let config = MailConfig {
            service_name: Some("Link<Jar>".to_owned()),
            ..MailConfig::default()
        };
        for template in [
            Template::ConfirmEmail,
            Template::UpdateEmail,
            Template::ResetPassword,
            Template::DeleteAccount,
            Template::PlcOperation,
        ] {
            let html = config.render(template, "ABCDE-FGHIJ", Some("alice.test"));
            assert!(!html.contains("{{"), "{template:?}");
            assert!(html.contains("ABCDE-FGHIJ"), "{template:?}");
            assert!(html.contains("Link&lt;Jar&gt;"), "{template:?}");
            let text = plain_text(&html);
            assert!(text.contains("ABCDE-FGHIJ"), "{template:?}");
            assert!(
                !text.contains('<') || text.contains("Link<Jar>"),
                "{template:?}"
            );
        }
        let with = config.render(Template::ConfirmEmail, "T", None);
        let without = MailConfig {
            disable_confirmation_link: true,
            ..config
        }
        .render(Template::ConfirmEmail, "T", None);
        assert!(with.contains("click here") && !without.contains("click here"));
        assert!(
            MailConfig::default()
                .render(Template::ResetPassword, "T", Some("alice.test"))
                .contains("alice.test")
        );
    }

    #[test]
    fn the_linkjar_profile_has_its_own_templates_and_a_notice() {
        let config = MailConfig {
            service_name: Some("LinkJar".to_owned()),
            home_url: Some("https://linkjar.io".to_owned()),
            linkjar_templates: true,
            ..MailConfig::default()
        };
        let confirm = config.render(Template::ConfirmEmail, "AAAAA-BBBBB", None);
        assert!(confirm.contains("AAAAA-BBBBB"));
        assert!(confirm.contains("https://linkjar.io/logo/linkjar-128.png"));
        let stock = MailConfig::default().render(Template::ConfirmEmail, "AAAAA-BBBBB", None);
        assert!(!stock.contains("linkjar"));

        let notice = config.render_with(
            Template::SignInMethodChange,
            "",
            None,
            &[
                ("change", "Google was linked to your account."),
                ("did", "did:plc:abc"),
                ("time", "2026-10-13T00:00:00.000Z"),
            ],
        );
        for part in [
            "Your sign-in method changed",
            "Google was linked to your account.",
            "did:plc:abc",
            "2026-10-13T00:00:00.000Z",
        ] {
            assert!(notice.contains(part), "{part}");
        }
        assert!(!notice.contains("{{"));
        assert!(plain_text(&notice).contains("Google was linked to your account."));
    }
}
