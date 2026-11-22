//! `airband radio`: a radio host for a terminal. It is the smallest complete host: the same loop the phone apps
//! run (airband-radio), with printing for speaking and typed lines for listening. It makes no sound and reads
//! no microphone: what speaks and what listens are chosen on a phone and run there, and a terminal has neither.
use crate::config::Config;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use airband_core::pairing::Pairing;
use airband_radio::{Outgoing, Radio, RadioHost, RadioStatus};

struct Terminal {
    lines: Mutex<Receiver<String>>,
    listening: Arc<AtomicBool>,
    window: Duration,
}

impl RadioHost for Terminal {
    fn told(&self, text: String) {
        println!("{text}");
    }
    fn speak(&self, text: String) {
        println!("🔊 {text}");
    }
    fn listen(&self) -> Option<String> {
        self.listening.store(true, Ordering::SeqCst);
        let heard = self.lines.lock().unwrap().recv_timeout(self.window).ok();
        self.listening.store(false, Ordering::SeqCst);
        heard.filter(|l| !l.trim().is_empty())
    }
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _rate: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn hearing(&self, _text: String) {}
    // a terminal prints what is said and takes a typed line as what he said
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn run_model(&self, _: String, _: String, _: airband_radio::ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
    fn unplaced(&self, message: airband_radio::Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        match elsewhere {
            Some(callsign) => println!("Not sent: “{}” is for {callsign}, on another computer.", message.text),
            None => println!("Who is that for? {}. Say the name.", ask.join(", or ")),
        }
    }
    fn status(&self, status: RadioStatus) {
        match status {
            RadioStatus::Offline => println!("· airband is not answering"),
            RadioStatus::Listening => println!("🎙  (type a reply, or Enter for none)"),
            _ => {}
        }
    }
}

pub fn run(cfg: &Config, args: &[String]) -> Result<(), String> {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    // with no link: straight to the tower on this machine
    let link = flag("--link").unwrap_or_else(|| Pairing::Direct { url: format!("http://127.0.0.1:{}", cfg.port), token: cfg.radio_token.clone() }.link());
    let route = match Pairing::parse(&link)? {
        Pairing::Relay { relay, .. } => format!("through the relay at {relay}"),
        Pairing::Near { at, .. } => format!("directly to {}, encrypted", at.join(" or ")),
        Pairing::Direct { url, .. } => format!("directly to {url}"),
    };
    let window = flag("--window").and_then(|s| s.parse().ok()).unwrap_or(20);
    let (tx, rx) = channel();
    let listening = Arc::new(AtomicBool::new(false));
    let host = Arc::new(Terminal {
        lines: Mutex::new(rx),
        listening: listening.clone(),
        window: Duration::from_secs(window),
    });
    println!("Connected, {route}. Type a line to talk; start it with an agent's name to pick who gets it.");
    let radio = Radio::start(link, String::new(), host).map_err(|e| e.to_string())?;
    radio.set_call_mode(true);
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if listening.load(Ordering::SeqCst) {
            let _ = tx.send(line);
        } else if !line.trim().is_empty() {
            // nobody asked: this line is him starting a conversation
            let _ = tx.send(line);
            radio.talk(None, None);
        }
    }
    // standard input closed (run from a script): keep the radio on
    loop {
        std::thread::park();
    }
}

