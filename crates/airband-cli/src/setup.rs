//! `airband setup`: choose what reads, what speaks and what listens, in the terminal, and pair a phone.
//! `airband keys`: the keys those use.
//!
//! What can be chosen is what `providers::offering` lists; nothing in the questions below names a provider.
//!
//! A key is kept once, under a name, with the provider it is for (`Config::keys`). The summaries, the voice
//! and the listening each use a key by its value, and the settings file writes that as the key's name, so
//! one key serves as many of them as its provider can do and a provider can have several keys.
use crate::config::{self, Config, StoredKey};
use crate::providers::{self, Job, Provider, Use};
use serde_json::json;
use std::io::{BufRead, IsTerminal, Write};

/// Where a job's choice is kept in the settings.
pub(crate) fn slot(cfg: &mut Config, job: Job) -> &mut Use {
    match job {
        Job::Summary => &mut cfg.summarizer,
        Job::Voice => &mut cfg.voice,
        Job::Ears => &mut cfg.ears,
    }
}

/// Try a job's choice for real: one call to the service. `sample` is a recording to play to a listening
/// service, when he gave one.
pub(crate) fn works(cfg: &Config, job: Job, sample: Option<&str>) -> Result<String, String> {
    match job {
        Job::Summary => crate::tower::try_model(cfg),
        Job::Voice => crate::tower::try_voice(cfg),
        Job::Ears => crate::tower::try_ears(cfg, sample),
    }
}

fn line(ask: &str) -> String {
    print!("{ask}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    s.trim().to_string()
}

/// A line he types that is not shown: a key.
fn secret(ask: &str) -> String {
    print!("{ask}");
    let _ = std::io::stdout().flush();
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
    let _ = std::io::stdin().lock().read_line(&mut s);
    #[cfg(unix)]
    if let Some(was) = was {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &was) };
        println!();
    }
    s.trim().to_string()
}

/// The key to use with a provider: one already kept for it, or a new one he pastes, kept under a name.
fn key_for(cfg: &mut Config, p: &Provider) -> Option<String> {
    let kept: Vec<String> = cfg.keys.iter().filter(|(_, k)| k.provider == p.id).map(|(name, _)| name.clone()).collect();
    for name in &kept {
        let answer = line(&format!("  Use the key you already gave, \"{name}\"? [Y/n] "));
        if answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            return cfg.keys.get(name).map(|k| k.secret.clone());
        }
    }
    if !p.keys_at.is_empty() {
        println!("  Its keys are made at {}.", p.keys_at);
    }
    let pasted = secret("  Paste the key (it will not show), or press return to go back: ");
    if pasted.is_empty() {
        return None;
    }
    // the provider's own name for the first, then a number
    let name = (1..).map(|n| if n == 1 { p.id.to_string() } else { format!("{}-{n}", p.id) }).find(|n| !cfg.keys.contains_key(n)).unwrap();
    cfg.keys.insert(name, StoredKey { provider: p.id.clone(), secret: pasted.clone() });
    Some(pasted)
}

