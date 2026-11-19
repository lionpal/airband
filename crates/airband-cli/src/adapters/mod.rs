//! Adapters: each turns one harness's own notifications into `airband.agent/1` events and posts them to the tower
//! on this machine. An adapter never holds its harness up: any failure is silent and it gives up after a second
//! and a half.
//!
//! To put another harness on the radio:
//! - with no code, call `airband report` from whatever the harness can run (see `report.rs`);
//! - or add a module here and a line in [`ADAPTERS`]: a function from the harness's notification to events,
//!   and, if the harness has to be told to call `airband hook`, how to set that up and take it out again.
//!   `claude_code.rs` is the model.
pub mod claude_accounts;
pub mod claude_code;
pub mod report;

use crate::config;
use airband_core::agent::{AgentEvent, PermissionAsk, PermissionState};
use airband_core::radio::{Account, Accounts};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

/// One harness the tower can hear from.
struct Adapter {
    /// What `airband hook <name>` is called with.
    name: &'static str,
    /// A notification of the harness's, as JSON, to the events it means (none, when it means nothing to the tower).
    events: fn(&Value) -> Vec<AgentEvent>,
    /// Set the harness up on this machine to run `airband hook <name>` (given the installed binary); returns
    /// what to tell him. Nothing for a harness that is wired up some other way.
    install: Option<fn(&std::path::Path) -> Result<String, String>>,
    uninstall: Option<fn() -> Result<(), String>>,
    /// For a harness that can let something else answer its permission prompts: turn its notification into the
    /// request to put to him (or nothing, when the notification is not one), and turn his answer into what the
    /// harness wants printed on standard output.
    permission: Option<(fn(&Value) -> Option<PermissionAsk>, fn(bool, &str) -> String)>,
    /// For a harness whose sessions can be moved between logins while they run.
    accounts: Option<AccountBook>,
    /// A look at one of its agents, when the harness keeps this somewhere the tower can read cheaply. Asked
    /// every couple of seconds while a radio is looking, so it must be quick.
    glance: Option<fn(&airband_core::agent::Agent) -> Glance>,
    /// The files and commands of an agent's latest steps, newest first, as the harness wrote them. For working
    /// out which project it is working on; asked now and then, not on every look.
    touched: Option<fn(&airband_core::agent::Agent) -> Vec<String>>,
    /// What he has said to an agent at its own keyboard since `from`, a place in the harness's record that an
    /// earlier call returned (0 the first time). Returns his words, oldest first, and the place to ask from next
    /// time. What he typed himself comes back with `his` set; what the tower passed on comes back without, when
    /// the agent has taken it in.
    told: Option<fn(&airband_core::agent::Agent, u64) -> (Vec<Told>, u64)>,
}

/// Something he said to an agent directly, at its keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Told {
    /// The harness's own id for it: the same words are never taken twice.
    pub id: String,
    /// When he said it, in milliseconds since 1970.
    pub at: u64,
    /// His words, as he typed them.
    pub text: String,
    /// False for something the tower passed on to the agent, seen here at the moment the agent took it in:
    /// that is how the tower knows a message of his has been read.
    pub his: bool,
}

/// What a look at an agent can tell. Anything the harness cannot say is left out.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Glance {
    /// What it is doing right now, in a few words.
    pub doing: Option<String>,
    /// The model it runs on, as a person would say it: "Opus 4.7".
    pub model: Option<String>,
    /// How hard it is set to think: "high".
    pub effort: Option<String>,
}

/// How one harness's accounts are listed, switched and kept. Every function answers in words for him.
struct AccountBook {
    /// Every account with how much of it is used, and anything to say about the list as a whole.
    list: fn() -> (Vec<Account>, Option<String>),
    /// Make this account the one in use. The swap is done on return; the thread finishes the tidying.
    switch: fn(&str) -> Result<std::thread::JoinHandle<()>, String>,
    /// Keep the login in use now. Returns what it is called.
    save: fn() -> Result<String, String>,
    /// Sign in to another account, at the computer, and keep it.
    add: fn() -> Result<String, String>,
    forget: fn(&str) -> Result<String, String>,
}

