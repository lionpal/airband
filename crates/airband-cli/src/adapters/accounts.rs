//! A coding tool's accounts: listing them for every tool that has them, switching, and `airband accounts`; and
//! who an account is, read the way its tool's row says.
use airband_core::radio::wire;
use super::*;
use airband_core::identity::{AccountIdentity, IdSource, Place, Under};

/// Every account of every harness that has them.
pub(crate) fn accounts() -> Accounts {
    let (mut all, mut notes) = (Vec::new(), Vec::new());
    for book in HARNESSES.iter().filter_map(|h| h.accounts()) {
        let (accounts, note) = (book.list)();
        all.extend(accounts);
        notes.extend(note);
    }
    let checked_at = airband_core::clock::now_ms();
    Accounts { schema: airband_core::radio::SCHEMA.into(), accounts: all, note: (!notes.is_empty()).then(|| notes.join(" ")), checked_at }
}
fn book(harness: &str) -> Result<&'static AccountBook, String> {
    self::harness(harness).and_then(|h| h.accounts()).ok_or_else(|| format!("{} has no accounts to switch between.", called(harness).0))
}
pub(crate) fn use_account(harness: &str, id: &str) -> Result<std::thread::JoinHandle<()>, String> {
    (book(harness)?.switch)(id)
}

/// Where a tool row's identity sources are looked for on this computer, and how its tool's environment and
/// keychain items are read. Tests give throwaway folders and stand-ins.
pub(crate) struct Looking<'a> {
    /// The tool's own folder (`Under::Home`).
    pub(crate) tool_home: std::path::PathBuf,
    /// His home folder (`Under::User`).
    pub(crate) user_home: std::path::PathBuf,
    pub(crate) env: &'a dyn Fn(&str) -> Option<String>,
    /// A keychain item's value. The one way in is the keychain read a tool's accounts already use; with none,
    /// keychain sources give nothing.
    pub(crate) keychain: &'a dyn Fn(&str) -> Option<String>,
}

/// Who the account a tool is signed in with is, as its row says to find out: the first of its `account_id`
/// sources that gives anything. A source that holds several (a table of credentials) gives one only when it holds
/// exactly one: with several, nothing says which is the one in use.
pub(crate) fn identity_of(tool: &str, at: &Looking) -> AccountIdentity {
    let Some(row) = super::tool_row(tool) else { return AccountIdentity::None };
    let Ok(sources) = airband_core::identity::sources(&row.with) else { return AccountIdentity::None };
    for src in &sources {
        let mut found = found_at(src, at);
        if found.len() == 1 {
            return found.remove(0);
        }
        if !found.is_empty() {
            return AccountIdentity::None;
        }
    }
    AccountIdentity::None
}

/// Who an account is, from a copy of one of its tool's own files that airband keeps (a saved login keeps the
/// account record its tool wrote). `file` is the file's name as the row gives it; only the row's sources in a
/// file of that name are read from the copy.
pub(crate) fn identity_in_copy(tool: &str, file: &str, doc: &Value) -> AccountIdentity {
    let Some(row) = super::tool_row(tool) else { return AccountIdentity::None };
    let Ok(sources) = airband_core::identity::sources(&row.with) else { return AccountIdentity::None };
    sources
        .iter()
        .filter(|s| matches!(&s.place, Place::File { file: f, .. } if f == file))
        .find_map(|s| s.identity(doc, None))
        .unwrap_or_default()
}

/// Every identity one source gives: none, one, or one per row of a table. What is read stays in here: a key is
/// turned into its fingerprint and last four characters and goes no further.
fn found_at(src: &IdSource, at: &Looking) -> Vec<AccountIdentity> {
    let folder = |under: &Under| match under {
        Under::Home => at.tool_home.clone(),
        Under::User => at.user_home.clone(),
    };
    let one = |doc: Option<Value>| doc.and_then(|d| src.identity(&d, None)).into_iter().collect();
    match &src.place {
        Place::File { file, under } => one(airband_io::files::read_bytes(&folder(under).join(file)).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok())),
        Place::Env { name } => one((at.env)(name).map(Value::String)),
        Place::Keychain { service } => one((at.keychain)(service).map(|v| serde_json::from_str(&v).unwrap_or(Value::String(v)))),
        Place::Sqlite { file, under, table, column, provider_column } => table_rows(&folder(under).join(file), table, column, provider_column)
            .into_iter()
            .filter_map(|(provider, doc)| src.identity(&doc, Some(&provider)))
            .collect(),
    }
}

/// Each row's provider and JSON value, read from a database the tool may be writing: opened read-only. Names
/// come from the built-in row and are quoted whole; one with a quote in it reads nothing.
fn table_rows(path: &std::path::Path, table: &str, column: &str, provider_column: &str) -> Vec<(String, Value)> {
    use rusqlite::{Connection, OpenFlags};
    if [table, column, provider_column].iter().any(|n| n.contains('"')) || !path.is_file() {
        return Vec::new();
    }
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return Vec::new() };
    let Ok(mut q) = db.prepare(&format!("SELECT \"{provider_column}\", \"{column}\" FROM \"{table}\"")) else { return Vec::new() };
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
    rows.map(|it| it.filter_map(Result::ok).filter_map(|(p, v)| Some((p, serde_json::from_str(&v).ok()?))).collect()).unwrap_or_default()
}

