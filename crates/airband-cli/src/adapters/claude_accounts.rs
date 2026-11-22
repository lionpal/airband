//! Claude Code's accounts: the logins this computer has saved, which one every session is using, how much of
//! each one's allowance is left, and switching between them. Nothing here leaves the computer except requests
//! to Anthropic's own servers, and no token is ever printed, logged or sent to a radio.
//!
//! The live login is the keychain item `Claude Code-credentials` (account: the user), JSON with the claude.ai
//! login under `claudeAiOauth` beside the MCP logins. Every running session reads it again within 30 seconds
//! and before a request, which is why a switch reaches sessions that are already running. `oauthAccount` in
//! `~/.claude.json` names the account.
//!
//! Saved logins are one keychain item each (service `airband`, account: the email), with the list of emails,
//! and nothing secret, in `~/.airband/claude-code/accounts.json`. A login's refresh token changes every time it
//! is used: Claude Code refreshes the live one, the tower refreshes saved ones, and a switch saves the outgoing
//! login back first. So there can be one keeper of saved logins on a computer: a second tool holding copies of
//! the same logins would log this one out, and be logged out by it.
//!
//! macOS only: elsewhere Claude Code keeps its login differently, and that is not written yet.
use crate::config;
use airband_core::radio::{Account, Limit};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

type Res<T> = Result<T, String>;

pub const HARNESS: &str = "claude-code";
/// The keychain item Claude Code keeps its login under, as its row names it.
fn live_base() -> String {
    super::row_says(HARNESS, "keychain_service")
}
/// The header value its account API asks for, as its row has it.
fn oauth_beta() -> String {
    super::row_says(HARNESS, "oauth_beta")
}
const ADD_AGAIN: &str = "Sign in to it again on the computer: airband accounts add";

/// Where the pieces are. The environment can point them elsewhere so the whole thing can be tried against
/// made-up logins and a stand-in server, without touching a real login.
fn place(key: &str, default: &str) -> String {
    super::env(key).unwrap_or_else(|| default.to_string())
}
fn api() -> String {
    place("AIRBAND_CLAUDE_API", &super::row_says(HARNESS, "accounts_api"))
}
fn token_url() -> String {
    place("AIRBAND_CLAUDE_TOKEN_URL", &super::row_says(HARNESS, "accounts_token"))
}
/// What the tool is called, and who makes it, as its row has them: for what he is told.
fn tool() -> String {
    super::called(HARNESS).0
}
fn maker() -> String {
    super::called(HARNESS).1
}
fn vault_service() -> String {
    place("AIRBAND_CLAUDE_VAULT_SERVICE", "airband")
}
fn claude_config_dir() -> Option<PathBuf> {
    super::env("CLAUDE_CONFIG_DIR").map(PathBuf::from)
}
fn live_service() -> String {
    if let Some(named) = super::env("AIRBAND_CLAUDE_LIVE_SERVICE") {
        return named;
    }
    match claude_config_dir() {
        Some(d) => format!("{}-{}", live_base(), sha8(&d.to_string_lossy())),
        None => live_base(),
    }
}
fn sha8(s: &str) -> String {
    airband_core::encode::hex(&Sha256::digest(s.as_bytes())[..4])
}
fn claude_json_path() -> PathBuf {
    claude_config_dir().map(|d| d.join(".claude.json")).unwrap_or_else(|| config::home().join(".claude.json"))
}
/// The account the keychain item is kept under: this computer's login name, as the system has it.
fn user() -> Result<String, String> {
    config::account().map(|a| a.login).filter(|l| !l.is_empty()).ok_or_else(|| "This computer's account has no login name.".to_string())
}
pub(super) fn dir() -> PathBuf {
    config::dir().join("claude-code")
}
fn now_ms() -> i64 {
    airband_core::clock::now_ms() as i64
}
fn http() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into()
}
fn read_json(p: &Path) -> Res<Value> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}
/// One thing at a time across every airband process on this computer: a saved login must never be refreshed
/// twice at once, or switched while it is being refreshed.
fn locked<T>(work: impl FnOnce() -> T) -> T {
    let _ = std::fs::create_dir_all(dir());
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir().join("accounts.lock")).ok();
    #[cfg(unix)]
    if let Some(f) = &file {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    }
    let out = work();
    drop(file);
    out
}

// ── the keychain, through /usr/bin/security exactly as Claude Code uses it, so Claude Code's own access to its
//    item never changes (an item written another way would make Claude Code ask him for permission) ──