/// One of the three choices: which provider does this job, with what key, and does it answer?
/// `set` puts the choice in the settings, or says why it cannot; `works` tries it for real.
fn choose(cfg: &mut Config, job: Job, title: &str, about: &str, none: &str) {
    println!("\n{title}\n  {about}\n  Now: {}", slot(cfg, job).describe(job));
    let offered: Vec<&Provider> = providers::offering(job).collect();
    loop {
        for (i, p) in offered.iter().enumerate() {
            let o = p.offers.iter().find(|o| o.job == job);
            println!("  {}. {}{}", i + 1, o.map(|o| &o.label).unwrap_or(&p.name), o.filter(|o| !o.about.is_empty()).map(|o| format!(". {}.", o.about)).unwrap_or_default());
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
            Some(p) => {
                let mut u = Use::of(&p.id);
                if p.key {
                    let Some(key) = key_for(cfg, p) else { continue };
                    u.key = key;
                }
                for a in p.offers.iter().filter(|o| o.job == job).flat_map(|o| &o.asks) {
                    let said = line(&format!("  {} [{}]: ", a.question, a.usual));
                    // what he leaves as usual is not written down, so it follows the row if the row changes
                    if !said.is_empty() && said != a.usual {
                        u.set.insert(a.field.clone(), json!(said));
                    }
                }
                u
            }
        };
        let none_chosen = chosen.is_none();
        *slot(cfg, job) = chosen;
        match if none_chosen { Ok("Set to none.".into()) } else { works(cfg, job, None) } {
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
pub fn setup(mut cfg: Config, start: bool) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("airband setup asks questions: run it in a terminal.".into());
    }
    if let Some(why) = config::keyless() {
        return Err(format!("Setup needs the keys, which cannot be read from here: {why}"));
    }
    println!("Three choices, then your phone. Keys you give stay on this computer{}.\nThey go to the service they are for and nowhere else: never to airband's servers.", format!(", {}", crate::vault::place()));
    choose(&mut cfg, Job::Summary, "1 of 3. Summaries", "What reads your agents' messages and tells you the short of them.", "None: you get every message in full");
    choose(&mut cfg, Job::Voice, "2 of 3. Voice", "What reads to you when you turn reading aloud on.", "None: nothing is read aloud");
    choose(&mut cfg, Job::Ears, "3 of 3. Listening", "What turns what you say into words. Your phone reaches it with a pass from this computer.", "None: you type");
    config::save(&cfg)?;
    if !start {
        println!("\nSaved.");
        return Ok(());
    }
    println!("\nSaved. Starting the tower with these.");
    crate::install::install()?;
    if cfg.pairings.is_empty() {
        println!("\nNow your phone: open airband on it and scan this.");
        crate::install::pair(config::load(), "phone")?;
    } else {
        println!("\nYour phone is already paired. To pair another:  airband pair <a name for it>");
    }
    Ok(())
}

/// What each kept key is used for, by name.
fn used_for(cfg: &Config, secret: &str) -> Vec<&'static str> {
    let has = |u: &Use| u.key == secret;
    let mut out = Vec::new();
    if has(&cfg.summarizer) || cfg.summarizer_fallback.as_ref().is_some_and(has) {
        out.push("summaries");
    }
    if has(&cfg.voice) {
        out.push("voice");
    }
    if has(&cfg.ears) {
        out.push("listening");
    }
    out
}

/// `airband keys [add <name> <provider> | remove <name> | lock]`. A key is never shown.
pub fn keys(args: &[String]) -> Result<(), String> {
    let mut cfg = config::load();
    if let Some(why) = config::keyless() {
        return Err(format!("The keys cannot be read from here: {why}"));
    }
    match args.first().map(String::as_str) {
        None => {
            println!("{}", if config::locked() { format!("Kept encrypted, {}. The settings file holds none.", crate::vault::place()) } else { "Still in the settings file from an older version. Move them:  airband keys lock".to_string() });
            if cfg.keys.is_empty() {
                println!("No keys yet. Add them with:  airband setup");
            }
            for (name, k) in &cfg.keys {
                let uses = used_for(&cfg, &k.secret);
                println!("  {name}  ({})  {}", airband_core::catalog::current().providers.iter().find(|p| p.id == k.provider).map(|p| &p.name).unwrap_or(&k.provider), if uses.is_empty() { "not in use".to_string() } else { format!("used for {}", uses.join(", ")) });
            }
            println!("Change them:  airband setup  |  airband keys add <name> <provider>  |  airband keys remove <name>");
        }
        Some("add") => {
            let (name, provider) = (args.get(1).ok_or("airband keys add <name> <provider>")?, args.get(2).ok_or("airband keys add <name> <provider>")?);
            let p = airband_core::catalog::current().providers.iter().find(|p| p.id == *provider && p.key).ok_or_else(|| format!("no provider called \"{provider}\" takes a key: {}", airband_core::catalog::current().providers.iter().filter(|p| p.key).map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")))?;
            if name.starts_with('-') || cfg.keys.contains_key(name) {
                return Err(format!("\"{name}\" is taken or is not a name."));
            }
            let pasted = secret("Paste the key (it will not show): ");
            if pasted.is_empty() {
                return Err("No key given.".into());
            }
            cfg.keys.insert(name.clone(), StoredKey { provider: p.id.clone(), secret: pasted });
            config::save(&cfg)?;
            println!("Kept as \"{name}\". Choose what uses it with:  airband setup");
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
        // settings written before the keys were always kept encrypted: move them now
        Some("lock") => {
            config::lock()?;
            println!("The keys are {}. The settings file holds none.", crate::vault::place());
        }
        // before installing a copy signed with another certificate, which the keychain would take for a stranger
        Some("carry") => match crate::vault::carry() {
            Ok(true) => println!("The keys are out of the keychain and sealed in a file. Install the newly signed copy now: it puts them back in the keychain as its own."),
            Ok(false) => println!("There is nothing in the keychain to move."),
            Err(why) => {
                std::fs::write(crate::vault::carry_asked(), "").map_err(|e| e.to_string())?;
                println!("The keychain cannot be read from here ({why}).\nThe tower will move the keys the next time it starts:  airband install");
            }
        },
        Some(other) => return Err(format!("airband keys takes add, remove, lock or carry, not \"{other}\"")),
    }
    Ok(())
}
