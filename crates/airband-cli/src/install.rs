//! `airband install` / `uninstall` / `pair` / `status`.
use crate::config::{self, Config};
use serde_json::{Value, json};
use airband_core::pairing::Pairing;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const LABEL: &str = "dev.airband.tower";

fn installed_exe() -> PathBuf {
    config::dir().join("bin").join(if cfg!(windows) { "airband.exe" } else { "airband" })
}
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    config::home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

/// The lasting path to this binary when Homebrew installed it, given where the binary really is. Homebrew keeps
/// each version in a folder of its own and points `<prefix>/opt/airband` at the current one, so that path still
/// works after `brew upgrade`. Homebrew is asked where that is (`brew --prefix airband`, which prints the path
/// alone); the caller keeps it only when it leads to this very binary.
fn homebrew_exe(real: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("brew").args(["--prefix", "airband"]).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let opt = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim_end_matches('\n'));
    Some(in_opt(&opt, real))
}
/// This binary's place under Homebrew's lasting folder for the formula.
fn in_opt(opt: &Path, real: &Path) -> PathBuf {
    opt.join("bin").join(real.file_name().unwrap_or_default())
}

pub(crate) fn install() -> Result<(), String> {
    config::load();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let real = fs::canonicalize(&me).unwrap_or_else(|_| me.clone());
    // Homebrew's copy is used where it is, so `brew upgrade` changes what the hooks and the tower run
    let brewed = homebrew_exe(&real).filter(|lasting| fs::canonicalize(lasting).is_ok_and(|to| to == real));
    let exe = match brewed {
        Some(lasting) => {
            println!("Using Homebrew's copy at {}. After `brew upgrade airband`, run `airband install` again to restart airband.", lasting.display());
            lasting
        }
        None => {
            // a copy of this binary that a rebuild cannot pull out from under running sessions
            let exe = installed_exe();
            fs::create_dir_all(exe.parent().unwrap()).map_err(|e| e.to_string())?;
            if me != exe {
                let tmp = exe.with_extension("new");
                fs::copy(&me, &tmp).map_err(|e| format!("cannot copy the binary to {}: {e}", exe.display()))?;
                fs::rename(&tmp, &exe).map_err(|e| e.to_string())?;
            }
            exe
        }
    };
    // each harness that needs something set up on this machine to report to the tower gets it
    for line in crate::adapters::install(&exe)? {
        println!("{line}");
    }
    start_at_login(&exe)
}

