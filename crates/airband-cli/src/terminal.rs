//! `airband radio`: a radio host for a terminal. It is the smallest complete host: the same loop the phone apps
//! run (airband-radio), with the computer's own voice for speaking and typed lines for listening.
use crate::config::Config;
use std::io::BufRead;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use airband_core::pairing::Pairing;
use airband_radio::{Outgoing, Radio, RadioHost, RadioStatus};

struct Terminal {
    lines: Mutex<Receiver<String>>,
    listening: Arc<AtomicBool>,
    quiet: bool,
    window: Duration,
    /// Audio from the tower's voice, gathered until the clip ends: (samples a second, PCM, when the first part came).
    playing: Mutex<(u32, Vec<u8>, Option<std::time::Instant>)>,
    asked: Mutex<std::time::Instant>,
    /// `--mic <file.wav>`: a recording to use as the microphone, so the listening service can be tried here.
    mic: Option<Vec<u8>>,
    mic_at: Mutex<usize>,
    /// `--keep <folder>`: where to leave what was played, for listening to afterwards.
    keep: Option<std::path::PathBuf>,
    clips: Mutex<u32>,
}

fn wav(rate: u32, pcm: &[u8]) -> Vec<u8> {
    let mut h = Vec::with_capacity(44 + pcm.len());
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    h.extend_from_slice(pcm);
    h
}

