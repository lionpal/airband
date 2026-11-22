//! Every test here keeps logins in memory and talks to a stand-in account service on this computer: no
//! keychain item, file of his or real service is ever touched.
use super::*;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::sync::{Arc, Mutex};

/// A stand-in for Anthropic's account service: who each token belongs to, and what the token address answers.
#[derive(Default)]
struct Service {
    /// Access token → email, for the profile and usage questions. A token not here is refused (401).
    tokens: HashMap<String, String>,
    /// What the token address answers: status and body.
    refresh: Option<(u16, String)>,
    /// What the usage question answers for a known token, when not the usual one.
    usage: Option<(u16, String)>,
    /// Every path asked for, in order.
    asked: Vec<String>,
}

fn serve(service: Arc<Mutex<Service>>) -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in l.incoming().flatten() {
            let service = service.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (mut auth, mut len) = (String::new(), 0usize);
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end().to_string();
                    if h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap();
                    match k.to_ascii_lowercase().as_str() {
                        "authorization" => auth = v.trim().trim_start_matches("Bearer ").to_string(),
                        "content-length" => len = v.trim().parse().unwrap(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let (status, answer) = {
                    let mut s = service.lock().unwrap();
                    s.asked.push(path.clone());
                    match path.as_str() {
                        "/token" => s.refresh.clone().unwrap_or((500, String::new())),
                        p => match s.tokens.get(&auth).cloned() {
                            None => (401, "{}".into()),
                            Some(email) if p == "/api/oauth/profile" => (200, json!({ "account": { "uuid": format!("u-{email}"), "email": email }, "organization": { "uuid": "o-1" } }).to_string()),
                            Some(_) => s.usage.clone().unwrap_or((200, json!({ "seven_day": { "utilization": 10.0 } }).to_string())),
                        },
                    }
                };
                let mut out = stream;
                let _ = write!(out, "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}", answer.len());
            });
        }
    });
    port
}

struct Bench {
    here: Here,
    service: Arc<Mutex<Service>>,
    _dir: Folder,
}
/// A folder of its own for one test, removed after.
struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bench(service: Service) -> Bench {
    let service = Arc::new(Mutex::new(service));
    let port = serve(service.clone());
    let dir = std::env::temp_dir().join(format!("airband-accounts-{}-{}", std::process::id(), sha8(&format!("{:?}{}", std::thread::current().id(), airband_core::clock::now_ms()))));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("account-file.json"), "{}").unwrap();
    let here = Here {
        vault: Box::new(vault::Memory::default()),
        api: format!("http://127.0.0.1:{port}"),
        token_url: format!("http://127.0.0.1:{port}/token"),
        dir: dir.join("airband"),
        account_file: dir.join("account-file.json"),
        live_service: "live".into(),
        vault_service: "saved".into(),
        user: Ok("me".into()),
    };
    Bench { here, service, _dir: Folder(dir) }
}

/// A login: its tokens, with an access token that has run out or not.
fn login(access: &str, refresh: &str, fresh: bool) -> Value {
    let at = airband_core::clock::now_ms() as i64 + if fresh { 3_600_000 } else { -60_000 };
    json!({ "accessToken": access, "refreshToken": refresh, "expiresAt": at })
}
fn put_live(b: &Bench, oauth: Value) {
    b.here.vault.write("live", "me", &json!({ "claudeAiOauth": oauth, "mcpOAuth": { "kept": "beside it" } }).to_string()).unwrap();
}
fn put_saved(b: &Bench, email: &str, oauth: Value) {
    saved::keep(&b.here, email, &oauth, &json!({ "accountUuid": format!("u-{email}"), "emailAddress": email })).unwrap();
}
fn refreshes(b: &Bench) -> usize {
    b.service.lock().unwrap().asked.iter().filter(|p| *p == "/token").count()
}

#[test]
fn the_account_record_is_made_from_the_profile_as_claude_code_makes_it() {
    let profile = json!({
        "account": { "uuid": "a-1", "email": "him@example.com", "display_name": "Him", "created_at": "2025-01-01T00:00:00Z" },
        "organization": { "uuid": "o-1", "billing_type": "stripe_subscription", "has_extra_usage_enabled": true, "seat_tier": null }
    });
    let (email, r) = service::account_record(&profile).unwrap();
    assert_eq!(email, "him@example.com");
    assert_eq!((r["accountUuid"].as_str(), r["organizationUuid"].as_str(), r["emailAddress"].as_str(), r["displayName"].as_str()), (Some("a-1"), Some("o-1"), Some("him@example.com"), Some("Him")));
    assert_eq!((r["billingType"].as_str(), r["hasExtraUsageEnabled"].as_bool()), (Some("stripe_subscription"), Some(true)));
    assert!(r.get("subscriptionCreatedAt").is_none());
    // without the account's own ids there is no record to write
    assert!(service::account_record(&json!({ "account": { "email": "him@example.com" } })).is_none());
}