/// Every harness `airband hook <name>` understands. A new one is a module and a line here.
const ADAPTERS: &[Adapter] = &[Adapter {
    name: "claude-code",
    events: claude_code::events,
    install: Some(claude_code::install),
    uninstall: Some(claude_code::uninstall),
    permission: Some((claude_code::permission, claude_code::permission_answer)),
    accounts: Some(AccountBook { list: claude_accounts::list, switch: claude_accounts::switch, save: claude_accounts::save, add: claude_accounts::add, forget: claude_accounts::forget }),
    glance: Some(claude_code::glance),
    touched: Some(claude_code::touched),
    told: Some(claude_code::told),
}];

/// `airband install`: set up every harness that can be. One line to say for each.
pub fn install(exe: &std::path::Path) -> Result<Vec<String>, String> {
    ADAPTERS.iter().filter_map(|a| a.install).map(|install| install(exe)).collect()
}
pub fn uninstall() -> Result<(), String> {
    ADAPTERS.iter().filter_map(|a| a.uninstall).try_for_each(|uninstall| uninstall())
}

/// What this agent is doing right now and what it runs on, as far as its harness's adapter can tell.
pub fn glance(agent: &airband_core::agent::Agent) -> Glance {
    ADAPTERS.iter().find(|a| a.name == agent.harness).and_then(|a| a.glance).map(|look| look(agent)).unwrap_or_default()
}

/// The files and commands of this agent's latest steps, if its harness's adapter can tell.
pub fn touched(agent: &airband_core::agent::Agent) -> Vec<String> {
    ADAPTERS.iter().find(|a| a.name == agent.harness).and_then(|a| a.touched).map(|ask| ask(agent)).unwrap_or_default()
}

/// What he has typed to this agent at its keyboard since `from`, if its harness's adapter can tell.
pub fn told(agent: &airband_core::agent::Agent, from: u64) -> (Vec<Told>, u64) {
    ADAPTERS.iter().find(|a| a.name == agent.harness).and_then(|a| a.told).map(|ask| ask(agent, from)).unwrap_or((Vec::new(), from))
}

/// Every account of every harness that has them.
pub fn accounts() -> Accounts {
    let (mut all, mut notes) = (Vec::new(), Vec::new());
    for book in ADAPTERS.iter().filter_map(|a| a.accounts.as_ref()) {
        let (accounts, note) = (book.list)();
        all.extend(accounts);
        notes.extend(note);
    }
    let checked_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    Accounts { schema: airband_core::radio::SCHEMA.into(), accounts: all, note: (!notes.is_empty()).then(|| notes.join(" ")), checked_at }
}
fn book(harness: &str) -> Result<&'static AccountBook, String> {
    ADAPTERS.iter().find(|a| a.name == harness).and_then(|a| a.accounts.as_ref()).ok_or_else(|| format!("{harness} has no accounts to switch between."))
}
pub fn use_account(harness: &str, id: &str) -> Result<std::thread::JoinHandle<()>, String> {
    (book(harness)?.switch)(id)
}

