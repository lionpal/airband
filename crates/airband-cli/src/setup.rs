//! `airband setup`: choose what reads, what speaks and what listens, in the terminal, and pair a phone.
//! `airband keys`: the keys those use.
//!
//! A provider is one row of [`PROVIDERS`]: what it can do, whether it needs a key, and what else it has to be
//! told. Adding one is a row here and the code that talks to it (a `Synth` for a voice, an arm of the model
//! call for summaries); nothing in the questions below names a provider.
//!
//! A key is kept once, under a name, with the provider it is for (`Config::keys`). The summaries, the voice
//! and the listening each use a key by its value, and the settings file writes that as the key's name, so
//! one key serves as many of them as its provider can do and a provider can have several keys.
use crate::config::{self, Config, StoredKey};
use serde_json::{Value, json};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Clone, Copy, PartialEq)]
enum Job {
    Summary,
    Voice,
    Ears,
}

struct Provider {
    /// Its name in the settings (`"kind"`), and the usual name for its key.
    id: &'static str,
    name: &'static str,
    does: &'static [Job],
    key: bool,
    /// What else it has to be told: the setting, the question, and the answer if he gives none.
    asks: &'static [(&'static str, &'static str, &'static str)],
    /// Where its keys are made.
    keys_at: &'static str,
}

const PROVIDERS: &[Provider] = &[
    Provider { id: "claude", name: "Claude on this computer, with your Claude login", does: &[Job::Summary], key: false, asks: &[], keys_at: "" },
    Provider { id: "gemini", name: "Google Gemini", does: &[Job::Summary, Job::Voice], key: true, asks: &[], keys_at: "aistudio.google.com/apikey" },
    Provider { id: "openai", name: "OpenAI, or any service that speaks its API", does: &[Job::Summary], key: true, asks: &[("url", "Address", "https://api.openai.com/v1"), ("model", "Model", "gpt-4o-mini")], keys_at: "platform.openai.com/api-keys" },
    Provider { id: "elevenlabs", name: "ElevenLabs", does: &[Job::Voice], key: true, asks: &[], keys_at: "elevenlabs.io/app/settings/api-keys" },
    Provider { id: "azure", name: "Microsoft Azure Speech", does: &[Job::Voice, Job::Ears], key: true, asks: &[("region", "Region", "eastus")], keys_at: "portal.azure.com, a Speech resource, Keys and Endpoint" },
];

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
    cfg.keys.insert(name, StoredKey { provider: p.id.into(), secret: pasted.clone() });
    Some(pasted)
}

/// One of the three choices: which provider does this job, with what key, and does it answer?
/// `set` puts the choice in the settings, or says why it cannot; `works` tries it for real.
fn choose(cfg: &mut Config, job: Job, title: &str, about: &str, none: &str, now: &str, set: fn(&mut Config, Option<Value>) -> Result<(), String>, works: fn(&Config) -> Result<String, String>) {
    println!("\n{title}\n  {about}\n  Now: {now}");
    let offered: Vec<&Provider> = PROVIDERS.iter().filter(|p| p.does.contains(&job)).collect();
    loop {
        for (i, p) in offered.iter().enumerate() {
            println!("  {}. {}", i + 1, p.name);
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
            None => None,
            Some(p) => {
                let mut v = json!({ "kind": p.id });
                if p.key {
                    let Some(key) = key_for(cfg, p) else { continue };
                    v["key"] = json!(key);
                }
                for (field, ask, usual) in p.asks {
                    let said = line(&format!("  {ask} [{usual}]: "));
                    v[*field] = json!(if said.is_empty() { usual.to_string() } else { said });
                }
                Some(v)
            }
        };
        let tried = set(cfg, chosen.clone()).and_then(|_| if chosen.is_some() { works(cfg) } else { Ok("Set to none.".into()) });
        match tried {
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

fn set_summary(cfg: &mut Config, v: Option<Value>) -> Result<(), String> {
    cfg.summarizer = match v {
        // Claude Code as a command: the usual one, with nothing to tell it
        Some(v) if v["kind"] == "claude" => config::Summarizer::default(),
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string())?,
        None => config::Summarizer::None,
    };
    Ok(())
}
fn set_voice(cfg: &mut Config, v: Option<Value>) -> Result<(), String> {
    cfg.voice = serde_json::from_value(v.unwrap_or(json!({ "kind": "device" }))).map_err(|e| e.to_string())?;
    Ok(())
}
fn set_ears(cfg: &mut Config, v: Option<Value>) -> Result<(), String> {
    cfg.ears = serde_json::from_value(v.unwrap_or(json!({ "kind": "device" }))).map_err(|e| e.to_string())?;
    Ok(())
}

fn kind_of<T: serde::Serialize>(v: &T) -> String {
    let v = serde_json::to_value(v).unwrap_or_default();
    let kind = v["kind"].as_str().unwrap_or("none");
    PROVIDERS.iter().find(|p| p.id == kind).map(|p| p.name.to_string()).unwrap_or_else(|| if kind == "command" { PROVIDERS[0].name.to_string() } else { "none".into() })
}

/// `airband setup [--no-start]`. With `--no-start` the choices are saved and nothing is started or paired.
pub fn setup(mut cfg: Config, start: bool) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("airband setup asks questions: run it in a terminal.".into());
    }
    if let Some(why) = config::keyless() {
        return Err(format!("Setup needs the keychain, which cannot be read from here: {why}"));
    }
    println!("Three choices, then your phone. Keys you give stay on this computer{}.\nThey go to the service they are for and nowhere else: never to airband's servers.", if config::locked() { ", in its keychain" } else { "" });
    let (now_summarizer, now_voice, now_ears) = (kind_of(&cfg.summarizer), kind_of(&cfg.voice), kind_of(&cfg.ears));
    choose(&mut cfg, Job::Summary, "1 of 3. Summaries", "What reads your agents' messages and tells you the short of them.", "None: you get every message as written", &now_summarizer, set_summary, crate::tower::try_model);
    choose(&mut cfg, Job::Voice, "2 of 3. Voice", "What reads to you when you turn reading aloud on.", "None: nothing is read aloud", &now_voice, set_voice, crate::tower::try_voice);
    choose(&mut cfg, Job::Ears, "3 of 3. Listening", "What turns what you say into words. Your phone reaches it with a pass from this computer.", "None: you type", &now_ears, set_ears, crate::tower::try_ears);
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
    let has = |v: Value| v["key"].as_str() == Some(secret);
    let mut out = Vec::new();
    if has(serde_json::to_value(&cfg.summarizer).unwrap_or_default()) || cfg.summarizer_fallback.as_ref().is_some_and(|f| has(serde_json::to_value(f).unwrap_or_default())) {
        out.push("summaries");
    }
    if has(serde_json::to_value(&cfg.voice).unwrap_or_default()) {
        out.push("voice");
    }
    if has(serde_json::to_value(&cfg.ears).unwrap_or_default()) {
        out.push("listening");
    }
    out
}

