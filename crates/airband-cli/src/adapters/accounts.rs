//! A coding tool's accounts: listing them for every tool that has them, switching, and `airband accounts`.
use super::*;

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

/// `airband accounts`: list them. `save`, `add`, `use <account>`, `forget <account>`. With a tower running,
/// listing and switching go through it, so there is one keeper of the saved logins at a time.
pub(crate) fn accounts_command(cfg: &config::Config, args: &[String]) -> Result<(), String> {
    let harness = HARNESSES.iter().find(|h| h.accounts().is_some()).map(|h| h.name()).ok_or("None of your coding tools has accounts to switch between here.")?;
    let tower = |method: &str, path: &str, body: Option<Value>| -> Option<Result<Accounts, String>> {
        let here = super::TowerHere::here(cfg, Duration::from_secs(60));
        let mut res = match (method, body) {
            ("POST", Some(b)) => here.post(path).send_json(b),
            _ => here.get(path).call(),
        }
        .ok()?;
        let text = res.body_mut().read_to_string().ok()?;
        let answer: Value = serde_json::from_str(&text).unwrap_or_default();
        Some(match (res.status().as_u16(), answer["error"].as_str()) {
            (200, Some(why)) => Err(why.to_string()),
            (200, None) => serde_json::from_value(answer).map_err(|e| e.to_string()),
            _ => Err("airband refused.".into()),
        })
    };
    let show = |a: &Accounts| {
        for acct in &a.accounts {
            let limits: Vec<String> = acct.limits.iter().map(|l| format!("{} {:.0}%", l.name, l.used * 100.0)).collect();
            println!("{} {:<34} {}", if acct.live { "▶" } else { " " }, acct.label, acct.problem.clone().unwrap_or_else(|| limits.join(", ")));
        }
        if let Some(note) = &a.note {
            println!("{note}");
        }
    };
    match args.first().map(String::as_str) {
        None => show(&tower("GET", "/v1/radio/accounts?fresh=1", None).unwrap_or_else(|| Ok(accounts()))?),
        Some("use") => {
            let id = args.get(1).ok_or("Which account? airband accounts use <part of its email>")?;
            match tower("POST", "/v1/radio/accounts/use", Some(serde_json::json!({ "harness": harness, "id": id }))) {
                Some(now) => show(&now?),
                None => {
                    let _ = use_account(harness, id)?.join();
                    show(&accounts());
                }
            }
        }
        Some("save") => println!("Saved {}.", (book(harness)?.save)()?),
        Some("add") => println!("Saved {}. Every account stays signed in; switch with: airband accounts use <account>", (book(harness)?.add)()?),
        Some("forget") => println!("Forgot {}.", (book(harness)?.forget)(args.get(1).ok_or("Which account? airband accounts forget <part of its email>")?)?),
        Some(other) => return Err(format!("unknown: accounts {other}. Try: airband accounts, accounts save, accounts add, accounts use <account>, accounts forget <account>")),
    }
    Ok(())
}
