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

pub(crate) const HARNESS: &str = "claude-code";
const ADD_AGAIN: &str = "Sign in to it again on the computer: airband accounts add";

/// What Claude Code's row says about it.
fn row(what: &str) -> String {
    super::row_says(HARNESS, what)
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

impl Here {
    fn this_computer() -> Here {
        let place = |key: &str, default: String| super::env(key).unwrap_or(default);
        let moved = super::claude_code::moved_home();
        let live_service = super::env("AIRBAND_CLAUDE_LIVE_SERVICE").unwrap_or_else(|| match &moved {
            // Claude Code names the item for a moved home after that folder
            Some(d) => format!("{}-{}", row("keychain_service"), sha8(&d.to_string_lossy())),
            None => row("keychain_service"),
        });
        Here {
            vault: Box::new(vault::Keychain),
            api: place("AIRBAND_CLAUDE_API", row("accounts_api")),
            token_url: place("AIRBAND_CLAUDE_TOKEN_URL", row("accounts_token")),
            dir: dir(),
            account_file: moved.unwrap_or_else(config::home).join(row("account_file")),
            live_service,
            vault_service: place("AIRBAND_CLAUDE_VAULT_SERVICE", "airband".into()),
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
fn locked<T>(here: &Here, work: impl FnOnce() -> T) -> T {
    let _ = std::fs::create_dir_all(&here.dir);
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(here.dir.join("accounts.lock")).ok();
    #[cfg(unix)]
    if let Some(f) = &file {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    }
    let out = work();
    drop(file);
    out
}

/// Save the login Claude Code is using now. Returns its email.
fn save_live(here: &Here) -> Res<String> {
    let blob = live::blob(here)?.ok_or_else(|| format!("{} is not signed in on this computer.", tool()))?;
    let oauth = blob.get("claudeAiOauth").cloned().ok_or_else(|| format!("{} is signed in with an API key or a setup token, which cannot be switched.", tool()))?;
    let token = oauth.get("accessToken").and_then(|t| t.as_str()).unwrap_or("");
    // whose login it is, from the server and never from the file a switch itself rewrites
    let (email, acct) = service::profile(here, token).ok_or_else(|| format!("Could not tell which account {} is signed in to: {} did not say.", tool(), maker()))?;
    saved::keep(here, &email, &oauth, &acct)?;
    Ok(email)
}

/// A saved login's access token. One that has run out or is about to is refreshed, with the new login saved at
/// once, but only when `may_refresh`: never while the login in use may be this one.
fn saved_token(here: &Here, email: &str, may_refresh: bool) -> Res<String> {
    let entry = saved::entry(here, email)?;
    let o = entry.get("claudeAiOauth").ok_or(ADD_AGAIN)?;
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

fn account(email: String, live: bool, checked: Result<Vec<airband_core::radio::Limit>, Unchecked>) -> Account {
    let (limits, problem) = match checked {
        Ok(l) => (l, None),
        // the login in use is refreshed by the sessions that use it, so a refusal means none has lately
        Err(Unchecked::Expired) if live => (Vec::new(), Some("Not checked: no session has used it lately.".to_string())),
        Err(Unchecked::Expired) => (Vec::new(), Some("Its login has run out.".to_string())),
        Err(Unchecked::Said(e)) => (Vec::new(), Some(e)),
    };
    Account { id: email.clone(), harness: HARNESS.into(), label: email, live, limits, problem }
}

fn list_in(here: &Here) -> (Vec<Account>, Option<String>) {
    locked(here, || {
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
                        account(e, is_live, token.map_err(Unchecked::from).and_then(|t| service::usage(here, &t)))
                    })
                })
                .collect();
            checks.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        match (&live, accounts.len()) {
            (Live::Unknown, _) => notes.push(format!("Could not tell which account {} is using just now: {} did not say.", tool(), maker())),
            (_, 0) => notes.push(format!("{} is not signed in on this computer.", tool())),
            (_, 1) => notes.push("To add another account, on the computer: airband accounts add".into()),
            _ => {}
        }
        (accounts, (!notes.is_empty()).then(|| notes.join(" ")))
    })
}

static SWITCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn switch_in(here: &Here, id: &str) -> Res<std::thread::JoinHandle<()>> {
    locked(here, || {
        let target = saved::resolve(here, id)?;
        // which account is in use comes first: a saved copy of it must not be refreshed, and asking blind could be
        // refreshing that very login
        match live::who(here, &saved::index(here)?).map_err(|e| format!("Did not switch: {e}"))? {
            Live::Known(e) if e == target => return Ok(std::thread::spawn(|| {})),
            Live::Unknown => return Err(format!("Did not switch: {} did not say which account {} is using just now, and switching without knowing could sign it out. Try again in a moment.", maker(), tool())),
            _ => {}
        }
        // the account record Claude Code is given is the real one, asked for with that account's own token;
        // without it there is no switch, rather than a record made up from the email
        let token = saved_token(here, &target, true)?;
        let acct = service::profile(here, &token).map(|(_, a)| a).ok_or_else(|| format!("Did not switch: {} would not say whose login that is just now. Try again in a moment.", maker()))?;
        let incoming = saved::entry(here, &target)?.get("claudeAiOauth").cloned().ok_or(ADD_AGAIN)?;
        let blob = live::blob(here).map_err(|e| format!("Did not switch: {e}"))?;
        // keep the outgoing account's newest login: its refresh token has changed since it was saved
        if blob.is_some() {
            save_live(here).map_err(|e| format!("Did not switch: the current login could not be saved first. {e}"))?;
        }
        // swap the claude.ai login inside the live item, keeping the MCP logins beside it
        let mut blob = blob.unwrap_or_default();
        blob.insert("claudeAiOauth".into(), incoming);
        here.vault.write(&here.live_service, here.user()?, &Value::Object(blob).to_string())?;
        live::set_oauth_account(&here.account_file, &acct)?;
        // a later switch ends this one's hold, or the two would fight over the name
        let mine = SWITCHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let file = here.account_file.clone();
        Ok(std::thread::spawn(move || {
            for _ in 0..5 {
                std::thread::sleep(Duration::from_secs(1));
                if SWITCHES.load(std::sync::atomic::Ordering::SeqCst) != mine {
                    return;
                }
                if live::oauth_account(&file).as_ref().and_then(|a| a.get("emailAddress")).and_then(|e| e.as_str()) != Some(target.as_str()) {
                    let _ = live::set_oauth_account(&file, &acct);
                }
            }
        }))
    })
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
    locked(&here, || save_live(&here))
}

/// `airband accounts add`: sign in to another account in a private folder, so the login in use and every
/// running session are untouched, then keep that login.
pub(crate) fn add() -> Res<String> {
    let here = Here::this_computer();
    std::fs::create_dir_all(&here.dir).map_err(|e| e.to_string())?;
    let tmp = here.dir.join(format!("login-{}", airband_core::clock::now_ms()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let service = format!("{}-{}", row("keychain_service"), sha8(&tmp.to_string_lossy()));
    std::fs::write(tmp.join(row("account_file")), "{\"hasCompletedOnboarding\":true}").map_err(|e| e.to_string())?;
    println!("A browser opens: sign in to the account to add. The login in use now is not touched.");
    let program = row("program");
    let status = Command::new(&program).args(["auth", "login"]).env(row("home_env"), &tmp).status().map_err(|e| format!("could not run {program}: {e}"));
    let result = (|| -> Res<String> {
        if !status?.success() {
            return Err("The sign-in did not finish.".into());
        }
        let blob: Value = serde_json::from_str(&here.vault.read(&service, here.user()?)?.ok_or("The sign-in saved no login.")?).map_err(|e| e.to_string())?;
        let oauth = blob.get("claudeAiOauth").cloned().ok_or("The sign-in saved no login for an account.")?;
        let acct = live::oauth_account(&tmp.join(row("account_file"))).ok_or("The sign-in saved no account record.")?;
        let email = acct.get("emailAddress").and_then(|e| e.as_str()).map(String::from).ok_or("The account record has no email.")?;
        locked(&here, || saved::keep(&here, &email, &oauth, &acct))?;
        Ok(email)
    })();
    if let Ok(user) = here.user() {
        here.vault.delete(&service, user);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// `airband accounts forget <account>`: delete a saved login.
pub(crate) fn forget(id: &str) -> Res<String> {
    let here = Here::this_computer();
    locked(&here, || {
        let email = saved::resolve(&here, id)?;
        saved::forget(&here, &email)?;
        Ok(email)
    })
}

#[cfg(test)]
mod tests;
