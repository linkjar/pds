//! Record validation against lexicon schemas, record-key types and the
//! data-model rules (`docs/SPEC.md` §6.1, §6.2, §6.3).
//!
//! [`Corpus::validate_record`] follows the validator the Reference runs on a
//! write at the pin: `@atproto/lex` (`packages/lex/lex-schema`) in strict
//! `validate` mode, over the schemas `lex-builder` compiles, as called from
//! `packages/pds/src/repo/prepare.ts`. The older `@atproto/lexicon` package
//! and the written lexicon specification differ from it in places; a comment
//! marks each one.
//!
//! Three things differ from the Reference on purpose:
//!
//! - The Reference's blob schema also requires a `raw` CID. Here that rule
//!   belongs to [`validate_data_model`], which the write path always runs
//!   (SPEC §6.3), so the upstream vector `full`, whose blob references carry
//!   `dag-cbor` CIDs, validates against its lexicon.
//! - Properties are checked in sorted order, not in the order the lexicon
//!   declares them, so the first error of a record with several may differ.
//! - [`BlobRules::Read`] relaxes the blob rules only. The Reference's
//!   lenient mode also relaxes the `datetime`, `at-uri` and `language`
//!   formats.

use std::collections::BTreeMap;
use std::fmt;

use pds_types::cbor::{MAX_DEPTH, MAX_SAFE_INTEGER};
use pds_types::content::RAW;
use pds_types::{
    AtIdentifier, AtUri, CidString, Data, Datetime, Did, Handle, Language, Nsid, RecordKey,
    SyntaxError, Tid, Uri,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::Corpus;
use crate::model::{
    LexArray, LexBlob, LexBoolean, LexBytes, LexInteger, LexObject, LexRecord, LexRefUnion,
    LexString, LexType, LexUserType,
};

/// The first path segment of every record error.
const ROOT: &str = "record";
/// The path of a record-key error.
const RKEY: &str = "rkey";
/// The key that names the type of an object.
const TYPE_KEY: &str = "$type";
/// The `$type` of a blob reference.
const BLOB: &str = "blob";
/// The longest chain of references followed without descending into the
/// data. A valid corpus needs one step; a longer chain is a reference cycle.
const MAX_REF_HOPS: usize = 8;

/// A value that fails record, record-key or data-model validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {rule}")]
pub struct ValidationError {
    /// The path of the failing value from the record root, for example
    /// `record/formats/handle`. A record key has the path `rkey`.
    pub path: String,
    /// The rule the value breaks.
    pub rule: Rule,
}

impl ValidationError {
    /// Prefixes the path with the segment of the enclosing value.
    fn within(mut self, segment: &str) -> Self {
        self.path = if self.path.is_empty() {
            segment.to_owned()
        } else {
            format!("{segment}/{}", self.path)
        };
        self
    }
}

impl From<Rule> for ValidationError {
    fn from(rule: Rule) -> Self {
        Self {
            path: String::new(),
            rule,
        }
    }
}

/// The rule a value breaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rule {
    /// The value has the wrong type.
    #[error("expected {expected}, found {found}")]
    Type {
        /// The type the schema requires.
        expected: &'static str,
        /// The type of the value.
        found: &'static str,
    },
    /// The value is not the constant, an enumerated value, the token or an
    /// allowed `$type`.
    #[error("expected {expected}")]
    Value {
        /// The values the schema allows.
        expected: String,
    },
    /// A required property is missing.
    #[error("missing required key {key:?}")]
    RequiredKey {
        /// The property name.
        key: String,
    },
    /// A string does not match its format.
    #[error("invalid {format}: {reason}")]
    Format {
        /// The lexicon string format.
        format: String,
        /// The syntax rule that failed.
        reason: String,
    },
    /// A length, size or number is under the minimum.
    #[error("{measure} too small (minimum {minimum}, got {actual})")]
    TooSmall {
        /// What was measured.
        measure: Measure,
        /// The minimum.
        minimum: i64,
        /// The measured value.
        actual: i64,
    },
    /// A length, size or number is over the maximum.
    #[error("{measure} too big (maximum {maximum}, got {actual})")]
    TooBig {
        /// What was measured.
        measure: Measure,
        /// The maximum.
        maximum: i64,
        /// The measured value.
        actual: i64,
    },
    /// An object is not a well-formed blob reference.
    #[error("malformed blob reference: {0}")]
    Blob(&'static str),
    /// A `$type` or a MIME type is the empty string.
    #[error("must not be empty")]
    Empty,
    /// Arrays and objects nest deeper than 32 levels (SPEC §6.1).
    #[error("nesting is deeper than {max} levels", max = MAX_DEPTH)]
    TooDeep,
    /// The corpus has no record lexicon for the `$type`.
    #[error("no record lexicon for {nsid}")]
    UnknownLexicon {
        /// The `$type` of the record.
        nsid: String,
    },
    /// The lexicon cannot be evaluated: an unresolved or cyclic reference, a
    /// reference to a definition that is not a data type, an unknown string
    /// format or record key type.
    #[error("lexicon: {0}")]
    Schema(String),
}

impl Rule {
    /// The error for the property `key` of the value under validation.
    fn within(self, key: &str) -> ValidationError {
        ValidationError::from(self).within(key)
    }
}

/// What a [`Rule::TooSmall`] or [`Rule::TooBig`] measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// Array length in elements.
    Array,
    /// String length in UTF-8 bytes.
    String,
    /// String length in grapheme clusters.
    Grapheme,
    /// Byte string length.
    Bytes,
    /// Blob size in bytes.
    Blob,
    /// Integer value.
    Integer,
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Array => "array",
            Self::String => "string",
            Self::Grapheme => "grapheme",
            Self::Bytes => "bytes",
            Self::Blob => "blob",
            Self::Integer => "integer",
        })
    }
}

/// Which blob rules apply (SPEC §6.3: the legacy blob form is accepted on
/// read and never written).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlobRules {
    /// A record a client writes: the Reference's strict mode. A blob is a
    /// typed reference that meets `accept` and `maxSize`.
    #[default]
    Write,
    /// A record already in a repository: the Reference's lenient mode for
    /// blobs. The legacy form `{cid, mimeType}` is accepted, and `accept`
    /// and `maxSize` are not checked.
    Read,
}

impl Corpus {
    /// The record definition of a collection: `<nsid>#main` when it is a
    /// `record`.
    pub fn record(&self, nsid: &str) -> Option<&LexRecord> {
        match self.get(nsid)?.defs.get("main")? {
            LexUserType::Record(record) => Some(record),
            _ => None,
        }
    }

    /// Validates a record a client writes against the lexicon its `$type`
    /// names (SPEC §6.3). [`Rule::UnknownLexicon`] reports a `$type` the
    /// corpus has no record lexicon for.
    pub fn validate_record(&self, data: &Data) -> Result<(), ValidationError> {
        self.validate_record_with(data, BlobRules::Write)
    }

    /// Validates a record against the lexicon its `$type` names, with the
    /// blob rules of `blobs`.
    pub fn validate_record_with(
        &self,
        data: &Data,
        blobs: BlobRules,
    ) -> Result<(), ValidationError> {
        let walk = Walk {
            corpus: self,
            blobs,
        };
        walk.record(data).map_err(|e| e.within(ROOT))
    }
}

/// Checks a record key against the key type its record lexicon declares:
/// `tid`, `nsid`, `any` or `literal:<value>` (SPEC §6.2).
pub fn validate_record_key(key_type: &str, rkey: &str) -> Result<(), ValidationError> {
    record_key(key_type, rkey).map_err(|e| e.within(RKEY))
}

fn record_key(key_type: &str, rkey: &str) -> Result<(), ValidationError> {
    // Every key meets the record-key syntax first, as in the Reference.
    string_format("record-key", rkey)?;
    match key_type {
        "any" => Ok(()),
        "tid" => string_format("tid", rkey),
        "nsid" => string_format("nsid", rkey),
        _ => match key_type.strip_prefix("literal:") {
            Some(literal) if literal == rkey => Ok(()),
            Some(literal) => Err(Rule::Value {
                expected: format!("{literal:?}"),
            }
            .into()),
            None => Err(Rule::Schema(format!("unknown record key type {key_type:?}")).into()),
        },
    }
}