#[cfg(target_os = "macos")]
fn start_at_login(exe: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    let log = config::dir().join("airband.log");
    let path = std::env::var("PATH").unwrap_or_else(|_| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".into());
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>run</string></array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#,
        exe = exe.display(),
        log = log.display()
    );
    fs::create_dir_all(plist_path().parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(plist_path(), plist).map_err(|e| e.to_string())?;
    let uid = unsafe { libc::getuid() };
    let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{LABEL}")]).output();
    // stopping the old one takes a moment, and starting before it has gone fails ("Input/output error"): this left
    // the tower down for some minutes once. Try again until it takes.
    let start = || Command::new("launchctl").args(["bootstrap", &format!("gui/{uid}"), &plist_path().display().to_string()]).output().map_err(|e| e.to_string());
    let mut out = start()?;
    for _ in 0..8 {
        if out.status.success() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        out = start()?;
    }
    if !out.status.success() {
        return Err(format!("airband could not be started. launchctl said: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    println!("airband is running, and starts again when you log in. Its log: {}", log.display());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn start_at_login(exe: &std::path::Path) -> Result<(), String> {
    println!("Starting at login is only set up on macOS so far. Start airband yourself:  \"{}\" run", exe.display());
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    crate::adapters::uninstall()?;
    #[cfg(target_os = "macos")]
    {
        let uid = unsafe { libc::getuid() };
        let _ = std::process::Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{LABEL}")]).output();
        let _ = fs::remove_file(plist_path());
    }
    println!("airband is stopped and will not start at login. Your coding tools no longer report to it.");
    Ok(())
}

/// `airband relay <url> [--admission <key>]` / `airband relay off`
pub(crate) fn relay(mut cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => println!("{}", cfg.relay.as_deref().map(|r| format!("Your phone reaches this computer through {r}.")).unwrap_or("No relay: your phone reaches this computer at its own addresses, on the same network or over a private network.".into())),
        Some("off") => {
            cfg.relay = None;
            cfg.relay_admission = None;
            config::save(&cfg)?;
            println!("No relay from now on. A phone paired through it must be paired again:  airband pair");
        }
        Some("on") => {
            cfg.relay = Some(config::RELAY.into());
            cfg.relay_admission = None;
            config::save(&cfg)?;
            println!("Relay set to {}. A phone paired without it must be paired again:  airband pair", config::RELAY);
        }
        Some(url) => {
            if airband_core::links::web(url).is_none() {
                return Err(format!("a relay address is a web address, like {}", config::RELAY));
            }
            cfg.relay = Some(url.trim_end_matches('/').to_string());
            cfg.relay_admission = crate::option(args, "--admission");
            config::save(&cfg)?;
            println!("Relay set to {url}. If airband is running it picks this up within ten seconds. Next: airband pair");
        }
    }
    Ok(())
}

/// `airband model`: show or change what reads. Everything it knows about providers comes from the catalogue
/// (`providers::offering`). What listens and what speaks are chosen on the phone, not here.
///
///   airband model                      what is set, and what can be
///   airband model none                 nothing reads
///   airband model <provider> [<offer>] [--key-env VARIABLE] [--<setting> <value> …]
///   airband model test                 one real call to what is set
///
/// A key is never given on the command line, where the shell and other programs can see it: it comes from an
/// environment variable, or is one already kept for that provider (`airband keys`).
/// What reads each agent's message and decides what is worth showing, and who his words are for.
pub(crate) fn model(mut cfg: Config, args: &[String]) -> Result<(), String> {
    use crate::providers::{self, Use};
    let (job, cmd, what, without) = (providers::Job::Summary, "model", "Model", "You get every message in full, and with several agents you are asked who yours is for.");
    let offered = || providers::offering(job).iter().map(|p| p.id.clone()).collect::<Vec<_>>().join(", ");
    let chosen = match args.first().map(String::as_str) {
        None => {
            let now = cfg.summarizer.clone();
            println!("{what}: {}.{}", now.describe(job), if now.is_none() { format!(" {without}") } else { String::new() });
            println!("Change it:  airband {cmd} <provider> [<offer>] [--key-env VARIABLE] [--setting value]  |  airband {cmd} none  |  airband {cmd} test");
            for (p, o) in providers::in_order(job) {
                let asks: Vec<String> = o.asks.iter().map(|a| if a.usual.is_empty() { format!("--{}", a.field) } else { format!("--{} ({})", a.field, a.usual) }).collect();
                println!("  {:<11} {:<15} {}{}", p.id, o.id, o.label, if asks.is_empty() { String::new() } else { format!(": {}", asks.join(", ")) });
            }
            return Ok(());
        }
        // one real call to what is set
        Some("test") => {
            println!("{}", crate::tower::try_model(&cfg)?);
            return Ok(());
        }
        Some("none") => Use::default(),
        Some(id) => {
            let p = providers::offering(job).into_iter().find(|p| p.id == id).ok_or_else(|| format!("nothing called \"{id}\" does this: {} or none", offered()))?;
            // which of its offers: the one named after it, or the only one it has for this job. With several
            // and none named, he is asked which.
            let ways: Vec<_> = providers::in_order(job).into_iter().filter(|(q, _)| q.id == p.id).map(|(_, o)| o).collect();
            let named = args.get(1).filter(|a| !a.starts_with("--"));
            let o = match (named, ways.as_slice()) {
                (Some(name), _) => ways.iter().find(|o| o.id == **name).ok_or_else(|| format!("{} has nothing called \"{name}\" for this: {}", p.name, ways.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", ")))?,
                (None, [only]) => only,
                (None, _) => return Err(format!("{} does this more than one way. Say which:  airband {cmd} {id} <{}>", p.name, ways.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(" | "))),
            };
            let mut u = Use::of(&p.id, &o.id);
            if p.key {
                u.key = match crate::option(args, "--key-env") {
                    Some(var) => std::env::var(&var).ok().filter(|k| !k.is_empty()).ok_or_else(|| format!("{var} is not set"))?,
                    // one he already gave for this provider will do
                    None => cfg.key_of(&p.id).ok_or_else(|| format!("{} needs a key: --key-env <VARIABLE>, or give one first with  airband keys add {id}", p.name))?,
                };
                if cfg.key_of(&p.id).as_ref() != Some(&u.key) {
                    cfg.keep_key(&p.id, u.key.clone());
                }
            }
            for a in &o.asks {
                if let Some(said) = crate::option(args, &format!("--{}", a.field)) {
                    u.set.insert(a.field.clone(), json!(said));
                }
            }
            u
        }
    };
    cfg.summarizer = chosen;
    config::save(&cfg)?;
    println!("Saved. If airband is running it uses this from its next start: airband install");
    Ok(())
}

/// `airband lan [on | off]`: whether a paired phone on the same network may reach this computer straight.
pub(crate) fn lan(mut cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {
            println!("{}", if cfg.lan { "On: a paired phone on the same network reaches this computer directly, encrypted. Away from that network it goes through the relay." } else { "Off: a paired phone always goes through the relay." });
            println!("Change it:  airband lan on  |  airband lan off");
            return Ok(());
        }
        Some("on") => cfg.lan = true,
        Some("off") => cfg.lan = false,
        Some(other) => return Err(format!("airband lan takes on or off, not \"{other}\"")),
    }
    config::save(&cfg)?;
    println!("Saved. If airband is running it uses this from its next start: airband install");
    Ok(())
}

pub(crate) fn pair(mut cfg: Config, name: &str) -> Result<(), String> {
    // a slip like `airband pair --help` must not make a pairing called "--help"
    if name.starts_with('-') {
        return Err(format!("\"{name}\" is not a name for a phone. Use:  airband pair [name]"));
    }
    // one secret for a radio, whichever way it reaches the tower
    let secret = match cfg.pairings.iter().find(|p| p.name == name) {
        Some(p) => p.secret.clone(),
        None => {
            let secret = airband_core::relay::new_secret();
            cfg.pairings.push(config::PairedRadio { name: name.into(), secret: secret.clone() });
            config::save(&cfg)?;
            secret
        }
    };
    let pairing = match &cfg.relay {
        Some(relay) => {
            println!("Pairing \"{name}\", through the relay at {relay}. Works from any network.");
            Pairing::Relay { relay: relay.clone(), secret }
        }
        None => {
            // where a radio can find this computer: its private network, and its own when that is allowed
            let at: Vec<String> = config::overlay_addrs().into_iter().chain(config::lan_addrs().into_iter().filter(|_| cfg.lan)).map(|a| format!("http://{a}:{}", cfg.port)).collect();
            if at.is_empty() {
                return Err("with no relay a phone must reach this computer itself, and it has no address a phone could use: join a network (and `airband lan on`), join a private network that the phone is on too, or `airband relay on`".into());
            }
            println!("Pairing \"{name}\" with no relay. It works where the phone can reach this computer: on its network, or over a private network both are on. Notifications need the relay, so none are sent.");
            Pairing::Near { at, secret }
        }
    };
    let link = pairing.link();
    // on a terminal, the link as a picture to point the phone's camera at; piped somewhere, only the link
    if std::io::IsTerminal::is_terminal(&std::io::stdout())
        && let Ok(code) = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::L) {
            // black on white whatever the terminal's own colours, so any camera reads it
            let picture = code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build();
            let lines: Vec<String> = picture.lines().map(|l| format!("\x1b[47;30m{l}\x1b[0m")).collect();
            println!("\nPoint the phone's camera at this:\n\n{}", lines.join("\n"));
        }
    println!("\nOr open this on your phone, or paste it into the app. It is the key to your agents: treat it like a password.\n\n{link}");
    Ok(())
}

