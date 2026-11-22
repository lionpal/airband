//! Who an account is, as its own tool records it, so the same account on two computers is known for one by
//! id and never by what it is called.
//!
//! A tool row says where its tool keeps that (`with.account_id`: a list of [`IdSource`]s, the first that gives
//! a value wins). A subscription sign-in has a typed account id. An API key has none: it is known by a
//! fingerprint of the whole key, made on the computer ([`fingerprint`]), and shown by its last four characters,
//! as the provider's own pages show it. The key itself never leaves the computer. An account with neither is
//! [`AccountIdentity::None`] and is never taken for another.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who an account is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountIdentity {
    /// The tool's own id for the person signed in, and the organisation or workspace when it records one apart.
    Id {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        org: Option<String>,
    },
    /// An API key: its fingerprint, and its last four characters to show.
    Key { fingerprint: String, last4: String },
    /// Nothing the tool records says who it is.
    #[default]
    #[serde(other)]
    None,
}

impl AccountIdentity {
    /// Whether two accounts are one: the same id in the same organisation, or the same key; never when either
    /// is unknown. (One person in two workspaces has two allowances, so two accounts.)
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (AccountIdentity::Id { id: a, org: oa }, AccountIdentity::Id { id: b, org: ob }) => a == b && oa == ob,
            (AccountIdentity::Key { fingerprint: a, .. }, AccountIdentity::Key { fingerprint: b, .. }) => a == b,
            _ => false,
        }
    }
    /// An API key's identity, made on the computer that has it.
    pub fn of_key(provider: &str, key: &str) -> Self {
        AccountIdentity::Key { fingerprint: fingerprint(provider, key), last4: last4(key) }
    }
}

/// The fingerprint of an API key: hex SHA-256 of the key with the provider in front, so the same key matches
/// on every computer and a bare hash of a secret never travels.
pub fn fingerprint(provider: &str, key: &str) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(b"airband-key-v1:");
    h.update(provider.as_bytes());
    h.update(b":");
    h.update(key.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The last four characters of a key, the way provider pages show one.
pub fn last4(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// Where a folder named in a source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Under {
    /// The tool's own home folder (its row's `home`, or its `home_env`).
    Home,
    /// His home folder.
    User,
}

/// One place a tool keeps who its account is, and what to take from there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdSource {
    pub place: Place,
    pub take: Take,
}

/// Where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Place {
    /// A JSON file.
    File { file: String, under: Under },
    /// An environment variable of the tool's: its value is the thing itself (a key).
    Env { name: String },
    /// A keychain item of the tool's: its value is the thing itself, or JSON.
    Keychain { service: String },
    /// A row of a SQLite table whose `column` holds JSON. `provider_column` names the provider of each row.
    Sqlite { file: String, under: Under, table: String, column: String, provider_column: String },
}

/// What to take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Take {
    /// A typed account id at `path` (each step one key, so keys with dots in them are kept whole), and the
    /// organisation's at `org_path`. With `jwt`, the field at that path is a token whose payload is read instead.
    Id {
        path: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        org_path: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jwt: Option<Vec<String>>,
    },
    /// An API key at `path` (none: the value is the key), for `provider` (none: the place names it).
    Key {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
}

impl IdSource {
    /// A source that can never give anything is refused when the row is read, not found out later.
    pub fn check(&self) -> Result<(), String> {
        match (&self.place, &self.take) {
            (Place::Env { .. }, Take::Id { .. }) => Err("an environment variable holds a key, not an account id".into()),
            (Place::Sqlite { .. }, Take::Key { provider: Some(_), .. }) => Err("a table row names its own provider".into()),
            (Place::Env { .. } | Place::Keychain { .. } | Place::File { .. }, Take::Key { provider: None, .. }) => Err("a key needs the provider it is for".into()),
            (_, Take::Id { path, .. }) | (_, Take::Key { path: Some(path), .. }) if path.is_empty() => Err("an empty path".into()),
            _ => Ok(()),
        }
    }

    /// Who the account is, from what was read at this source: the JSON document (or, for a value that is the
    /// key itself, a JSON string), and the row's provider for a table. `None` when it gives nothing.
    pub fn identity(&self, doc: &Value, row_provider: Option<&str>) -> Option<AccountIdentity> {
        match &self.take {
            Take::Id { path, org_path, jwt } => {
                let claims;
                let from = match jwt {
                    Some(at) => {
                        claims = jwt_payload(at_path(doc, at)?.as_str()?)?;
                        &claims
                    }
                    None => doc,
                };
                let id = at_path(from, path)?.as_str().filter(|s| !s.is_empty())?.to_string();
                let org = org_path.as_ref().and_then(|p| at_path(from, p)?.as_str().map(String::from));
                Some(AccountIdentity::Id { id, org })
            }
            Take::Key { path, provider } => {
                let key = match path {
                    Some(p) => at_path(doc, p)?.as_str()?,
                    None => doc.as_str()?,
                };
                let provider = provider.as_deref().or(row_provider)?;
                (!key.is_empty()).then(|| AccountIdentity::of_key(provider, key))
            }
        }
    }
}

/// The value at a path of keys.
fn at_path<'a>(doc: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(doc, |v, k| v.get(k))
}

/// The payload of a JSON web token, read in memory. Nothing is checked: it names who signed in, and is never
/// trusted for anything else.
fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A tool row's `with.account_id`, read and checked. A row without one has no sources.
pub fn sources(with: &std::collections::BTreeMap<String, Value>) -> Result<Vec<IdSource>, String> {
    let Some(v) = with.get("account_id") else { return Ok(Vec::new()) };
    let all: Vec<IdSource> = serde_json::from_value(v.clone()).map_err(|e| format!("account_id: {e}"))?;
    all.iter().try_for_each(IdSource::check)?;
    Ok(all)
}

#[cfg(test)]
mod tests;