/// Checks the rules every record meets whatever its lexicon (SPEC §6.3).
///
/// The top level is an object. A `$type`, on any object, is a non-empty
/// string. An object with `$type: "blob"` is a blob reference: `ref` is a
/// link to a `raw` CID, `mimeType` is a non-empty string and `size` is an
/// integer greater than zero. The legacy blob form `{cid, mimeType}` has no
/// `$type` and passes. Other `$`-prefixed keys are ignored. Nesting stops at
/// 32 levels (SPEC §6.1).
///
/// Floats and malformed `$link` and `$bytes` objects never reach this
/// function: [`Data::from_json_str`] rejects them.
///
/// The size and MIME rules are those of SPEC §6.3. The Reference's strict
/// blob check differs: it accepts a size of zero, and it requires a `/` in
/// the MIME type and no other keys.
pub fn validate_data_model(data: &Data) -> Result<(), ValidationError> {
    let checked = match data {
        Data::Object(fields) => model_object(fields, 1),
        other => Err(wrong_type("object", other)),
    };
    checked.map_err(|e| e.within(ROOT))
}

fn model_value(value: &Data, depth: usize) -> Result<(), ValidationError> {
    match value {
        Data::Object(fields) => model_object(fields, depth),
        Data::Array(items) => {
            if depth > MAX_DEPTH {
                return Err(Rule::TooDeep.into());
            }
            items.iter().enumerate().try_for_each(|(index, item)| {
                model_value(item, depth + 1).map_err(|e| e.within(&index.to_string()))
            })
        }
        Data::Null
        | Data::Bool(_)
        | Data::Integer(_)
        | Data::String(_)
        | Data::Bytes(_)
        | Data::Link(_) => Ok(()),
    }
}

fn model_object(fields: &BTreeMap<String, Data>, depth: usize) -> Result<(), ValidationError> {
    if depth > MAX_DEPTH {
        return Err(Rule::TooDeep.into());
    }
    match fields.get(TYPE_KEY) {
        Some(Data::String(tag)) if tag.is_empty() => return Err(Rule::Empty.within(TYPE_KEY)),
        Some(Data::String(tag)) if tag == BLOB => model_blob(fields)?,
        Some(Data::String(_)) | None => {}
        Some(other) => return Err(wrong_type("string", other).within(TYPE_KEY)),
    }
    fields
        .iter()
        .try_for_each(|(key, value)| model_value(value, depth + 1).map_err(|e| e.within(key)))
}

fn model_blob(fields: &BTreeMap<String, Data>) -> Result<(), ValidationError> {
    match required(fields, "ref")? {
        // CIDv0 is always `dag-pb`, so a `raw` codec implies CIDv1.
        Data::Link(link) if link.0.codec() == RAW => {}
        Data::Link(_) => return Err(Rule::Blob("ref is not a raw CID").within("ref")),
        other => return Err(wrong_type("cid-link", other).within("ref")),
    }
    match required(fields, "mimeType")? {
        Data::String(mime_type) if mime_type.is_empty() => {
            return Err(Rule::Empty.within("mimeType"));
        }
        Data::String(_) => {}
        other => return Err(wrong_type("string", other).within("mimeType")),
    }
    match required(fields, "size")? {
        Data::Integer(size) if *size > 0 => Ok(()),
        Data::Integer(size) => Err(Rule::TooSmall {
            measure: Measure::Blob,
            minimum: 1,
            actual: *size,
        }
        .within("size")),
        other => Err(wrong_type("integer", other).within("size")),
    }
}

/// One validation of a record against the corpus.
struct Walk<'a> {
    corpus: &'a Corpus,
    blobs: BlobRules,
}

impl Walk<'_> {
    fn record(&self, data: &Data) -> Result<(), ValidationError> {
        let fields = object_fields(data)?;
        let nsid = match required(fields, TYPE_KEY)? {
            Data::String(nsid) => nsid,
            other => return Err(wrong_type("string", other).within(TYPE_KEY)),
        };
        // The Reference looks the `$type` up as written, so `<nsid>#main`
        // names no record.
        let Some(record) = self.corpus.record(nsid) else {
            return Err(Rule::UnknownLexicon { nsid: nsid.clone() }.within(TYPE_KEY));
        };
        self.object(record.record.object(), nsid, fields, 1)
    }

    /// Validates the properties an object schema declares. Other properties
    /// are allowed. `depth` is the nesting level of this object.
    ///
    /// The Reference validates without applying defaults, so a required
    /// property with a `default` must still be present; `@atproto/lexicon`
    /// accepted its absence.
    fn object(
        &self,
        schema: &LexObject,
        doc: &str,
        fields: &BTreeMap<String, Data>,
        depth: usize,
    ) -> Result<(), ValidationError> {
        if depth > MAX_DEPTH {
            return Err(Rule::TooDeep.into());
        }
        for (key, property) in &schema.properties {
            match fields.get(key) {
                None if schema.required.contains(key) => {
                    return Err(Rule::RequiredKey { key: key.clone() }.into());
                }
                None => {}
                Some(Data::Null) if schema.nullable.contains(key) => {}
                Some(value) => self
                    .value(property, doc, value, depth + 1)
                    .map_err(|e| e.within(key))?,
            }
        }
        Ok(())
    }

    /// Validates a property or an array item. `depth` is the nesting level
    /// the value has when it is an array or an object.
    fn value(
        &self,
        schema: &LexType,
        doc: &str,
        value: &Data,
        depth: usize,
    ) -> Result<(), ValidationError> {
        match schema {
            LexType::String(schema) => string(schema, value),
            LexType::Integer(schema) => integer(schema, value),
            LexType::Boolean(schema) => boolean(schema, value),
            LexType::Bytes(schema) => bytes(schema, value),
            LexType::CidLink(_) => cid_link(value),
            LexType::Blob(schema) => self.blob(schema, value),
            LexType::Array(schema) => self.array(schema, doc, value, depth),
            LexType::Object(schema) => self.object(schema, doc, object_fields(value)?, depth),
            LexType::Ref(schema) => self.reference(&schema.ref_, doc, value, depth, 0),
            LexType::Union(schema) => self.union(schema, doc, value, depth, 0),
            LexType::Unknown(_) => unknown(value),
        }
    }

    /// Validates a value against the definition `<doc>#<name>` a reference
    /// leads to.
    fn definition(
        &self,
        def: &LexUserType,
        doc: &str,
        name: &str,
        value: &Data,
        depth: usize,
        hops: usize,
    ) -> Result<(), ValidationError> {
        match def {
            // The Reference rejects a `$type` that names another definition;
            // `@atproto/lexicon` did not look at it.
            LexUserType::Object(schema) => {
                let fields = object_fields(value)?;
                if fields
                    .get(TYPE_KEY)
                    .is_some_and(|found| !is_type(found, doc, name))
                {
                    return Err(wrong_type_id(doc, name));
                }
                self.object(schema, doc, fields, depth)
            }
            LexUserType::Record(schema) => {
                let fields = object_fields(value)?;
                self.object(schema.record.object(), doc, fields, depth)?;
                if fields
                    .get(TYPE_KEY)
                    .is_some_and(|found| is_type(found, doc, name))
                {
                    Ok(())
                } else {
                    Err(wrong_type_id(doc, name))
                }
            }
            // A token is the string `<nsid>#<name>`. `@atproto/lexicon`
            // rejected every value for a reference to a token.
            LexUserType::Token(_) => match value {
                found @ Data::String(_) if is_type(found, doc, name) => Ok(()),
                Data::String(_) => Err(expected_type(doc, name).into()),
                other => Err(wrong_type("token", other)),
            },
            LexUserType::String(schema) => string(schema, value),
            LexUserType::Integer(schema) => integer(schema, value),
            LexUserType::Boolean(schema) => boolean(schema, value),
            LexUserType::Bytes(schema) => bytes(schema, value),
            LexUserType::CidLink(_) => cid_link(value),
            LexUserType::Blob(schema) => self.blob(schema, value),
            LexUserType::Array(schema) => self.array(schema, doc, value, depth),
            LexUserType::Ref(schema) => self.reference(&schema.ref_, doc, value, depth, hops),
            LexUserType::Union(schema) => self.union(schema, doc, value, depth, hops),
            LexUserType::Unknown(_) => unknown(value),
            LexUserType::Query(_)
            | LexUserType::Procedure(_)
            | LexUserType::Subscription(_)
            | LexUserType::PermissionSet(_)
            | LexUserType::Permission(_) => Err(Rule::Schema(format!(
                "{} is a {}, not a data type",
                type_id(doc, name),
                def.type_name()
            ))
            .into()),
        }
    }

    /// Follows a reference written in the document `from`. The data does not
    /// descend, so `hops` bounds the chain.
    fn reference(
        &self,
        reference: &str,
        from: &str,
        value: &Data,
        depth: usize,
        hops: usize,
    ) -> Result<(), ValidationError> {
        if hops >= MAX_REF_HOPS {
            return Err(Rule::Schema(format!(
                "reference {reference} in {from} is part of a chain longer than {MAX_REF_HOPS}"
            ))
            .into());
        }
        let (doc, def) = self
            .corpus
            .resolve_ref(from, reference)
            .map_err(|e| Rule::Schema(e.to_string()))?;
        let (_, name) = ref_target(from, reference);
        self.definition(def, doc, name, value, depth, hops + 1)
    }

    /// Validates a union member. The value is an object with a `$type`; a
    /// `$type` outside `refs` passes unless the union is closed.
    ///
    /// The Reference compares the `$type` with each member as written:
    /// `<nsid>#main` does not select the member `<nsid>`. `@atproto/lexicon`
    /// treated the two forms as equal.
    fn union(
        &self,
        schema: &LexRefUnion,
        from: &str,
        value: &Data,
        depth: usize,
        hops: usize,
    ) -> Result<(), ValidationError> {
        let tag = match value {
            Data::Object(fields) => fields.get(TYPE_KEY),
            _ => None,
        };
        let Some(tag) = tag else {
            return Err(wrong_type("object with $type", value));
        };
        let member = schema.refs.iter().find(|reference| {
            let (nsid, name) = ref_target(from, reference);
            is_type(tag, nsid, name)
        });
        if let Some(reference) = member {
            return self.reference(reference, from, value, depth, hops);
        }
        if schema.closed == Some(true) {
            let members: Vec<String> = schema
                .refs
                .iter()
                .map(|reference| {
                    let (nsid, name) = ref_target(from, reference);
                    type_id(nsid, name)
                })
                .collect();
            return Err(Rule::Value {
                expected: one_of(&members),
            }
            .within(TYPE_KEY));
        }
        match tag {
            Data::String(_) => Ok(()),
            other => Err(wrong_type("string", other).within(TYPE_KEY)),
        }
    }

    fn array(
        &self,
        schema: &LexArray,
        doc: &str,
        value: &Data,
        depth: usize,
    ) -> Result<(), ValidationError> {
        let Data::Array(items) = value else {
            return Err(wrong_type("array", value));
        };
        if depth > MAX_DEPTH {
            return Err(Rule::TooDeep.into());
        }
        check_len(
            Measure::Array,
            items.len(),
            schema.min_length,
            schema.max_length,
        )?;
        items.iter().enumerate().try_for_each(|(index, item)| {
            self.value(&schema.items, doc, item, depth + 1)
                .map_err(|e| e.within(&index.to_string()))
        })
    }

    /// Validates a blob reference: `{$type: "blob", ref, mimeType, size}`
    /// with no other key.
    ///
    /// The Reference checks `accept` and `maxSize` here on a write;
    /// `@atproto/lexicon` checked neither. The `raw` CID rule is in
    /// [`validate_data_model`].
    fn blob(&self, schema: &LexBlob, value: &Data) -> Result<(), ValidationError> {
        let Data::Object(fields) = value else {
            return Err(wrong_type(BLOB, value));
        };
        if !fields.contains_key(TYPE_KEY) {
            return if self.blobs == BlobRules::Read && is_legacy_blob(fields) {
                Ok(())
            } else {
                Err(Rule::Blob("not a typed blob reference").into())
            };
        }
        let (mime_type, size) = typed_blob(fields, self.blobs)?;
        if self.blobs == BlobRules::Read {
            return Ok(());
        }
        if let Some(accept) = &schema.accept
            && !mime_accepted(mime_type, accept)
        {
            return Err(Rule::Value {
                expected: one_of(accept),
            }
            .within("mimeType"));
        }
        if let Some(maximum) = schema.max_size
            && size > saturate(maximum)
        {
            return Err(Rule::TooBig {
                measure: Measure::Blob,
                maximum: saturate(maximum),
                actual: size,
            }
            .into());
        }
        Ok(())
    }
}