/// `security`, run as the user it really is: the login keychain is found through the home folder, and with
/// HOME pointed anywhere else `security` finds no keychain and waits for ever on a dialog.
fn security() -> Command {
    let mut c = Command::new("security");
    if let Some(a) = config::account().filter(|a| !a.home.is_empty()) {
        c.env("HOME", a.home);
    }
    c.stdin(Stdio::null());
    c
}

fn key_read(service: &str, account: &str) -> Res<Option<String>> {
    let out = security().args(["find-generic-password", "-a", account, "-s", service, "-w"]).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        // `security` exits with 44 when there is no such item (errSecItemNotFound)
        return if out.status.code() == Some(44) { Ok(None) } else { Err(format!("the keychain would not give {service} ({})", out.status)) };
    }
    let s = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string();
    // `security -w` prints anything that is not plain text as hex
    if !s.is_empty() {
        if let Some(t) = airband_core::encode::unhex(&s).and_then(|b| String::from_utf8(b).ok()).filter(|t| t.starts_with('{')) {
            return Ok(Some(t));
        }
    }
    Ok(Some(s))
}
fn key_write(service: &str, account: &str, data: &str) -> Res<()> {
    let hex = airband_core::encode::hex(data.as_bytes());
    let line = format!("add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{hex}\"\n");
    // On standard input, so the login is never in the process list. `security -i` takes a line of about 4 KB
    // at most. A longer one could only go as an argument, where any program on this computer can read it
    // while the command runs, so it is not handed over at all. (Writing the item another way, through the
    // system's own keychain calls, would make it this program's item, and Claude Code would then have to
    // ask him before reading its own login.)
    if line.len() > 4032 {
        return Err("This login is too long for airband to put in the keychain without other programs on this computer being able to see it for a moment, so nothing was changed.".into());
    }
    let status = {
        let mut child = security().arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
        child.stdin.take().expect("piped").write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?
    };
    if !status.success() {
        return Err(format!("the keychain would not keep {service} ({status})"));
    }
    match key_read(service, account)? {
        Some(back) if back == data => Ok(()),
        _ => Err(format!("the keychain did not keep {service}")),
    }
}
fn key_delete(service: &str, account: &str) {
    let _ = security().args(["delete-generic-password", "-a", account, "-s", service]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

// ── saved logins ──

fn index_path() -> PathBuf {
    dir().join("accounts.json")
}
fn index() -> Vec<String> {
    read_json(&index_path()).ok().and_then(|v| v.get("accounts").and_then(|a| a.as_array().cloned())).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}
fn write_index(list: &[String]) -> Res<()> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    config::write_private(&index_path(), json!({ "accounts": list }).to_string()).map_err(|e| e.to_string())
}
fn saved(email: &str) -> Res<Option<Value>> {
    Ok(match key_read(&vault_service(), email)? {
        Some(s) => Some(serde_json::from_str(&s).map_err(|e| format!("the saved login for {email} cannot be read: {e}"))?),
        None => None,
    })
}
fn keep(email: &str, oauth: &Value, account: &Value) -> Res<()> {
    key_write(&vault_service(), email, &json!({ "email": email, "claudeAiOauth": oauth, "oauthAccount": account, "savedAt": now_ms() }).to_string())?;
    let mut list = index();
    if !list.iter().any(|e| e == email) {
        list.push(email.into());
        write_index(&list)?;
    }
    Ok(())
}

/// The saved account he means: its whole email, or the number it is listed under (`airband accounts`).
fn resolve(query: &str) -> Res<String> {
    let list = index();
    let numbered = query.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| list.get(i));
    match list.iter().find(|e| e.as_str() == query).or(numbered) {
        Some(e) => Ok(e.clone()),
        None => Err(format!("No saved account is \"{query}\". Give its whole email or its number: {}.", if list.is_empty() { "none saved".into() } else { list.iter().enumerate().map(|(i, e)| format!("{}. {e}", i + 1)).collect::<Vec<_>>().join(", ") })),
    }
}

// ── the live login ──