/// What came of asking the tower on this computer.
#[derive(Debug)]
enum ByTower {
    /// Nothing is listening: no tower runs here, so the command does the work itself.
    NotRunning,
    /// The tower took it: its answer, or why there is none. A tower that took the request and did not answer in
    /// time may still be doing it, so the work is never done a second time here.
    Said(Result<Accounts, String>),
}

fn by_tower(sent: Result<airband_io::http::Said<Accounts>, airband_io::http::Error>) -> ByTower {
    use airband_io::http::{Error, Said};
    ByTower::Said(match sent {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => return ByTower::NotRunning,
        Err(Error::ConnectionFailed) => return ByTower::NotRunning,
        Err(Error::Timeout(_)) => Err("airband on this computer did not answer in time. It may still be doing it: look again with: airband accounts".into()),
        Err(e) => Err(format!("airband on this computer could not be asked: {e}")),
        Ok(Said::Took(now)) => Ok(now),
        Ok(Said::No(no)) => Err(no.reason().map_or_else(|| format!("airband refused ({}).", no.status()), String::from)),
        Ok(Said::Trouble(n)) => Err(format!("airband's answer could not be read ({n}).")),
    })
}

/// The tool whose accounts the command is about: the one he names with `--tool`, or the only one that has
/// accounts.
fn which_tool(args: &mut Vec<String>) -> Result<&'static str, String> {
    let with: Vec<&'static str> = HARNESSES.iter().filter(|h| h.accounts().is_some()).map(|h| h.name()).collect();
    if let Some(at) = args.iter().position(|a| a == "--tool") {
        let named = args.get(at + 1).cloned().ok_or("Which tool? --tool <id>")?;
        args.drain(at..=at + 1);
        return with.iter().copied().find(|t| *t == named).ok_or_else(|| format!("{named} has no accounts to switch between here. These do: {}.", with.join(", ")));
    }
    match with.as_slice() {
        [] => Err("None of your coding tools has accounts to switch between here.".into()),
        [only] => Ok(only),
        many => Err(format!("Say which tool: airband accounts --tool <{}>", many.join("|"))),
    }
}