/// The MIME type and size of a typed blob reference, after the shape checks
/// of the Reference: a MIME type with a `/`, a size that is not negative, a
/// link, and no other key. [`BlobRules::Read`] also accepts the size `-1`,
/// which the Reference gives a converted legacy reference.
fn typed_blob(
    fields: &BTreeMap<String, Data>,
    blobs: BlobRules,
) -> Result<(&str, i64), ValidationError> {
    if !matches!(fields.get(TYPE_KEY), Some(Data::String(tag)) if tag == BLOB) {
        return Err(Rule::Value {
            expected: format!("{BLOB:?}"),
        }
        .within(TYPE_KEY));
    }
    match required(fields, "ref")? {
        Data::Link(_) => {}
        other => return Err(wrong_type("cid-link", other).within("ref")),
    }
    let mime_type = match required(fields, "mimeType")? {
        Data::String(mime_type) if mime_type.contains('/') => mime_type.as_str(),
        Data::String(_) => return Err(Rule::Blob("MIME type has no \"/\"").within("mimeType")),
        other => return Err(wrong_type("string", other).within("mimeType")),
    };
    let size = match required(fields, "size")? {
        Data::Integer(size) => *size,
        other => return Err(wrong_type("integer", other).within("size")),
    };
    let minimum = if blobs == BlobRules::Read { -1 } else { 0 };
    if size < minimum {
        return Err(Rule::TooSmall {
            measure: Measure::Blob,
            minimum,
            actual: size,
        }
        .within("size"));
    }
    if size > MAX_SAFE_INTEGER {
        return Err(unsafe_integer().within("size"));
    }
    let extra = fields
        .keys()
        .find(|key| !matches!(key.as_str(), TYPE_KEY | "ref" | "mimeType" | "size"));
    match extra {
        Some(key) => Err(Rule::Blob("unexpected key").within(key)),
        None => Ok((mime_type, size)),
    }
}

/// Whether an object is the legacy blob form: a CID string, a non-empty
/// MIME type and no other key.
fn is_legacy_blob(fields: &BTreeMap<String, Data>) -> bool {
    fields.len() == 2
        && matches!(fields.get("cid"), Some(Data::String(cid)) if CidString::parse(cid).is_ok())
        && matches!(fields.get("mimeType"), Some(Data::String(mime_type)) if !mime_type.is_empty())
}

/// Matches a MIME type against an `accept` list: `*/*`, an exact type, or a
/// `type/*` glob.
fn mime_accepted(mime_type: &str, accept: &[String]) -> bool {
    accept.iter().any(|accepted| {
        accepted == "*/*"
            || accepted == mime_type
            || accepted
                .strip_suffix('*')
                .is_some_and(|prefix| prefix.ends_with('/') && mime_type.starts_with(prefix))
    })
}

/// Validates a string: `minLength` and `maxLength` in UTF-8 bytes,
/// `minGraphemes` and `maxGraphemes` in grapheme clusters, then `format`.
/// `knownValues` is not enforced.
fn string(schema: &LexString, value: &Data) -> Result<(), ValidationError> {
    let found = match value {
        Data::String(found) => Some(found),
        _ => None,
    };
    if let Some(checked) = fixed(schema.const_.as_ref(), schema.enum_.as_deref(), found) {
        return checked;
    }
    let Some(text) = found else {
        return Err(wrong_type("string", value));
    };
    check_len(
        Measure::String,
        text.len(),
        schema.min_length,
        schema.max_length,
    )?;
    if schema.min_graphemes.is_some() || schema.max_graphemes.is_some() {
        check_len(
            Measure::Grapheme,
            text.graphemes(true).count(),
            schema.min_graphemes,
            schema.max_graphemes,
        )?;
    }
    schema
        .format
        .as_ref()
        .map_or(Ok(()), |format| string_format(format, text))
}

/// Checks a string against a lexicon string format with the syntax types of
/// `pds-types`. These are the strict forms the Reference applies on a
/// write; `@atproto/lexicon` took the lenient datetime form.
fn string_format(format: &str, text: &str) -> Result<(), ValidationError> {
    let checked = match format {
        "at-identifier" => syntax(AtIdentifier::parse(text)),
        "at-uri" => syntax(AtUri::parse(text)),
        "cid" => syntax(CidString::parse(text)),
        "datetime" => syntax(Datetime::parse(text)),
        "did" => syntax(Did::parse(text)),
        "handle" => syntax(Handle::parse(text)),
        "language" => syntax(Language::parse(text)),
        "nsid" => syntax(Nsid::parse(text)),
        "record-key" => syntax(RecordKey::parse(text)),
        "tid" => syntax(Tid::parse(text)),
        "uri" => syntax(Uri::parse(text)),
        _ => return Err(Rule::Schema(format!("unknown string format {format:?}")).into()),
    };
    checked.map_err(|e| {
        Rule::Format {
            format: format.to_owned(),
            reason: e.message.to_owned(),
        }
        .into()
    })
}

fn syntax<T>(parsed: Result<T, SyntaxError>) -> Result<(), SyntaxError> {
    parsed.map(|_| ())
}

