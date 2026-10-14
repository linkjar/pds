//! The Reference's wording of a validation error.
//!
//! A client reads the `message` of a refused write, and the Harness compares
//! it in the response comparison (`docs/SPEC.md` §17.3). The Reference at the pin builds the
//! message from the issue and a JSON path
//! (`packages/lex/lex-schema/src/core/validation-issue.ts`):
//! `Missing required key "text" at $.record`.
//!
//! The wording is the Reference's for the first issue of a value. Where a
//! value breaks several rules, the Reference reports the first in the order
//! its lexicon declares the properties and this crate the first in sorted
//! order (see [`crate::validate`]), so the two may name different issues.

use pds_types::Data;

use crate::validate::{Rule, ValidationError};

/// The most characters of a string that a message quotes.
const PREVIEW: usize = 256;

/// A JSON string literal of `text`, as `JSON.stringify` writes it.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
}

/// A value as a message shows it: `stringifyValue` of the Reference.
pub fn stringify_value(value: &Data) -> String {
    fn list<T>(items: impl Iterator<Item = T>, total: usize, show: impl Fn(T) -> String) -> String {
        let mut text = items.take(2).map(show).collect::<Vec<_>>().join(", ");
        if total > 2 {
            text.push_str(", ...");
        }
        text
    }
    match value {
        Data::Null => "null".to_owned(),
        Data::Bool(value) => value.to_string(),
        Data::Integer(value) => value.to_string(),
        Data::String(text) => stringify_str(text),
        Data::Array(items) => format!("[{}]", list(items.iter(), items.len(), stringify_value)),
        Data::Object(fields) => format!(
            "{{{}}}",
            list(fields.keys(), fields.len(), |key| format!(
                "{}: ...",
                json_string(key)
            ))
        ),
        Data::Link(_) => "cid".to_owned(),
        Data::Bytes(_) => "object".to_owned(),
    }
}

/// A string as a message shows it: quoted, and cut at 256 UTF-16 units.
pub fn stringify_str(text: &str) -> String {
    if text.encode_utf16().count() < PREVIEW {
        return json_string(text);
    }
    let mut units = 0;
    let mut cut = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > PREVIEW - 1 {
            break;
        }
        cut.push(c);
    }
    cut.push('…');
    json_string(&cut)
}

/// One segment of a JSON path, as `toJsonPathSegment` of the Reference
/// writes it. A segment of digits is an array index.
pub fn json_path_segment(segment: &str) -> String {
    let mut chars = segment.chars();
    let identifier = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if identifier {
        format!(".{segment}")
    } else if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
        format!("[{segment}]")
    } else {
        format!("[{}]", json_string(segment))
    }
}

fn format_description(format: &str) -> &str {
    match format {
        "at-identifier" => "AT identifier",
        "did" => "DID",
        "nsid" => "NSID",
        "cid" => "CID string",
        "tid" => "TID string",
        "record-key" => "record key",
        other => other,
    }
}

fn one_of(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("one of {} or {last}", rest.join(", ")),
    }
}

/// The message of a value that has the wrong type.
pub fn invalid_type(expected: &[&str], got: &str) -> String {
    format!("Expected {} value type (got {got})", one_of(expected))
}

/// The message of a string that does not match its format.
pub fn invalid_format(format: &str, got: &str) -> String {
    format!("Invalid {} (got {got})", format_description(format))
}

/// The message of a missing property.
pub fn required_key(key: &str) -> String {
    format!("Missing required key \"{key}\"")
}

impl ValidationError {
    /// The message the Reference gives this error.
    ///
    /// `root` is the value that was validated: the record, or for a record
    /// key a string. `at` is the JSON path of that value in the request:
    /// `$.record`, `$.writes[0].record`, or `$` for a record key.
    pub fn reference_message(&self, root: &Data, at: &str) -> String {
        // The first segment names the root: `record` or `rkey`.
        let mut segments = self.path.split('/').skip(1);
        let mut path = String::from(at);
        let mut value = Some(root);
        for segment in &mut segments {
            path.push_str(&json_path_segment(segment));
            value = value.and_then(|parent| match parent {
                Data::Object(fields) => fields.get(segment),
                Data::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
                _ => None,
            });
        }
        let got = value.map_or_else(|| "undefined".to_owned(), stringify_value);
        let message = match &self.rule {
            Rule::Type { expected, .. } => {
                let expected = if *expected == "cid-link" {
                    "cid"
                } else {
                    expected
                };
                invalid_type(&[expected], &got)
            }
            Rule::Value { expected } => format!("Expected {expected} (got {got})"),
            Rule::RequiredKey { key } => required_key(key),
            Rule::Format { format, .. } => invalid_format(format, &got),
            other => other.to_string(),
        };
        format!("{message} at {path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Corpus;
    use crate::validate::validate_record_key;

    fn data(json: &str) -> Data {
        Data::from_json_str(json).unwrap()
    }

    #[test]
    fn a_missing_key_is_worded_as_the_reference_words_it() {
        let record =
            data(r#"{"$type":"app.bsky.feed.post","createdAt":"2026-01-01T00:00:00.000Z"}"#);
        let error = Corpus::bundled().validate_record(&record).unwrap_err();
        assert_eq!(
            error.reference_message(&record, "$.record"),
            r#"Missing required key "text" at $.record"#
        );
        assert_eq!(
            error.reference_message(&record, "$.writes[1].record"),
            r#"Missing required key "text" at $.writes[1].record"#
        );
    }

    #[test]
    fn a_record_key_error_is_worded_as_the_reference_words_it() {
        let key = Data::String("not-a-tid".to_owned());
        let error = validate_record_key("tid", "not-a-tid").unwrap_err();
        assert_eq!(
            error.reference_message(&key, "$"),
            r#"Invalid TID string (got "not-a-tid") at $"#
        );
        let key = Data::String("other".to_owned());
        let error = validate_record_key("literal:self", "other").unwrap_err();
        assert_eq!(
            error.reference_message(&key, "$"),
            r#"Expected "self" (got "other") at $"#
        );
    }

    #[test]
    fn a_nested_error_names_its_json_path() {
        let record = data(
            r#"{"$type":"app.bsky.feed.post","text":"x","createdAt":"2026-01-01T00:00:00.000Z","langs":["en",7]}"#,
        );
        let error = Corpus::bundled().validate_record(&record).unwrap_err();
        assert_eq!(
            error.reference_message(&record, "$.record"),
            "Expected string value type (got 7) at $.record.langs[1]"
        );
    }

    #[test]
    fn a_long_string_is_cut_in_a_message() {
        let long = "k".repeat(300);
        let shown = stringify_str(&long);
        assert_eq!(shown.chars().count(), 255 + 1 + 2);
        assert!(shown.ends_with("…\""));
        assert_eq!(stringify_str("a\"b"), r#""a\"b""#);
    }

    #[test]
    fn every_vendored_lexicon_is_in_the_bundle() {
        assert_eq!(Corpus::bundled().len(), crate::bundled::DOCS.len());
        assert!(Corpus::bundled().record("app.bsky.feed.post").is_some());
    }
}
