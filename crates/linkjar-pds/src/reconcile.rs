//! `reconcile` (SPEC §21.3): the accounts of an entryway against the
//! repositories of its member host.
//!
//! A sign-up has three steps on two servers: the member host reserves a
//! signing key, creates the repository, and the entryway keeps the
//! account. A crash after the second step leaves a repository without an
//! account. This command finds such repositories, and accounts without a
//! repository. It removes a repository only when the operator asks, and
//! never one with a commit of the last ten minutes: that may be a sign-up
//! in progress.

use std::collections::BTreeSet;
use std::fmt;

use crate::config::Config;

/// A repository younger than this may belong to a sign-up in progress.
const YOUNG_MICROS: u64 = 10 * 60 * 1_000_000;

/// What the two servers disagree on.
#[derive(Debug, Default)]
pub struct Report {
    /// Repositories of the member host that no account of the entryway
    /// names.
    pub only_at_member: Vec<String>,
    /// Accounts of the entryway without a repository on the member host.
    pub only_at_entryway: Vec<String>,
    /// Of `only_at_member`, the repositories that the command removed.
    pub removed: Vec<String>,
    /// Of `only_at_member`, the repositories too young to remove.
    pub young: Vec<String>,
}

impl Report {
    /// Whether the two servers agree.
    pub fn is_clean(&self) -> bool {
        self.only_at_member.is_empty() && self.only_at_entryway.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for did in &self.only_at_member {
            let note = if self.removed.contains(did) {
                " (removed)"
            } else if self.young.contains(did) {
                " (a commit of the last ten minutes: kept)"
            } else {
                ""
            };
            writeln!(f, "repository without an account: {did}{note}")?;
        }
        for did in &self.only_at_entryway {
            writeln!(f, "account without a repository: {did}")?;
        }
        if self.is_clean() {
            writeln!(f, "the entryway and the member host agree")?;
        }
        Ok(())
    }
}

/// The error of the command, as a message.
type Failure = Box<dyn std::error::Error>;

/// Compares the two servers, and removes the repositories without an
/// account when `remove` is set. Runs at an entryway.
pub async fn reconcile(config: &Config, remove: bool) -> Result<Report, Failure> {
    let now_micros = u64::try_from(crate::serve::now_ms()).unwrap_or(0) * 1000;
    reconcile_at(config, remove, now_micros).await
}

/// [`reconcile`] at the time `now_micros`, which decides what is young.
pub async fn reconcile_at(
    config: &Config,
    remove: bool,
    now_micros: u64,
) -> Result<Report, Failure> {
    let host = config
        .member_host
        .as_ref()
        .ok_or("reconcile runs at an entryway: PDS_MEMBER_URL is not set")?;
    let accounts: BTreeSet<String> = config
        .layout
        .accounts()?
        .all_actors()?
        .into_iter()
        .map(|actor| actor.did)
        .collect();
    let http = reqwest::Client::new();
    let repos = member_repos(&http, &host.internal_url).await?;
    let mut report = Report::default();
    for (did, rev) in &repos {
        if accounts.contains(did) {
            continue;
        }
        report.only_at_member.push(did.clone());
        let young = rev
            .parse::<pds_types::Tid>()
            .is_ok_and(|rev| now_micros.saturating_sub(rev.timestamp_micros()) < YOUNG_MICROS);
        if young {
            report.young.push(did.clone());
        } else if remove {
            remove_repo(&http, host, did).await?;
            report.removed.push(did.clone());
        }
    }
    let held: BTreeSet<&String> = repos.iter().map(|(did, _)| did).collect();
    report.only_at_entryway = accounts
        .iter()
        .filter(|did| !held.contains(did))
        .cloned()
        .collect();
    Ok(report)
}

/// Every repository of the member host as `(did, rev)`, from
/// `com.atproto.sync.listRepos`.
async fn member_repos(
    http: &reqwest::Client,
    internal_url: &str,
) -> Result<Vec<(String, String)>, Failure> {
    let mut repos = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = format!("{internal_url}/xrpc/com.atproto.sync.listRepos?limit=1000");
        // The cursor is a DID: it needs no escaping, and anything else is
        // refused.
        if let Some(cursor) = &cursor {
            if !cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"%:._-".contains(&b))
            {
                return Err(
                    format!("the member host sent a cursor that is no DID: {cursor}").into(),
                );
            }
            url.push_str("&cursor=");
            url.push_str(cursor);
        }
        let answer = http.get(&url).send().await?;
        if !answer.status().is_success() {
            return Err(
                format!("the member host answered {} to listRepos", answer.status()).into(),
            );
        }
        let page: serde_json::Value = serde_json::from_slice(&answer.bytes().await?)?;
        let entries = page["repos"].as_array().cloned().unwrap_or_default();
        for entry in &entries {
            if let (Some(did), Some(rev)) = (entry["did"].as_str(), entry["rev"].as_str()) {
                repos.push((did.to_owned(), rev.to_owned()));
            }
        }
        match page["cursor"].as_str() {
            Some(next) if !entries.is_empty() => cursor = Some(next.to_owned()),
            _ => return Ok(repos),
        }
    }
}

/// Removes a repository at the member host, as the entryway's own call so
/// that the host does not pass it back.
async fn remove_repo(
    http: &reqwest::Client,
    host: &crate::config::MemberHostConfig,
    did: &str,
) -> Result<(), Failure> {
    let answer = http
        .post(format!(
            "{}/xrpc/com.atproto.admin.deleteAccount",
            host.internal_url
        ))
        .basic_auth("admin", Some(&host.admin_password))
        .header(pds_xrpc::api::entryway::FROM_ENTRYWAY, "1")
        .header("content-type", "application/json")
        .body(serde_json::json!({ "did": did }).to_string())
        .send()
        .await?;
    if answer.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "the member host answered {} to the removal of {did}",
            answer.status()
        )
        .into())
    }
}
