//! The lexicon schema language as Rust data (atproto Lexicon specification,
//! revision with `permission-set` and `permission` types). This is the model
//! the loader fills, the validator walks and the code generator reads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One lexicon document: `{"lexicon": 1, "id": "<nsid>", "defs": {...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexiconDoc {
    /// Always `1`.
    pub lexicon: u32,
    /// The NSID the document defines.
    pub id: String,
    /// Optional revision number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Definitions by name; `main` is the primary definition.
    pub defs: BTreeMap<String, LexUserType>,
}

/// A top-level definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexUserType {
    /// A record type stored in repositories.
    Record(LexRecord),
    /// An XRPC query (HTTP GET).
    Query(LexXrpcQuery),
    /// An XRPC procedure (HTTP POST).
    Procedure(LexXrpcProcedure),
    /// An XRPC subscription (WebSocket).
    Subscription(LexXrpcSubscription),
    /// A permission set for OAuth scopes.
    PermissionSet(LexPermissionSet),
    /// A single permission, used inside permission sets.
    Permission(LexPermission),
    /// A reusable object.
    Object(LexObject),
    /// A token: a named constant.
    Token(LexToken),
    /// A string type.
    String(LexString),
    /// An integer type.
    Integer(LexInteger),
    /// A boolean type.
    Boolean(LexBoolean),
    /// A byte string type.
    Bytes(LexBytes),
    /// A CID link type.
    CidLink(LexCidLink),
    /// A blob reference type.
    Blob(LexBlob),
    /// An array type.
    Array(LexArray),
    /// A reference to another definition.
    Ref(LexRef),
    /// A union of references.
    Union(LexRefUnion),
    /// Any data model value.
    Unknown(LexUnknown),
}

/// A record definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    /// The record key type: `tid`, `nsid`, `any`, or `literal:<value>`.
    pub key: String,
    /// The record's object schema; the lexicon requires `"type": "object"`.
    pub record: LexRecordSchema,
}

/// A record schema is always an object; the tag is kept so a record whose
/// schema is not an object fails to parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexRecordSchema {
    /// The object schema.
    Object(LexObject),
}

impl LexRecordSchema {
    /// The object schema.
    pub const fn object(&self) -> &LexObject {
        match self {
            Self::Object(o) => o,
        }
    }
}

/// A query definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Query parameters.
    pub parameters: Option<LexXrpcParameters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The response body.
    pub output: Option<LexXrpcBody>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Named errors the method can return.
    pub errors: Vec<LexXrpcError>,
}

/// A procedure definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcProcedure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Query parameters.
    pub parameters: Option<LexXrpcParameters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The request body.
    pub input: Option<LexXrpcBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The response body.
    pub output: Option<LexXrpcBody>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Named errors the method can return.
    pub errors: Vec<LexXrpcError>,
}

/// A subscription definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcSubscription {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Query parameters.
    pub parameters: Option<LexXrpcParameters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The message schema, a union of event types.
    pub message: Option<LexXrpcSubscriptionMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Named errors the stream can emit.
    pub errors: Vec<LexXrpcError>,
}

/// The message schema of a subscription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcSubscriptionMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    /// The message type, a union.
    pub schema: LexRefUnionOrUnion,
}

/// A subscription message schema is a union, written either inline as a
/// `union` type object or as a reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexRefUnionOrUnion {
    /// An inline union.
    Union(LexRefUnion),
    /// A reference.
    Ref(LexRef),
}

/// Query parameters: an object whose properties are primitives or arrays of
/// primitives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcParameters {
    /// Always `params`.
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Required parameter names.
    pub required: Vec<String>,
    #[serde(default)]
    /// The parameters.
    pub properties: BTreeMap<String, LexPrimitiveOrArray>,
}

/// A request or response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    /// The MIME type, for example `application/json` or `*/*`.
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The schema for JSON bodies.
    pub schema: Option<LexObjectOrRef>,
}

/// A body schema is an object, a reference or a union.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexObjectOrRef {
    /// An inline object.
    Object(LexObject),
    /// A reference.
    Ref(LexRef),
    /// A union.
    Union(LexRefUnion),
}

/// A named XRPC error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexXrpcError {
    /// The error name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
}

/// A permission set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexPermissionSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Title shown on consent screens.
    pub title: Option<String>,
    #[serde(default, rename = "detail", skip_serializing_if = "Option::is_none")]
    /// Detail shown on consent screens.
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// The permissions in the set.
    pub permissions: Vec<LexPermission>,
    /// Any other keys (localized titles, future fields).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// A single permission inside a set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexPermission {
    /// The resource, for example `repo`, `rpc`, `blob`, `account`, `identity`.
    pub resource: String,
    /// Resource-specific parameters, kept open.
    #[serde(flatten)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// An object schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexObject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Required property names.
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Nullable property names.
    pub nullable: Vec<String>,
    #[serde(default)]
    /// The properties.
    pub properties: BTreeMap<String, LexType>,
}

