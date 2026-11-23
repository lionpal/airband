//! The live login: the keychain item the tool keeps it in (JSON, with the maker's own login under
//! the key its row names beside its other logins), the account record in its account file, and which account it is.
use super::{login_key, saved, service, tool, Here, Res};
use serde_json::{Map, Value};
use std::path::Path;

/// The live item, or nothing when Claude Code is not signed in. One that is there and cannot be read is an
/// error: replacing it would lose the logins kept beside the maker's own one.
pub(super) fn blob(here: &Here) -> Res<Option<Map<String, Value>>> {
    let Some(s) = here.vault.read(&here.live_service, here.user.as_ref().map_err(|e| e.clone())?)? else { return Ok(None) };
    match serde_json::from_str::<Value>(&s).map_err(|e| format!("{}'s login cannot be read ({e}), so nothing was changed.", tool()))? {
        Value::Object(m) => Ok(Some(m)),
        _ => Err(format!("{}'s login is not in the form expected, so nothing was changed.", tool())),
    }
}
/// The maker's own login in the live item, when there is one.
fn oauth(blob: &Map<String, Value>) -> Option<&Value> {
    blob.get(&login_key())
}
fn field<'a>(o: &'a Value, name: &str) -> Option<&'a str> {
    o.get(name)?.as_str().filter(|t| !t.is_empty())
}
/// The live login's access token.
pub(super) fn token(here: &Here) -> Option<String> {
    oauth(&blob(here).ok()??).and_then(|o| field(o, "accessToken")).map(String::from)
}

/// The account record in Claude Code's account file.
pub(super) fn oauth_account(file: &Path) -> Option<Value> {
    let d: Value = serde_json::from_str(&airband_io::files::read_text(file).ok()??).ok()?;
    d.get("oauthAccount").cloned()
}
/// Set `oauthAccount`, reading the file again just before writing so nothing else in it is lost.
pub(super) fn set_oauth_account(file: &Path, acct: &Value) -> Res<()> {
    write_account_file(file, with_oauth_account(file, acct)?)
}
/// The account file as it is now with `oauthAccount` set, to be written: an error, before anything is changed,
/// when it cannot be read.
pub(super) fn with_oauth_account(file: &Path, acct: &Value) -> Res<String> {
    let text = airband_io::files::read_text(file).map_err(|e| format!("{}: {e}", file.display()))?.ok_or_else(|| format!("{} is not there", file.display()))?;
    let mut d: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    d.as_object_mut().ok_or_else(|| format!("{} is not in the form expected", file.display()))?.insert("oauthAccount".into(), acct.clone());
    serde_json::to_string_pretty(&d).map_err(|e| e.to_string())
}
/// His own file, which holds his login: whole or not at all, and as private as it was.
pub(super) fn write_account_file(file: &Path, text: String) -> Res<()> {
    airband_io::files::write_keeping(file, text).map_err(|e| format!("{}: {e}", file.display()))
}

/// Which account the live login is.
#[derive(PartialEq)]
pub(super) enum Live {
    /// Claude Code is not signed in to an account (nothing there, or an API key).
    Out,
    /// This account.
    Known(String),
    /// Signed in, and neither the server nor a saved login says to which account. Nothing saved may be refreshed
    /// then: it could be this very login, whose refresh token both copies share.
    Unknown,
}

/// Which account is in use: what the server says about the live token, or else the saved login that holds the
/// very same refresh token.
pub(super) fn who(here: &Here, saved: &[String]) -> Res<Live> {
    let Some(blob) = blob(here)? else { return Ok(Live::Out) };
    let Some(o) = oauth(&blob) else { return Ok(Live::Out) };
    if let Some((email, _)) = field(o, "accessToken").and_then(|t| service::profile(here, t)) {
        return Ok(Live::Known(email));
    }
    let Some(refresh) = field(o, "refreshToken") else { return Ok(Live::Unknown) };
    Ok(saved.iter().find(|e| saved::refresh_token_of(here, e).as_deref() == Some(refresh)).map_or(Live::Unknown, |e| Live::Known(e.clone())))
}

/// An account's email is never in a Debug form.
impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Live::Out => f.write_str("Out"),
            Live::Known(email) => write!(f, "Known({})", crate::unshown(email)),
            Live::Unknown => f.write_str("Unknown"),
        }
    }
}

#[cfg(test)]
mod debug_tests {
    #[test]
    fn the_live_accounts_email_is_never_in_its_debug_form() {
        assert!(!format!("{:?}", super::Live::Known("me@example.com".into())).contains("example"));
    }
}
