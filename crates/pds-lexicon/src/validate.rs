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
mod tests;
