//! Query parameters, validated as the Reference validates them.
//!
//! The Harness compares error messages (SPEC §4.2), so the order of the
//! checks and their wording follow `@atproto/lex-schema` at the pin:
//!
//! 1. The query is read in the order it was written. An empty value is
//!    ignored. A value for an integer or boolean parameter is converted, and
//!    one that does not convert is the error.
//! 2. The parameters are then checked in the order the lexicon declares
//!    them: a required one must be present, a scalar must have one value,
//!    and each value must have its format and be in its range.

use std::collections::HashMap;

use cid::Cid;
use pds_types::{AtIdentifier, Did, Handle, Nsid, RecordKey, Tid};

use crate::error::XrpcError;

/// The format of a string parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Any string.
    Any,
    /// A DID.
    Did,
    /// A handle.
    Handle,
    /// A DID or a handle.
    AtIdentifier,
    /// An NSID.
    Nsid,
    /// A record key.
    RecordKey,
    /// A TID.
    Tid,
    /// A CID in its string form.
    Cid,
}

impl Format {
    pub(crate) fn check(self, value: &str) -> Result<(), &'static str> {
        let ok = match self {
            Self::Any => true,
            Self::Did => value.parse::<Did>().is_ok(),
            Self::Handle => value.parse::<Handle>().is_ok(),
            Self::AtIdentifier => AtIdentifier::parse(value).is_ok(),
            Self::Nsid => value.parse::<Nsid>().is_ok(),
            Self::RecordKey => value.parse::<RecordKey>().is_ok(),
            Self::Tid => value.parse::<Tid>().is_ok(),
            Self::Cid => Cid::try_from(value).is_ok_and(|cid| cid.to_string() == value),
        };
        if ok {
            return Ok(());
        }
        Err(match self {
            Self::Any => "string",
            Self::Did => "DID",
            Self::Handle => "handle",
            Self::AtIdentifier => "AT identifier",
            Self::Nsid => "NSID",
            Self::RecordKey => "record key",
            Self::Tid => "TID string",
            Self::Cid => "CID string",
        })
    }
}

/// The type of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One string.
    String(Format),
    /// Any number of strings: the parameter may repeat.
    Strings(Format),
    /// One integer in `min..=max`, with a value when it is absent.
    Integer {
        /// The least value.
        min: i64,
        /// The greatest value.
        max: i64,
        /// The value of an absent parameter.
        default: i64,
    },
    /// One boolean, with a value when it is absent.
    Boolean {
        /// The value of an absent parameter.
        default: bool,
    },
}

/// One parameter of a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// The name.
    pub name: &'static str,
    /// The type.
    pub kind: Kind,
    /// Whether the request must give it.
    pub required: bool,
}

impl Spec {
    /// A required string.
    pub const fn required(name: &'static str, format: Format) -> Self {
        Self {
            name,
            kind: Kind::String(format),
            required: true,
        }
    }

    /// An optional string.
    pub const fn optional(name: &'static str, format: Format) -> Self {
        Self {
            name,
            kind: Kind::String(format),
            required: false,
        }
    }

    /// A required list of strings.
    pub const fn list(name: &'static str, format: Format) -> Self {
        Self {
            name,
            kind: Kind::Strings(format),
            required: true,
        }
    }

    /// An optional integer.
    pub const fn integer(name: &'static str, min: i64, max: i64, default: i64) -> Self {
        Self {
            name,
            kind: Kind::Integer { min, max, default },
            required: false,
        }
    }

