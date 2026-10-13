//! The names that LinkJar reserves under `.linkjar.social`, on top of the
//! upstream list (`handle/linkjar-reserved.ts` of patch 093).
//!
//! The list covers infrastructure and brand names. The check also covers
//! their digit lookalikes and their numbered and hyphenated variants. It
//! is a bounded list, not a detector of every deceptive spelling.

/// The reserved labels, sorted.
pub const LABELS: &[&str] = &[
    "account",
    "accounts",
    "admin",
    "api",
    "app",
    "assets",
    "atproto",
    "auth",
    "autoconfig",
    "autodiscover",
    "backup",
    "backups",
    "beta",
    "billing",
    "blob",
    "blobs",
    "blog",
    "bluesky",
    "bsky",
    "cdn",
    "dev",
    "dns",
    "docs",
    "download",
    "downloads",
    "events",
    "ftp",
    "git",
    "github",
    "grafana",
    "health",
    "help",
    "id",
    "images",
    "imap",
    "indexer",
    "jar",
    "linkjar",
    "login",
    "m",
    "mail",
    "media",
    "metrics",
    "mobile",
    "monitor",
    "mta-sts",
    "mx",
    "ns",
    "oauth",
    "pay",
    "payments",
    "pds",
    "plc",
    "pop",
    "preview",
    "profile",
    "profiles",
    "r2",
    "register",
    "relay",
    "search",
    "secure",
    "security",
    "settings",
    "sftp",
    "signin",
    "signup",
    "smtp",
    "ssh",
    "sso",
    "stage",
    "staging",
    "static",
    "status",
    "storage",
    "support",
    "tap",
    "test",
    "verification",
    "verify",
    "vpn",
    "web",
    "webmail",
    "workbench",
    "www",
    "www1",
];

fn listed(label: &str) -> bool {
    LABELS.binary_search(&label).is_ok()
}

/// The label with each digit that stands for a letter replaced by the
/// letter. `one` is the letter that `1` stands for.
fn skeleton(label: &str, one: char) -> String {
    label
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => one,
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            c => c,
        })
        .collect()
}

/// Whether the label, the part of a handle before `.linkjar.social`, is
/// reserved. A name that only contains a reserved word, such as `olivia`
/// or `jared`, is free.
pub fn is_reserved_label(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    [lower.clone(), skeleton(&lower, 'i'), skeleton(&lower, 'l')]
        .iter()
        .any(|value| {
            listed(value)
                || listed(&value.replace('-', ""))
                || value.split('-').any(listed)
                || listed(value.trim_end_matches(|c: char| c.is_ascii_digit()))
                || value.starts_with("xn--")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_has_no_duplicate() {
        assert!(LABELS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(LABELS.len(), 86);
    }

    #[test]
    fn service_names_and_their_lookalikes_are_reserved() {
        for label in [
            "linkjar", "pds", "api", "support", "status", "pds2", "api-prod", "link-jar",
            "1inkjar", "l1nkj4r", "supp0rt", "LinkJar", "xn--abc", "www1", "mta-sts",
        ] {
            assert!(is_reserved_label(label), "{label}");
        }
        for label in [
            "olivia",
            "jared",
            "jared-5f3fb5",
            "reader",
            "nova",
            "apiary",
        ] {
            assert!(!is_reserved_label(label), "{label}");
        }
    }
}
