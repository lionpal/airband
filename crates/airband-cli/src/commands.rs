//! The commands that change how this computer's tower is set up, and say how it is: `relay`, `model`, `lan`,
//! `pair`, `unpair`, `quick`, `status`.
#![expect(clippy::disallowed_methods, reason = "lists, opens or removes files, or closes a channel, directly: to move to airband_io (audit 6)")]
use crate::config::{self, Config};
use serde::Serialize;
use airband_core::pairing::Pairing;

/// `airband relay <url> [--admission <key>]` / `airband relay off`
pub(crate) fn relay(cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => println!("{}", cfg.relay.as_deref().map(|r| format!("Your phone reaches this computer through {r}.")).unwrap_or("No relay: your phone reaches this computer at its own addresses, on the same network or over a private network.".into())),
        Some("off") => {
            config::change(|now| {
                now.relay = None;
                now.relay_admission = None;
                Ok(())
            })?;
            println!("No relay from now on. A phone paired through it must be paired again:  airband pair");
        }
        Some("on") => {
            config::change(|now| {
                now.relay = Some(config::RELAY.into());
                now.relay_admission = None;
                Ok(())
            })?;
            println!("Relay set to {}. A phone paired without it must be paired again:  airband pair", config::RELAY);
        }
        Some(url) => {
            if airband_core::links::web(url).is_none() {
                return Err(format!("a relay address is a web address, like {}", config::RELAY));
            }
            config::change(|now| {
                now.relay = Some(url.trim_end_matches('/').to_string());
                now.relay_admission = crate::option(args, "--admission");
                Ok(())
            })?;
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
/// environment variable, or is one already kept for that provider (`airband keys`). What reads decides what
/// of each agent's message is worth showing, and who his words are for.
pub(crate) fn model(cfg: Config, args: &[String]) -> Result<(), String> {
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
        Some(word) if word == Use::NONE => Use::default(),
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
            }
            for a in &o.asks {
                if let Some(said) = crate::option(args, &format!("--{}", a.field)) {
                    u.set.insert(a.field.clone(), serde_json::Value::String(said.to_string()));
                }
            }
            u
        }
    };
    config::change(|now| {
        if !chosen.key.is_empty() && now.key_of(&chosen.kind).as_ref() != Some(&chosen.key) {
            now.keep_key(&chosen.kind, chosen.key.clone());
        }
        now.summarizer = chosen;
        Ok(())
    })?;
    // the tower reads its settings again whenever the file changes
    println!("Saved. airband uses it from now on.");
    Ok(())
}

/// `airband lan [on | off]`: whether a paired phone on the same network may reach this computer straight.
pub(crate) fn lan(cfg: Config, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {
            println!("{}", if cfg.lan { "On: a paired phone on the same network reaches this computer directly, encrypted. Away from that network it goes through the relay." } else { "Off: a paired phone always goes through the relay." });
            println!("Change it:  airband lan on  |  airband lan off");
            return Ok(());
        }
        Some("on" | "off") => {}
        Some(other) => return Err(format!("airband lan takes on or off, not \"{other}\"")),
    }
    let on = args.first().map(String::as_str) == Some("on");
    config::change(|now| {
        now.lan = on;
        Ok(())
    })?;
    println!("Saved. If airband is running it uses this from its next start: airband install");
    Ok(())
}