/// Says nothing but what he has to be told about something he sent.
struct Silent;
impl RadioHost for Silent {
    fn told(&self, _: String) {}
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn status(&self, _: RadioStatus) {}
    fn outbox(&self, _: Vec<Outgoing>, note: Option<String>) {
        if let Some(n) = note.filter(|n| !n.is_empty()) {
            println!("{n}");
        }
    }
    fn hearing(&self, _text: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn run_model(&self, _: String, _: String, _: airband_radio::ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
    fn unplaced(&self, message: airband_radio::Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        match elsewhere {
            Some(callsign) => eprintln!("Not sent: “{}” is for {callsign}, on another computer.", message.text),
            None => eprintln!("Not sent: “{}” could be for {}. Tag one: @name.", message.text, ask.join(", or ")),
        }
    }
}

fn link_from(cfg: &Config, args: &[String]) -> String {
    if let Some(why) = crate::config::keyless().filter(|_| !args.iter().any(|a| a == "--link")) {
        eprintln!("airband: this needs airband's keys, which are in the keychain and could not be read here: {why}");
        std::process::exit(2);
    }
    args.iter().position(|a| a == "--link").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| Pairing::Direct { url: format!("http://127.0.0.1:{}", cfg.port), token: cfg.radio_token.clone() }.link())
}

/// `airband feed [--team <team> | --agent "<callsign>"] [--once]`: the chat, in a terminal. Each message is the
/// short version.
pub fn feed(cfg: &Config, args: &[String]) -> Result<(), String> {
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent)).map_err(|e| e.to_string())?;
    let once = args.iter().any(|a| a == "--once");
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let team = flag("--team");
    // the agent he named, by the id its callsign has now
    let agent = match flag("--agent") {
        Some(named) => {
            let roster = radio.feed(None, None, u64::MAX, 0).map_err(|e| e.to_string())?.agents;
            Some(roster.into_iter().find(|a| a.callsign == named || a.parts().text() == named).map(|a| a.id).ok_or(format!("No agent is called {named}."))?)
        }
        None => None,
    };
    let mut after = 0;
    loop {
        let f = radio.feed(team.clone(), agent.clone(), after, if once { 0 } else { 25 }).map_err(|e| e.to_string())?;
        if after == 0 {
            let who: Vec<String> = f.agents.iter().map(|a| format!("{}{}", a.callsign, if a.waiting { " (waiting)" } else { "" })).collect();
            println!("Your agents: {}", if who.is_empty() { "nobody".into() } else { who.join(", ") });
        }
        for m in &f.messages {
            let to = if m.to.is_empty() { String::new() } else { format!(" → {}", m.to.join(", ")) };
            let mark = if m.needs_you { "‼" } else if m.fyi { "·" } else { " " };
            println!("{mark} [{}{to}] {}{}", m.from, m.text, if m.long { "  (+more)" } else { "" });
            if !m.pictures.is_empty() {
                println!("    {} picture{}", m.pictures.len(), if m.pictures.len() == 1 { "" } else { "s" });
            }
            after = after.max(m.seq);
        }
        if once {
            return Ok(());
        }
    }
}

/// `airband launch <team> [--tool <id>] [--model <name>] [--effort <name>] [task…]`: start a new agent in one of
/// his teams. The flags are this command's own; the task is everything else, in his words.
pub fn launch(cfg: &Config, args: &[String]) -> Result<(), String> {
    let mut rest: Vec<String> = Vec::new();
    let mut flags: std::collections::HashMap<&str, String> = Default::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match ["--tool", "--model", "--effort", "--link"].into_iter().find(|flag| flag == word) {
            Some(flag) => {
                flags.insert(flag, words.next().cloned().ok_or(format!("{flag} needs a value"))?);
            }
            None => rest.push(word.clone()),
        }
    }
    let Some((project, task)) = rest.split_first() else {
        return Err("say which team: airband launch <team> [--tool <id>] [--model <name>] [--effort <name>] [task]".into());
    };
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent)).map_err(|e| e.to_string())?;
    println!("{}", radio.launch(project.clone(), flags.remove("--tool"), flags.remove("--model"), flags.remove("--effort"), task.join(" ")).map_err(|e| e.to_string())?);
    Ok(())
}

/// `airband send "@shop2 leave them"`: say something from the terminal.
pub fn send(cfg: &Config, args: &[String]) -> Result<(), String> {
    let words: Vec<&str> = args.iter().take_while(|a| *a != "--link").map(String::as_str).collect();
    if words.is_empty() {
        return Err("say what to send: airband send \"@shop2 leave them\"  (@group and @all work too)".into());
    }
    let words: Vec<&str> = words.into_iter().filter(|w| *w != "--keep-trying").collect();
    if args.iter().any(|a| a == "--keep-trying") {
        // what the phone app does: hand it over and let the radio get it through whenever it can
        let dir = crate::config::dir().join("radio");
        let radio = Radio::start(link_from(cfg, args), dir.display().to_string(), Arc::new(Silent)).map_err(|e| e.to_string())?;
        // the tower says what came of it in the feed, which the radio follows as the app does
        {
            let radio = radio.clone();
            std::thread::spawn(move || loop {
                if radio.follow(25).is_err() {
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        }
        radio.queue(words.join(" "), None, None, None);
        let id = radio.unsent().last().map(|o| o.id.clone()).unwrap_or_default();
        // until there is nothing more for this command to wait for: his computer has dealt with it, the relay
        // is keeping it for a computer that is away (he has been told), or it was given up on
        loop {
            match radio.unsent().into_iter().find(|o| o.id == id) {
                None => {
                    println!("Delivered.");
                    return Ok(());
                }
                Some(o) if o.failed.is_some() => return Err(o.failed.unwrap_or_default()),
                Some(o) if o.stage == airband_radio::Stage::Left => return Ok(()),
                Some(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent)).map_err(|e| e.to_string())?;
    println!("{}", radio.send(words.join(" "), None).map_err(|e| e.to_string())?);
    Ok(())
}
