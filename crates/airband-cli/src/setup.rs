//! `airband setup`: choose what reads his agents' messages, in the terminal, and pair a phone. What listens
//! and what speaks are chosen on the phone and run there: nothing of either is set on this computer.
//! `airband keys`: the keys reading uses.
//!
//! What can be chosen is what `providers::offering` lists; nothing in the questions below names a provider.
//!
//! A key is kept once, with the provider it is for (`Config::keys`): one key per provider, found by that
//! provider (`Config::key_of`). The settings file writes a use of it as the key's name.
#![allow(clippy::disallowed_methods, reason = "setup writes the first settings file")]
use crate::config::{self, Config};
use crate::providers::{self, Job, Provider, Use};
use std::io::{BufRead, IsTerminal, Write};

fn line(ask: &str) -> String {
    print!("{ask}");
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s).ok();
    s.trim().to_string()
}

/// A line he types that is not shown: a key.
fn secret(ask: &str) -> String {
    print!("{ask}");
    std::io::stdout().flush().ok();
    #[cfg(unix)]
    let was = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        (libc::tcgetattr(0, &mut t) == 0).then(|| {
            let was = t;
            t.c_lflag &= !libc::ECHO;
            libc::tcsetattr(0, libc::TCSANOW, &t);
            was
        })
    };
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s).ok();
    #[cfg(unix)]
    if let Some(was) = was {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &was) };
        println!();
    }
    s.trim().to_string()
}

/// The key to use with a provider: one already kept for it, or a new one he pastes, kept under a name.
fn key_for(cfg: &mut Config, p: &Provider) -> Option<String> {
    if let Some(kept) = cfg.key_of(&p.id) {
        let answer = line(&format!("  Use the {} you already gave? [Y/n] ", p.key_name));
        if answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            return Some(kept);
        }
    }
    if !p.keys_at.is_empty() {
        println!("  Its keys are made at {}.", p.keys_at);
    }
    let pasted = secret("  Paste the key (it will not show), or press return to go back: ");
    if pasted.is_empty() {
        return None;
    }
    cfg.keep_key(&p.id, pasted.clone());
    Some(pasted)
}

/// The one choice made here: which provider reads, with what key, and does it answer?
fn choose(cfg: &mut Config, title: &str, about: &str, none: &str) {
    let job = Job::Summary;
    println!("\n{title}\n  {about}\n  Now: {}", cfg.summarizer.describe(job));
    // every offer for the job: a provider that does it more than one way is listed once for each
    let offered = providers::in_order(job);
    loop {
        for (i, (_, o)) in offered.iter().enumerate() {
            println!("  {}. {}{}", i + 1, o.label, if o.about.is_empty() { String::new() } else { format!(". {}.", o.about) });
        }
        println!("  {}. {none}", offered.len() + 1);
        let pick = line("  Which? (return keeps what you have) ");
        if pick.is_empty() {
            return;
        }
        let Some(n) = pick.parse::<usize>().ok().filter(|n| (1..=offered.len() + 1).contains(n)) else {
            println!("  Give its number.");
            continue;
        };
        let before = serde_json::to_value(&*cfg).unwrap();
        let chosen = match offered.get(n - 1) {
            None => Use::default(),
            Some((p, o)) => {
                let mut u = Use::of(&p.id, &o.id);
                if p.key {
                    // the key is the provider's: one already given for another of its offers is this one's too
                    let Some(key) = key_for(cfg, p) else { continue };
                    u.key = key;
                }
                for a in &o.asks {
                    let said = line(&format!("  {} [{}]: ", a.question, a.usual));
                    // what he leaves as usual is not written down, so it follows the row if the row changes
                    if !said.is_empty() && said != a.usual {
                        u.set.insert(a.field.clone(), serde_json::Value::String(said.to_string()));
                    }
                }
                u
            }
        };
        let none_chosen = chosen.is_none();
        cfg.summarizer = chosen;
        match if none_chosen { Ok("Set to none.".into()) } else { crate::tower::try_model(cfg) } {
            Ok(said) => {
                println!("  {said}");
                return;
            }
            Err(why) => {
                println!("  That did not work: {why}");
                // nothing of a choice that failed is kept, its key included
                *cfg = serde_json::from_value(before).unwrap();
            }
        }
    }
}

