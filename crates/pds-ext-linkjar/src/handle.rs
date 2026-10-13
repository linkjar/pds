//! Hosted handles of the `linkjar` profile (patch 093, `docs/SPEC.md`
//! §12.4 and §13): the handle a new provider account gets, and the one
//! hosted rename that it may make.

use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};
use pds_identity::reserved;
use pds_types::Handle;

use crate::reserved::is_reserved_label;

/// The hosted domain of the profile.
pub const DOMAIN: &str = ".linkjar.social";
/// A new provider account may choose one new hosted handle for 30 days.
pub const RENAME_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// How often the server draws a suffix for a name that is taken.
pub const ATTEMPTS: usize = 32;

/// Whether `handle` is a hosted handle of the profile: one label of 3 to
/// 18 letters, digits and hyphens under [`DOMAIN`].
pub fn is_linkjar_handle(handle: &str) -> bool {
    handle.strip_suffix(DOMAIN).is_some_and(|label| {
        let bytes = label.as_bytes();
        (3..=18).contains(&bytes.len())
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            && bytes.first() != Some(&b'-')
            && bytes.last() != Some(&b'-')
    })
}

/// A name as a label: compatibility decomposition, marks removed, lower
/// case, every run of other characters as one hyphen, at most 18
/// characters, no hyphen at either end.
pub fn normalized_label(name: &str) -> String {
    let marks = CodePointMapData::<GeneralCategory>::new();
    let head: String = name.chars().take(512).collect();
    let decomposed = DecomposingNormalizerBorrowed::new_nfkd().normalize(&head);
    let mut label = String::new();
    let mut gap = false;
    for c in decomposed.chars() {
        if GeneralCategoryGroup::Mark.contains(marks.get(c)) {
            continue;
        }
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap {
                label.push('-');
            }
            gap = false;
            label.push(c);
        } else {
            // A character outside ASCII lower case counts as a separator,
            // also one that has a lower-case form of its own.
            gap = true;
        }
    }
    if gap {
        label.push('-');
    }
    let cut: String = label.chars().take(18).collect();
    cut.trim_matches('-').to_owned()
}

/// Whether a label may be a hosted handle: the syntax of a handle, the
/// length of a hosted label, and neither list of reserved names.
pub fn allowed(label: &str) -> bool {
    let handle = format!("{label}{DOMAIN}");
    Handle::parse(&handle).is_ok()
        && !label.contains('.')
        && (3..=18).contains(&label.len())
        && !reserved::is_reserved(label)
        && !is_reserved_label(label)
}

/// The base of the handle of a new account: the display name, then the
/// local part of the email address, then `reader`.
pub fn base_label(display_name: Option<&str>, email: Option<&str>) -> String {
    [
        display_name,
        email.and_then(|email| email.split('@').next()),
        Some("reader"),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .map(normalized_label)
    .find(|label| allowed(label))
    .unwrap_or_else(|| "reader".to_owned())
}

/// The label of an attempt. The first is the base. Each later one is the
/// first 11 characters of the base and six hexadecimal digits, which
/// `suffix` gives.
pub fn attempt_label(base: &str, attempt: usize, suffix: [u8; 3]) -> String {
    if attempt == 0 {
        return base.to_owned();
    }
    let head: String = base.chars().take(11).collect();
    format!(
        "{}-{:02x}{:02x}{:02x}",
        head.trim_end_matches('-'),
        suffix[0],
        suffix[1],
        suffix[2]
    )
}

/// A handle for a new account. `is_taken` says whether a handle has an
/// account, and `suffix` draws three random bytes. `None` after
/// [`ATTEMPTS`] attempts.
pub fn derive<E>(
    display_name: Option<&str>,
    email: Option<&str>,
    mut is_taken: impl FnMut(&str) -> Result<bool, E>,
    mut suffix: impl FnMut() -> [u8; 3],
) -> Result<Option<String>, E> {
    let base = base_label(display_name, email);
    for attempt in 0..ATTEMPTS {
        let label = attempt_label(&base, attempt, suffix());
        if !allowed(&label) {
            continue;
        }
        let handle = format!("{label}{DOMAIN}");
        if !is_taken(&handle)? {
            return Ok(Some(handle));
        }
    }
    Ok(None)
}

/// What the rename rule says of a change of handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rename {
    /// The rule does not apply, or the target is the one already claimed.
    Free,
    /// The change spends the allowance: the target is to be recorded.
    Claim,
    /// The allowance is spent, or its 30 days are over.
    Refused,
}

