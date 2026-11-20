//! airband: one channel for every coding agent on this machine.
//!
//!   airband run                  the tower itself, in the foreground
//!   airband install              add the Claude Code hooks and start the tower at login
//!   airband uninstall            remove both
//!   airband status               agents, and what is waiting to be heard
//!   airband relay <url>          reach radios through this relay (`airband relay off` to stop)
//!   airband voice [device|gemini|azure …]   what speaks on a call (outbound)
//!   airband ears [device|azure …]          what listens on a call (inbound), set separately
//!   airband pair [name]          print the link that sets up a radio (default name: phone)
//!   airband unpair <name>        cut a paired radio off
//!   airband radio [--quiet] [--link <pairing link>]
//!                              a radio in this terminal: transmissions are spoken, typed lines are replies
//!   airband feed [all|<group>|"<callsign>"] [--once]
//!                              the chat in this terminal: every message, cut to a line or two
//!   airband send "@shop2 ..."    say something; @group and @all work too
//!   airband launch <project> <task>
//!                              start a new agent in a project the tower knows (`airband status` lists them)
//!   airband hook <harness>       adapter run by a harness's own hooks (claude-code)
//!   airband report ...           put any other agent on the radio from a script (see `airband report` with no arguments)
//!   airband --version            which version this is
mod adapters;
mod config;
#[cfg(unix)]
mod hold;
mod install;
mod terminal;
mod tower;
mod vault;
mod setup;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => Ok(println!("airband {}", env!("CARGO_PKG_VERSION"))),
        Some("hook") => adapters::hook(args.get(1).map(String::as_str).unwrap_or("claude-code")),
        Some("report") => adapters::report::run(&args[1..]),
        #[cfg(unix)]
        Some("hold") => hold::run(&args[1..]),
        None | Some("run") => tower::run(config::load()),
        Some("install") => install::install(),
        Some("uninstall") => install::uninstall(),
        Some("status") => Ok(install::status(&config::load())),
        Some("pair") => install::pair(config::load(), args.get(1).map(String::as_str).unwrap_or("phone")),
        Some("unpair") => install::unpair(config::load(), args.get(1).map(String::as_str)),
        Some("quick") => install::quick(config::load(), &args[1..]),
        Some("accounts") => adapters::accounts_command(&config::load(), &args[1..]),
        Some("relay") => install::relay(config::load(), &args[1..]),
        Some("lan") => install::lan(config::load(), &args[1..]),
        Some("keys") => setup::keys(&args[1..]),
        Some("setup") => setup::setup(config::load(), !args.iter().any(|a| a == "--no-start")),
        Some("model") => install::model(config::load(), &args[1..]),
        Some("voice") => install::voice(config::load(), &args[1..]),
        Some("ears") => install::ears(config::load(), &args[1..]),
        Some("radio") => terminal::run(&config::load(), &args[1..]),
        Some("launch") => terminal::send(&config::load(), &[vec!["/launch".to_string()], args[1..].to_vec()].concat()),
        Some("triage") => tower::triage(config::load()),
        Some("progress") => tower::progress(config::load()),
        Some("feed") => terminal::feed(&config::load(), &args[1..]),
        Some("send") => terminal::send(&config::load(), &args[1..]),
        Some(other) => Err(format!("unknown command \"{other}\": setup, install, uninstall, status, keys, relay, lan, model, voice, ears, pair, unpair, quick, accounts, radio, feed, send, launch, hook, report, run")),
    };
    if let Err(e) = result {
        eprintln!("airband: {e}");
        std::process::exit(1);
    }
}
