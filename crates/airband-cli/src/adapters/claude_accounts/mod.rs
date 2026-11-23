//! Claude Code's accounts: the logins this computer has saved, which one every session is using, how much of
//! each one's allowance is left, and switching between them. Nothing here leaves the computer except requests
//! to Anthropic's own servers, and no token is ever printed, logged or sent to a radio.
//!
//! The live login is a keychain item (`live`). Every running session reads it again within 30 seconds and
//! before a request, which is why a switch reaches sessions that are already running. `oauthAccount` in Claude
//! Code's account file names the account.
//!
//! Saved logins are kept beside it (`saved`). A login's refresh token changes every time it is used: Claude Code
//! refreshes the live one, the tower refreshes saved ones, and a switch saves the outgoing login back first. A
//! saved copy of the login in use shares its refresh token, so it is never refreshed: that would sign the live
//! one out. And there can be one keeper of saved logins on a computer: a second tool holding copies of the same
//! logins would log this one out, and be logged out by it.
//!
//! Only where Claude Code keeps its login in the keychain; elsewhere it keeps it differently, and that is not
//! written yet.
mod live;
mod saved;
mod service;
mod vault;

use crate::config;
use airband_core::radio::Account;
use live::Live;
use serde_json::Value;
use service::Unchecked;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

type Res<T> = Result<T, String>;

use super::claude_code::HARNESS;
const ADD_AGAIN: &str = "Sign in to it again on the computer: airband accounts add";

