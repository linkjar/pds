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
}

impl Template {
    const fn source(self) -> &'static str {
        match self {
            Self::ConfirmEmail => include_str!("../templates/confirm-email.hbs"),
            Self::UpdateEmail => include_str!("../templates/update-email.hbs"),
            Self::ResetPassword => include_str!("../templates/reset-password.hbs"),
            Self::DeleteAccount => include_str!("../templates/delete-account.hbs"),
            Self::PlcOperation => include_str!("../templates/plc-operation.hbs"),
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
    fn value(&self, name: &str, token: &str, handle: Option<&str>) -> Option<String> {
        const LOGO: &str = "https://bsky.social/about/images/email/email_logo_default.png";
        const MARK: &str = "https://bsky.social/about/images/email/email_mark_dark.png";
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
        let mut out = String::new();
        let mut rest = template.source();
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
            } else if !hidden && let Some(value) = self.value(name, token, handle) {
                out.push_str(&escape(&value));
            }
            rest = &tag[close + 2..];
        }
        if !hidden {
            out.push_str(rest);
        }
        out
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
        let text = plain_text(&html);
        let from: Mailbox = self
            .from_address
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(XrpcError::internal)?;
        let to: Mailbox = to.parse().map_err(XrpcError::internal)?;
        let message = Message::builder()
            .from(from)
            .to(to)
            .subject(template.subject())
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
}