pub(crate) fn unpair(mut cfg: Config, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or_else(|| format!("which one? paired: {}", cfg.pairings.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))?;
    let Some(gone) = cfg.pairings.iter().find(|p| p.name == name).cloned() else {
        return Err(format!("no phone is paired as \"{name}\""));
    };
    cfg.pairings.retain(|p| p.name != name);
    config::save(&cfg)?;
    // tell the relay the channel is finished with, so it does not keep it on record for ever. If the relay
    // cannot be reached the radio is cut off all the same: the tower no longer answers on that channel.
    if let (Some(relay), Ok(keys)) = (&cfg.relay, airband_core::relay::Keys::derive(&gone.secret)) {
        let _ = airband_core::net::agent(std::time::Duration::from_secs(8)).delete(format!("{relay}/v1/channels/{}", keys.channel)).header("authorization", format!("Bearer {}", keys.pull_token)).call();
    }
    println!("\"{name}\" is cut off. airband stops answering it within ten seconds.");
    Ok(())
}

/// `airband quick`: his own quick messages, offered on every radio after the standard ones.
///   airband quick                                  list them
///   airband quick add "Make no mistakes"           the label is also what is sent
///   airband quick add "Anti-AI" "Reread the writing style guide and fix what breaks it."
///   airband quick remove "Make no mistakes"
pub(crate) fn quick(mut cfg: Config, args: &[String]) -> Result<(), String> {
    match (args.first().map(String::as_str), args.get(1)) {
        (None, _) => {
            for q in airband_core::radio::quick_defaults().iter().chain(&cfg.quick) {
                println!("{:<18} {}", q.label, q.text);
            }
            return Ok(());
        }
        (Some("add"), Some(label)) => {
            let text = args.get(2).cloned().unwrap_or_else(|| format!("{}.", label.trim_end_matches('.')));
            cfg.quick.retain(|q| q.label != *label);
            cfg.quick.push(airband_core::radio::Quick { label: label.clone(), text });
        }
        (Some("remove"), Some(label)) => {
            let before = cfg.quick.len();
            cfg.quick.retain(|q| q.label != *label);
            if cfg.quick.len() == before {
                return Err(format!("no quick message of yours is called \"{label}\""));
            }
        }
        _ => return Err("airband quick  |  airband quick add \"<label>\" [\"<what to send>\"]  |  airband quick remove \"<label>\"".into()),
    }
    config::save(&cfg)?;
    println!("Saved. airband reads it when it next starts:  airband install");
    Ok(())
}

