//! OAuth scopes (`docs/SPEC.md` §5.5, §11.3) as `@atproto/oauth-scopes`
//! at the pin reads them.
//!
//! A scope is one of four fixed words, or a permission of the form
//! `resource:value?name=value&name=value`. The `transition:` scopes grant
//! what a legacy app-password session could do.

use std::collections::BTreeSet;

/// The scopes without parameters.
pub const STATIC: [&str; 4] = [
    "atproto",
    "transition:email",
    "transition:generic",
    "transition:chat.bsky",
];

/// One permission scope, split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Permission {
    resource: String,
    /// The value after the colon.
    positional: Option<String>,
    params: Vec<(String, String)>,
}

impl Permission {
    fn parse(scope: &str) -> Option<Self> {
        let (head, query) = match scope.split_once('?') {
            Some((head, query)) => (head, Some(query)),
            None => (scope, None),
        };
        let (resource, positional) = match head.split_once(':') {
            Some((resource, value)) => (resource, Some(percent_decode(value)?)),
            None => (head, None),
        };
        let mut params = Vec::new();
        for pair in query.into_iter().flat_map(|query| query.split('&')) {
            let (name, value) = pair.split_once('=')?;
            params.push((name.to_owned(), percent_decode(value)?));
        }
        let permission = Self {
            resource: resource.to_owned(),
            positional,
            params,
        };
        permission.is_valid().then_some(permission)
    }

    /// The values of a parameter: the positional value when the parameter
    /// is the resource's own, and the named ones.
    fn values<'a>(&'a self, name: &'a str, own: bool) -> impl Iterator<Item = &'a str> {
        self.positional
            .as_deref()
            .filter(|_| own)
            .into_iter()
            .chain(
                self.params
                    .iter()
                    .filter(move |(key, _)| key == name)
                    .map(|(_, value)| value.as_str()),
            )
    }

    fn only_params(&self, allowed: &[&str]) -> bool {
        self.params
            .iter()
            .all(|(name, value)| allowed.contains(&name.as_str()) && !value.is_empty())
    }