    /// An optional boolean.
    pub const fn boolean(name: &'static str, default: bool) -> Self {
        Self {
            name,
            kind: Kind::Boolean { default },
            required: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Integer(i64),
    Boolean(bool),
}

impl Value {
    fn show(&self) -> String {
        match self {
            Self::Text(text) => quoted(text),
            Self::Integer(number) => number.to_string(),
            Self::Boolean(flag) => flag.to_string(),
        }
    }
}

/// The longest string an error message quotes in full, in UTF-16 units.
const PREVIEW_LEN: usize = 256;

/// Renders a string as the Reference's messages quote it: as JSON, cut
/// short when it is long.
fn quoted(value: &str) -> String {
    if value.encode_utf16().count() < PREVIEW_LEN {
        return serde_json::Value::from(value).to_string();
    }
    let mut units: Vec<u16> = value.encode_utf16().take(PREVIEW_LEN - 1).collect();
    units.extend("…".encode_utf16());
    serde_json::Value::from(String::from_utf16_lossy(&units)).to_string()
}

/// Renders a list as the Reference's messages do: the first two items.
fn shown_list(values: &[Value]) -> String {
    let mut out = values
        .iter()
        .take(2)
        .map(Value::show)
        .collect::<Vec<_>>()
        .join(", ");
    if values.len() > 2 {
        out.push_str(", ...");
    }
    format!("[{out}]")
}

/// The validated parameters of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    values: HashMap<&'static str, Vec<Value>>,
}

impl Params {
    /// Reads and validates a raw query string for the method `nsid`.
    pub fn parse(nsid: &str, query: Option<&str>, specs: &[Spec]) -> Result<Self, XrpcError> {
        let invalid =
            |reason: String| XrpcError::invalid_request(format!("Invalid {nsid} params: {reason}"));
        let wrong_type = |expected: &str, got: String| {
            invalid(format!("Expected {expected} value type (got {got})"))
        };

        let mut values: HashMap<&'static str, Vec<Value>> = HashMap::new();
        for part in query.unwrap_or_default().split('&') {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            let (name, value) = (decode(name), decode(value));
            if value.is_empty() {
                continue;
            }
            // A parameter the lexicon does not declare is ignored.
            let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
                continue;
            };
            let converted = match spec.kind {
                Kind::String(_) | Kind::Strings(_) => Value::Text(value),
                Kind::Integer { .. } if !is_plain_integer(&value) => {
                    return Err(wrong_type("integer", quoted(&value)));
                }
                Kind::Integer { .. } => match value.parse::<i64>() {
                    Ok(number) if number.unsigned_abs() <= MAX_SAFE_INTEGER => {
                        Value::Integer(number)
                    }
                    // The Reference converts the digits to a JavaScript
                    // number first, so it shows the number and not the text.
                    _ => return Err(wrong_type("integer", js_number(&value))),
                },
                Kind::Boolean { .. } => match value.as_str() {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    _ => return Err(wrong_type("boolean", quoted(&value))),
                },
            };
            values.entry(spec.name).or_default().push(converted);
        }

        for spec in specs {
            let Some(given) = values.get(spec.name) else {
                if spec.required {
                    return Err(invalid(format!(
                        "Missing required key {}",
                        quoted(spec.name)
                    )));
                }
                continue;
            };
            let (type_name, format) = match spec.kind {
                Kind::String(format) => ("string", Some(format)),
                Kind::Strings(format) => ("array", Some(format)),
                Kind::Integer { .. } => ("integer", None),
                Kind::Boolean { .. } => ("boolean", None),
            };
            if !matches!(spec.kind, Kind::Strings(_)) && given.len() > 1 {
                return Err(wrong_type(type_name, shown_list(given)));
            }
            for value in given {
                match (value, spec.kind) {
                    (Value::Text(text), _) => {
                        if let Some(Err(what)) = format.map(|format| format.check(text)) {
                            return Err(invalid(format!("Invalid {what} (got {})", quoted(text))));
                        }
                    }
                    (Value::Integer(number), Kind::Integer { min, max, .. }) => {
                        if *number < min {
                            return Err(invalid(format!(
                                "integer too small (minimum {min}, got {number})"
                            )));
                        }
                        if *number > max {
                            return Err(invalid(format!(
                                "integer too big (maximum {max}, got {number})"
                            )));
                        }
                    }
                    _ => {}
                }
            }
        }
        for spec in specs {
            let default = match spec.kind {
                Kind::Integer { default, .. } => Value::Integer(default),
                Kind::Boolean { default } => Value::Boolean(default),
                Kind::String(_) | Kind::Strings(_) => continue,
            };
            values.entry(spec.name).or_insert_with(|| vec![default]);
        }
        Ok(Self { values })
    }