/// `airband keys [add <name> <provider> | remove <name> | lock | unlock]`. A key is never shown.
pub fn keys(args: &[String]) -> Result<(), String> {
    let mut cfg = config::load();
    if let Some(why) = config::keyless() {
        return Err(format!("The keys are in the keychain, which cannot be read from here: {why}"));
    }
    match args.first().map(String::as_str) {
        None => {
            println!("{}", if config::locked() { "Kept in the keychain, encrypted with your login password. The settings file holds none." } else { "Kept in the settings file, which only you can read." });
            if cfg.keys.is_empty() {
                println!("No keys yet. Add them with:  airband setup");
            }
            for (name, k) in &cfg.keys {
                let uses = used_for(&cfg, &k.secret);
                println!("  {name}  ({})  {}", PROVIDERS.iter().find(|p| p.id == k.provider).map(|p| p.name).unwrap_or(&k.provider), if uses.is_empty() { "not in use".to_string() } else { format!("used for {}", uses.join(", ")) });
            }
            println!("Change them:  airband setup  |  airband keys add <name> <provider>  |  airband keys remove <name>  |  airband keys lock  |  airband keys unlock");
        }
        Some("add") => {
            let (name, provider) = (args.get(1).ok_or("airband keys add <name> <provider>")?, args.get(2).ok_or("airband keys add <name> <provider>")?);
            let p = PROVIDERS.iter().find(|p| p.id == *provider && p.key).ok_or_else(|| format!("no provider called \"{provider}\" takes a key: {}", PROVIDERS.iter().filter(|p| p.key).map(|p| p.id).collect::<Vec<_>>().join(", ")))?;
            if name.starts_with('-') || cfg.keys.contains_key(name) {
                return Err(format!("\"{name}\" is taken or is not a name."));
            }
            let pasted = secret("Paste the key (it will not show): ");
            if pasted.is_empty() {
                return Err("No key given.".into());
            }
            cfg.keys.insert(name.clone(), StoredKey { provider: p.id.into(), secret: pasted });
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
        Some("lock") => {
            if !crate::vault::signed() {
                return Err("This copy of airband is not signed with a developer certificate, so the keychain would ask about it after every update. The keys stay in the settings file, which only you can read.".into());
            }
            config::lock()?;
            println!("The keys are in the keychain. The settings file holds none.");
        }
        Some("unlock") => {
            config::unlock()?;
            println!("The keys are back in the settings file, which only you can read.");
        }
        Some(other) => return Err(format!("airband keys takes add, remove, lock or unlock, not \"{other}\"")),
    }
    Ok(())
}
