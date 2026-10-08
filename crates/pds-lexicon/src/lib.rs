//! Lexicon document model, loader, record validation and permission-set
//! resolution (`docs/SPEC.md` §2.1, §5.5, §6.3).
//!
//! Unit 0 ships the model ([`model`]) and the corpus loader ([`Corpus`]) with
//! reference resolution. Record validation in the three modes of SPEC §6.3
//! and permission-set resolution follow in later units.

pub mod model;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use model::{LexArray, LexObject, LexObjectOrRef, LexRefUnionOrUnion, LexType, LexUserType, LexXrpcParameters, LexiconDoc};

/// A loaded set of lexicon documents indexed by NSID.
#[derive(Debug, Default, Clone)]
pub struct Corpus {
    docs: BTreeMap<String, LexiconDoc>,
}

/// Errors from loading a corpus.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// A file could not be read.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A file did not parse as a lexicon document.
    #[error("{path}: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: serde_json::Error,
    },
    /// Two files define the same NSID.
    #[error("duplicate lexicon id {id} in {path}")]
    Duplicate {
        /// The NSID.
        id: String,
        /// The second file.
        path: PathBuf,
    },
    /// The `lexicon` field is not `1`.
    #[error("{path}: lexicon version must be 1")]
    Version {
        /// The file.
        path: PathBuf,
    },
}

/// A lexicon document that parses but breaks a rule of the lexicon
/// specification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{id}: {message}")]
pub struct DocError {
    /// The document id.
    pub id: String,
    /// The rule that failed.
    pub message: String,
}