    fn is_valid(&self) -> bool {
        let actions_in =
            |allowed: &[&str]| self.values("action", false).all(|a| allowed.contains(&a));
        match self.resource.as_str() {
            "account" => {
                self.only_params(&["attr", "action"])
                    && self.values("attr", true).count() == 1
                    && self
                        .values("attr", true)
                        .all(|attr| matches!(attr, "email" | "repo" | "status"))
                    && actions_in(&["read", "manage"])
            }
            "identity" => {
                self.only_params(&["attr"])
                    && self.values("attr", true).count() == 1
                    && self
                        .values("attr", true)
                        .all(|attr| matches!(attr, "handle" | "*"))
            }
            "blob" => {
                self.only_params(&["accept"])
                    && self.values("accept", true).count() > 0
                    && self.values("accept", true).all(is_accept)
            }
            "repo" => {
                self.only_params(&["collection", "action"])
                    && self.values("collection", true).count() > 0
                    && self
                        .values("collection", true)
                        .all(|collection| collection == "*" || is_nsid(collection))
                    && actions_in(&["create", "update", "delete"])
            }
            "rpc" => {
                let methods: Vec<&str> = self.values("lxm", true).collect();
                let audiences: Vec<&str> = self.values("aud", false).collect();
                self.only_params(&["lxm", "aud"])
                    && !methods.is_empty()
                    && audiences.len() == 1
                    && methods.iter().all(|lxm| *lxm == "*" || is_nsid(lxm))
                    && (audiences[0] == "*" || audiences[0].starts_with("did:"))
                    // Every method of every service is not a permission.
                    && !(audiences[0] == "*" && methods.contains(&"*"))
            }
            "include" => {
                self.only_params(&["nsid", "aud"])
                    && self.values("nsid", true).count() == 1
                    && self.values("nsid", true).all(is_nsid)
            }
            _ => false,
        }
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_nsid(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    labels.len() >= 3
        && text.len() <= 317
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn is_accept(text: &str) -> bool {
    text == "*/*"
        || text
            .split_once('/')
            .is_some_and(|(kind, sub)| !kind.is_empty() && !sub.is_empty() && kind != "*")
}

/// Whether the server knows the scope.
pub fn is_scope(scope: &str) -> bool {
    STATIC.contains(&scope) || Permission::parse(scope).is_some()
}

/// What a set of scopes allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permissions {
    scopes: BTreeSet<String>,
    permissions: Vec<Permission>,
}

impl Permissions {
    /// Reads a space-separated scope.
    pub fn new(scope: &str) -> Self {
        let scopes: BTreeSet<String> = scope
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect();
        let permissions = scopes
            .iter()
            .filter_map(|scope| Permission::parse(scope))
            .collect();
        Self {
            scopes,
            permissions,
        }
    }

    /// Whether the set has the scope word for word.
    pub fn has(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    fn generic(&self) -> bool {
        self.has("transition:generic")
    }

    fn of(&self, resource: &str) -> impl Iterator<Item = &Permission> {
        self.permissions
            .iter()
            .filter(move |permission| permission.resource == resource)
    }

    /// Whether the account attribute `attr` (`email`, `repo`, `status`)
    /// may be read, or managed when `manage` is set.
    pub fn allows_account(&self, attr: &str, manage: bool) -> bool {
        if attr == "email" && !manage && self.has("transition:email") {
            return true;
        }
        self.of("account").any(|permission| {
            permission.values("attr", true).any(|own| own == attr)
                && (!manage || permission.values("action", false).any(|a| a == "manage"))
        })
    }

    /// Whether the handle may be changed.
    pub fn allows_identity(&self, attr: &str) -> bool {
        self.of("identity").any(|permission| {
            permission
                .values("attr", true)
                .any(|own| own == attr || own == "*")
        })
    }

    /// Whether a blob of the type may be uploaded.
    pub fn allows_blob(&self, mime: &str) -> bool {
        self.generic()
            || self.of("blob").any(|permission| {
                permission.values("accept", true).any(|accept| {
                    accept == "*/*"
                        || accept == mime
                        || accept
                            .strip_suffix('*')
                            .is_some_and(|prefix| mime.starts_with(prefix))
                })
            })
    }

    /// Whether `action` (`create`, `update`, `delete`) is allowed on the
    /// collection.
    pub fn allows_repo(&self, collection: &str, action: &str) -> bool {
        self.generic()
            || self.of("repo").any(|permission| {
                permission
                    .values("collection", true)
                    .any(|own| own == "*" || own == collection)
                    && (permission.values("action", false).next().is_none()
                        || permission.values("action", false).any(|a| a == action))
            })
    }

    /// Whether the method may be called on the service `aud`
    /// (`did:web:api.bsky.app#bsky_appview`).
    pub fn allows_rpc(&self, lxm: &str, aud: &str) -> bool {
        let chat = lxm.starts_with("chat.bsky.");
        if (self.generic() && !chat) || (self.has("transition:chat.bsky") && chat) {
            return true;
        }
        self.of("rpc").any(|permission| {
            permission
                .values("lxm", true)
                .any(|own| own == "*" || own == lxm)
                && permission
                    .values("aud", false)
                    .any(|own| own == "*" || own == aud)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_recognized() {
        for good in [
            "atproto",
            "transition:generic",
            "identity:handle",
            "identity:*",
            "account:email",
            "account:repo?action=manage",
            "blob:*/*",
            "blob?accept=image/*&accept=video/mp4",
            "repo:app.bsky.feed.post",
            "repo:*?action=delete",
            "repo?collection=app.bsky.feed.post&collection=app.bsky.feed.like",
            "rpc:app.bsky.feed.getTimeline?aud=did:web:api.bsky.app%23bsky_appview",
            "rpc:*?aud=did:web:api.bsky.app%23bsky_appview",
            "include:app.bsky.authFullApp",
        ] {
            assert!(is_scope(good), "{good}");
        }
        for bad in [
            "",
            "openid",
            "transition:other",
            "identity:email",
            "account:password",
            "account:email?action=write",
            "blob",
            "repo",
            "repo:notansid",
            "rpc:app.bsky.feed.getTimeline",
            "rpc:*?aud=*",
            "repo:app.bsky.feed.post?unknown=1",
        ] {
            assert!(!is_scope(bad), "{bad}");
        }
    }

    #[test]
    fn transition_scopes_grant_what_a_legacy_session_could_do() {
        let generic = Permissions::new("atproto transition:generic");
        assert!(generic.allows_repo("app.bsky.feed.post", "create"));
        assert!(generic.allows_blob("image/png"));
        assert!(generic.allows_rpc("app.bsky.feed.getTimeline", "did:web:x#bsky_appview"));
        assert!(!generic.allows_rpc("chat.bsky.convo.listConvos", "did:web:x#bsky_chat"));
        assert!(!generic.allows_account("email", false));
        assert!(!generic.allows_identity("handle"));

        let identity = Permissions::new("atproto");
        assert!(!identity.allows_repo("app.bsky.feed.post", "create"));
        assert!(!identity.allows_rpc("app.bsky.feed.getTimeline", "did:web:x#bsky_appview"));

        let more =
            Permissions::new("atproto transition:email transition:chat.bsky identity:handle");
        assert!(more.allows_account("email", false));
        assert!(!more.allows_account("email", true));
        assert!(more.allows_rpc("chat.bsky.convo.listConvos", "did:web:x#bsky_chat"));
        assert!(more.allows_identity("handle"));
    }

    #[test]
    fn permission_scopes_grant_what_they_name() {
        let set = Permissions::new(
            "atproto repo:app.bsky.feed.post?action=create blob:image/* account:repo?action=manage rpc:app.bsky.feed.getTimeline?aud=did:web:api.bsky.app%23bsky_appview",
        );
        assert!(set.allows_repo("app.bsky.feed.post", "create"));
        assert!(!set.allows_repo("app.bsky.feed.post", "delete"));
        assert!(!set.allows_repo("app.bsky.feed.like", "create"));
        assert!(set.allows_blob("image/png"));
        assert!(!set.allows_blob("video/mp4"));
        assert!(set.allows_account("repo", true));
        assert!(set.allows_rpc(
            "app.bsky.feed.getTimeline",
            "did:web:api.bsky.app#bsky_appview"
        ));
        assert!(!set.allows_rpc("app.bsky.feed.getTimeline", "did:web:other#bsky_appview"));
    }
}
