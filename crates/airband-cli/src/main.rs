//! airband: one channel for every coding agent on this machine.
//!
//!   airband run                  the tower itself, in the foreground
//!   airband setup [--no-start]   choose the coding tools and what reads, then install
//!   airband start                install, and show the code a phone scans when none is paired
//!   airband install              add the hooks of the coding tools he uses and start the tower at login
//!   airband uninstall            remove both
//!   airband status               agents, and what is waiting to be heard
//!   airband relay <url>          reach radios through this relay (`airband relay off` to stop)
//!   airband lan [on | off]       whether a phone on the same network may reach this computer straight
//!   airband model [<provider>]   what reads your agents' messages (listening and voice are chosen on the phone)
//!   airband pair [name]          print the link that sets up a radio (default name: phone)
//!   airband unpair <name>        cut a paired radio off
//!   airband quick [add | remove] the quick messages a phone offers
//!   airband accounts             the logins of a coding tool, and which one its agents run under
//!   airband radio [--link <pairing link>]
//!                              a radio in this terminal: transmissions are printed, typed lines are replies
//!                              (it makes no sound: voice is the phone's)
//!   airband feed [--team <team> | --agent "<callsign>"] [--once]
//!                              the chat in this terminal: every message, cut to a line or two
//!   airband send "@shop2 ..."    say something; @group and @all work too
//!   airband launch <team> [--tool <id>] [--model <name>] [--effort <name>] [task…]
//!                              start a new agent in one of his teams that has an agent here (`airband status` lists them)
//!   airband keys [add <provider> | remove <name>]
//!                              the keys he gave, one per provider
//!   airband hook <tool>          run by a coding tool's own hooks, as its row names them
//!   airband report ...           put any other agent on the radio from a script (see `airband report` with no arguments)
//!   airband hold ...             run by the tower: drive a coding tool's program it holds
//!   airband catalog ...          the catalogue built into this program: export, sign, check, stamp
//!   airband triage | progress    try what reads messages by hand, on standard input
//!   airband --version            which version this is
#![warn(clippy::let_underscore_must_use)]
// test code may touch files and build JSON by hand to stand in for the outside
#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_macros, clippy::let_underscore_must_use))]
#[cfg(unix)]
mod acp;
mod adapters;
mod catalog;
mod config;
#[cfg(unix)]
mod hold;
mod commands;
mod install;
mod lock;
mod platform;
mod providers;
mod terminal;
mod tower;
mod vault;
mod setup;

/// The value given after an option on the command line (`--name value`), when it is there.
pub(crate) fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// A command's words, read by [`words`].
pub(crate) struct Words {
    /// Each flag it was given, without its dashes, with the value after it.
    pub(crate) flags: std::collections::BTreeMap<String, String>,
    /// Every other word, in order.
    pub(crate) rest: Vec<String>,
    /// Everything after a lone `--`, as it is.
    pub(crate) after: Vec<String>,
}

/// Read a command's words: each flag of `takes` (as written, `--model`) with the value after it, everything
/// after a lone `--` as it is, and every other word in order. A flag that is not one of `takes`, or one with no
/// value, is refused: the one way a command's words are read.
pub(crate) fn words(command: &str, args: &[String], takes: &[&str]) -> Result<Words, String> {
    let mut read = Words { flags: Default::default(), rest: Vec::new(), after: Vec::new() };
    let mut it = args.iter();
    while let Some(word) = it.next() {
        match word.as_str() {
            "--" => read.after = it.by_ref().cloned().collect(),
            flag if takes.contains(&flag) => {
                read.flags.insert(flag[2..].to_string(), it.next().cloned().ok_or_else(|| format!("airband {command}: {flag} needs a value"))?);
            }
            flag if flag.starts_with("--") => return Err(format!("airband {command} does not take \"{flag}\"")),
            plain => read.rest.push(plain.to_string()),
        }
    }
    Ok(read)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The catalogue: the newest signed copy this computer has kept, for everything but the small commands a
    // harness runs many times a minute. Where he is about to choose from it, a newer one is fetched first.
    match args.first().map(String::as_str) {
        Some("hook" | "report" | "hold" | "catalog") => {}
        Some("setup" | "model") => {
            catalog::load();
            // what the copy had to say (one refused, rows this build cannot use) is said before he chooses
            catalog::refresh().1.iter().for_each(|line| eprintln!("{line}"));
        }
        // the tower says it in its log, with what fetching a newer copy had to say
        Some("run") => catalog::load(),
        _ => {
            catalog::load();
            catalog::said().iter().for_each(|line| eprintln!("{line}"));
        }
    }
    let result = match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            println!("airband {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        },
        Some("hook") => adapters::hook(args.get(1).map(String::as_str)),
        Some("report") => adapters::report::run(&args[1..]),
        #[cfg(unix)]
        Some("hold") => hold::run(&args[1..]),
        None | Some("run") => tower::run(config::load()),
        Some("install") => install::install(),
        // what a new computer runs: start the tower, keep it running, and show the code a phone scans. What
        // reads, speaks and listens is chosen on the phone.
        Some("start") => install::install().and_then(|()| {
            let cfg = config::load();
            if cfg.pairings.is_empty() {
                println!("\nOpen airband on your phone and scan this.");
                commands::pair(cfg, "phone")
            } else {
                println!("\nA phone is already paired. To pair another:  airband pair <a name for it>");
                Ok(())
            }
        }),
        Some("uninstall") => install::uninstall(),
        Some("status") => {
            commands::status(&config::load());
            Ok(())
        },
        Some("pair") => commands::pair(config::load(), args.get(1).map(String::as_str).unwrap_or("phone")),
        Some("unpair") => commands::unpair(config::load(), args.get(1).map(String::as_str)),
        Some("quick") => commands::quick(config::load(), &args[1..]),
        Some("accounts") => adapters::accounts_command(&config::load(), &args[1..]),
        Some("relay") => commands::relay(config::load(), &args[1..]),
        Some("lan") => commands::lan(config::load(), &args[1..]),
        Some("keys") => setup::keys(&args[1..]),
        Some("catalog") => catalog::command(&args[1..]),
        Some("setup") => setup::setup(config::load(), !args.iter().any(|a| a == "--no-start")),
        Some("model") => commands::model(config::load(), &args[1..]),
        Some("radio") => terminal::run(&config::load(), &args[1..]),
        Some("launch") => terminal::launch(&config::load(), &args[1..]),
        Some("triage") => tower::triage(config::load()),
        Some("progress") => tower::progress(config::load()),
        Some("feed") => terminal::feed(&config::load(), &args[1..]),
        Some("send") => terminal::send(&config::load(), &args[1..]),
        Some(other) => Err(format!("unknown command \"{other}\": start, setup, install, uninstall, status, keys, relay, lan, model, pair, unpair, quick, accounts, catalog, radio, feed, send, launch, triage, progress, hook, report, run")),
    };
    if let Err(e) = result {
        eprintln!("airband: {e}");
        std::process::exit(1);
    }
}