#[test]
fn a_field_the_profile_does_not_have_is_left_out_of_the_record_not_made_up() {
    let profile = json!({ "account": { "uuid": "a-1", "email": "him@example.com" }, "organization": { "uuid": "o-1" } });
    let (_, r) = service::account_record(&profile).unwrap();
    assert!(r.get("hasExtraUsageEnabled").is_none(), "made up: {r}");
    assert!(r.get("ccOnboardingFlags").is_none(), "made up: {r}");
    for field in ["accountCreatedAt", "claudeCodeTrialEndsAt", "claudeCodeTrialDurationDays", "seatTier"] {
        assert!(r.get(field).is_none(), "made up: {r}");
    }
}

#[test]
fn limits_come_from_the_usage_answer() {
    let usage = json!({ "five_hour": null, "seven_day": { "utilization": 59.0, "resets_at": "2026-11-25T01:00:00+00:00" }, "seven_day_opus": null });
    assert_eq!(service::limits_in(&usage), vec![airband_core::radio::Limit { name: "week".into(), used: 0.59, resets_at: Some(1_795_568_400_000) }]);
    let both = json!({ "five_hour": { "utilization": 12, "resets_at": "2026-11-19T10:00:00Z" }, "seven_day": { "utilization": 140.0 } });
    let limits = service::limits_in(&both);
    assert_eq!((limits[0].name.as_str(), limits[0].used, limits[0].resets_at), ("week", 1.0, None));
    assert_eq!((limits[1].name.as_str(), limits[1].used), ("5 hours", 0.12));
    assert!(service::limits_in(&json!({})).is_empty());
}

#[test]
fn a_usage_answer_that_cannot_be_read_is_said_to_be_so_not_shown_as_no_limits() {
    let b = bench(Service { tokens: HashMap::from([("t".into(), "a@x".into())]), usage: Some((200, "not json".into())), ..Default::default() });
    match service::usage(&b.here, "t") {
        Err(Unchecked::Said(why)) => assert!(why.contains("could not be read"), "{why}"),
        Ok(l) => panic!("an unreadable answer was taken for {} limits", l.len()),
        Err(Unchecked::Expired) => panic!("taken for a refusal"),
    }
}

#[test]
fn a_refresh_the_service_could_not_do_just_now_is_not_told_as_sign_in_again() {
    for (status, body) in [(500, ""), (503, "{}"), (429, "{}"), (200, "not json")] {
        let b = bench(Service { refresh: Some((status, body.into())), ..Default::default() });
        let why = service::refresh(&b.here, &login("old", "r1", false)).unwrap_err();
        assert_ne!(why, ADD_AGAIN, "{status} {body:?} was told as a login that has gone");
    }
    // a refusal of the refresh token itself is
    let b = bench(Service { refresh: Some((400, json!({ "error": "invalid_grant" }).to_string())), ..Default::default() });
    assert_eq!(service::refresh(&b.here, &login("old", "r1", false)).unwrap_err(), ADD_AGAIN);
}

#[test]
fn switching_to_the_account_already_in_use_never_refreshes_its_saved_copy() {
    // the saved copy has run out and shares the live login's refresh token: refreshing it would sign the live one out
    let b = bench(Service { tokens: HashMap::from([("live-access".into(), "a@x".into())]), refresh: Some((200, json!({ "access_token": "new", "refresh_token": "r2" }).to_string())), ..Default::default() });
    put_live(&b, login("live-access", "r1", true));
    put_saved(&b, "a@x", login("old-access", "r1", false));
    switch_in(&b.here, "a@x").unwrap().join().unwrap();
    assert_eq!(refreshes(&b), 0, "the saved copy of the login in use was refreshed");
    assert_eq!(saved::refresh_token_of(&b.here, "a@x").as_deref(), Some("r1"));
}

#[test]
fn listing_with_the_live_login_run_out_does_not_refresh_its_saved_copy_or_lose_it() {
    // the live access token has run out (the service refuses it) and a saved copy holds the same refresh token
    let b = bench(Service { refresh: Some((200, json!({ "access_token": "new", "refresh_token": "r2" }).to_string())), ..Default::default() });
    put_live(&b, login("expired", "r1", false));
    put_saved(&b, "a@x", login("old-access", "r1", false));
    let (accounts, _) = list_in(&b.here);
    assert_eq!(refreshes(&b), 0, "the saved copy of the login in use was refreshed");
    assert_eq!(accounts.iter().map(|a| (a.label.as_str(), a.live)).collect::<Vec<_>>(), vec![("a@x", true)]);
}

