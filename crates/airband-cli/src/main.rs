//! airband: one channel for every coding agent on this machine.
//!
//!   airband run                  the tower itself, in the foreground
//!   airband install              add the Claude Code hooks and start the tower at login
//!   airband uninstall            remove both
//!   airband status               agents, and what is waiting to be heard
//!   airband relay <url>          reach radios through this relay (`airband relay off` to stop)
//!   airband model [<provider>]   what reads your agents' messages (listening and voice are chosen on the phone)
//!   airband pair [name]          print the link that sets up a radio (default name: phone)
//!   airband unpair <name>        cut a paired radio off
//!   airband radio [--link <pairing link>]
//!                              a radio in this terminal: transmissions are printed, typed lines are replies
//!                              (it makes no sound: voice is the phone's)
//!   airband feed [all|<group>|"<callsign>"] [--once]
//!                              the chat in this terminal: every message, cut to a line or two
//!   airband send "@shop2 ..."    say something; @group and @all work too
//!   airband launch <project> <task>
//!                              start a new agent in a project the tower knows (`airband status` lists them)
//!   airband hook <harness>       adapter run by a harness's own hooks (claude-code, codex)
//!   airband report ...           put any other agent on the radio from a script (see `airband report` with no arguments)
//!   airband --version            which version this is
#[cfg(unix)]
mod acp;
mod adapters;
mod catalog;
mod config;
#[cfg(unix)]
mod hold;
mod install;
mod providers;
mod terminal;
mod tower;
mod vault;
mod setup;

/// The value given after an option on the command line (`--name value`), when it is there.
pub(crate) fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The catalogue: the newest signed copy this computer has kept, for everything but the small commands a
    // harness runs many times a minute. Where he is about to choose from it, a newer one is fetched first.
    match args.first().map(String::as_str) {
        Some("hook" | "report" | "hold" | "catalog") => {}
        Some("setup" | "model") => {
            catalog::load();
            catalog::refresh();
        }
        _ => catalog::load(),
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
                install::pair(cfg, "phone")
            } else {
                println!("\nA phone is already paired. To pair another:  airband pair <a name for it>");
                Ok(())
            }
        }),
        Some("uninstall") => install::uninstall(),
        Some("status") => {
            install::status(&config::load());
            Ok(())
        },
        Some("pair") => install::pair(config::load(), args.get(1).map(String::as_str).unwrap_or("phone")),
        Some("unpair") => install::unpair(config::load(), args.get(1).map(String::as_str)),
        Some("quick") => install::quick(config::load(), &args[1..]),
        Some("accounts") => adapters::accounts_command(&config::load(), &args[1..]),
        Some("relay") => install::relay(config::load(), &args[1..]),
        Some("lan") => install::lan(config::load(), &args[1..]),
        Some("keys") => setup::keys(&args[1..]),
        Some("catalog") => catalog::command(&args[1..]),
        Some("setup") => setup::setup(config::load(), !args.iter().any(|a| a == "--no-start")),
        Some("model") => install::model(config::load(), &args[1..]),
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