fn live_blob() -> Res<Map<String, Value>> {
    let s = key_read(&live_service(), &user()?)?.ok_or_else(|| format!("{} is not signed in on this computer.", tool()))?;
    match serde_json::from_str::<Value>(&s).map_err(|e| format!("{}'s login cannot be read: {e}", tool()))? {
        Value::Object(m) => Ok(m),
        _ => Err(format!("{}'s login is not in the form expected.", tool())),
    }
}
fn live_token() -> Option<String> {
    live_blob().ok()?.get("claudeAiOauth")?.get("accessToken")?.as_str().map(String::from)
}
fn oauth_account() -> Option<Value> {
    read_json(&claude_json_path()).ok()?.get("oauthAccount").cloned()
}
fn email_of(v: &Value) -> Option<String> {
    v.get("emailAddress").and_then(|e| e.as_str()).map(String::from)
}
/// Set `oauthAccount`, reading the file again just before writing so nothing else in it is lost.
fn set_oauth_account(acct: &Value) -> Res<()> {
    let p = claude_json_path();
    let mut d = read_json(&p)?;
    d.as_object_mut().ok_or("~/.claude.json is not in the form expected")?.insert("oauthAccount".into(), acct.clone());
    // his own file, which holds his login: whole or not at all, and as private as it was
    airband_core::kept::write_keeping(&p, serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
/// Ask whose token this is. Nothing when the server cannot say (expired, offline).
fn profile_email(token: &str) -> Option<String> {
    let mut res = http().get(format!("{}/api/oauth/profile", api())).header("authorization", format!("Bearer {token}")).header("anthropic-beta", oauth_beta()).call().ok()?;
    if res.status() != 200 {
        return None;
    }
    let v: Value = serde_json::from_str(&res.body_mut().read_to_string().ok()?).ok()?;
    v.pointer("/account/email_address").or_else(|| v.pointer("/account/email")).and_then(|e| e.as_str()).map(String::from)
}
/// Who the live login belongs to: what the server says about its token, else what `~/.claude.json` says.
fn live_identity() -> Option<String> {
    live_token().and_then(|t| profile_email(&t)).or_else(|| oauth_account().as_ref().and_then(email_of))
}

/// Save the login Claude Code is using now. Returns its email.
fn save_live() -> Res<String> {
    let blob = live_blob()?;
    let oauth = blob.get("claudeAiOauth").cloned().ok_or_else(|| format!("{} is signed in with an API key or a setup token, which cannot be switched.", tool()))?;
    let token = oauth.get("accessToken").and_then(|t| t.as_str()).unwrap_or("");
    let email = profile_email(token).or_else(|| oauth_account().as_ref().and_then(email_of)).ok_or_else(|| format!("Could not tell which account {} is signed in to.", tool()))?;
    let mut acct = oauth_account().unwrap_or(json!({}));
    if email_of(&acct).as_deref() != Some(email.as_str()) {
        acct = saved(&email)?.and_then(|v| v.get("oauthAccount").cloned()).unwrap_or(json!({ "emailAddress": email }));
    }
    keep(&email, &oauth, &acct)?;
    Ok(email)
}

/// A saved login's access token, refreshed when it has run out or is about to, with the new login saved at once.
fn saved_token(email: &str) -> Res<String> {
    let entry = saved(email)?.ok_or(ADD_AGAIN)?;
    let o = entry.get("claudeAiOauth").ok_or(ADD_AGAIN)?;
    let token = o.get("accessToken").and_then(|x| x.as_str()).unwrap_or("");
    // also while the refresh token has under a week left, so a login unused for weeks never lapses
    let refresh_good = o.get("refreshTokenExpiresAt").and_then(|x| x.as_i64()).is_none_or(|t| t > now_ms() + 7 * 86_400_000);
    if refresh_good && !token.is_empty() && o.get("expiresAt").and_then(|x| x.as_i64()).is_some_and(|t| t > now_ms() + 120_000) {
        return Ok(token.to_string());
    }
    let refresh = o.get("refreshToken").and_then(|x| x.as_str()).filter(|x| !x.is_empty()).ok_or(ADD_AGAIN)?;
    let scopes = o.get("scopes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    // the login says which client it was made by; one that does not was made by Claude Code's own, as its row has it
    let client = o.get("clientId").and_then(|x| x.as_str()).map(String::from).unwrap_or_else(|| super::row_says(HARNESS, "oauth_client"));
    let mut res = http()
        .post(token_url())
        .header("content-type", "application/json")
        .send(json!({ "grant_type": "refresh_token", "refresh_token": refresh, "client_id": client, "scope": scopes }).to_string())
        .map_err(|_| format!("Could not reach {} to check it.", maker()))?;
    let body: Value = serde_json::from_str(&res.body_mut().read_to_string().map_err(|e| e.to_string())?).unwrap_or(json!({}));
    let new_token = body.get("access_token").and_then(|x| x.as_str()).ok_or(ADD_AGAIN)?;
    let mut n = o.clone();
    n["accessToken"] = json!(new_token);
    if let Some(r) = body.get("refresh_token").and_then(|x| x.as_str()) {
        n["refreshToken"] = json!(r);
    }
    if let Some(e) = body.get("expires_in").and_then(|x| x.as_i64()) {
        n["expiresAt"] = json!(now_ms() + e * 1000);
    }
    if let Some(e) = body.get("refresh_token_expires_in").and_then(|x| x.as_i64()) {
        n["refreshTokenExpiresAt"] = json!(now_ms() + e * 1000);
    }
    keep(email, &n, entry.get("oauthAccount").unwrap_or(&json!({ "emailAddress": email })))?;
    Ok(new_token.to_string())
}

// ── allowances ──


/// What Anthropic's usage answer says, as limits: the week, then the 5 hours when that window is open.
fn limits_in(usage: &Value) -> Vec<Limit> {
    [("week", "seven_day"), ("5 hours", "five_hour")]
        .into_iter()
        .filter_map(|(name, key)| {
            let w = usage.get(key)?;
            Some(Limit { name: name.into(), used: (w.get("utilization")?.as_f64()? / 100.0).clamp(0.0, 1.0), resets_at: w.get("resets_at").and_then(|r| r.as_str()).and_then(airband_core::clock::parse_rfc3339) })
        })
        .collect()
}

/// How much of an account is used. Costs nothing: it is the same question Claude Code's own usage page asks.
fn usage(token: &str) -> Result<Vec<Limit>, Unchecked> {
    let mut res = http()
        .get(format!("{}/api/oauth/usage", api()))
        .header("authorization", format!("Bearer {token}"))
        .header("anthropic-beta", oauth_beta())
        .call()
        .map_err(|_| Unchecked::Said(format!("Could not reach {} to check it.", maker())))?;
    match res.status().as_u16() {
        200 => Ok(limits_in(&serde_json::from_str(&res.body_mut().read_to_string().map_err(|e| Unchecked::Said(e.to_string()))?).unwrap_or(json!({})))),
        401 | 403 => Err(Unchecked::Expired),
        n => Err(Unchecked::Said(format!("{} would not say how much is used ({n}).", maker()))),
    }
}

// ── what the rest of airband calls ──

/// Why an account's allowance could not be read.
enum Unchecked {
    /// The service refused the login it was asked with.
    Expired,
    /// Anything else, in words for him.
    Said(String),
}
impl From<String> for Unchecked {
    fn from(said: String) -> Self {
        Unchecked::Said(said)
    }
}

fn account(email: String, live: bool, checked: Result<Vec<Limit>, Unchecked>) -> Account {
    let (limits, problem) = match checked {
        Ok(l) => (l, None),
        // the login in use is refreshed by the sessions that use it, so a refusal means none has lately
        Err(Unchecked::Expired) if live => (Vec::new(), Some("Not checked: no session has used it lately.".to_string())),
        Err(Unchecked::Expired) => (Vec::new(), Some("Its login has run out.".to_string())),
        Err(Unchecked::Said(e)) => (Vec::new(), Some(e)),
    };
    Account { id: email.clone(), harness: HARNESS.into(), label: email, live, limits, problem }
}

/// Every account, with its allowances, and anything he should know about the list.
pub fn list() -> (Vec<Account>, Option<String>) {
    if !cfg!(target_os = "macos") {
        return (Vec::new(), Some(format!("Switching {} accounts works on a Mac only for now.", tool())));
    }
    locked(|| {
        let live = live_identity();
        let mut emails = index();
        if let Some(l) = live.as_ref().filter(|l| !emails.contains(l)) {
            emails.insert(0, l.clone());
        }
        // each account is asked about at the same time; a saved login is only ever refreshed here, under the lock
        let checks: Vec<_> = emails
            .into_iter()
            .map(|e| {
                let is_live = live.as_deref() == Some(e.as_str());
                std::thread::spawn(move || {
                    let token = if is_live { live_token().ok_or_else(|| format!("{} is not signed in.", tool())) } else { saved_token(&e) };
                    let checked = token.map_err(Unchecked::from).and_then(|t| usage(&t));
                    account(e, is_live, checked)
                })
            })
            .collect();
        let accounts: Vec<Account> = checks.into_iter().filter_map(|h| h.join().ok()).collect();
        let note = match accounts.len() {
            0 => Some(format!("{} is not signed in on this computer.", tool())),
            1 => Some("To add another account, on the computer: airband accounts add".into()),
            _ => None,
        };
        (accounts, note)
    })
}

static SWITCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Make a saved account the one every running session uses. The login is swapped at once; the returned thread
/// then holds the account's name in place for a few seconds, because a session still on the old login may
/// write its own back.
pub fn switch(id: &str) -> Res<std::thread::JoinHandle<()>> {
    locked(|| {
        let target = resolve(id)?;
        let entry = saved(&target)?.ok_or(ADD_AGAIN)?;
        let incoming = entry.get("claudeAiOauth").cloned().ok_or(ADD_AGAIN)?;
        // keep the outgoing account's newest login: its refresh token has changed since it was saved
        if live_blob().is_ok() {
            match save_live() {
                Ok(out) if out == target => return Ok(std::thread::spawn(|| {})),
                Ok(_) => {}
                Err(e) => return Err(format!("Did not switch: the current login could not be saved first. {e}")),
            }
        }
        // swap the claude.ai login inside the live item, keeping the MCP logins beside it
        let mut blob = live_blob().unwrap_or_default();
        blob.insert("claudeAiOauth".into(), incoming);
        key_write(&live_service(), &user()?, &Value::Object(blob).to_string())?;
        let acct = entry.get("oauthAccount").cloned().unwrap_or(json!({ "emailAddress": target }));
        set_oauth_account(&acct)?;
        // a later switch ends this one's hold, or the two would fight over the name
        let mine = SWITCHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Ok(std::thread::spawn(move || {
            for _ in 0..5 {
                std::thread::sleep(Duration::from_secs(1));
                if SWITCHES.load(std::sync::atomic::Ordering::SeqCst) != mine {
                    return;
                }
                if oauth_account().as_ref().and_then(email_of).as_deref() != Some(target.as_str()) {
                    let _ = set_oauth_account(&acct);
                }
            }
        }))
    })
}

/// `airband accounts save`: keep the login Claude Code is using now.
pub fn save() -> Res<String> {
    locked(save_live)
}

/// `airband accounts add`: sign in to another account in a private folder, so the login in use and every
/// running session are untouched, then keep that login.
pub fn add() -> Res<String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = dir().join(format!("login-{}", now_ms()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let service = format!("{}-{}", live_base(), sha8(&tmp.to_string_lossy()));
    std::fs::write(tmp.join(".claude.json"), "{\"hasCompletedOnboarding\":true}").map_err(|e| e.to_string())?;
    println!("A browser opens: sign in to the account to add. The login in use now is not touched.");
    let program = super::row_says(HARNESS, "program");
    let status = Command::new(&program).args(["auth", "login"]).env("CLAUDE_CONFIG_DIR", &tmp).status().map_err(|e| format!("could not run {program}: {e}"));
    let result = (|| -> Res<String> {
        if !status?.success() {
            return Err("The sign-in did not finish.".into());
        }
        let blob: Value = serde_json::from_str(&key_read(&service, &user()?)?.ok_or("The sign-in saved no login.")?).map_err(|e| e.to_string())?;
        let oauth = blob.get("claudeAiOauth").cloned().ok_or("The sign-in saved no login for an account.")?;
        let acct = read_json(&tmp.join(".claude.json"))?.get("oauthAccount").cloned().ok_or("The sign-in saved no account record.")?;
        let email = email_of(&acct).ok_or("The account record has no email.")?;
        locked(|| keep(&email, &oauth, &acct))?;
        Ok(email)
    })();
    if let Ok(user) = user() {
        key_delete(&service, &user);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// `airband accounts forget <account>`: delete a saved login.
pub fn forget(id: &str) -> Res<String> {
    locked(|| {
        let email = resolve(id)?;
        key_delete(&vault_service(), &email);
        write_index(&index().into_iter().filter(|x| *x != email).collect::<Vec<_>>())?;
        Ok(email)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_come_from_the_usage_answer() {
        let usage = json!({ "five_hour": null, "seven_day": { "utilization": 59.0, "resets_at": "2026-11-25T01:00:00+00:00" }, "seven_day_opus": null });
        assert_eq!(limits_in(&usage), vec![Limit { name: "week".into(), used: 0.59, resets_at: Some(1_795_568_400_000) }]);
        let both = json!({ "five_hour": { "utilization": 12, "resets_at": "2026-11-19T10:00:00Z" }, "seven_day": { "utilization": 140.0 } });
        let limits = limits_in(&both);
        assert_eq!((limits[0].name.as_str(), limits[0].used, limits[0].resets_at), ("week", 1.0, None));
        assert_eq!((limits[1].name.as_str(), limits[1].used), ("5 hours", 0.12));
        assert!(limits_in(&json!({})).is_empty());
    }
}