/// Validates an integer: within plus or minus 2^53 - 1 (SPEC §6.1), then
/// `minimum` and `maximum`.
fn integer(schema: &LexInteger, value: &Data) -> Result<(), ValidationError> {
    let found = match value {
        Data::Integer(found) => Some(found),
        _ => None,
    };
    if let Some(checked) = fixed(schema.const_.as_ref(), schema.enum_.as_deref(), found) {
        return checked;
    }
    let Some(&found) = found else {
        return Err(wrong_type("integer", value));
    };
    if found.unsigned_abs() > MAX_SAFE_INTEGER.unsigned_abs() {
        return Err(unsafe_integer());
    }
    if let Some(minimum) = schema.minimum
        && found < minimum
    {
        return Err(Rule::TooSmall {
            measure: Measure::Integer,
            minimum,
            actual: found,
        }
        .into());
    }
    if let Some(maximum) = schema.maximum
        && found > maximum
    {
        return Err(Rule::TooBig {
            measure: Measure::Integer,
            maximum,
            actual: found,
        }
        .into());
    }
    Ok(())
}

fn boolean(schema: &LexBoolean, value: &Data) -> Result<(), ValidationError> {
    let found = match value {
        Data::Bool(found) => Some(found),
        _ => None,
    };
    if let Some(checked) = fixed(schema.const_.as_ref(), None, found) {
        return checked;
    }
    match found {
        Some(_) => Ok(()),
        None => Err(wrong_type("boolean", value)),
    }
}

fn bytes(schema: &LexBytes, value: &Data) -> Result<(), ValidationError> {
    let Data::Bytes(found) = value else {
        return Err(wrong_type("bytes", value));
    };
    check_len(
        Measure::Bytes,
        found.0.len(),
        schema.min_length,
        schema.max_length,
    )
}

fn cid_link(value: &Data) -> Result<(), ValidationError> {
    match value {
        Data::Link(_) => Ok(()),
        other => Err(wrong_type("cid-link", other)),
    }
}

/// Validates an `unknown` value: an object.
///
/// The Reference compiles `unknown` to a map of data-model values, so a
/// boolean, a byte string, a link and an array fail, and an object shaped
/// like a blob reference passes. The specification also forbids the blob
/// shape; `@atproto/lexicon` accepted byte strings, links and arrays.
fn unknown(value: &Data) -> Result<(), ValidationError> {
    object_fields(value).map(|_| ())
}

/// Applies `const` and `enum` as the Reference compiles them: a literal or
/// an enumeration that replaces every other constraint of the schema. A
/// `const` outside its `enum` accepts nothing. Returns `None` when the
/// schema has neither. `found` is the value when it has the schema's type.
///
/// `@atproto/lexicon` checked lengths and formats as well.
fn fixed<T: PartialEq + fmt::Debug>(
    constant: Option<&T>,
    allowed: Option<&[T]>,
    found: Option<&T>,
) -> Option<Result<(), ValidationError>> {
    let accepted: &[T] = match (constant, allowed) {
        (Some(constant), Some(allowed)) if !allowed.contains(constant) => &[],
        (Some(constant), _) => std::slice::from_ref(constant),
        (None, Some(allowed)) => allowed,
        (None, None) => return None,
    };
    Some(if found.is_some_and(|found| accepted.contains(found)) {
        Ok(())
    } else {
        Err(Rule::Value {
            expected: one_of(accepted),
        }
        .into())
    })
}

fn check_len(
    measure: Measure,
    actual: usize,
    minimum: Option<u64>,
    maximum: Option<u64>,
) -> Result<(), ValidationError> {
    let actual = i64::try_from(actual).unwrap_or(i64::MAX);
    if let Some(minimum) = minimum.map(saturate)
        && actual < minimum
    {
        return Err(Rule::TooSmall {
            measure,
            minimum,
            actual,
        }
        .into());
    }
    if let Some(maximum) = maximum.map(saturate)
        && actual > maximum
    {
        return Err(Rule::TooBig {
            measure,
            maximum,
            actual,
        }
        .into());
    }
    Ok(())
}

fn saturate(limit: u64) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

fn object_fields(value: &Data) -> Result<&BTreeMap<String, Data>, ValidationError> {
    match value {
        Data::Object(fields) => Ok(fields),
        other => Err(wrong_type("object", other)),
    }
}

fn required<'a>(
    fields: &'a BTreeMap<String, Data>,
    key: &str,
) -> Result<&'a Data, ValidationError> {
    fields.get(key).ok_or_else(|| {
        Rule::RequiredKey {
            key: key.to_owned(),
        }
        .into()
    })
}

fn wrong_type(expected: &'static str, found: &Data) -> ValidationError {
    let found = match found {
        Data::Null => "null",
        Data::Bool(_) => "boolean",
        Data::Integer(_) => "integer",
        Data::String(_) => "string",
        Data::Bytes(_) => "bytes",
        Data::Link(_) => "cid-link",
        Data::Array(_) => "array",
        Data::Object(_) => "object",
    };
    Rule::Type { expected, found }.into()
}

fn unsafe_integer() -> ValidationError {
    Rule::Type {
        expected: "integer",
        found: "integer outside the safe range",
    }
    .into()
}

/// The rule a string breaks when it is not the `$type` of a definition.
fn expected_type(doc: &str, name: &str) -> Rule {
    Rule::Value {
        expected: format!("{:?}", type_id(doc, name)),
    }
}

fn wrong_type_id(doc: &str, name: &str) -> ValidationError {
    expected_type(doc, name).within(TYPE_KEY)
}

/// The document and definition name a reference written in `from` names.
fn ref_target<'a>(from: &'a str, reference: &'a str) -> (&'a str, &'a str) {
    match reference.split_once('#') {
        Some(("", name)) => (from, name),
        Some((nsid, name)) => (nsid, name),
        None => (reference, "main"),
    }
}

/// The `$type` of a definition: `<nsid>` for `main`, else `<nsid>#<name>`.
fn type_id(nsid: &str, name: &str) -> String {
    if name == "main" {
        nsid.to_owned()
    } else {
        format!("{nsid}#{name}")
    }
}

/// Whether a value is the string that [`type_id`] gives.
fn is_type(value: &Data, nsid: &str, name: &str) -> bool {
    let Data::String(found) = value else {
        return false;
    };
    if name == "main" {
        found == nsid
    } else {
        found
            .strip_prefix(nsid)
            .and_then(|rest| rest.strip_prefix('#'))
            == Some(name)
    }
}