/// What Claude Code's row says about it.
fn row(what: &str) -> String {
    super::row_says(HARNESS, what)
}
/// The arguments that start a sign-in, as the row lists them.
pub(super) fn sign_in_args() -> Vec<String> {
    super::tool_row(HARNESS).and_then(|t| t.with.get("sign_in")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// Where in the tool's login item, and in a saved copy of it, the account's login is kept: its row says.
pub(super) fn login_key() -> String {
    row("login_key")
}
/// What the tool is called, and who makes it, as its row has them: for what he is told.
fn tool() -> String {
    super::called(HARNESS).0
}
fn maker() -> String {
    super::called(HARNESS).1
}

/// Where everything is for one run: the keychain, the service, the files. The environment can point them
/// elsewhere so the whole thing can be tried against made-up logins and a stand-in server, without touching a
/// real login; the tests build one of their own.
struct Here {
    vault: Box<dyn vault::Vault>,
    api: String,
    token_url: String,
    /// airband's own folder for these accounts: the list of saved ones, and the lock.
    dir: PathBuf,
    /// Claude Code's account file, which holds `oauthAccount`.
    account_file: PathBuf,
    live_service: String,
    vault_service: String,
    /// The account the keychain items are kept under: this computer's login name.
    user: Res<String>,
}

fn sha8(s: &str) -> String {
    airband_core::encode::hex(&Sha256::digest(s.as_bytes())[..4])
}

/// The keychain item Claude Code keeps its login in for a home folder: named after the folder when it was moved
/// from the usual one.
fn login_item(moved: Option<&std::path::Path>) -> String {
    match moved {
        Some(d) => format!("{}-{}", row("keychain_service"), sha8(&d.to_string_lossy())),
        None => row("keychain_service"),
    }
}

/// A stand-in the end-to-end suites put in place of the account service and the keychain items. Only a build made
/// with the `stand-ins` feature reads these: the airband he installs never does, so nothing set in his shell can
/// send his logins anywhere else.
use super::stand_in;

impl Here {
    fn this_computer() -> Here {
        let place = |key: &str, default: String| stand_in(key).unwrap_or(default);
        let moved = super::moved_home(HARNESS);
        let live_service = stand_in("AIRBAND_STAND_IN_LIVE_ITEM").unwrap_or_else(|| login_item(moved.as_deref()));
        Here {
            vault: Box::new(vault::Keychain),
            api: place("AIRBAND_STAND_IN_ACCOUNTS", row("accounts_api")),
            token_url: place("AIRBAND_STAND_IN_TOKENS", row("accounts_token")),
            dir: dir(),
            account_file: moved.unwrap_or_else(config::home).join(row("account_file")),
            live_service,
            vault_service: place("AIRBAND_STAND_IN_VAULT_ITEM", "airband".into()),
            user: config::account().map(|a| a.login).filter(|l| !l.is_empty()).ok_or_else(|| "This computer's account has no login name.".to_string()),
        }
    }
    fn user(&self) -> Res<&str> {
        self.user.as_deref().map_err(|e| e.clone())
    }
}

/// airband's folder for Claude Code.
pub(super) fn dir() -> PathBuf {
    config::dir().join(HARNESS)
}

/// One thing at a time across every airband process on this computer: a saved login must never be refreshed
/// twice at once, or switched while it is being refreshed.
fn locked<T>(here: &Here, work: impl FnOnce() -> T) -> Res<T> {
    let _held = crate::lock::take(&here.dir.join("accounts.lock"))?;
    Ok(work())
}

/// Save the login Claude Code is using now. Returns its email.
/// Keep the newest login of the account a switch moves away from, known to be `email`: found by the server
/// or by the refresh token a saved copy shares. Its record is the saved one, or the server's when there is none.
fn keep_outgoing(here: &Here, email: &str, blob: &serde_json::Map<String, Value>) -> Res<()> {
    let oauth = blob.get(&login_key()).ok_or_else(|| format!("{} is signed in with an API key or a setup token, which cannot be switched.", tool()))?;
    let acct = match saved::entry(here, email).ok().and_then(|e| e.get("oauthAccount").filter(|a| a["accountUuid"].is_string()).cloned()) {
        Some(acct) => acct,
        None => service::profile(here, access_token(oauth)?).filter(|(e, _)| e == email).map(|(_, a)| a).ok_or_else(|| format!("Could not tell which account {} is signed in to: {} did not say.", tool(), maker()))?,
    };
    saved::keep(here, email, oauth, &acct)
}

/// A login's access token. A login with none is said, never sent as an empty one.
fn access_token(oauth: &Value) -> Res<&str> {
    oauth.get("accessToken").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).ok_or_else(|| format!("{} is signed in without an access token. Sign in to it again.", tool()))
}

fn save_live(here: &Here) -> Res<String> {
    let blob = live::blob(here)?.ok_or_else(|| format!("{} is not signed in on this computer.", tool()))?;
    let oauth = blob.get(&login_key()).cloned().ok_or_else(|| format!("{} is signed in with an API key or a setup token, which cannot be switched.", tool()))?;
    // whose login it is, from the server and never from the file a switch itself rewrites
    let (email, acct) = service::profile(here, access_token(&oauth)?).ok_or_else(|| format!("Could not tell which account {} is signed in to: {} did not say.", tool(), maker()))?;
    saved::keep(here, &email, &oauth, &acct)?;
    Ok(email)
}

/// A saved login's access token. One that has run out or is about to is refreshed, with the new login saved at
/// once, but only when `may_refresh`: never while the login in use may be this one.
fn saved_token(here: &Here, email: &str, may_refresh: bool) -> Res<String> {
    let entry = saved::entry(here, email)?;
    let o = entry.get(login_key()).ok_or(ADD_AGAIN)?;
    let token = o.get("accessToken").and_then(|x| x.as_str()).unwrap_or("");
    let now = airband_core::clock::now_ms() as i64;
    // also while the refresh token has under a week left, so a login unused for weeks never lapses
    let refresh_good = o.get("refreshTokenExpiresAt").and_then(|x| x.as_i64()).is_none_or(|t| t > now + 7 * 86_400_000);
    if refresh_good && !token.is_empty() && o.get("expiresAt").and_then(|x| x.as_i64()).is_some_and(|t| t > now + 120_000) {
        return Ok(token.to_string());
    }
    if !may_refresh {
        return Err(format!("Not checked: {} did not say which account is in use, so this login was not refreshed in case it is that one.", maker()));
    }
    let n = service::refresh(here, o)?;
    let new_token = n["accessToken"].as_str().unwrap_or_default().to_string();
    // the account record it was saved with, or the real one fetched now with its own token
    let acct = entry.get("oauthAccount").filter(|a| a["accountUuid"].is_string()).cloned().or_else(|| service::profile(here, &new_token).map(|(_, a)| a)).ok_or(ADD_AGAIN)?;
    saved::keep(here, email, &n, &acct)?;
    Ok(new_token)
}

/// Who a login is, as Claude Code's own account record says, read the way the tool's row says and never from the
/// email: the live one's where Claude Code keeps it (the folder its account file is in, moved or not), a saved
/// one's from the record kept with it. A login by email is never known by a key: keys are not looked at here.
fn identity(here: &Here, email: &str, live: bool) -> airband_core::identity::AccountIdentity {
    if live {
        let folder = here.account_file.parent().map(PathBuf::from).unwrap_or_default();
        let at = super::Looking { tool_home: folder.clone(), user_home: folder, env: &|_| None, keychain: &|_| None };
        return super::identity_of(HARNESS, &at);
    }
    saved::saved(here, email).ok().flatten().map(|d| super::identity_in_copy(HARNESS, &row("account_file"), &d)).unwrap_or_default()
}

fn account(email: String, identity: airband_core::identity::AccountIdentity, live: bool, checked: Result<Vec<airband_core::radio::Limit>, Unchecked>) -> Account {
    let (limits, problem) = match checked {
        Ok(l) => (l, None),
        // the login in use is refreshed by the sessions that use it, so a refusal means none has lately
        Err(Unchecked::Expired) if live => (Vec::new(), Some("Not checked: no session has used it lately.".to_string())),
        Err(Unchecked::Expired) => (Vec::new(), Some("Its login has run out.".to_string())),
        Err(Unchecked::Said(e)) => (Vec::new(), Some(e)),
    };
    Account { id: email.clone(), harness: HARNESS.into(), label: email, identity, live, limits, problem }
}

fn list_in(here: &Here) -> (Vec<Account>, Option<String>) {
    let listed = locked(here, || {
        let mut notes: Vec<String> = Vec::new();
        let mut emails = saved::index(here).unwrap_or_else(|e| {
            notes.push(e);
            Vec::new()
        });
        let live = live::who(here, &emails).unwrap_or_else(|e| {
            notes.push(e);
            Live::Unknown
        });
        if let Live::Known(l) = &live
            && !emails.contains(l)
        {
            emails.insert(0, l.clone());
        }
        let may_refresh = live != Live::Unknown;
        // each account is asked about at the same time; a saved login is only ever refreshed here, under the lock
        let accounts: Vec<Account> = std::thread::scope(|s| {
            let checks: Vec<_> = emails
                .into_iter()
                .map(|e| {
                    let is_live = live == Live::Known(e.clone());
                    s.spawn(move || {
                        let token = if is_live { live::token(here).ok_or_else(|| format!("{} is not signed in.", tool())) } else { saved_token(here, &e, may_refresh) };
                        account(e.clone(), identity(here, &e, is_live), is_live, token.map_err(Unchecked::from).and_then(|t| service::usage(here, &t)))
                    })
                })
                .collect();
            // one whose check failed inside is said, not left off the list as if it were not saved
            checks.into_iter().filter_map(|h| h.join().map_err(|_| notes.push("One saved account could not be checked: airband ran into trouble with it.".into())).ok()).collect()
        });
        match (&live, accounts.len()) {
            (Live::Unknown, _) => notes.push(format!("Could not tell which account {} is using just now: {} did not say.", tool(), maker())),
            (_, 0) => notes.push(format!("{} is not signed in on this computer.", tool())),
            (_, 1) => notes.push("To add another account, on the computer: airband accounts add".into()),
            _ => {}
        }
        (accounts, (!notes.is_empty()).then(|| notes.join(" ")))
    });
    listed.unwrap_or_else(|why| (Vec::new(), Some(why)))
}

static SWITCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn switch_in(here: &Here, id: &str) -> Res<std::thread::JoinHandle<()>> {
    locked(here, || {
        let target = saved::resolve(here, id)?;
        // which account is in use comes first: a saved copy of it must not be refreshed, and asking blind could be
        // refreshing that very login
        let outgoing = match live::who(here, &saved::index(here)?).map_err(|e| format!("Did not switch: {e}"))? {
            Live::Known(e) if e == target => return Ok(std::thread::spawn(|| {})),
            Live::Unknown => return Err(format!("Did not switch: {} did not say which account {} is using just now, and switching without knowing could sign it out. Try again in a moment.", maker(), tool())),
            Live::Known(e) => Some(e),
            Live::Out => None,
        };
        // the account record Claude Code is given is the real one, asked for with that account's own token;
        // without it there is no switch, rather than a record made up from the email
        let token = saved_token(here, &target, true)?;
        let acct = service::profile(here, &token).map(|(_, a)| a).ok_or_else(|| format!("Did not switch: {} would not say whose login that is just now. Try again in a moment.", maker()))?;
        let incoming = saved::entry(here, &target)?.get(login_key()).cloned().ok_or(ADD_AGAIN)?;
        // the account file must take the new record, or the login is not swapped at all
        let record = live::with_oauth_account(&here.account_file, &acct).map_err(|e| format!("Did not switch: {e}"))?;
        // Keep the outgoing account's newest login (its refresh token has changed since it was saved), then read
        // the live item once more: the running tool may have refreshed it meanwhile, and the copy kept must be
        // the one it uses. What it holds when it is written over is what was kept.
        let mut blob = live::blob(here).map_err(|e| format!("Did not switch: {e}"))?;
        loop {
            if let (Some(email), Some(blob)) = (&outgoing, &blob) {
                keep_outgoing(here, email, blob).map_err(|e| format!("Did not switch: the current login could not be saved first. {e}"))?;
            }
            let again = live::blob(here).map_err(|e| format!("Did not switch: {e}"))?;
            if again == blob {
                break;
            }
            blob = again;
        }
        // swap the claude.ai login inside the live item, keeping the MCP logins beside it
        let before = blob.clone().map(|b| Value::Object(b).to_string());
        let mut blob = blob.unwrap_or_default();
        blob.insert(login_key(), incoming);
        let user = here.user()?;
        let swapped = Value::Object(blob).to_string();
        if let Err(e) = here.vault.write(&here.live_service, user, &swapped) {
            // what the item holds now says what became of it: as before, nothing changed; anything else was
            // written by something besides this switch, and he is told so, with nothing written over it
            return Err(match here.vault.read(&here.live_service, user) {
                Ok(now) if now == before => format!("Did not switch: {e}"),
                Ok(now) if now.as_deref() == Some(swapped.as_str()) => format!("The switch went half way: the login was swapped and could not be checked ({e}). Switch again, or sign in to {} again.", tool()),
                _ => format!("Did not switch, and {}'s login changed while switching ({e}). Sign in to {} again if it is signed out.", tool(), tool()),
            });
        }
        if let Err(e) = live::write_account_file(&here.account_file, record) {
            // the login goes back as it was, so the two never disagree
            let undone = match &before {
                Some(before) => here.vault.write(&here.live_service, user, before),
                None => here.vault.delete(&here.live_service, user),
            };
            return Err(match undone {
                Ok(()) => format!("Did not switch: {e}"),
                Err(u) => format!("The switch went half way: the login was swapped, the account file could not be written ({e}), and the login could not be put back ({u}). Sign in to {} again.", tool()),
            });
        }
        // a later switch ends this one's hold, or the two would fight over the name
        let mine = SWITCHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let file = here.account_file.clone();
        Ok(std::thread::spawn(move || {
            for _ in 0..5 {
                std::thread::sleep(Duration::from_secs(1));
                if SWITCHES.load(std::sync::atomic::Ordering::SeqCst) != mine {
                    return;
                }
                if !same_account(live::oauth_account(&file).as_ref(), &acct)
                    && let Err(e) = live::set_oauth_account(&file, &acct)
                {
                    eprintln!("airband: the account file could not be set back to the account chosen: {e}");
                }
            }
        }))
    })?
}

/// Whether the account file holds this account: by the account's own id, never by its email.
fn same_account(file: Option<&Value>, acct: &Value) -> bool {
    let id = |a: &Value| a.get("accountUuid").and_then(|u| u.as_str()).map(String::from);
    matches!((file.and_then(id), id(acct)), (Some(a), Some(b)) if a == b)
}

// ── what the rest of airband calls ──

/// Every account, with its allowances, and anything he should know about the list.
pub(crate) fn list() -> (Vec<Account>, Option<String>) {
    if !cfg!(target_os = "macos") {
        return (Vec::new(), Some(format!("Switching {} accounts is not available on this computer yet.", tool())));
    }
    list_in(&Here::this_computer())
}

/// Make a saved account the one every running session uses. The login is swapped at once; the returned thread
/// then holds the account's name in place for a few seconds, because a session still on the old login may
/// write its own back.
pub(crate) fn switch(id: &str) -> Res<std::thread::JoinHandle<()>> {
    switch_in(&Here::this_computer(), id)
}

/// `airband accounts save`: keep the login Claude Code is using now.
pub(crate) fn save() -> Res<String> {
    let here = Here::this_computer();
    locked(&here, || save_live(&here))?
}

/// `airband accounts add`: sign in to another account in a private folder, so the login in use and every
/// running session are untouched, then keep that login.
pub(crate) fn add() -> Res<String> {
    let here = Here::this_computer();
    airband_io::files::make_folder(&here.dir).map_err(|e| e.to_string())?;
    let tmp = here.dir.join(format!("login-{}", airband_core::clock::now_ms()));
    airband_io::files::make_folder(&tmp).map_err(|e| e.to_string())?;
    let service = login_item(Some(&tmp));
    // what the tool's own account file must hold for a sign-in to start at once, as its row has it
    airband_io::files::write_atomic(&tmp.join(row("account_file")), row("account_seed")).map_err(|e| e.to_string())?;
    println!("A browser opens: sign in to the account to add. The login in use now is not touched.");
    let program = row("program");
    let status = Command::new(&program).args(sign_in_args()).env(row("home_env"), &tmp).status().map_err(|e| format!("could not run {program}: {e}"));
    let result = (|| -> Res<String> {
        if !status?.success() {
            return Err("The sign-in did not finish.".into());
        }
        let blob: Value = serde_json::from_str(&here.vault.read(&service, here.user()?)?.ok_or("The sign-in saved no login.")?).map_err(|e| e.to_string())?;
        let oauth = blob.get(login_key()).cloned().ok_or("The sign-in saved no login for an account.")?;
        let acct = live::oauth_account(&tmp.join(row("account_file"))).ok_or("The sign-in saved no account record.")?;
        let email = acct.get(row("account_email")).and_then(|e| e.as_str()).map(String::from).ok_or("The account record has no email.")?;
        locked(&here, || saved::keep(&here, &email, &oauth, &acct))??;
        Ok(email)
    })();
    // the sign-in's own copy, in an item of its own: one left behind is said, never passed over
    if let Ok(user) = here.user()
        && let Err(e) = here.vault.delete(&service, user)
    {
        eprintln!("The sign-in's copy of the login could not be taken out of the keychain ({e}). It is the item {service}.");
    }
    if let Err(e) = airband_io::files::remove_folder(&tmp) {
        eprintln!("The sign-in's folder could not be removed ({e}). It is {}.", tmp.display());
    }
    result
}

/// `airband accounts forget <account>`: delete a saved login.
pub(crate) fn forget(id: &str) -> Res<String> {
    let here = Here::this_computer();
    locked(&here, || {
        let email = saved::resolve(&here, id)?;
        saved::forget(&here, &email)?;
        Ok(email)
    })?
}

#[cfg(test)]
mod tests;