#[test]
fn listing_when_nothing_says_which_account_is_in_use_refreshes_nothing_and_says_so() {
    let b = bench(Service { refresh: Some((200, json!({ "access_token": "new", "refresh_token": "r9" }).to_string())), ..Default::default() });
    put_live(&b, login("expired", "r-live", false));
    put_saved(&b, "b@x", login("old", "r-b", false));
    let (accounts, note) = list_in(&b.here);
    assert_eq!(refreshes(&b), 0);
    assert!(note.unwrap_or_default().contains("Could not tell which account"));
    assert!(accounts.iter().all(|a| !a.live));
    // and a switch refuses rather than risk it
    assert!(switch_in(&b.here, "b@x").unwrap_err().starts_with("Did not switch"));
    assert_eq!(refreshes(&b), 0);
}

#[test]
fn a_live_login_that_cannot_be_read_is_never_replaced() {
    let b = bench(Service { tokens: HashMap::from([("b-access".into(), "b@x".into())]), ..Default::default() });
    b.here.vault.write("live", "me", "not json, and the MCP logins are in here").unwrap();
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    assert!(switch_in(&b.here, "b@x").is_err());
    assert_eq!(b.here.vault.read("live", "me").unwrap().as_deref(), Some("not json, and the MCP logins are in here"));
}

#[test]
fn a_list_of_saved_accounts_that_cannot_be_read_is_never_written_over() {
    let b = bench(Service::default());
    std::fs::create_dir_all(&b.here.dir).unwrap();
    let list = b.here.dir.join("accounts.json");
    std::fs::write(&list, "{\"accounts\": [\"a@x\", \"b@x\"").unwrap();
    assert!(saved::keep(&b.here, "c@x", &login("c", "r-c", true), &json!({})).is_err());
    assert_eq!(std::fs::read_to_string(&list).unwrap(), "{\"accounts\": [\"a@x\", \"b@x\"");
    assert!(saved::forget(&b.here, "a@x").is_err());
    assert_eq!(std::fs::read_to_string(&list).unwrap(), "{\"accounts\": [\"a@x\", \"b@x\"");
}

#[test]
fn a_switch_saves_the_outgoing_login_and_keeps_what_is_beside_it() {
    let b = bench(Service { tokens: HashMap::from([("a-access".into(), "a@x".into()), ("b-access".into(), "b@x".into())]), ..Default::default() });
    put_live(&b, login("a-access", "r-a2", true));
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    switch_in(&b.here, "b@x").unwrap();
    let live: Value = serde_json::from_str(&b.here.vault.read("live", "me").unwrap().unwrap()).unwrap();
    assert_eq!(live["claudeAiOauth"]["accessToken"], "b-access");
    assert_eq!(live["mcpOAuth"]["kept"], "beside it");
    assert_eq!(saved::refresh_token_of(&b.here, "a@x").as_deref(), Some("r-a2"));
    assert_eq!(live::oauth_account(&b.here.account_file).unwrap()["emailAddress"], "b@x");
}

#[test]
fn a_switch_the_account_file_cannot_take_changes_nothing() {
    let b = bench(Service { tokens: HashMap::from([("a-access".into(), "a@x".into()), ("b-access".into(), "b@x".into())]), ..Default::default() });
    put_live(&b, login("a-access", "r-a", true));
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    std::fs::remove_file(&b.here.account_file).unwrap();
    let said = switch_in(&b.here, "b@x").map(|_| ()).unwrap_err();
    assert!(said.starts_with("Did not switch"), "{said}");
    let live: Value = serde_json::from_str(&b.here.vault.read("live", "me").unwrap().unwrap()).unwrap();
    assert_eq!(live["claudeAiOauth"]["accessToken"], "a-access");
}

#[test]
fn a_switch_away_from_an_account_known_only_by_its_refresh_token_goes_ahead() {
    // the server does not know the live access token (it has just been refreshed elsewhere, say): the saved copy
    // that shares its refresh token says whose it is
    let b = bench(Service { tokens: HashMap::from([("b-access".into(), "b@x".into())]), ..Default::default() });
    put_saved(&b, "a@x", login("a-old", "r-a", true));
    put_live(&b, login("a-new", "r-a", true));
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    switch_in(&b.here, "b@x").unwrap();
    let live: Value = serde_json::from_str(&b.here.vault.read("live", "me").unwrap().unwrap()).unwrap();
    assert_eq!(live["claudeAiOauth"]["accessToken"], "b-access");
    // the outgoing login is kept as it was last, under its own account
    let kept: Value = serde_json::from_str(&b.here.vault.read("saved", "a@x").unwrap().unwrap()).unwrap();
    assert_eq!(kept["claudeAiOauth"]["accessToken"], "a-new");
}