impl RadioHost for Terminal {
    fn speak(&self, text: String) {
        println!("🔊 {text}");
        if self.quiet {
            return;
        }
        let _ = if cfg!(target_os = "macos") {
            Command::new("say").arg(&text).status()
        } else if cfg!(windows) {
            Command::new("powershell")
                .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak($env:AIRBAND_SAY)"])
                .env("AIRBAND_SAY", &text)
                .status()
        } else {
            Command::new("espeak").arg(&text).status()
        };
    }
    fn listen(&self) -> Option<String> {
        self.listening.store(true, Ordering::SeqCst);
        let heard = self.lines.lock().unwrap().recv_timeout(self.window).ok();
        self.listening.store(false, Ordering::SeqCst);
        heard.filter(|l| !l.trim().is_empty())
    }
    // The tower's voice. A terminal cannot play a stream, so the parts are gathered and played when the clip
    // ends; the time to the first part is printed, since that is when a phone would start to sound.
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        let mut p = self.playing.lock().unwrap();
        if p.2.is_none() && !pcm.is_empty() {
            p.2 = Some(std::time::Instant::now());
            println!("🔈 first sound {:.2} s after it was asked for", self.asked.lock().unwrap().elapsed().as_secs_f32());
        }
        p.0 = rate;
        p.1.extend_from_slice(&pcm);
        if !last {
            return;
        }
        let (rate, pcm, _) = std::mem::replace(&mut *p, (rate, Vec::new(), None));
        drop(p);
        if pcm.is_empty() {
            return;
        }
        println!("🔈 {:.1} s of the tower's voice", pcm.len() as f32 / 2.0 / rate as f32);
        let n = {
            let mut c = self.clips.lock().unwrap();
            *c += 1;
            *c
        };
        let file = self.keep.clone().unwrap_or_else(std::env::temp_dir).join(format!("airband-clip-{n}.wav"));
        if std::fs::write(&file, wav(rate, &pcm)).is_ok() && !self.quiet && cfg!(target_os = "macos") {
            let _ = Command::new("afplay").arg(&file).status();
        }
        if self.keep.is_none() {
            let _ = std::fs::remove_file(&file);
        }
    }
    fn record_start(&self, _rate: u32) -> bool {
        *self.mic_at.lock().unwrap() = 0;
        if self.mic.is_some() {
            println!("🎙  (listening to the recording)");
        }
        self.mic.is_some()
    }
    // the recording, a tenth of a second at a time and at the speed it was spoken, then silence
    fn record_read(&self) -> Vec<u8> {
        let Some(mic) = &self.mic else { return Vec::new() };
        std::thread::sleep(Duration::from_millis(100));
        let mut at = self.mic_at.lock().unwrap();
        let chunk = 3200;
        let out = if *at < mic.len() { mic[*at..(*at + chunk).min(mic.len())].to_vec() } else { vec![0u8; chunk] };
        *at += chunk;
        out
    }
    fn record_stop(&self) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn hearing(&self, _text: String) {}
    // a terminal prints what is said and takes a typed line as what he said
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn transcribe(&self, _: String, _: Vec<u8>) -> Option<String> {
        None
    }
    fn own_voice(&self) -> bool {
        true
    }
    fn unplaced(&self, message: airband_radio::Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        match elsewhere {
            Some(callsign) => println!("Not sent: “{}” is for {callsign}, on another computer.", message.text),
            None => println!("Who is that for? {}. Say the name.", ask.join(", or ")),
        }
    }
    fn status(&self, status: RadioStatus) {
        if matches!(status, RadioStatus::Speaking { .. }) {
            *self.asked.lock().unwrap() = std::time::Instant::now();
        }
        match status {
            RadioStatus::Offline => println!("· no tower"),
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
        Pairing::Near { at, .. } => format!("straight to {}, sealed", at.join(" or ")),
        Pairing::Direct { url, .. } => format!("straight to {url}"),
    };
    let window = flag("--window").and_then(|s| s.parse().ok()).unwrap_or(20);
    let (tx, rx) = channel();
    let listening = Arc::new(AtomicBool::new(false));
    // a 16 kHz mono 16-bit WAV to stand in for the microphone
    let mic = flag("--mic").map(|f| std::fs::read(&f).map(|b| b[44.min(b.len())..].to_vec()).map_err(|e| format!("cannot read {f}: {e}"))).transpose()?;
    let host = Arc::new(Terminal {
        lines: Mutex::new(rx),
        listening: listening.clone(),
        quiet: args.iter().any(|a| a == "--quiet"),
        window: Duration::from_secs(window),
        playing: Mutex::new((24000, Vec::new(), None)),
        asked: Mutex::new(std::time::Instant::now()),
        mic,
        mic_at: Mutex::new(0),
        keep: flag("--keep").map(std::path::PathBuf::from),
        clips: Mutex::new(0),
    });
    println!("Radio on, {route}. Type a line to talk; start it with a callsign to pick who hears it.");
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

/// Says nothing; the flag is set when the outbox has emptied.
struct Silent(Arc<AtomicBool>);
impl RadioHost for Silent {
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
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        if let Some(n) = note {
            println!("{n}");
        }
        if waiting.is_empty() {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    fn hearing(&self, _text: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn transcribe(&self, _: String, _: Vec<u8>) -> Option<String> {
        None
    }
    fn own_voice(&self) -> bool {
        true
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
        eprintln!("airband: this needs the tower's keys, which are in the keychain and could not be read here: {why}");
        std::process::exit(2);
    }
    args.iter().position(|a| a == "--link").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| Pairing::Direct { url: format!("http://127.0.0.1:{}", cfg.port), token: cfg.radio_token.clone() }.link())
}

/// `airband feed [all | <group> | "<callsign>"] [--once]`: the chat, in a terminal. Each message is the short version.
pub fn feed(cfg: &Config, args: &[String]) -> Result<(), String> {
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent(Default::default()))).map_err(|e| e.to_string())?;
    let once = args.iter().any(|a| a == "--once");
    let named = args.iter().find(|a| !a.starts_with("--") && !a.starts_with("airband://")).cloned().unwrap_or_else(|| "all".into());
    let scope = match named.as_str() {
        "all" => "all".to_string(),
        n if n.contains(' ') => format!("agent:{n}"),
        n => format!("group:{n}"),
    };
    let mut after = 0;
    loop {
        let f = radio.feed(scope.clone(), after, if once { 0 } else { 25 }).map_err(|e| e.to_string())?;
        if after == 0 {
            let who: Vec<String> = f.agents.iter().map(|a| format!("{}{}", a.callsign, if a.waiting { " (waiting)" } else { "" })).collect();
            println!("On the radio: {}", if who.is_empty() { "nobody".into() } else { who.join(", ") });
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

/// `airband send "@shop2 leave them"`: say something from the terminal.
pub fn send(cfg: &Config, args: &[String]) -> Result<(), String> {
    let words: Vec<&str> = args.iter().take_while(|a| *a != "--link").map(String::as_str).collect();
    if words.is_empty() {
        return Err("say what to send: airband send \"@shop2 leave them\"  (@group and @all work too)".into());
    }
    let words: Vec<&str> = words.into_iter().filter(|w| *w != "--keep-trying").collect();
    let sent = Arc::new(AtomicBool::new(false));
    if args.iter().any(|a| a == "--keep-trying") {
        // what the phone app does: hand it over and let the radio get it through whenever it can
        let dir = crate::config::dir().join("radio");
        let radio = Radio::start(link_from(cfg, args), dir.display().to_string(), Arc::new(Silent(sent.clone()))).map_err(|e| e.to_string())?;
        radio.queue(words.join(" "), None, None, None);
        while !sent.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("Delivered.");
        return Ok(());
    }
    let radio = Radio::start(link_from(cfg, args), String::new(), Arc::new(Silent(sent))).map_err(|e| e.to_string())?;
    println!("{}", radio.send(words.join(" "), None).map_err(|e| e.to_string())?);
    Ok(())
}