    /// The value of a string parameter, when the request gave one.
    pub fn get(&self, name: &str) -> Option<&str> {
        match self.values.get(name)?.first()? {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The value of a required string parameter. Empty only when the
    /// handler asks for a name its own list does not declare as required.
    pub fn text(&self, name: &str) -> &str {
        self.get(name).unwrap_or_default()
    }

    /// Every value of a list parameter, in order.
    pub fn list(&self, name: &str) -> Vec<&str> {
        self.values
            .get(name)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| match value {
                        Value::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The value of an integer parameter, or its default when the request
    /// gave none. `0` only when the handler asks for a name its own list
    /// does not declare as an integer.
    pub fn integer(&self, name: &str) -> i64 {
        match self.values.get(name).and_then(|values| values.first()) {
            Some(Value::Integer(number)) => *number,
            _ => 0,
        }
    }

    /// The value of an integer parameter that bounds a page. `parse` has
    /// checked its range, so the value fits. A value that does not fit gives
    /// `0`, an empty page, and never an unbounded one.
    pub fn limit(&self, name: &str) -> u32 {
        u32::try_from(self.integer(name)).unwrap_or_default()
    }

    /// The value of a boolean parameter, or its default.
    pub fn boolean(&self, name: &str) -> bool {
        matches!(
            self.values.get(name).and_then(|values| values.first()),
            Some(Value::Boolean(true))
        )
    }
}

/// The greatest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// A string of digits as JavaScript prints the number it converts to.
fn js_number(digits: &str) -> String {
    let number: f64 = digits.parse().unwrap_or(f64::INFINITY);
    if number.abs() < 1e21 {
        return format!("{number:.0}");
    }
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    // Rust writes 1e21; JavaScript writes 1e+21.
    format!("{number:e}").replacen('e', "e+", 1)
}

/// Digits with an optional minus sign: what the Reference reads as an
/// integer parameter.
fn is_plain_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Percent-decodes one query component, with `+` as a space. Bytes that do
/// not form UTF-8 are replaced, and a broken escape is kept as written.
fn decode(component: &str) -> String {
    let bytes = component.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let escape = (byte == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (byte, escape) {
            (_, Some(value)) => {
                out.push(value);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NSID: &str = "com.atproto.repo.listRecords";
    const SPECS: &[Spec] = &[
        Spec::required("repo", Format::AtIdentifier),
        Spec::required("collection", Format::Nsid),
        Spec::integer("limit", 1, 100, 50),
        Spec::optional("cursor", Format::Any),
        Spec::boolean("reverse", false),
    ];

    fn error(query: &str) -> String {
        let message = Params::parse(NSID, Some(query), SPECS).unwrap_err().message;
        message
            .strip_prefix("Invalid com.atproto.repo.listRecords params: ")
            .unwrap()
            .to_owned()
    }

    // Each expectation below is what the reference image answered on
    // 2026-10-12.

    #[test]
    fn defaults_fill_what_the_request_leaves_out() {
        let p = Params::parse(
            NSID,
            Some("repo=did:plc:abc&collection=a.b.c&unknown=1"),
            SPECS,
        )
        .unwrap();
        assert_eq!(
            (p.text("repo"), p.text("collection")),
            ("did:plc:abc", "a.b.c")
        );
        assert_eq!(
            (p.integer("limit"), p.boolean("reverse"), p.get("cursor")),
            (50, false, None)
        );
    }

    #[test]
    fn an_empty_value_is_an_absent_one() {
        let p = Params::parse(
            NSID,
            Some("repo=did:plc:abc&repo=&collection=a.b.c&limit=&reverse=&cursor="),
            SPECS,
        )
        .unwrap();
        assert_eq!(
            (p.integer("limit"), p.boolean("reverse"), p.get("cursor")),
            (50, false, None)
        );
        assert_eq!(
            error("repo=&collection=a.b.c"),
            "Missing required key \"repo\""
        );
    }

    #[test]
    fn conversion_comes_before_every_other_check() {
        assert_eq!(
            error("limit=abc&collection=bad&repo=bad_id"),
            "Expected integer value type (got \"abc\")"
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=&limit=abc"),
            "Expected integer value type (got \"abc\")"
        );
        assert_eq!(
            error("reverse=maybe"),
            "Expected boolean value type (got \"maybe\")"
        );
        assert_eq!(
            error("reverse=TRUE"),
            "Expected boolean value type (got \"TRUE\")"
        );
        assert_eq!(
            error("limit=1e1"),
            "Expected integer value type (got \"1e1\")"
        );
    }

    #[test]
    fn parameters_are_checked_in_the_order_of_the_lexicon() {
        assert_eq!(
            error("collection=bad&repo=bad_id"),
            "Invalid AT identifier (got \"bad_id\")"
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=bad"),
            "Invalid NSID (got \"bad\")"
        );
        assert_eq!(
            error("repo=did:plc:abc"),
            "Missing required key \"collection\""
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=a.b.c&limit=0"),
            "integer too small (minimum 1, got 0)"
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=a.b.c&limit=101"),
            "integer too big (maximum 100, got 101)"
        );
        assert_eq!(
            Params::parse(
                NSID,
                Some("repo=did:plc:abc&collection=a.b.c&limit=07"),
                SPECS
            )
            .unwrap()
            .integer("limit"),
            7
        );
    }

    #[test]
    fn an_integer_beyond_a_javascript_number_is_shown_as_one() {
        let base = "repo=did:plc:abc&collection=a.b.c&limit=";
        assert_eq!(
            error(&format!("{base}99999999999999999999")),
            "Expected integer value type (got 100000000000000000000)"
        );
        assert_eq!(
            error(&format!("{base}9007199254740993")),
            "Expected integer value type (got 9007199254740992)"
        );
        assert_eq!(
            error(&format!("{base}1000000000000000000000")),
            "Expected integer value type (got 1e+21)"
        );
        assert_eq!(
            error(&format!("{base}9007199254740991")),
            "integer too big (maximum 100, got 9007199254740991)"
        );
    }

    #[test]
    fn a_repeated_scalar_is_shown_as_a_list() {
        assert_eq!(
            error("repo=did:plc:abc&repo=did:plc:abd&collection=a.b.c"),
            "Expected string value type (got [\"did:plc:abc\", \"did:plc:abd\"])"
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=a.b.c&limit=1&limit=2"),
            "Expected integer value type (got [1, 2])"
        );
        assert_eq!(
            error("repo=did:plc:abc&collection=a.b.c&reverse=true&reverse=false&reverse=true"),
            "Expected boolean value type (got [true, false, ...])"
        );
    }

    #[test]
    fn a_long_value_is_cut_in_the_message() {
        let long = "x".repeat(300);
        let message = error(&format!("repo={long}&collection=a.b.c"));
        assert_eq!(
            message,
            format!("Invalid AT identifier (got \"{}…\")", "x".repeat(255))
        );
    }

    #[test]
    fn components_are_percent_decoded() {
        let p = Params::parse(
            NSID,
            Some("repo=did%3Aplc%3Aabc&collection=a.b.c&cursor=a+b%zz"),
            SPECS,
        )
        .unwrap();
        assert_eq!(
            (p.text("repo"), p.get("cursor")),
            ("did:plc:abc", Some("a b%zz"))
        );
    }

    #[test]
    fn a_list_keeps_every_value() {
        let specs = &[
            Spec::required("did", Format::Did),
            Spec::list("cids", Format::Cid),
        ];
        let cid = "bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku";
        let p = Params::parse(
            "x.y.z",
            Some(&format!("did=did:plc:abc&cids={cid}&cids={cid}")),
            specs,
        )
        .unwrap();
        assert_eq!(p.list("cids"), [cid, cid]);
        let missing = Params::parse("x.y.z", Some("did=did:plc:abc&cids="), specs).unwrap_err();
        assert_eq!(
            missing.message,
            "Invalid x.y.z params: Missing required key \"cids\""
        );
        let bad = Params::parse("x.y.z", Some("did=did:plc:abc&cids=zzz"), specs).unwrap_err();
        assert_eq!(
            bad.message,
            "Invalid x.y.z params: Invalid CID string (got \"zzz\")"
        );
    }
}
