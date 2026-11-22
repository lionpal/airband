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

/// What came of asking the tower on this computer.
#[derive(Debug)]
enum ByTower {
    /// Nothing is listening: no tower runs here, so the command does the work itself.
    NotRunning,
    /// The tower took it: its answer, or why there is none. A tower that took the request and did not answer in
    /// time may still be doing it, so the work is never done a second time here.
    Said(Result<Accounts, String>),
}

fn by_tower(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> ByTower {
    let mut res = match sent {
        Ok(res) => res,
        Err(ureq::Error::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => return ByTower::NotRunning,
        Err(ureq::Error::ConnectionFailed) => return ByTower::NotRunning,
        Err(ureq::Error::Timeout(_)) => return ByTower::Said(Err("airband on this computer did not answer in time. It may still be doing it: look again with: airband accounts".into())),
        Err(e) => return ByTower::Said(Err(format!("airband on this computer could not be asked: {e}"))),
    };
    let status = res.status().as_u16();
    let Ok(text) = res.body_mut().read_to_string() else { return ByTower::Said(Err("airband's answer could not be read.".into())) };
    #[derive(serde::Deserialize)]
    struct Refusal {
        error: String,
    }
    ByTower::Said(match (status, serde_json::from_str::<Refusal>(&text)) {
        (200, Ok(r)) => Err(r.error),
        (200, Err(_)) => serde_json::from_str::<Accounts>(&text).map_err(|_| "airband's answer could not be read.".to_string()),
        (n, _) => Err(format!("airband refused ({n}).")),
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
            println!("{} {:<34} {}", if acct.live { "▶" } else { " " }, acct.label, acct.problem.clone().unwrap_or_else(|| limits.join(", ")));
        }
        if let Some(note) = &a.note {
            println!("{note}");
        }
    };
    match args.first().map(String::as_str) {
        None => match by_tower(here.get("/v1/radio/accounts?fresh=1").call()) {
            ByTower::Said(now) => show(&now?),
            ByTower::NotRunning => show(&accounts()),
        },
        Some("use") => {
            let id = args.get(1).ok_or("Which account? airband accounts use <its email or number>")?;
            match by_tower(here.post("/v1/radio/accounts/use").send_json(serde_json::json!({ "harness": harness, "id": id }))) {
                ByTower::Said(now) => show(&now?),
                ByTower::NotRunning => {
                    let _ = use_account(harness, id)?.join();
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
        let agent = airband_io::http::agent(Duration::from_millis(500));
        match by_tower(agent.post(format!("http://127.0.0.1:{port}/v1/radio/accounts/use")).send_json(serde_json::json!({}))) {
            ByTower::Said(Err(why)) => assert!(why.contains("did not answer in time"), "{why}"),
            other => panic!("a slow tower must not make the switch run here as well: {other:?}"),
        }
    }

    #[test]
    fn with_nothing_listening_the_command_does_it_itself() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let agent = airband_io::http::agent(Duration::from_secs(5));
        assert!(matches!(by_tower(agent.get(format!("http://127.0.0.1:{port}/v1/radio/accounts")).call()), ByTower::NotRunning));
    }
}