impl LexiconDoc {
    /// Checks the rules the parser cannot express: a valid NSID id, primary
    /// types only as `main`, no bare `ref` or `unknown` definitions, and
    /// `params` typed as such.
    pub fn validate(&self) -> Result<(), DocError> {
        let err = |message: String| DocError { id: self.id.clone(), message };
        if self.lexicon != 1 {
            return Err(err("lexicon version must be 1".into()));
        }
        pds_types::Nsid::parse(&self.id).map_err(|e| err(format!("id is not a valid NSID: {e}")))?;
        for (name, def) in &self.defs {
            if name.is_empty() {
                return Err(err("definition names must be non-empty".into()));
            }
            let primary = matches!(
                def,
                LexUserType::Record(_) | LexUserType::Query(_) | LexUserType::Procedure(_) | LexUserType::Subscription(_)
            );
            if primary && name != "main" {
                return Err(err(format!("primary definition {name:?} must be named main")));
            }
            match def {
                LexUserType::Ref(_) => return Err(err(format!("definition {name:?} may not be a bare ref"))),
                LexUserType::Unknown(_) => return Err(err(format!("definition {name:?} may not be a bare unknown"))),
                LexUserType::Query(q) => check_params(q.parameters.as_ref()).map_err(err)?,
                LexUserType::Procedure(p) => check_params(p.parameters.as_ref()).map_err(err)?,
                LexUserType::Subscription(s) => check_params(s.parameters.as_ref()).map_err(err)?,
                LexUserType::Record(r) => {
                    let key = r.key.as_str();
                    if !(key == "tid" || key == "nsid" || key == "any" || key.starts_with("literal:")) {
                        return Err(err(format!("record key type {key:?} is not tid, nsid, any or literal:<value>")));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn check_params(p: Option<&LexXrpcParameters>) -> Result<(), String> {
    match p {
        Some(p) if p.type_ != "params" => Err(format!("parameters must have type \"params\", found {:?}", p.type_)),
        _ => Ok(()),
    }
}

/// A reference that does not resolve.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unresolved reference {reference} in {from}")]
pub struct UnresolvedRef {
    /// The document the reference appears in.
    pub from: String,
    /// The reference as written.
    pub reference: String,
}

impl Corpus {
    /// An empty corpus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a document, rejecting duplicate ids.
    pub fn insert(&mut self, doc: LexiconDoc) -> Result<(), String> {
        if self.docs.contains_key(&doc.id) {
            return Err(doc.id);
        }
        self.docs.insert(doc.id.clone(), doc);
        Ok(())
    }

    /// Loads every `*.json` file under `root`, recursively, sorted by path.
    pub fn load_dir(root: &Path) -> Result<Self, LoadError> {
        let mut files = Vec::new();
        collect_json(root, &mut files).map_err(|source| LoadError::Io { path: root.to_path_buf(), source })?;
        files.sort();
        let mut corpus = Self::new();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|source| LoadError::Io { path: path.clone(), source })?;
            let doc: LexiconDoc = serde_json::from_str(&text).map_err(|source| LoadError::Parse { path: path.clone(), source })?;
            if doc.lexicon != 1 {
                return Err(LoadError::Version { path });
            }
            if let Err(id) = corpus.insert(doc) {
                return Err(LoadError::Duplicate { id, path });
            }
        }
        Ok(corpus)
    }

    /// The document for an NSID.
    pub fn get(&self, id: &str) -> Option<&LexiconDoc> {
        self.docs.get(id)
    }

    /// All documents in id order.
    pub fn docs(&self) -> impl Iterator<Item = &LexiconDoc> {
        self.docs.values()
    }

    /// Number of documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether the corpus is empty.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Resolves a reference written in `from` to `(nsid, def name)`.
    ///
    /// `#local` resolves inside `from`; `nsid#def` and `nsid` (meaning
    /// `nsid#main`) resolve across the corpus.
    pub fn resolve_ref<'a>(&'a self, from: &str, reference: &str) -> Result<(&'a str, &'a LexUserType), UnresolvedRef> {
        let (nsid, def) = match reference.split_once('#') {
            Some(("", def)) => (from, def),
            Some((nsid, def)) => (nsid, def),
            None => (reference, "main"),
        };
        self.docs
            .get(nsid)
            .and_then(|doc| doc.defs.get_key_value(def).map(|(_, t)| (doc.id.as_str(), t)))
            .ok_or_else(|| UnresolvedRef { from: from.to_owned(), reference: reference.to_owned() })
    }

    /// Every reference in the corpus that does not resolve.
    pub fn unresolved_refs(&self) -> Vec<UnresolvedRef> {
        let mut out = Vec::new();
        for doc in self.docs.values() {
            let mut refs = Vec::new();
            for def in doc.defs.values() {
                collect_user_type_refs(def, &mut refs);
            }
            for r in refs {
                if let Err(e) = self.resolve_ref(&doc.id, &r) {
                    out.push(e);
                }
            }
        }
        out
    }
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// Collects every `ref` string reachable from a top-level definition.
pub fn collect_user_type_refs(def: &LexUserType, out: &mut Vec<String>) {
    match def {
        LexUserType::Record(r) => collect_object_refs(r.record.object(), out),
        LexUserType::Query(q) => {
            collect_params_refs(q.parameters.as_ref(), out);
            if let Some(b) = &q.output {
                collect_body_refs(b.schema.as_ref(), out);
            }
        }
        LexUserType::Procedure(p) => {
            collect_params_refs(p.parameters.as_ref(), out);
            if let Some(b) = &p.input {
                collect_body_refs(b.schema.as_ref(), out);
            }
            if let Some(b) = &p.output {
                collect_body_refs(b.schema.as_ref(), out);
            }
        }
        LexUserType::Subscription(s) => {
            collect_params_refs(s.parameters.as_ref(), out);
            if let Some(m) = &s.message {
                match &m.schema {
                    LexRefUnionOrUnion::Union(u) => out.extend(u.refs.iter().cloned()),
                    LexRefUnionOrUnion::Ref(r) => out.push(r.ref_.clone()),
                }
            }
        }
        LexUserType::Object(o) => collect_object_refs(o, out),
        LexUserType::Array(a) => collect_array_refs(a, out),
        LexUserType::Ref(r) => out.push(r.ref_.clone()),
        LexUserType::Union(u) => out.extend(u.refs.iter().cloned()),
        LexUserType::PermissionSet(_)
        | LexUserType::Permission(_)
        | LexUserType::Token(_)
        | LexUserType::String(_)
        | LexUserType::Integer(_)
        | LexUserType::Boolean(_)
        | LexUserType::Bytes(_)
        | LexUserType::CidLink(_)
        | LexUserType::Blob(_)
        | LexUserType::Unknown(_) => {}
    }
}

fn collect_params_refs(params: Option<&LexXrpcParameters>, out: &mut Vec<String>) {
    if let Some(p) = params {
        for v in p.properties.values() {
            if let model::LexPrimitiveOrArray::Array(a) = v {
                collect_array_refs(a, out);
            }
        }
    }
}

fn collect_body_refs(schema: Option<&LexObjectOrRef>, out: &mut Vec<String>) {
    match schema {
        Some(LexObjectOrRef::Object(o)) => collect_object_refs(o, out),
        Some(LexObjectOrRef::Ref(r)) => out.push(r.ref_.clone()),
        Some(LexObjectOrRef::Union(u)) => out.extend(u.refs.iter().cloned()),
        None => {}
    }
}

fn collect_object_refs(obj: &LexObject, out: &mut Vec<String>) {
    for t in obj.properties.values() {
        collect_type_refs(t, out);
    }
}

fn collect_array_refs(arr: &LexArray, out: &mut Vec<String>) {
    collect_type_refs(&arr.items, out);
}

fn collect_type_refs(t: &LexType, out: &mut Vec<String>) {
    match t {
        LexType::Ref(r) => out.push(r.ref_.clone()),
        LexType::Union(u) => out.extend(u.refs.iter().cloned()),
        LexType::Array(a) => collect_array_refs(a, out),
        LexType::Object(o) => collect_object_refs(o, out),
        LexType::String(_)
        | LexType::Integer(_)
        | LexType::Boolean(_)
        | LexType::Bytes(_)
        | LexType::CidLink(_)
        | LexType::Blob(_)
        | LexType::Unknown(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn every_vendored_lexicon_parses_and_every_ref_resolves() {
        let corpus = Corpus::load_dir(&repo_root().join("lexicons")).expect("lexicons load");
        assert!(corpus.len() > 250, "expected the vendored corpus, got {}", corpus.len());
        assert!(corpus.get("com.atproto.repo.createRecord").is_some());
        assert!(corpus.get("io.linkjar.account.getSignupReceipt").is_some());
        assert!(corpus.get("io.linkjar.pds.audit.checkpoint").is_some());
        for doc in corpus.docs() {
            doc.validate().unwrap_or_else(|e| panic!("{e}"));
        }
        let unresolved = corpus.unresolved_refs();
        assert!(unresolved.is_empty(), "unresolved references:\n{}", unresolved.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn interop_lexicon_catalog_parses() {
        let dir = repo_root().join("interop/lexicon/catalog");
        let corpus = Corpus::load_dir(&dir).expect("catalog loads");
        assert_eq!(corpus.len(), 5);
    }

    #[test]
    fn interop_valid_lexicons_parse_and_invalid_ones_do_not() {
        let dir = repo_root().join("interop/lexicon");
        let valid: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("lexicon-valid.json")).unwrap()).unwrap();
        for case in valid.as_array().unwrap() {
            let name = case["name"].as_str().unwrap_or("");
            let text = serde_json::to_string(&case["lexicon"]).unwrap();
            let parsed: Result<LexiconDoc, _> = serde_json::from_str(&text);
            assert!(parsed.is_ok(), "valid case {name:?} failed to parse: {:?}", parsed.err());
            let validated = parsed.unwrap().validate();
            assert!(validated.is_ok(), "valid case {name:?} failed validation: {:?}", validated.err());
        }
        let invalid: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("lexicon-invalid.json")).unwrap()).unwrap();
        // Structural problems fail at parse time; semantic ones (for example
        // a record key of the wrong form) need the document validator that
        // lands with record validation. KNOWN_FAILURES lists the latter.
        const KNOWN_FAILURES: &[&str] = &[];
        let mut unexpected = Vec::new();
        for case in invalid.as_array().unwrap() {
            let name = case["name"].as_str().unwrap_or("");
            let text = serde_json::to_string(&case["lexicon"]).unwrap();
            let parsed: Result<LexiconDoc, _> = serde_json::from_str(&text);
            let rejected = parsed.map_or(true, |d| d.validate().is_err());
            if !rejected && !KNOWN_FAILURES.contains(&name) {
                unexpected.push(name.to_owned());
            }
        }
        assert!(unexpected.is_empty(), "invalid lexicons accepted by the parser (add a validator rule or a KNOWN_FAILURES entry): {unexpected:?}");
    }
}