/// The message of a refused rename.
pub const RENAME_REFUSED: &str = "One hosted handle change is available within 30 days of signup. You can still use a domain you own.";

/// Decides a change of handle from `previous` to `handle`.
///
/// `policy` is the account's row when it has one: when the account was
/// created, in milliseconds, and the target it claimed. An account without
/// a row, a handle under another domain and an unchanged handle are free.
pub fn rename(
    policy: Option<(Option<i64>, Option<&str>)>,
    previous: Option<&str>,
    handle: &str,
    now_ms: i64,
) -> Rename {
    if !is_linkjar_handle(handle) || Some(handle) == previous {
        return Rename::Free;
    }
    let Some((created_ms, claimed)) = policy else {
        return Rename::Free;
    };
    if claimed == Some(handle) {
        return Rename::Free;
    }
    match created_ms {
        Some(created)
            if claimed.is_none() && now_ms >= created && now_ms < created + RENAME_WINDOW_MS =>
        {
            Rename::Claim
        }
        _ => Rename::Refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_an_ascii_label() {
        for (name, label) in [
            ("Nova 5f3fb5", "nova-5f3fb5"),
            ("Zoë  Müller", "zoe-muller"),
            ("ﬁona", "fiona"),
            ("  --Ada--  ", "ada"),
            ("quiet.5f3fb5", "quiet-5f3fb5"),
            ("A very long display name indeed", "a-very-long-displa"),
            ("abcdefghijklmnopq rs", "abcdefghijklmnopq"),
            ("日本語", ""),
        ] {
            assert_eq!(normalized_label(name), label, "{name}");
        }
    }

    #[test]
    fn the_base_is_the_name_then_the_email_then_reader() {
        assert_eq!(base_label(Some("Nova"), Some("x@example.com")), "nova");
        assert_eq!(base_label(None, Some("quiet.one@example.com")), "quiet-one");
        // A name in a script without an ASCII form, a reserved name and a short one fall through.
        assert_eq!(base_label(Some("日本語"), Some("mika@example.com")), "mika");
        assert_eq!(
            base_label(Some("Support"), Some("al@example.com")),
            "reader"
        );
        assert_eq!(base_label(None, None), "reader");
    }

    #[test]
    fn a_taken_name_gets_six_hexadecimal_digits() {
        let mut taken = vec!["nova.linkjar.social".to_owned()];
        let found = derive::<()>(
            Some("Nova"),
            None,
            |handle| Ok(taken.iter().any(|own| own == handle)),
            || [0xab, 0x01, 0xff],
        );
        assert_eq!(found, Ok(Some("nova-ab01ff.linkjar.social".to_owned())));
        taken.push("nova-ab01ff.linkjar.social".to_owned());
        let none = derive::<()>(Some("Nova"), None, |_| Ok(true), || [0, 0, 0]);
        assert_eq!(none, Ok(None));
        assert_eq!(
            attempt_label("abcdefghij-lmnop", 1, [1, 2, 3]),
            "abcdefghij-010203"
        );
        assert!(is_linkjar_handle("nova-ab01ff.linkjar.social"));
        assert!(!is_linkjar_handle("ab.linkjar.social"));
        assert!(!is_linkjar_handle("a.b.linkjar.social"));
        assert!(!is_linkjar_handle("nova.example.com"));
    }

    #[test]
    fn one_hosted_rename_is_free_for_thirty_days() {
        let day = 24 * 60 * 60 * 1000;
        let old = Some("fresh.linkjar.social");
        let target = "chosen.linkjar.social";
        // No row: a password account.
        assert_eq!(rename(None, old, target, 0), Rename::Free);
        assert_eq!(
            rename(Some((Some(0), None)), old, "fresh.linkjar.social", day),
            Rename::Free
        );
        assert_eq!(
            rename(Some((Some(0), None)), old, "own.example.org", day),
            Rename::Free
        );
        assert_eq!(
            rename(Some((Some(0), None)), old, target, day),
            Rename::Claim
        );
        assert_eq!(
            rename(Some((Some(0), None)), old, target, 30 * day),
            Rename::Refused
        );
        // The claimed target stays free, also after the 30 days.
        assert_eq!(
            rename(Some((Some(0), Some(target))), old, target, 40 * day),
            Rename::Free
        );
        assert_eq!(
            rename(
                Some((Some(0), Some(target))),
                old,
                "another.linkjar.social",
                day
            ),
            Rename::Refused
        );
        assert_eq!(
            rename(Some((None, None)), old, target, day),
            Rename::Refused
        );
    }
}