fn one_of<T: fmt::Debug>(values: &[T]) -> String {
    match values {
        [] => "no value".to_owned(),
        [value] => format!("{value:?}"),
        _ => {
            let listed: Vec<String> = values.iter().map(|value| format!("{value:?}")).collect();
            format!("one of {}", listed.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;
    use crate::model::LexiconDoc;

    /// Vectors that do not pass, as `<file>: <case>`, with the finding that
    /// explains each. A listed case must still fail: remove the entry when
    /// it starts to pass.
    const KNOWN_FAILURES: &[(&str, &str)] = &[];

    /// Records one vector outcome, checked against `KNOWN_FAILURES`.
    fn check(case: &str, passes: bool, detail: &str, problems: &mut Vec<String>) {
        let known = KNOWN_FAILURES.iter().find(|(name, _)| *name == case);
        match (known, passes) {
            (None, true) | (Some(_), false) => {}
            (None, false) => problems.push(format!("{case}: {detail}")),
            (Some((_, finding)), true) => problems.push(format!(
                "{case}: passes, remove it from KNOWN_FAILURES ({finding})"
            )),
        }
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn cases(path: &str) -> Vec<Value> {
        let text = std::fs::read_to_string(repo_root().join(path)).expect("vector file");
        serde_json::from_str(&text).expect("vector JSON")
    }

    fn parse(value: &Value) -> Result<Data, String> {
        let text = serde_json::to_string(value).expect("JSON text");
        Data::from_json_str(&text).map_err(|e| e.to_string())
    }

    fn data(value: &Value) -> Data {
        parse(value).expect("a data model value")
    }

    fn catalog() -> Corpus {
        Corpus::load_dir(&repo_root().join("interop/lexicon/catalog")).expect("catalog")
    }

    fn lexicons() -> Corpus {
        Corpus::load_dir(&repo_root().join("lexicons")).expect("lexicons")
    }

    fn code(rule: &Rule) -> &'static str {
        match rule {
            Rule::Type { .. } => "type",
            Rule::Value { .. } => "value",
            Rule::RequiredKey { .. } => "required_key",
            Rule::Format { .. } => "format",
            Rule::TooSmall { .. } => "too_small",
            Rule::TooBig { .. } => "too_big",
            Rule::Blob(_) => "blob",
            Rule::Empty => "empty",
            Rule::TooDeep => "too_deep",
            Rule::UnknownLexicon { .. } => "unknown_lexicon",
            Rule::Schema(_) => "schema",
        }
    }

    /// The path and rule of a rejection, or `accepted`.
    fn outcome(result: &Result<(), ValidationError>) -> String {
        match result {
            Ok(()) => "accepted".to_owned(),
            Err(e) => format!("{}: {}", e.path, code(&e.rule)),
        }
    }

    #[test]
    fn record_data_valid_vectors_validate() {
        let corpus = catalog();
        let mut problems = Vec::new();
        for case in cases("interop/lexicon/record-data-valid.json") {
            let name = case["name"].as_str().expect("name");
            let rkey = case["rkey"].as_str().expect("rkey");
            let result = parse(&case["data"])
                .map_err(|e| format!("JSON parsing: {e}"))
                .and_then(|data| {
                    corpus
                        .validate_record(&data)
                        .map_err(|e| format!("validate_record: {e}"))?;
                    let nsid = data.type_name().expect("$type");
                    let key = &corpus.record(nsid).expect("record lexicon").key;
                    validate_record_key(key, rkey).map_err(|e| format!("validate_record_key: {e}"))
                });
            check(
                &format!("record-data-valid: {name}"),
                result.is_ok(),
                &result.err().unwrap_or_default(),
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// What rejects each `record-data-invalid` case, in file order: the path
    /// and the rule from `validate_record`, or `json` for JSON parsing into
    /// `Data`.
    ///
    /// The three `unknown wrong type` cases omit the required `integer`, so
    /// `validate_record` fails on that key and does not reach `unknown`;
    /// `unknown_is_an_object` covers the type.
    const INVALID_RECORD_DATA: &[(&str, &str)] = &[
        ("missing required field", "record: required_key"),
        ("invalid boolean field", "record/boolean: type"),
        ("invalid integer field", "record/integer: type"),
        ("invalid non-nullable string field", "record/string: type"),
        ("invalid string field", "record/string: type"),
        ("invalid bytes field", "record/bytes: type"),
        ("invalid bytes: empty object", "record/bytes: type"),
        ("invalid bytes: wrong type", "record/bytes: type"),
        ("invalid cid-link field", "record/cid-link: type"),
        ("invalid blob field", "record/blob: type"),
        ("invalid blob: wrong type", "record/blob: blob"),
        ("invalid array", "record/array: type"),
        ("invalid array element", "record/array/0: type"),
        ("object wrong data type", "record/object: type"),
        ("object nested wrong data type", "record/object/a: type"),
        ("invalid token ref type", "record/ref: type"),
        ("invalid ref value", "record/ref: type"),
        (
            "invalid string format handle",
            "record/formats/handle: format",
        ),
        ("invalid string format did", "record/formats/did: format"),
        (
            "invalid string format atidentifier",
            "record/formats/atidentifier: format",
        ),
        ("invalid string format nsid", "record/formats/nsid: format"),
        (
            "invalid string format aturi",
            "record/formats/aturi: format",
        ),
        ("invalid string format cid", "record/formats/cid: format"),
        (
            "invalid string format datetime",
            "record/formats/datetime: format",
        ),
        (
            "invalid string format language",
            "record/formats/language: format",
        ),
        ("invalid string format uri", "record/formats/uri: format"),
        ("invalid string format tid", "record/formats/tid: format"),
        (
            "invalid string format recordkey",
            "record/formats/recordkey: format",
        ),
        ("wrong const value", "record/constInteger: value"),
        ("integer not in enum", "record/enumInteger: value"),
        ("out of integer range", "record/rangeInteger: too_big"),
        ("string too short", "record/lenString: too_small"),
        ("string too long", "record/lenString: too_big"),
        (
            "string too short (graphemes)",
            "record/graphemeString: too_small",
        ),
        (
            "string too long (graphemes)",
            "record/graphemeString: too_big",
        ),
        ("out of enum string", "record/enumString: value"),
        ("bytes too short", "record/sizeBytes: too_small"),
        ("bytes too long", "record/sizeBytes: too_big"),
        ("array too short", "record/lenArray: too_small"),
        ("array too long", "record/lenArray: too_big"),
        ("blob too large", "record/sizeBlob: too_big"),
        ("blob wrong type", "record/acceptBlob/mimeType: value"),
        ("open union wrong data type", "record/union: type"),
        ("open union missing $type", "record/union: type"),
        ("out of closed union", "record/closedUnion/$type: value"),
        ("union inner invalid", "record/closedUnion/$type: value"),
        ("union inner invalid", "record/union/a: type"),
        ("unknown wrong type (bool)", "record: required_key"),
        ("unknown wrong type (bytes)", "record: required_key"),
        ("unknown wrong type (blob)", "record: required_key"),
    ];

    #[test]
    fn record_data_invalid_vectors_are_rejected() {
        let corpus = catalog();
        let cases = cases("interop/lexicon/record-data-invalid.json");
        assert_eq!(cases.len(), INVALID_RECORD_DATA.len());
        let mut problems = Vec::new();
        let mut also_data_model = Vec::new();
        for (case, (expected_name, expected)) in cases.iter().zip(INVALID_RECORD_DATA) {
            let name = case["name"].as_str().expect("name");
            assert_eq!(name, *expected_name, "the table follows the file order");
            let found = match parse(&case["data"]) {
                Ok(data) => {
                    if validate_data_model(&data).is_err() {
                        also_data_model.push(name);
                    }
                    outcome(&corpus.validate_record(&data))
                }
                Err(_) => "json".to_owned(),
            };
            check(
                &format!("record-data-invalid: {name}"),
                found == *expected,
                &format!("validate_record gave {found:?}, expected {expected:?}"),
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
        // These blob references carry `dag-cbor` CIDs, so the data-model
        // check rejects them as well.
        assert_eq!(
            also_data_model,
            [
                "blob too large",
                "blob wrong type",
                "unknown wrong type (blob)"
            ]
        );
    }

    #[test]
    fn the_full_vector_fails_the_data_model_check_for_its_blob_cids() {
        let corpus = catalog();
        let cases = cases("interop/lexicon/record-data-valid.json");
        let full = &cases
            .iter()
            .find(|case| case["name"] == "full")
            .expect("full")["data"];
        // `123` is not canonical base64: its last character has unused bits
        // set. The Reference reads two bytes, and so does `pds-types`.
        assert_eq!(full["bytes"], json!({ "$bytes": "123" }));
        let record = data(full);
        assert_eq!(outcome(&corpus.validate_record(&record)), "accepted");
        // The blob references of the vector carry `dag-cbor` CIDs, which
        // SPEC §6.3 and the Reference's strict blob check reject.
        assert_eq!(
            outcome(&validate_data_model(&record)),
            "record/acceptBlob/ref: blob"
        );
        let minimal = data(&cases[0]["data"]);
        assert_eq!(outcome(&validate_data_model(&minimal)), "accepted");
    }

    #[test]
    fn data_model_valid_vectors_pass() {
        let mut problems = Vec::new();
        for case in cases("interop/data-model/data-model-valid.json") {
            let note = case["note"].as_str().expect("note");
            let result = parse(&case["json"])
                .map_err(|e| format!("JSON parsing: {e}"))
                .and_then(|data| {
                    validate_data_model(&data).map_err(|e| format!("validate_data_model: {e}"))
                });
            check(
                &format!("data-model-valid: {note}"),
                result.is_ok(),
                &result.err().unwrap_or_default(),
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// What rejects each `data-model-invalid` case, in file order: `json`
    /// for JSON parsing into `Data`, else the path and rule from
    /// `validate_data_model`.
    const INVALID_DATA_MODEL: &[(&str, &str)] = &[
        ("top-level not an object", "record: type"),
        ("float", "json"),
        ("record with $type null", "record/rcrd/$type: type"),
        ("record with $type wrong type", "record/rcrd/$type: type"),
        ("record with empty $type string", "record/rcrd/$type: empty"),
        ("blob with string size", "record/blb/size: type"),
        ("blob with missing key", "record/blb: required_key"),
        ("bytes with wrong field type", "json"),
        ("bytes with extra fields", "json"),
        ("link with wrong field type", "json"),
        ("link with bogus CID", "json"),
        ("link with extra fields", "json"),
    ];

    #[test]
    fn data_model_invalid_vectors_are_rejected() {
        let cases = cases("interop/data-model/data-model-invalid.json");
        assert_eq!(cases.len(), INVALID_DATA_MODEL.len());
        let mut problems = Vec::new();
        for (case, (expected_note, expected)) in cases.iter().zip(INVALID_DATA_MODEL) {
            let note = case["note"].as_str().expect("note");
            assert_eq!(note, *expected_note, "the table follows the file order");
            let found = parse(&case["json"]).map_or_else(
                |_| "json".to_owned(),
                |data| outcome(&validate_data_model(&data)),
            );
            check(
                &format!("data-model-invalid: {note}"),
                found == *expected,
                &format!("got {found:?}, expected {expected:?}"),
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    const RAW_CID: &str = "bafkreiccldh766hwcnuxnf2wh6jgzepf2nlu2lvcllt63eww5p6chi4ity";
    const CBOR_CID: &str = "bafyreiclp443lavogvhj3d2ob2cxbfuscni2k5jk7bebjzg7khl3esabwq";

    fn blob(mime_type: &str, size: i64) -> Value {
        json!({ "$type": "blob", "ref": { "$link": RAW_CID }, "mimeType": mime_type, "size": size })
    }

    fn post() -> Value {
        let parent = json!({
            "uri": "at://did:plc:ewvi7nxzyoun6zhxrhs64oiz/app.bsky.feed.post/3kznmn7xqxl22",
            "cid": CBOR_CID
        });
        json!({
            "$type": "app.bsky.feed.post",
            "text": "Read the specification at atproto.com @alice.example #atproto",
            "createdAt": "2026-10-12T09:30:00.000Z",
            "langs": ["en", "pt-BR"],
            "facets": [
                {
                    "index": { "byteStart": 26, "byteEnd": 37 },
                    "features": [
                        { "$type": "app.bsky.richtext.facet#link", "uri": "https://atproto.com" }
                    ]
                },
                {
                    "index": { "byteStart": 38, "byteEnd": 52 },
                    "features": [{
                        "$type": "app.bsky.richtext.facet#mention",
                        "did": "did:plc:ewvi7nxzyoun6zhxrhs64oiz"
                    }]
                },
                {
                    "index": { "byteStart": 53, "byteEnd": 61 },
                    "features": [{ "$type": "app.bsky.richtext.facet#tag", "tag": "atproto" }]
                }
            ],
            "reply": { "root": parent, "parent": parent },
            "embed": {
                "$type": "app.bsky.embed.images",
                "images": [{
                    "alt": "A diagram",
                    "image": blob("image/png", 51_234),
                    "aspectRatio": { "width": 16, "height": 9 }
                }]
            },
            "labels": {
                "$type": "com.atproto.label.defs#selfLabels",
                "values": [{ "val": "graphic-media" }]
            },
            "tags": ["atproto"]
        })
    }

    /// The valid post with the value at a JSON pointer replaced.
    fn post_with(pointer: &str, value: Value) -> Value {
        let mut record = post();
        *record.pointer_mut(pointer).expect("pointer") = value;
        record
    }

    /// The valid post without one key of the object at a JSON pointer.
    fn post_without(pointer: &str, key: &str) -> Value {
        let mut record = post();
        let object = record.pointer_mut(pointer).expect("pointer");
        object.as_object_mut().expect("object").remove(key);
        record
    }

    #[test]
    fn a_post_validates_against_the_vendored_corpus() {
        let corpus = lexicons();
        let record = data(&post());
        assert_eq!(outcome(&validate_data_model(&record)), "accepted");
        assert_eq!(outcome(&corpus.validate_record(&record)), "accepted");
        let key = &corpus.record("app.bsky.feed.post").expect("post").key;
        assert_eq!(
            outcome(&validate_record_key(key, "3kznmn7xqxl22")),
            "accepted"
        );
        assert_eq!(outcome(&validate_record_key(key, "self")), "rkey: format");
    }

    #[test]
    fn invalid_posts_are_rejected() {
        let long = "a".repeat(129);
        let image = "/embed/images/0/image";
        let cases = [
            (post_without("", "createdAt"), "record: required_key"),
            (
                post_with("/createdAt", json!("2026-10-12 09:30:00")),
                "record/createdAt: format",
            ),
            (post_with("/text", json!(5)), "record/text: type"),
            (
                post_with("/langs", json!(["en", "pt", "de", "fr"])),
                "record/langs: too_big",
            ),
            (
                post_with("/langs/1", json!("english!")),
                "record/langs/1: format",
            ),
            (
                post_with("/facets/0/features/0/uri", json!("no scheme")),
                "record/facets/0/features/0/uri: format",
            ),
            (
                post_with("/facets/1/features/0/did", json!("alice.example")),
                "record/facets/1/features/0/did: format",
            ),
            (
                post_with("/facets/2/index/byteStart", json!(-1)),
                "record/facets/2/index/byteStart: too_small",
            ),
            (
                post_without("/reply", "parent"),
                "record/reply: required_key",
            ),
            (
                post_with("/reply/root/cid", json!("nope")),
                "record/reply/root/cid: format",
            ),
            (post_without("/embed", "$type"), "record/embed: type"),
            (
                post_with(image, blob("application/pdf", 51_234)),
                "record/embed/images/0/image/mimeType: value",
            ),
            (
                post_with(image, blob("image/png", 2_000_001)),
                "record/embed/images/0/image: too_big",
            ),
            (
                post_with(image, json!({ "cid": RAW_CID, "mimeType": "image/png" })),
                "record/embed/images/0/image: blob",
            ),
            (
                post_with("/embed/images/0/aspectRatio/width", json!(0)),
                "record/embed/images/0/aspectRatio/width: too_small",
            ),
            (
                post_with("/labels/values/0/val", json!(long)),
                "record/labels/values/0/val: too_big",
            ),
        ];
        let corpus = lexicons();
        for (record, expected) in cases {
            assert_eq!(outcome(&corpus.validate_record(&data(&record))), expected);
        }
    }

    #[test]
    fn post_text_is_limited_in_bytes_and_in_graphemes() {
        let corpus = lexicons();
        let rule = |text: String| {
            corpus
                .validate_record(&data(&post_with("/text", json!(text))))
                .map_err(|e| e.rule)
        };
        // 300 graphemes of two bytes each.
        assert_eq!(rule("é".repeat(300)), Ok(()));
        assert_eq!(
            rule("é".repeat(301)),
            Err(Rule::TooBig {
                measure: Measure::Grapheme,
                maximum: 300,
                actual: 301
            })
        );
        // One grapheme of 25 bytes: four people joined by zero-width joiners.
        let family = "👩‍👩‍👦‍👦";
        assert_eq!(rule(family.repeat(120)), Ok(()));
        assert_eq!(
            rule(family.repeat(121)),
            Err(Rule::TooBig {
                measure: Measure::String,
                maximum: 3000,
                actual: 3025
            })
        );
    }

    #[test]
    fn an_open_union_accepts_a_type_outside_its_refs() {
        let corpus = lexicons();
        let record = post_with("/embed", json!({ "$type": "com.example.embed", "x": 1 }));
        assert_eq!(outcome(&corpus.validate_record(&data(&record))), "accepted");
    }

    #[test]
    fn a_lexicon_schema_record_validates_against_the_vendored_corpus() {
        let corpus = lexicons();
        let record = json!({
            "$type": "com.atproto.lexicon.schema",
            "lexicon": 1,
            "id": "com.example.thing",
            "defs": { "main": { "type": "token" } }
        });
        assert_eq!(outcome(&corpus.validate_record(&data(&record))), "accepted");
        let key = &corpus
            .record("com.atproto.lexicon.schema")
            .expect("schema")
            .key;
        assert_eq!(
            outcome(&validate_record_key(key, "com.example.thing")),
            "accepted"
        );
        assert_eq!(outcome(&validate_record_key(key, "self")), "rkey: format");

        let mut without = record.clone();
        without.as_object_mut().expect("object").remove("lexicon");
        assert_eq!(
            outcome(&corpus.validate_record(&data(&without))),
            "record: required_key"
        );
        let mut text = record;
        text["lexicon"] = json!("1");
        assert_eq!(
            outcome(&corpus.validate_record(&data(&text))),
            "record/lexicon: type"
        );
    }

    #[test]
    fn a_type_without_a_record_lexicon_is_reported_as_unknown() {
        let corpus = lexicons();
        for nsid in [
            "com.example.unknown",
            // An object, not a record.
            "com.atproto.repo.strongRef",
            // The Reference looks the `$type` up as written.
            "app.bsky.feed.post#main",
        ] {
            let result = corpus.validate_record(&data(&json!({ "$type": nsid })));
            assert_eq!(outcome(&result), "record/$type: unknown_lexicon");
        }
        let untyped = corpus.validate_record(&data(&json!({ "text": "x" })));
        assert_eq!(outcome(&untyped), "record: required_key");
        let number = corpus.validate_record(&data(&json!({ "$type": 5 })));
        assert_eq!(outcome(&number), "record/$type: type");
        assert_eq!(
            outcome(&corpus.validate_record(&Data::Null)),
            "record: type"
        );
    }

    #[test]
    fn record_key_types_are_enforced() {
        let cases = [
            ("tid", "3kznmn7xqxl22", "accepted"),
            ("tid", "self", "rkey: format"),
            ("nsid", "com.example.thing", "accepted"),
            ("nsid", "self", "rkey: format"),
            ("any", "self", "accepted"),
            ("any", "a/b", "rkey: format"),
            ("any", "..", "rkey: format"),
            ("literal:self", "self", "accepted"),
            ("literal:self", "other", "rkey: value"),
            ("literal:self", "", "rkey: format"),
            ("uuid", "self", "rkey: schema"),
        ];
        for (key_type, rkey, expected) in cases {
            assert_eq!(
                outcome(&validate_record_key(key_type, rkey)),
                expected,
                "{key_type} {rkey:?}"
            );
        }
    }

    #[test]
    fn every_vendored_string_format_and_record_key_type_is_known() {
        fn formats(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    if map.get("type") == Some(&json!("string"))
                        && let Some(Value::String(format)) = map.get("format")
                    {
                        out.push(format.clone());
                    }
                    map.values().for_each(|v| formats(v, out));
                }
                Value::Array(items) => items.iter().for_each(|v| formats(v, out)),
                _ => {}
            }
        }
        let corpus = lexicons();
        let mut found = Vec::new();
        for doc in corpus.docs() {
            formats(&serde_json::to_value(doc).expect("doc"), &mut found);
            if let Some(record) = corpus.record(&doc.id) {
                let result = validate_record_key(&record.key, "self");
                assert_ne!(outcome(&result), "rkey: schema", "{}", doc.id);
            }
        }
        found.sort();
        found.dedup();
        assert!(found.len() > 5, "formats found: {found:?}");
        for format in found {
            let result = string_format(&format, "");
            assert!(
                !matches!(
                    result,
                    Err(ValidationError {
                        rule: Rule::Schema(_),
                        ..
                    })
                ),
                "unknown string format {format}"
            );
        }
    }

    /// Two documents that exercise the rules the vectors do not reach.
    fn rules() -> Corpus {
        let docs = [
            json!({
                "lexicon": 1,
                "id": "com.example.rules",
                "defs": {
                    "main": {
                        "type": "record",
                        "key": "any",
                        "record": {
                            "type": "object",
                            "nullable": ["maybe"],
                            "properties": {
                                "constString": { "type": "string", "const": "abc", "maxLength": 1 },
                                "enumString": { "type": "string", "enum": ["long"], "maxLength": 1 },
                                "impossible": { "type": "string", "const": "x", "enum": ["y"] },
                                "flag": { "type": "boolean", "const": false },
                                "maybe": { "type": "string" },
                                "number": { "type": "integer" },
                                "defaulted": { "type": "ref", "ref": "#defaulted" },
                                "token": { "type": "ref", "ref": "#tok" },
                                "item": { "type": "ref", "ref": "#item" },
                                "open": { "type": "union", "refs": ["#item", "com.example.other"] },
                                "closed": {
                                    "type": "union",
                                    "closed": true,
                                    "refs": ["#item", "com.example.other"]
                                },
                                "image": {
                                    "type": "blob",
                                    "accept": ["image/*", "application/pdf"],
                                    "maxSize": 100
                                },
                                "anyBlob": { "type": "blob", "accept": ["*/*"] },
                                "anything": { "type": "unknown" },
                                "cycle": { "type": "ref", "ref": "#a" },
                                "dangling": { "type": "ref", "ref": "#nowhere" },
                                "method": { "type": "ref", "ref": "com.example.query" },
                                "odd": { "type": "string", "format": "made-up" },
                                "tree": { "type": "ref", "ref": "#node" }
                            }
                        }
                    },
                    "defaulted": {
                        "type": "object",
                        "required": ["n"],
                        "properties": { "n": { "type": "integer", "default": 7 } }
                    },
                    "tok": { "type": "token" },
                    "item": { "type": "object", "properties": { "a": { "type": "integer" } } },
                    "a": { "type": "ref", "ref": "#b" },
                    "b": { "type": "ref", "ref": "#a" },
                    "node": {
                        "type": "object",
                        "properties": {
                            "children": { "type": "array", "items": { "type": "ref", "ref": "#node" } }
                        }
                    }
                }
            }),
            json!({
                "lexicon": 1,
                "id": "com.example.other",
                "defs": {
                    "main": {
                        "type": "object",
                        "required": ["v"],
                        "properties": { "v": { "type": "string" } }
                    }
                }
            }),
            json!({
                "lexicon": 1,
                "id": "com.example.query",
                "defs": { "main": { "type": "query" } }
            }),
        ];
        let mut corpus = Corpus::new();
        for doc in docs {
            let doc: LexiconDoc = serde_json::from_value(doc).expect("lexicon");
            corpus.insert(doc).expect("unique id");
        }
        corpus
    }

    /// The outcome of validating a `com.example.rules` record with one
    /// property.
    fn rule_for(key: &str, value: Value) -> String {
        rule_in(BlobRules::Write, key, value)
    }

    fn rule_in(blobs: BlobRules, key: &str, value: Value) -> String {
        let mut record = json!({ "$type": "com.example.rules" });
        record[key] = value;
        outcome(&rules().validate_record_with(&data(&record), blobs))
    }

    #[test]
    fn const_and_enum_replace_the_other_constraints() {
        assert_eq!(rule_for("constString", json!("abc")), "accepted");
        assert_eq!(
            rule_for("constString", json!("a")),
            "record/constString: value"
        );
        assert_eq!(
            rule_for("constString", json!(1)),
            "record/constString: value"
        );
        assert_eq!(rule_for("enumString", json!("long")), "accepted");
        assert_eq!(
            rule_for("enumString", json!("l")),
            "record/enumString: value"
        );
        assert_eq!(
            rule_for("impossible", json!("x")),
            "record/impossible: value"
        );
        assert_eq!(
            rule_for("impossible", json!("y")),
            "record/impossible: value"
        );
        assert_eq!(rule_for("flag", json!(false)), "accepted");
        assert_eq!(rule_for("flag", json!(true)), "record/flag: value");
    }

    #[test]
    fn nullable_and_required_follow_the_object_schema() {
        assert_eq!(rule_for("maybe", json!(null)), "accepted");
        assert_eq!(rule_for("maybe", json!("x")), "accepted");
        assert_eq!(rule_for("number", json!(null)), "record/number: type");
        // A default does not stand in for a required property.
        assert_eq!(
            rule_for("defaulted", json!({})),
            "record/defaulted: required_key"
        );
        assert_eq!(rule_for("defaulted", json!({ "n": 1 })), "accepted");
        // Properties outside the schema are allowed.
        assert_eq!(rule_for("undeclared", json!([1, "two"])), "accepted");
    }

    #[test]
    fn an_integer_stays_within_the_safe_range() {
        let record = |number: i64| {
            Data::Object(BTreeMap::from([
                (
                    TYPE_KEY.to_owned(),
                    Data::String("com.example.rules".to_owned()),
                ),
                ("number".to_owned(), Data::Integer(number)),
            ]))
        };
        let corpus = rules();
        assert_eq!(
            outcome(&corpus.validate_record(&record(MAX_SAFE_INTEGER))),
            "accepted"
        );
        assert_eq!(
            outcome(&corpus.validate_record(&record(-MAX_SAFE_INTEGER - 1))),
            "record/number: type"
        );
    }

    #[test]
    fn a_reference_checks_the_type_it_leads_to() {
        assert_eq!(
            rule_for("token", json!("com.example.rules#tok")),
            "accepted"
        );
        assert_eq!(
            rule_for("token", json!("com.example.rules#other")),
            "record/token: value"
        );
        assert_eq!(rule_for("token", json!(5)), "record/token: type");
        assert_eq!(rule_for("item", json!({ "a": 1 })), "accepted");
        let typed = json!({ "$type": "com.example.rules#item", "a": 1 });
        assert_eq!(rule_for("item", typed), "accepted");
        let other = json!({ "$type": "com.example.other", "a": 1 });
        assert_eq!(rule_for("item", other), "record/item/$type: value");
        assert_eq!(
            rule_for("item", json!({ "$type": 5 })),
            "record/item/$type: value"
        );
        assert_eq!(rule_for("item", json!({ "a": "x" })), "record/item/a: type");
    }

    #[test]
    fn a_union_selects_its_member_by_the_type_as_written() {
        let item = json!({ "$type": "com.example.rules#item", "a": 1 });
        let other = json!({ "$type": "com.example.other", "v": "x" });
        for key in ["open", "closed"] {
            assert_eq!(rule_for(key, item.clone()), "accepted");
            assert_eq!(rule_for(key, other.clone()), "accepted");
            let bad = json!({ "$type": "com.example.other" });
            assert_eq!(rule_for(key, bad), format!("record/{key}: required_key"));
            assert_eq!(
                rule_for(key, json!({ "v": "x" })),
                format!("record/{key}: type")
            );
            assert_eq!(rule_for(key, json!("x")), format!("record/{key}: type"));
        }
        // `<nsid>#main` is not the member `<nsid>`: an open union does not
        // validate it and a closed union rejects it.
        let main = json!({ "$type": "com.example.other#main" });
        assert_eq!(rule_for("open", main.clone()), "accepted");
        assert_eq!(rule_for("closed", main), "record/closed/$type: value");
        assert_eq!(
            rule_for("open", json!({ "$type": "com.example.new" })),
            "accepted"
        );
        assert_eq!(
            rule_for("open", json!({ "$type": 5 })),
            "record/open/$type: type"
        );
        assert_eq!(
            rule_for("closed", json!({ "$type": 5 })),
            "record/closed/$type: value"
        );
    }

    #[test]
    fn a_blob_meets_accept_and_max_size_on_a_write() {
        assert_eq!(rule_for("image", blob("image/png", 100)), "accepted");
        assert_eq!(rule_for("image", blob("application/pdf", 0)), "accepted");
        assert_eq!(
            rule_for("image", blob("image/png", 101)),
            "record/image: too_big"
        );
        assert_eq!(
            rule_for("image", blob("text/plain", 1)),
            "record/image/mimeType: value"
        );
        assert_eq!(
            rule_for("image", blob("imagex/png", 1)),
            "record/image/mimeType: value"
        );
        assert_eq!(rule_for("anyBlob", blob("text/plain", 1)), "accepted");
        assert_eq!(
            rule_for("anyBlob", blob("plain", 1)),
            "record/anyBlob/mimeType: blob"
        );
        assert_eq!(
            rule_for("anyBlob", blob("text/plain", -1)),
            "record/anyBlob/size: too_small"
        );
        let mut extra = blob("text/plain", 1);
        extra["name"] = json!("a.txt");
        assert_eq!(rule_for("anyBlob", extra), "record/anyBlob/name: blob");
        let mut untyped = blob("text/plain", 1);
        untyped["$type"] = json!("com.example.blob");
        assert_eq!(rule_for("anyBlob", untyped), "record/anyBlob/$type: value");
        let mut unlinked = blob("text/plain", 1);
        unlinked["ref"] = json!(RAW_CID);
        assert_eq!(rule_for("anyBlob", unlinked), "record/anyBlob/ref: type");
    }

    #[test]
    fn the_legacy_blob_form_is_accepted_on_read_only() {
        let legacy = json!({ "cid": RAW_CID, "mimeType": "image/png" });
        assert_eq!(
            rule_in(BlobRules::Write, "image", legacy.clone()),
            "record/image: blob"
        );
        assert_eq!(rule_in(BlobRules::Read, "image", legacy), "accepted");
        let bad_cid = json!({ "cid": "nope", "mimeType": "image/png" });
        assert_eq!(
            rule_in(BlobRules::Read, "image", bad_cid),
            "record/image: blob"
        );
        // On read the declared type and size are not held to the lexicon.
        assert_eq!(
            rule_in(BlobRules::Read, "image", blob("text/plain", 101)),
            "accepted"
        );
        assert_eq!(
            rule_in(BlobRules::Read, "image", blob("text/plain", -1)),
            "accepted"
        );
        assert_eq!(
            rule_in(BlobRules::Read, "image", blob("text/plain", -2)),
            "record/image/size: too_small"
        );
    }

    #[test]
    fn unknown_is_an_object() {
        assert_eq!(rule_for("anything", json!({ "a": [1, null] })), "accepted");
        assert_eq!(rule_for("anything", json!({})), "accepted");
        for value in [
            json!(false),
            json!(null),
            json!("text"),
            json!([1]),
            json!({ "$bytes": "AQID" }),
            json!({ "$link": RAW_CID }),
        ] {
            assert_eq!(rule_for("anything", value), "record/anything: type");
        }
        // The Reference accepts an object shaped like a blob reference; the
        // specification forbids it.
        assert_eq!(rule_for("anything", blob("text/plain", 1)), "accepted");
    }

    #[test]
    fn a_lexicon_that_cannot_be_evaluated_is_a_schema_error() {
        assert_eq!(rule_for("cycle", json!(1)), "record/cycle: schema");
        assert_eq!(rule_for("dangling", json!(1)), "record/dangling: schema");
        assert_eq!(rule_for("method", json!(1)), "record/method: schema");
        assert_eq!(rule_for("odd", json!("x")), "record/odd: schema");
    }

    /// `containers` nested objects and arrays: a node, its `children`, a
    /// node, and so on.
    fn tree(containers: usize) -> Data {
        let mut value: Option<Data> = None;
        for level in (0..containers).rev() {
            value = Some(if level % 2 == 0 {
                Data::Object(
                    value
                        .map(|inner| ("children".to_owned(), inner))
                        .into_iter()
                        .collect(),
                )
            } else {
                Data::Array(value.into_iter().collect())
            });
        }
        value.expect("at least one container")
    }

    #[test]
    fn nesting_stops_at_32_levels() {
        // The record is level 1, so `containers` more reach level
        // `containers + 1`.
        let record = |containers: usize| {
            Data::Object(BTreeMap::from([
                (
                    TYPE_KEY.to_owned(),
                    Data::String("com.example.rules".to_owned()),
                ),
                ("tree".to_owned(), tree(containers)),
            ]))
        };
        let corpus = rules();
        let at_limit = record(MAX_DEPTH - 1);
        assert_eq!(outcome(&validate_data_model(&at_limit)), "accepted");
        assert_eq!(outcome(&corpus.validate_record(&at_limit)), "accepted");
        let beyond = record(MAX_DEPTH);
        let path = format!("record/tree{}", "/children/0".repeat((MAX_DEPTH - 1) / 2));
        let expected = format!("{path}/children: too_deep");
        assert_eq!(outcome(&validate_data_model(&beyond)), expected);
        assert_eq!(outcome(&corpus.validate_record(&beyond)), expected);
    }

    #[test]
    fn the_data_model_rules_hold_at_any_depth() {
        let check = |value: Value| outcome(&validate_data_model(&data(&value)));
        assert_eq!(
            check(json!({ "a": [{ "b": blob("image/png", 1) }] })),
            "accepted"
        );
        // Unknown `$`-prefixed keys are ignored, and the legacy blob form
        // has no `$type`.
        assert_eq!(
            check(json!({ "$unknown": 1, "$type": "com.example.x" })),
            "accepted"
        );
        assert_eq!(
            check(json!({ "b": { "cid": RAW_CID, "mimeType": "image/png" } })),
            "accepted"
        );
        assert_eq!(
            check(json!({ "a": [{ "$type": "" }] })),
            "record/a/0/$type: empty"
        );
        assert_eq!(
            check(json!({ "a": [{ "$type": ["x"] }] })),
            "record/a/0/$type: type"
        );
        assert_eq!(
            check(json!({ "b": blob("image/png", 0) })),
            "record/b/size: too_small"
        );
        assert_eq!(
            check(json!({ "b": blob("", 1) })),
            "record/b/mimeType: empty"
        );
        let mut cbor = blob("image/png", 1);
        cbor["ref"] = json!({ "$link": CBOR_CID });
        assert_eq!(check(json!({ "b": cbor })), "record/b/ref: blob");
        let mut text = blob("image/png", 1);
        text["ref"] = json!(RAW_CID);
        assert_eq!(check(json!({ "b": text })), "record/b/ref: type");
        assert_eq!(check(json!([])), "record: type");
    }

    #[test]
    fn an_error_names_the_path_and_the_rule() {
        let corpus = catalog();
        let record = json!({
            "$type": "example.lexicon.record",
            "integer": 1,
            "formats": { "handle": "123" }
        });
        let error = corpus
            .validate_record(&data(&record))
            .expect_err("invalid handle");
        assert_eq!(error.path, "record/formats/handle");
        assert!(matches!(&error.rule, Rule::Format { format, .. } if format == "handle"));
        assert!(
            error
                .to_string()
                .starts_with("record/formats/handle: invalid handle: ")
        );
        let _: &dyn std::error::Error = &error;
    }
}