pub(crate) fn status(cfg: &Config) {
    let res = crate::config::taken(crate::config::Tower::here(cfg, std::time::Duration::from_secs(5)).get("/v1/status").call());
    let Some(j) = res.and_then(|mut r| r.body_mut().read_json::<Value>().ok()) else {
        println!("airband is not running.");
        return;
    };
    println!("airband is running. Phone {}{}.", if j["phone"].as_bool() == Some(true) { "connected" } else { "not connected" }, if j["held"].as_bool() == Some(true) { " (standing by)" } else { "" });
    let sessions = j["sessions"].as_array().cloned().unwrap_or_default();
    for s in &sessions {
        println!("  {:<26} {:<8} {}", s["callsign"].as_str().unwrap_or(""), s["state"].as_str().unwrap_or(""), s["cwd"].as_str().unwrap_or(""));
    }
    if sessions.is_empty() {
        println!("  no sessions have checked in yet");
    }
    if let Some(kept) = j.get("kept") {
        println!("  kept: {} messages, {} raw events", kept["messages"], kept["events"]);
    }
    if let Some(projects) = j["projects"].as_object().filter(|p| !p.is_empty()) {
        println!("  can launch in: {}", projects.keys().cloned().collect::<Vec<_>>().join(", "));
    }
    for p in j["pending"].as_array().cloned().unwrap_or_default() {
        println!("  waiting to be heard: {} ({})", p["callsign"].as_str().unwrap_or("?"), p["kind"].as_str().unwrap_or("not yet read"));
    }
    // the last changes to what is read out, each with what it was and what it became: why something was not read
    let aloud = j["aloud"].as_array().cloned().unwrap_or_default();
    if !aloud.is_empty() {
        println!("  read aloud, lately:");
        for line in aloud {
            println!("    {}", line.as_str().unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_homebrew_binary_is_used_from_the_path_that_outlives_an_upgrade() {
        assert_eq!(in_opt(Path::new("/opt/homebrew/opt/airband"), Path::new("/opt/homebrew/Cellar/airband/0.1.1/bin/airband")), PathBuf::from("/opt/homebrew/opt/airband/bin/airband"));
    }
}