pub(crate) fn pair(cfg: Config, name: &str) -> Result<(), String> {
    // a slip like `airband pair --help` must not make a pairing called "--help"
    if name.starts_with('-') {
        return Err(format!("\"{name}\" is not a name for a phone. Use:  airband pair [name]"));
    }
    // one secret for a radio, whichever way it reaches the tower
    // read and, for a new one, added while the file is held: a pairing made meanwhile elsewhere is kept, and
    // two made at once under one name share one secret
    let secret = config::change(|now| {
        Ok(match now.pairings.iter().find(|p| p.name == name) {
            Some(p) => p.secret.clone(),
            None => {
                let secret = airband_core::relay::new_secret();
                now.pairings.push(config::PairedRadio { name: name.into(), secret: secret.clone() });
                secret
            }
        })
    })?;
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
    // the phone shows this computer by its name from the start, before it has reached it
    let link = pairing.link_named(&config::station(&cfg));
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

pub(crate) fn unpair(cfg: Config, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or_else(|| format!("which one? paired: {}", cfg.pairings.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))?;
    let gone = config::change(|now| {
        let gone = now.pairings.iter().find(|p| p.name == name).cloned().ok_or_else(|| format!("no phone is paired as \"{name}\""))?;
        now.pairings.retain(|p| p.name != name);
        Ok(gone)
    })?;
    // tell the relay the channel is finished with, so it does not keep it on record for ever. If the relay
    // cannot be reached the radio is cut off all the same: the tower no longer answers on that channel.
    if let (Some(relay), Ok(keys)) = (&cfg.relay, airband_core::relay::Keys::derive(&gone.secret)) {
        match airband_io::http::agent(std::time::Duration::from_secs(8)).delete(keys.at(relay, "")).header("authorization", keys.as_tower()).call().map(|r| r.status().as_u16()) {
            Ok(200..=299 | 404) => {}
            Ok(status) => eprintln!("{relay} did not close its channel ({status}). The phone is cut off all the same."),
            Err(e) => eprintln!("Could not reach {relay} to close its channel ({e}). The phone is cut off all the same."),
        }
    }
    println!("\"{name}\" is cut off. airband stops answering it within ten seconds.");
    Ok(())
}

/// `airband quick`: his own quick messages, offered on every radio after the standard ones.
///   airband quick                                  list them
///   airband quick add "Make no mistakes"           the label is also what is sent
///   airband quick add "Anti-AI" "Reread the writing style guide and fix what breaks it."
///   airband quick remove "Make no mistakes"
pub(crate) fn quick(cfg: Config, args: &[String]) -> Result<(), String> {
    match (args.first().map(String::as_str), args.get(1)) {
        (None, _) => {
            for q in airband_core::radio::quick_defaults().iter().chain(&cfg.quick) {
                println!("{:<18} {}", q.label, q.text);
            }
            return Ok(());
        }
        (Some("add"), Some(label)) => {
            let text = args.get(2).cloned().unwrap_or_else(|| format!("{}.", label.trim_end_matches('.')));
            config::change(|now| {
                now.quick.retain(|q| q.label != *label);
                now.quick.push(airband_core::radio::Quick { label: label.clone(), text });
                Ok(())
            })?;
        }
        (Some("remove"), Some(label)) => config::change(|now| {
            let before = now.quick.len();
            now.quick.retain(|q| q.label != *label);
            if now.quick.len() == before { Err(format!("no quick message of yours is called \"{label}\"")) } else { Ok(()) }
        })?,
        _ => return Err("airband quick  |  airband quick add \"<label>\" [\"<what to send>\"]  |  airband quick remove \"<label>\"".into()),
    }
    println!("Saved. Your phone shows it within a moment.");
    Ok(())
}

/// A state or a kind as the wire writes it ("waiting", "needs-you").
fn word<T: Serialize>(t: &T) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

pub(crate) fn status(cfg: &Config) {
    let res = crate::adapters::taken(crate::adapters::TowerHere::here(cfg, std::time::Duration::from_secs(5)).get("/v1/status").call());
    let Some(status) = res.and_then(|mut r| r.body_mut().read_json::<crate::tower::Status>().ok()) else {
        println!("airband is not running, or is a version that says how it stands another way.");
        return;
    };
    println!("airband is running. Phone {}{}.", if status.phone { "connected" } else { "not connected" }, if status.held { " (standing by)" } else { "" });
    for s in &status.sessions {
        println!("  {:<26} {:<8} {}", s.callsign, word(&s.state), s.cwd.as_deref().unwrap_or("no folder"));
    }
    if status.sessions.is_empty() {
        println!("  no sessions have checked in yet");
    }
    match &status.kept {
        Some(kept) => println!("  kept: {} messages, {} raw events", kept.messages, kept.events),
        None => println!("  the database could not say how much it keeps"),
    }
    if !status.projects.is_empty() {
        println!("  can launch in: {}", status.projects.keys().cloned().collect::<Vec<_>>().join(", "));
    }
    for p in &status.pending {
        match p.kind {
            Some(kind) => println!("  waiting to be heard: {} ({})", p.callsign, word(&kind)),
            None => println!("  waiting to be heard: {}, not read yet", p.callsign),
        }
    }
    // the last changes to what is read out, each with what it was and what it became: why something was not read
    if !status.aloud.is_empty() {
        println!("  read aloud, lately:");
        for line in &status.aloud {
            println!("    {line}");
        }
    }
}