/// `airband accounts`: list them. `save`, `add`, `use <account>`, `forget <account>`. With a tower running,
/// listing and switching go through it, so there is one keeper of the saved logins at a time.
pub(crate) fn accounts_command(cfg: &config::Config, args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let harness = which_tool(&mut args)?;
    let here = super::TowerHere::here(cfg, Duration::from_secs(60));
    let show = |a: &Accounts| {
        for acct in &a.accounts {
            let limits: Vec<String> = acct.limits.iter().map(|l| format!("{} {:.0}%", l.name, l.used * 100.0)).collect();
            println!("{:<7}{:<34} {}", if acct.live { "in use" } else { "" }, acct.label, acct.problem.clone().unwrap_or_else(|| limits.join(", ")));
        }
        if let Some(note) = &a.note {
            println!("{note}");
        }
    };
    match args.first().map(String::as_str) {
        None => match by_tower(here.call::<wire::at::Accounts>(None, &wire::AccountsQuery { fresh: true }, None)) {
            ByTower::Said(now) => show(&now?),
            ByTower::NotRunning => show(&accounts()),
        },
        Some("use") => {
            let id = args.get(1).ok_or("Which account? airband accounts use <its email or number>")?;
            match by_tower(here.call::<wire::at::UseAccount>(None, &wire::Nothing {}, Some(&airband_core::radio::UseAccount { harness: harness.to_string(), id: id.to_string() }))) {
                ByTower::Said(now) => show(&now?),
                ByTower::NotRunning => {
                    use_account(harness, id)?.join().map_err(|_| "The switch ran into trouble partway; check which account is in use: airband accounts".to_string())?;
                    show(&accounts());
                }
            }
        }
        Some("save") => println!("Saved {}.", (book(harness)?.save)()?),
        Some("add") => println!("Saved {}. Every account stays signed in; switch with: airband accounts use <account>", (book(harness)?.add)()?),
        Some("forget") => println!("Forgot {}.", (book(harness)?.forget)(args.get(1).ok_or("Which account? airband accounts forget <its email or number>")?)?),
        Some(other) => return Err(format!("unknown: accounts {other}. Try: airband accounts, accounts save, accounts add, accounts use <account>, accounts forget <account>")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tower that takes the connection and never answers.
    fn silent_tower() -> (std::net::TcpListener, u16) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    #[test]
    fn a_tower_that_took_the_request_and_did_not_answer_in_time_is_not_taken_for_no_tower() {
        let (_held, port) = silent_tower();
        let agent = airband_io::http::Client::new(Duration::from_millis(500));
        match by_tower(airband_io::http::call::<wire::at::UseAccount>(&agent, &format!("http://127.0.0.1:{port}"), &[], None, &wire::Nothing {}, Some(&airband_core::radio::UseAccount { harness: "t".into(), id: "1".into() }))) {
            ByTower::Said(Err(why)) => assert!(why.contains("did not answer in time"), "{why}"),
            other => panic!("a slow tower must not make the switch run here as well: {other:?}"),
        }
    }

    /// A throwaway folder for one test: his home and the tool's own, with nothing of his in it.
    fn folder() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("airband-identity-{}", config::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn looking<'a>(home: &std::path::Path, env: &'a dyn Fn(&str) -> Option<String>, keychain: &'a dyn Fn(&str) -> Option<String>) -> Looking<'a> {
        Looking { tool_home: home.to_path_buf(), user_home: home.to_path_buf(), env, keychain }
    }
    const NOTHING: &dyn Fn(&str) -> Option<String> = &|_| None;

    #[test]
    fn a_signed_in_account_is_known_by_the_id_its_tool_wrote_and_a_key_by_its_fingerprint() {
        let home = folder();
        // nothing there: nothing known, never a guess
        assert_eq!(identity_of("claude-code", &looking(&home, NOTHING, NOTHING)), AccountIdentity::None);
        // a key in the tool's environment, with no account record: its fingerprint, and its last four to show
        let env = |k: &str| (k == "ANTHROPIC_API_KEY").then(|| "fake-key-0000-wxyz".to_string());
        assert_eq!(identity_of("claude-code", &looking(&home, &env, NOTHING)), AccountIdentity::of_key("anthropic", "fake-key-0000-wxyz"));
        // the account record wins over the key: the first source that gives anything
        std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"person-1","organizationUuid":"org-1","emailAddress":"x"}}"#).unwrap();
        assert_eq!(identity_of("claude-code", &looking(&home, &env, NOTHING)), AccountIdentity::Id { id: "person-1".into(), org: Some("org-1".into()) });
        // a saved copy of that record is read the same way
        let saved = serde_json::json!({ "email": "x", "oauthAccount": { "accountUuid": "person-2", "organizationUuid": "org-1" } });
        assert_eq!(identity_in_copy("claude-code", ".claude.json", &saved), AccountIdentity::Id { id: "person-2".into(), org: Some("org-1".into()) });
        assert_eq!(identity_in_copy("claude-code", "another.json", &saved), AccountIdentity::None, "only sources in a file of that name");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn an_id_inside_a_token_is_read_from_its_payload_and_the_person_is_told_from_the_workspace() {
        let home = folder();
        let auth = serde_json::json!({ "https://api.openai.com/auth": { "chatgpt_user_id": "person-1", "chatgpt_account_id": "workspace-1" } });
        let token = format!("head.{}.sig", airband_core::encode::base64_url(auth.to_string().as_bytes()));
        std::fs::write(home.join("auth.json"), serde_json::json!({ "tokens": { "id_token": token } }).to_string()).unwrap();
        assert_eq!(identity_of("codex", &looking(&home, NOTHING, NOTHING)), AccountIdentity::Id { id: "person-1".into(), org: Some("workspace-1".into()) });
        // signed in with a key instead: the key in the same file
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"fake-openai-1234"}"#).unwrap();
        assert_eq!(identity_of("codex", &looking(&home, NOTHING, NOTHING)), AccountIdentity::of_key("openai", "fake-openai-1234"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_table_of_credentials_gives_its_one_key_by_its_rows_provider_and_nothing_when_it_holds_several() {
        let home = folder();
        let db_path = home.join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch("CREATE TABLE credential (id TEXT, integration_id TEXT, value TEXT);").unwrap();
        db.execute("INSERT INTO credential VALUES ('c1', 'provider-a', ?1)", [r#"{"type":"key","key":"fake-a-5678"}"#]).unwrap();
        assert_eq!(identity_of("opencode", &looking(&home, NOTHING, NOTHING)), AccountIdentity::of_key("provider-a", "fake-a-5678"));
        db.execute("INSERT INTO credential VALUES ('c2', 'provider-b', ?1)", [r#"{"type":"key","key":"fake-b-9999"}"#]).unwrap();
        assert_eq!(identity_of("opencode", &looking(&home, NOTHING, NOTHING)), AccountIdentity::None, "two keys and nothing says which is in use");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_keychain_source_reads_only_through_the_keychain_it_is_given() {
        let src: IdSource = serde_json::from_value(serde_json::json!({ "place": { "kind": "keychain", "service": "a-service" }, "take": { "kind": "key", "provider": "p" } })).unwrap();
        let home = folder();
        assert!(found_at(&src, &looking(&home, NOTHING, NOTHING)).is_empty());
        let kc = |s: &str| (s == "a-service").then(|| "fake-kc-4321".to_string());
        assert_eq!(found_at(&src, &looking(&home, NOTHING, &kc)), vec![AccountIdentity::of_key("p", "fake-kc-4321")]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn with_nothing_listening_the_command_does_it_itself() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let agent = airband_io::http::Client::new(Duration::from_secs(5));
        assert!(matches!(by_tower(airband_io::http::call::<wire::at::Accounts>(&agent, &format!("http://127.0.0.1:{port}"), &[], None, &Default::default(), None)), ByTower::NotRunning));
    }
}