/// Any type that may appear as an object property or array item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexType {
    /// A string.
    String(LexString),
    /// An integer.
    Integer(LexInteger),
    /// A boolean.
    Boolean(LexBoolean),
    /// A byte string.
    Bytes(LexBytes),
    /// A CID link.
    CidLink(LexCidLink),
    /// A blob reference.
    Blob(LexBlob),
    /// An array.
    Array(LexArray),
    /// An inline object.
    Object(LexObject),
    /// A reference.
    Ref(LexRef),
    /// A union.
    Union(LexRefUnion),
    /// Any value.
    Unknown(LexUnknown),
}

/// A query parameter type: primitives and arrays of primitives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LexPrimitiveOrArray {
    /// A string.
    String(LexString),
    /// An integer.
    Integer(LexInteger),
    /// A boolean.
    Boolean(LexBoolean),
    /// An array of primitives.
    Array(LexArray),
    /// Any value (used by a few upstream parameters).
    Unknown(LexUnknown),
}

/// A string with optional format and constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexString {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// A string format such as `did`, `handle`, `datetime`.
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Default value.
    pub default: Option<String>,
    #[serde(default, rename = "minLength", skip_serializing_if = "Option::is_none")]
    /// Minimum length in UTF-8 bytes.
    pub min_length: Option<u64>,
    #[serde(default, rename = "maxLength", skip_serializing_if = "Option::is_none")]
    /// Maximum length in UTF-8 bytes.
    pub max_length: Option<u64>,
    #[serde(default, rename = "minGraphemes", skip_serializing_if = "Option::is_none")]
    /// Minimum length in graphemes.
    pub min_graphemes: Option<u64>,
    #[serde(default, rename = "maxGraphemes", skip_serializing_if = "Option::is_none")]
    /// Maximum length in graphemes.
    pub max_graphemes: Option<u64>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    /// Allowed values.
    pub enum_: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// A fixed value.
    pub const_: Option<String>,
    #[serde(default, rename = "knownValues", skip_serializing_if = "Option::is_none")]
    /// Suggested values; others remain valid.
    pub known_values: Option<Vec<String>>,
}

/// An integer with optional range and constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexInteger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Default value.
    pub default: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Minimum value.
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Maximum value.
    pub maximum: Option<i64>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    /// Allowed values.
    pub enum_: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// A fixed value.
    pub const_: Option<i64>,
}

/// A boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexBoolean {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Default value.
    pub default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// A fixed value.
    pub const_: Option<bool>,
}

/// A byte string with optional length limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexBytes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, rename = "minLength", skip_serializing_if = "Option::is_none")]
    /// Minimum length in bytes.
    pub min_length: Option<u64>,
    #[serde(default, rename = "maxLength", skip_serializing_if = "Option::is_none")]
    /// Maximum length in bytes.
    pub max_length: Option<u64>,
}

/// A CID link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexCidLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
}

/// A blob reference with optional accept list and size limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexBlob {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Accepted MIME type globs.
    pub accept: Option<Vec<String>>,
    #[serde(default, rename = "maxSize", skip_serializing_if = "Option::is_none")]
    /// Maximum size in bytes.
    pub max_size: Option<u64>,
}

/// An array with an item type and optional length limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexArray {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    /// The item type.
    pub items: Box<LexType>,
    #[serde(default, rename = "minLength", skip_serializing_if = "Option::is_none")]
    /// Minimum number of items.
    pub min_length: Option<u64>,
    #[serde(default, rename = "maxLength", skip_serializing_if = "Option::is_none")]
    /// Maximum number of items.
    pub max_length: Option<u64>,
}

/// A reference: `#local`, `nsid#def` or `nsid` (meaning `nsid#main`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    /// The reference.
    #[serde(rename = "ref")]
    pub ref_: String,
}

/// A union of references, open unless `closed` is true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexRefUnion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
    #[serde(default)]
    /// The member references.
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Whether values outside `refs` are rejected.
    pub closed: Option<bool>,
}

/// A token definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexToken {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
}

/// Any data model value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LexUnknown {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Description.
    pub description: Option<String>,
}

impl LexUserType {
    /// The type name as written in the lexicon.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Record(_) => "record",
            Self::Query(_) => "query",
            Self::Procedure(_) => "procedure",
            Self::Subscription(_) => "subscription",
            Self::PermissionSet(_) => "permission-set",
            Self::Permission(_) => "permission",
            Self::Object(_) => "object",
            Self::Token(_) => "token",
            Self::String(_) => "string",
            Self::Integer(_) => "integer",
            Self::Boolean(_) => "boolean",
            Self::Bytes(_) => "bytes",
            Self::CidLink(_) => "cid-link",
            Self::Blob(_) => "blob",
            Self::Array(_) => "array",
            Self::Ref(_) => "ref",
            Self::Union(_) => "union",
            Self::Unknown(_) => "unknown",
        }
    }
}