/// `airband setup [--no-start]`. With `--no-start` the choices are saved and nothing is started or paired.
pub(crate) fn setup(mut cfg: Config, start: bool) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("airband setup asks questions: run it in a terminal.".into());
    }
    if let Some(why) = cfg.keyless.clone() {
        return Err(format!("Setup needs the keys, which cannot be read from here: {why}"));
    }
    // what his agents are told his name is: the one this computer's account has for him until he gives another
    let name = line(&format!("Your name, as your agents are told it [{}]: ", cfg.name));
    if !name.is_empty() {
        cfg.name = name;
    }
    println!("One choice, then your phone. What listens to you and what speaks are chosen in the app. A key you give stays on this computer, {}.\nIt goes to the service it is for and nowhere else: never to airband's servers.", crate::vault::place());
    choose(&mut cfg, "Summaries", "What reads your agents' messages and tells you the short of them.", "None: you get every message in full");
    config::save(&cfg)?;
    if !start {
        println!("\nSaved.");
        return Ok(());
    }
    println!("\nSaved. Starting airband with these.");
    crate::install::install()?;
    if cfg.pairings.is_empty() {
        println!("\nNow your phone: open airband on it and scan this.");
        crate::commands::pair(config::load(), "phone")?;
    } else {
        println!("\nYour phone is already paired. To pair another:  airband pair <a name for it>");
    }
    Ok(())
}

/// What each kept key is used for, by name.
fn used_for(cfg: &Config, secret: &str) -> Vec<&'static str> {
    let has = |u: &Use| u.key == secret;
    let mut out = Vec::new();
    if has(&cfg.summarizer) {
        out.push("summaries");
    }
    out
}

/// `airband keys [add <name> <provider> | remove <name>]`. A key is never shown.
pub(crate) fn keys(args: &[String]) -> Result<(), String> {
    let mut cfg = config::load();
    if let Some(why) = cfg.keyless.clone() {
        return Err(format!("The keys cannot be read from here: {why}"));
    }
    match args.first().map(String::as_str) {
        None => {
            println!("Kept encrypted, {}. The settings file holds none.", crate::vault::place());
            if cfg.keys.is_empty() {
                println!("No keys yet. Add them with:  airband setup");
            }
            for (name, k) in &cfg.keys {
                let uses = used_for(&cfg, &k.secret);
                println!("  {name}  ({})  {}", airband_core::catalog::current().providers.iter().find(|p| p.id == k.provider).map(|p| &p.name).unwrap_or(&k.provider), if uses.is_empty() { "not in use".to_string() } else { format!("used for {}", uses.join(", ")) });
            }
            println!("Change them:  airband setup  |  airband keys add <provider>  |  airband keys remove <name>");
        }
        Some("add") => {
            let provider = args.get(1).ok_or("airband keys add <provider>")?;
            let p = providers::provider(provider).filter(|p| p.key).ok_or_else(|| format!("no provider called \"{provider}\" takes a key: {}", airband_core::catalog::current().providers.iter().filter(|p| p.key).map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")))?;
            let pasted = secret("Paste the key (it will not show): ");
            if pasted.is_empty() {
                return Err("No key given.".into());
            }
            // a provider has one key: this one takes the place of any before it
            cfg.keep_key(&p.id, pasted);
            config::save(&cfg)?;
            println!("Kept as \"{}\". Choose what uses it with:  airband setup", p.id);
        }
        Some("remove") => {
            let name = args.get(1).ok_or("airband keys remove <name>")?;
            let k = cfg.keys.get(name).ok_or_else(|| format!("there is no key called \"{name}\""))?;
            let uses = used_for(&cfg, &k.secret);
            if !uses.is_empty() {
                return Err(format!("\"{name}\" is used for {}. Choose something else for that first:  airband setup", uses.join(", ")));
            }
            cfg.keys.remove(name);
            config::save(&cfg)?;
            println!("Removed.");
        }
        // before installing a copy signed with another certificate, which the keychain would take for a stranger
        Some("carry") => match crate::vault::carry() {
            Ok(true) => println!("The keys are out of the keychain and kept in an encrypted file. Install the new copy of airband now: it puts them back in the keychain."),
            Ok(false) => println!("There is nothing in the keychain to move."),
            Err(why) => {
                std::fs::write(crate::vault::carry_asked(), "").map_err(|e| e.to_string())?;
                println!("The keychain cannot be read from here ({why}).\nairband moves the keys the next time it starts:  airband install");
            }
        },
        Some(other) => return Err(format!("airband keys takes add, remove or carry, not \"{other}\"")),
    }
    Ok(())
}