/// `airband accounts`: list them. `save`, `add`, `use <account>`, `forget <account>`. With a tower running,
/// listing and switching go through it, so there is one keeper of the saved logins at a time.
pub fn accounts_command(cfg: &config::Config, args: &[String]) -> Result<(), String> {
    let harness = ADAPTERS.iter().find(|a| a.accounts.is_some()).map(|a| a.name).ok_or("No harness here has accounts.")?;
    let tower = |method: &str, path: &str, body: Option<Value>| -> Option<Result<Accounts, String>> {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).http_status_as_error(false).build().into();
        let url = format!("http://127.0.0.1:{}{path}", cfg.port);
        let auth = format!("Bearer {}", cfg.token);
        let mut res = match (method, body) {
            ("POST", Some(b)) => agent.post(&url).header("authorization", &auth).header("content-type", "application/json").send(b.to_string()),
            _ => agent.get(&url).header("authorization", &auth).call(),
        }
        .ok()?;
        let text = res.body_mut().read_to_string().ok()?;
        // a tower from before accounts existed: do it here instead
        if res.status() == 404 {
            return None;
        }
        let answer: Value = serde_json::from_str(&text).unwrap_or_default();
        Some(match (res.status().as_u16(), answer["error"].as_str()) {
            (200, Some(why)) => Err(why.to_string()),
            (200, None) => serde_json::from_value(answer).map_err(|e| e.to_string()),
            _ => Err("The tower refused.".into()),
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

pub fn names() -> Vec<&'static str> {
    ADAPTERS.iter().map(|a| a.name).collect()
}

pub(crate) fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// `airband hook <harness>`: read the harness's notification from standard input, translate it, post it.
pub fn hook(harness: &str) -> Result<(), String> {
    let Some(adapter) = ADAPTERS.iter().find(|a| a.name == harness) else {
        // said aloud, because this is a mistake in someone's setup, not a failure to hide from a running agent
        return Err(format!("no adapter for \"{harness}\"; there is one for: {}. Any other harness can use `airband report`.", names().join(", ")));
    };
    // the tower's own model calls must not report to the tower
    if env("AIRBAND_INTERNAL").is_some() {
        return Ok(());
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    // the harness is about to ask for permission at the keyboard: ask him on the radio first
    if let Some((ask, answer)) = adapter.permission {
        if let Some(request) = serde_json::from_str(&input).ok().and_then(|notification| ask(&notification)) {
            if let Some((allow, reason)) = ask_him(&request) {
                println!("{}", answer(allow, &reason));
            }
            // no answer in time, or nobody there: print nothing, and the harness asks in its own way
            return Ok(());
        }
    }
    for event in serde_json::from_str(&input).map(|notification| (adapter.events)(&notification)).unwrap_or_default() {
        post(&event);
    }
    Ok(())
}

/// Put a permission request to him through the tower, and wait for his answer. Nothing when there is none to be
/// had: no tower, no radio there to ask, or he did not answer in the time the tower allows. Then the harness
/// asks at the keyboard as it always has. Waiting never turns into a yes.
pub(crate) fn ask_him(request: &PermissionAsk) -> Option<(bool, String)> {
    let cfg: config::Config = serde_json::from_str(&std::fs::read_to_string(config::dir().join("config.json")).ok()?).ok()?;
    let base = format!("http://127.0.0.1:{}/v1/agent/permission", cfg.port);
    let auth = format!("Bearer {}", cfg.token);
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(40))).build().into();
    let asked: PermissionState = agent.post(&base).header("authorization", &auth).send_json(request).ok()?.body_mut().read_json().ok()?;
    if asked.state != "pending" {
        return None;
    }
    let until = std::time::Instant::now() + Duration::from_secs(asked.wait_seconds.max(5));
    while std::time::Instant::now() < until {
        let left = until.saturating_duration_since(std::time::Instant::now()).as_secs().clamp(1, 20);
        let now: PermissionState = match agent.get(format!("{base}/{}?wait={left}", asked.id)).header("authorization", &auth).call() {
            Ok(mut res) => res.body_mut().read_json().ok()?,
            // the tower went away mid-wait: stop waiting and let the harness ask
            Err(_) => return None,
        };
        match now.state.as_str() {
            "allow" => return Some((true, String::new())),
            "deny" => return Some((false, now.message)),
            "pending" => {}
            _ => return None,
        }
    }
    // he did not answer: take the question back, so the feed does not offer an answer that no longer does anything
    let _ = agent.delete(format!("{base}/{}", asked.id)).header("authorization", &auth).call();
    None
}

/// Post one event to the tower on this machine. Silent on failure.
pub fn post(event: &AgentEvent) -> bool {
    let Ok(text) = std::fs::read_to_string(config::dir().join("config.json")) else { return false };
    let Ok(cfg) = serde_json::from_str::<config::Config>(&text) else { return false };
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_millis(1500))).build().into();
    agent.post(format!("http://127.0.0.1:{}/v1/agent/events", cfg.port)).header("authorization", format!("Bearer {}", cfg.token)).send_json(event).is_ok()
}
