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

/// What a radio in a terminal does with sound: nothing. No sound is played or recorded, no model is run on
/// this side, and no words are shown as they are heard. The same for every terminal host.
macro_rules! no_sound {
    () => {
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn hearing(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn run_model(&self, _: String, _: String, _: airband_radio::ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    };
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
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    // a terminal prints what is said and takes a typed line as what he said
    no_sound!();
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

pub(crate) fn run(cfg: &Config, args: &[String]) -> Result<(), String> {
    let flag = |name: &str| crate::option(args, name);
    let link = link_from(cfg, args);
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
            tx.send(line).ok();
        } else if !line.trim().is_empty() {
            // nobody asked: this line is him starting a conversation
            tx.send(line).ok();
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
    fn status(&self, _: RadioStatus) {}
    no_sound!();
    fn outbox(&self, _: Vec<Outgoing>, note: Option<String>) {
        if let Some(n) = note.filter(|n| !n.is_empty()) {
            println!("{n}");
        }
    }
    fn unplaced(&self, message: airband_radio::Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        match elsewhere {
            Some(callsign) => eprintln!("Not sent: “{}” is for {callsign}, on another computer.", message.text),
            None => eprintln!("Not sent: “{}” could be for {}. Tag one: @name.", message.text, ask.join(", or ")),
        }
    }
}

fn link_from(cfg: &Config, args: &[String]) -> String {
    if let Some(why) = cfg.keyless.clone().filter(|_| !args.iter().any(|a| a == "--link")) {
        eprintln!("airband: this needs airband's keys, which are in the keychain and could not be read here: {why}");
        std::process::exit(2);
    }
    link_or_here(cfg, args)
}

/// The pairing link he gave with `--link`, or with none, straight to the tower on this machine.
fn link_or_here(cfg: &Config, args: &[String]) -> String {
    crate::option(args, "--link").unwrap_or_else(|| Pairing::Direct { url: format!("http://127.0.0.1:{}", cfg.port), token: cfg.radio_token.clone() }.link())
}

/// An agent as the feed's roster has it, for finding by name: its id, its team's id, and its callsign as shown
/// and as its parts.
struct Named {
    id: String,
    team: String,
    shown: String,
    parts: airband_core::routing::Callsign,
}

/// The agents on the radio now, as the feed's roster has them.
fn roster(radio: &Radio) -> Result<Vec<Named>, String> {
    Ok(radio.feed(None, None, u64::MAX, 0).map_err(|e| e.to_string())?.agents.iter().map(|a| Named { id: a.id.clone(), team: a.team.clone(), shown: a.callsign.clone(), parts: a.parts() }).collect())
}

/// The id of the team called exactly `named`, from the agents in it. Names are distinct on one computer.
fn team_named(roster: &[Named], named: &str) -> Result<String, String> {
    roster.iter().find(|a| a.parts.project == named).map(|a| a.team.clone()).ok_or(format!("No team called {named} has an agent here."))
}

/// The id of the one agent called exactly `named`. Two called alike are not told apart by a guess: he is told,
/// and nothing is shown.
fn agent_named(roster: &[Named], named: &str) -> Result<String, String> {
    match roster.iter().filter(|a| a.shown == named || a.parts.text() == named).collect::<Vec<_>>().as_slice() {
        [] => Err(format!("No agent is called {named}.")),
        [one] => Ok(one.id.clone()),
        _ => Err(format!("More than one agent is called {named}.")),
    }
}

/// `airband feed [--team <team> | --agent "<callsign>"] [--once]`: the chat, in a terminal. Each message is the
/// short version.
pub(crate) fn feed(cfg: &Config, args: &[String]) -> Result<(), String> {
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent)).map_err(|e| e.to_string())?;
    let once = args.iter().any(|a| a == "--once");
    let flag = |name: &str| crate::option(args, name);
    // the team and the agent he named, by their ids: the feed goes by ids, and a name is only shown
    let (team, agent) = match (flag("--team"), flag("--agent")) {
        (None, None) => (None, None),
        (team, agent) => {
            let roster = roster(&radio)?;
            (team.map(|named| team_named(&roster, &named)).transpose()?, agent.map(|named| agent_named(&roster, &named)).transpose()?)
        }
    };
    let mut after = 0;
    loop {
        let f = radio.feed(team.clone(), agent.clone(), after, if once { 0 } else { airband_core::radio::wire::FeedQuery::WAIT_MOST }).map_err(|e| e.to_string())?;
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
pub(crate) fn launch(cfg: &Config, args: &[String]) -> Result<(), String> {
    let crate::Words { mut flags, rest, .. } = crate::words("launch", args, &["--tool", "--model", "--effort", "--link"])?;
    let Some((project, task)) = rest.split_first() else {
        return Err("say which team: airband launch <team> [--tool <id>] [--model <name>] [--effort <name>] [task]".into());
    };
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent)).map_err(|e| e.to_string())?;
    // the tower starts it in a team by the team's id: the name he typed only finds it
    let team = team_named(&roster(&radio)?, project)?;
    println!("{}", radio.launch(team, flags.remove("tool"), flags.remove("model"), flags.remove("effort"), task.join(" ")).map_err(|e| e.to_string())?);
    Ok(())
}

/// `airband send "@shop2 leave them"`: say something from the terminal.
pub(crate) fn send(cfg: &Config, args: &[String]) -> Result<(), String> {
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
                if radio.follow(airband_core::radio::wire::FeedQuery::WAIT_MOST).is_err() {
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        }
        let id = radio.queue(words.join(" "), None, None, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, team: &str, project: &str, number: &str) -> Named {
        let parts = airband_core::routing::Callsign { project: project.into(), number: number.into(), named: false };
        Named { id: id.into(), team: team.into(), shown: parts.shown(), parts }
    }

    #[test]
    fn the_feed_is_asked_for_a_team_and_an_agent_by_their_ids() {
        let roster = [named("a", "/w/shop", "shop", "one"), named("b", "/w/garden", "garden", "one"), named("c", "/w/garden", "garden", "two")];
        // the name he typed is never what the feed is asked for
        assert_eq!(team_named(&roster, "garden"), Ok("/w/garden".to_string()));
        assert!(team_named(&roster, "orchard").is_err());
        assert_eq!(agent_named(&roster, "garden two"), Ok("c".to_string()));
        assert!(agent_named(&roster, "garden three").is_err());
        // two answering to one name are not told apart by which came first
        let twins = [named("a", "/w/shop", "shop", "one"), named("b", "/v/shop", "shop", "one")];
        assert!(agent_named(&twins, "shop one").is_err());
    }
}
