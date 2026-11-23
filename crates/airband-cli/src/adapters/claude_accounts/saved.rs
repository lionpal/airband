//! Saved logins: one keychain item each (service `airband`, account: the email), with the list of emails, and
//! nothing secret, in `accounts.json`. A list that cannot be read is never taken for an empty one: writing it
//! back would orphan every login it names.
#![expect(clippy::disallowed_macros, reason = "a saved login holds the tool's own records (its login and oauthAccount), and airband's list of them is written beside it in the same small shape")]
use super::{Here, Res, ADD_AGAIN};
use crate::config;
use serde_json::{json, Value};

fn index_path(here: &Here) -> std::path::PathBuf {
    here.dir.join("accounts.json")
}

/// The saved accounts, in the order they were saved. No list yet is none saved; a list that is there and cannot
/// be read is an error, and nothing is changed.
pub(super) fn index(here: &Here) -> Res<Vec<String>> {
    let p = index_path(here);
    let text = match airband_io::files::read_text(&p) {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(Vec::new()),
        Err(e) => return Err(unreadable(&p, &e.to_string())),
    };
    #[derive(serde::Deserialize)]
    struct List {
        accounts: Vec<String>,
    }
    serde_json::from_str::<List>(&text).map(|l| l.accounts).map_err(|e| unreadable(&p, &e.to_string()))
}
fn unreadable(p: &std::path::Path, why: &str) -> String {
    format!("The list of saved accounts ({}) cannot be read ({why}), so nothing was changed. Put it right or move it aside, then try again.", p.display())
}
fn write_index(here: &Here, list: &[String]) -> Res<()> {
    airband_io::files::make_folder(&here.dir).map_err(|e| e.to_string())?;
    config::write_private(&index_path(here), json!({ "accounts": list }).to_string()).map_err(|e| e.to_string())
}

/// A saved login, as it was kept.
pub(super) fn saved(here: &Here, email: &str) -> Res<Option<Value>> {
    Ok(match here.vault.read(&here.vault_service, email)? {
        Some(s) => Some(serde_json::from_str(&s).map_err(|e| format!("the saved login for {email} cannot be read: {e}"))?),
        None => None,
    })
}

/// Keep a login, and list it if it is new.
pub(super) fn keep(here: &Here, email: &str, oauth: &Value, account: &Value) -> Res<()> {
    // the list first: one that cannot be read stops this before anything is written
    let mut list = index(here)?;
    here.vault.write(&here.vault_service, email, &{
        let mut entry = json!({ "email": email, "oauthAccount": account, "savedAt": airband_core::clock::now_ms() });
        entry[super::login_key()] = oauth.clone();
        entry
    }.to_string())?;
    if !list.iter().any(|e| e == email) {
        list.push(email.into());
        write_index(here, &list)?;
    }
    Ok(())
}

/// Delete a saved login and take it off the list.
pub(super) fn forget(here: &Here, email: &str) -> Res<()> {
    let list = index(here)?;
    // still listed while the keychain still has it: never a login kept that he can no longer see
    here.vault.delete(&here.vault_service, email).map_err(|e| format!("Did not forget {email}: {e}"))?;
    write_index(here, &list.into_iter().filter(|x| x != email).collect::<Vec<_>>())
}

/// The saved account he means: its whole email, or the number it is listed under (`airband accounts`).
pub(super) fn resolve(here: &Here, query: &str) -> Res<String> {
    let list = index(here)?;
    let numbered = query.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| list.get(i));
    match list.iter().find(|e| e.as_str() == query).or(numbered) {
        Some(e) => Ok(e.clone()),
        None => Err(format!("No saved account is \"{query}\". Give its whole email or its number: {}.", if list.is_empty() { "none saved".into() } else { list.iter().enumerate().map(|(i, e)| format!("{}. {e}", i + 1)).collect::<Vec<_>>().join(", ") })),
    }
}

/// A saved login's refresh token, compared whole to tell whether it is the very login in use.
pub(super) fn refresh_token_of(here: &Here, email: &str) -> Option<String> {
    saved(here, email).ok().flatten()?.get(super::login_key())?.get("refreshToken")?.as_str().map(String::from)
}

/// The saved login's own record, or a reason to sign in again.
pub(super) fn entry(here: &Here, email: &str) -> Res<Value> {
    saved(here, email)?.ok_or_else(|| ADD_AGAIN.to_string())
}