// N39: a switch happens whole or not at all. When the account record cannot be written, the login is left as
// it was and he is told it did not switch.
#[cfg(unix)]
#[test]
fn n39_a_switch_whose_account_record_cannot_be_written_leaves_the_login_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let b = bench(Service { tokens: HashMap::from([("a-access".into(), "a@x".into()), ("b-access".into(), "b@x".into())]), ..Default::default() });
    put_live(&b, login("a-access", "r-a2", true));
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    // the folder the account record is in takes no new files, so the record cannot be replaced
    let folder = b.here.account_file.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    let switched = switch_in(&b.here, "b@x");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    let live: Value = serde_json::from_str(&b.here.vault.read("live", "me").unwrap().unwrap()).unwrap();
    assert!(matches!(&switched, Err(e) if e.starts_with("Did not switch")), "{:?}", switched.as_ref().err());
    assert_eq!(live["claudeAiOauth"]["accessToken"], "a-access", "the login was swapped and the record was not");
}

#[test]
fn the_account_file_is_known_to_hold_an_account_by_its_id_never_by_its_email() {
    let chosen = json!({"accountUuid": "person-1", "emailAddress": "a@x"});
    assert!(same_account(Some(&json!({"accountUuid": "person-1", "emailAddress": "renamed@x"})), &chosen));
    assert!(!same_account(Some(&json!({"accountUuid": "person-2", "emailAddress": "a@x"})), &chosen), "same email, another person");
    assert!(!same_account(Some(&json!({"emailAddress": "a@x"})), &chosen), "no id says nothing");
    assert!(!same_account(None, &chosen));
}

use vault::Vault as _;

/// A keychain that keeps logins and will not let one go.
struct Clinging(vault::Memory);
impl vault::Vault for Clinging {
    fn read(&self, service: &str, account: &str) -> Res<Option<String>> {
        self.0.read(service, account)
    }
    fn write(&self, service: &str, account: &str, data: &str) -> Res<()> {
        self.0.write(service, account, data)
    }
    fn delete(&self, _: &str, _: &str) -> Res<()> {
        Err("the keychain is locked".into())
    }
}

#[test]
fn a_saved_login_the_keychain_will_not_delete_stays_listed() {
    let mut b = bench(Service::default());
    put_saved(&b, "a@x", login("a", "r-a", true));
    let kept = std::mem::replace(&mut b.here.vault, Box::new(vault::Memory::default()));
    let memory = vault::Memory::default();
    memory.write("saved", "a@x", &kept.read("saved", "a@x").unwrap().unwrap()).unwrap();
    b.here.vault = Box::new(Clinging(memory));
    assert!(saved::forget(&b.here, "a@x").is_err());
    assert_eq!(saved::resolve(&b.here, "a@x").unwrap(), "a@x", "still listed, as the keychain still has it");
}

/// A keychain in which the running tool refreshes its login once, just as the outgoing one is being kept.
struct RefreshedMeanwhile(vault::Memory, std::sync::atomic::AtomicBool);
impl vault::Vault for RefreshedMeanwhile {
    fn read(&self, service: &str, account: &str) -> Res<Option<String>> {
        self.0.read(service, account)
    }
    fn write(&self, service: &str, account: &str, data: &str) -> Res<()> {
        self.0.write(service, account, data)?;
        if service == "saved" && account == "a@x" && !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.0.write("live", "me", &json!({ "claudeAiOauth": login("a-access3", "r-a3", true), "mcpOAuth": { "kept": "beside it" } }).to_string())?;
        }
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Res<()> {
        self.0.delete(service, account)
    }
}

#[test]
fn a_login_the_tool_refreshes_while_switching_is_the_one_kept() {
    let mut b = bench(Service { tokens: HashMap::from([("a-access".into(), "a@x".into()), ("b-access".into(), "b@x".into())]), ..Default::default() });
    b.here.vault = Box::new(RefreshedMeanwhile(vault::Memory::default(), Default::default()));
    put_live(&b, login("a-access", "r-a2", true));
    put_saved(&b, "b@x", login("b-access", "r-b", true));
    switch_in(&b.here, "b@x").unwrap();
    // the refresh token the tool moved on to is the one kept for that account, never the one it threw away
    assert_eq!(saved::refresh_token_of(&b.here, "a@x").as_deref(), Some("r-a3"));
    let live: Value = serde_json::from_str(&b.here.vault.read("live", "me").unwrap().unwrap()).unwrap();
    assert_eq!(live["claudeAiOauth"]["accessToken"], "b-access");
}
