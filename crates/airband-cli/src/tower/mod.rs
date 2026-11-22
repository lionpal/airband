//! `airband run`: the tower.
//!
//! Agents → him: adapters post `airband.agent/1` events. When an agent finishes a turn, the tower cuts what it
//! wrote down to one or two spoken sentences, puts the agent's callsign in front, and hands it to the radio,
//! one transmission at a time, blocking questions first. Nothing is summarised while no radio is connected.
//! Him → agents: the radio posts what he said as text. A leading callsign picks the agent; otherwise it goes
//! to whoever spoke last. The tower delivers it the way that agent said it can be reached.
//!
//! Radios reach it through a relay (relay_link.rs), sealed end to end, or directly: it listens on 127.0.0.1
//! and this machine's address on a private network laid over the internet, if it has one (`config::overlay_addrs`), and every direct request needs the token.
//!
//! Where things are:
//! - this file: starting the tower up, and the pieces every module shares
//! - `state`: what the tower remembers, all of it in one place (`Inner`), and what is asked of the whole
//! - `sessions`: the agents on the radio, what each is called and which team it is in
//! - `entries`: the chat log's lines
//! - `aloud`: where each thing said stands on its way to being heard on a call
//! - `calls`: each radio's place in a call
//! - `intake`: agents' events coming in
//! - `judge`: what is worth his attention, and in how few words (asks the model)
//! - `route`: who a reply is for, and the controls the tower answers itself
//! - `deliver`: the ways of getting his words to an agent
//! - `launch`: starting a new agent in a known project
//! - `pump`: call mode's one-at-a-time queue
//! - `http`: the routes
//! - `permit`: permission an agent's harness asks for, put to him and answered from the phone
//! - `pictures`: small copies of the pictures agents look at or make for him
//! - `store`: the database: the chat log, the raw event record, receipts, delivery attempts
//! - `relay_link`: the outbound connection to the relay, one per paired radio
//! - `push`: waking his phone with a sealed notification when a message deserves it and he is not looking
mod accounts;
mod aloud;
mod attachments;
mod calls;
mod deliver;
mod entries;
mod held;
mod http;
mod intake;
pub(crate) mod judge;
mod launch;
mod local;
mod permit;
mod place;
mod links;
mod pictures;
mod pump;
mod push;
mod relay_link;
mod route;
mod sessions;
mod state;
mod store;

use aloud::*;
use calls::*;
use deliver::*;
use entries::*;
use intake::*;
use judge::*;
use launch::*;
use permit::Permit;
use pump::*;
use route::*;
use sessions::*;
use state::*;
use store::*;

use crate::config::{self, Config, Summarizer};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use airband_core::clock::{now_ms, now_secs};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use airband_core::agent::{self, Agent, AgentEvent, Deliver, Event};
use airband_core::radio::{Act, AgentState, Feed, Heard, Kind, Message, More, Reply, RosterEntry, Transmission, Utterance, in_scope};
use airband_core::routing::{Callsign, Control, parse_utterance};
use airband_core::speech::{opening, spoken};

/// The header a radio names itself in, so each has its own place in a call. A radio that comes through the
/// relay is named by its pairing; one that sends nothing is "radio".
pub(crate) const RADIO_HEADER: &str = "x-airband-radio";

/// The header the tower's own link puts on a radio's request to say how it came: `the relay`, `his own
/// network`, or `the relay, kept while the tower was away`. Only for the log.
pub(crate) const VIA_HEADER: &str = "x-airband-via";
/// The header a radio says where it stands in, as JSON (`airband.radio/2`'s `Presence`). A radio that comes
/// through the relay says it inside the sealed request, and the relay link puts it here.
pub(crate) const PRESENCE_HEADER: &str = "x-airband-presence";

/// How long `GET next` is held open.
const HOLD: Duration = Duration::from_secs(25);
/// How long after a radio last showed it was there it is still taken to be: the one clock for that (see
/// [`state::Presence`]). Longer than a radio waits for an answer before it asks again (50 seconds), so one
/// request lost on the way does not make it gone.
const THERE: Duration = Duration::from_secs(60);
/// How old something an agent said can be and still be read out when he turns reading aloud back on.
const STILL_NEWS: Duration = Duration::from_secs(600);
/// A radio that took a transmission and never said "ready" is assumed free again after this.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// How long an agent that stopped to wait for background work of its own still counts as working. A product
/// rule: no tool says whether that work is still running.
const STILL_ITS_OWN_WORK_MS: u64 = 20 * 60 * 1000;

/// One radio's line: what it said is worked on in the order it was put here.
type Line = tokio::sync::mpsc::UnboundedSender<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>;

pub struct App {
    pub cfg: Config,
    store: Store,
    inner: Mutex<Inner>,
    kick: Notify,
    /// What each radio has said and the tower has yet to answer, in the order it came: one is finished before
    /// the next of the same radio is begun, so an agent gets what he said in the order he said it.
    said: Mutex<HashMap<String, Line>>,
    accounts: accounts::Kept,
    /// Where each paired phone can be woken with a notification.
    push: push::Wake,
}


/// What each working agent is doing and what it runs on, asked of its harness's adapter every couple of seconds,
/// and only while a radio has read the feed lately: with nobody looking this costs nothing. An adapter that
/// cannot say leaves whatever the agent reported itself (an `activity` or a `model` event).
async fn activity_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // the ones at work, and any whose model is not known yet (one that has been waiting since the tower started)
        let (looked_at, everyone): (Vec<agent::Agent>, Vec<(agent::Agent, u64)>) = {
            let inner = app.read();
            if !inner.following() {
                continue;
            }
            (inner.sessions.iter().filter(|s| !s.waiting || s.model.is_none()).map(|s| s.agent.clone()).collect(), inner.sessions.iter().map(|s| (s.agent.clone(), s.told_upto)).collect())
        };
        if everyone.is_empty() {
            continue;
        }
        let (found, typed) = tokio::task::spawn_blocking(move || {
            // what he has typed to each at its own keyboard since the last look: he sees it in the chat as he goes
            let typed: Vec<_> = everyone.into_iter().map(|(a, upto)| (crate::adapters::told(&a, upto), a.id, upto)).filter(|((told, now), _, was)| !told.is_empty() || now != was).collect();
            // pictures he pasted in with them are shrunk and kept here, off the async threads and before the
            // tower's state is locked
            let typed: Vec<_> = typed
                .into_iter()
                .map(|((told, now), id, was)| {
                    let pictures: Vec<Vec<airband_core::radio::Picture>> = told.iter().map(|t| if t.his { t.images.iter().filter_map(|bytes| pictures::keep_bytes(bytes)).collect() } else { Vec::new() }).collect();
                    ((told, now), pictures, id, was)
                })
                .collect();
            (looked_at.into_iter().map(|a| (crate::adapters::glance(&a), a.id)).collect::<Vec<_>>(), typed)
        })
        .await
        .unwrap_or_default();
        let mut inner = app.change();
        let mut learned = !typed.is_empty();
        for ((told, upto), pictures, id, _) in typed {
            inner.told(&id, told, pictures, upto);
        }
        for (seen, id) in found {
            // looked at every couple of seconds: what it is doing is not kept, what it runs on is
            let Some(s) = inner.unkept().session(&id) else { continue };
            if let Some(doing) = seen.doing.filter(|_| !s.waiting) {
                s.doing = Some(doing);
            }
            // what it runs on changes rarely (he switched model or effort part way): kept when it does
            if seen.model.is_some() && (s.model != seen.model || s.effort != seen.effort) {
                (s.model, s.effort) = (seen.model, seen.effort);
                learned = true;
            }
        }
        if learned {
            inner.keep();
        }
    }
}

/// An agent that works for a long time says nothing until it stops, and things he would want to know happen
/// on the way. Once a minute: for each agent that has been at work without a word for some minutes, the labels
/// of its steps since then go to the model, which writes one line if there is something in them for him and
/// nothing if there is not. The line goes in the chat as a note: no notification, not read aloud, and it does
/// not count as the agent having stopped or having spoken past a question.
async fn progress_loop(app: Arc<App>) {
    const QUIET_FOR: u64 = 4 * 60 * 1000;
    const FEWEST: usize = 4;
    // when each agent's steps were last looked at, so a quiet stretch with nothing in it is asked about once
    // every few minutes and not every time round
    let mut looked: HashMap<String, u64> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let due: Vec<(agent::Agent, u64)> = {
            let inner = app.read();
            let live: HashSet<&str> = inner.sessions.iter().map(|s| s.agent.id.as_str()).collect();
            looked.retain(|id, _| live.contains(id.as_str()));
            inner
                .sessions
                .iter()
                .filter(|s| !s.waiting)
                .filter_map(|s| {
                    // since it last said anything, or he last said anything to it
                    let since = inner.log.iter().rev().find(|e| e.agent.as_deref() == Some(s.agent.id.as_str()) || e.to_agents.contains(&s.agent.id)).map(|e| e.at)?;
                    let last = since.max(looked.get(&s.agent.id).copied().unwrap_or(0));
                    (now_ms().saturating_sub(last) >= QUIET_FOR).then(|| (s.agent.clone(), since))
                })
                .collect()
        };
        for (agent, since) in due {
            looked.insert(agent.id.clone(), now_ms());
            let looked_at = agent.clone();
            let steps = tokio::task::spawn_blocking(move || crate::adapters::steps(&looked_at, since)).await.unwrap_or_default();
            if steps.len() < FEWEST {
                continue;
            }
            let Some(note) = judge::progress(&app.cfg, &steps).await else { continue };
            let mut inner = app.change();
            // it spoke for itself while the model was reading: its own words are the news
            let Some(s) = inner.find(&agent.id).filter(|s| !s.waiting).cloned() else { continue };
            if inner.log.iter().any(|e| e.agent.as_deref() == Some(agent.id.as_str()) && e.at > since) {
                continue;
            }
            log(format!("{}: a note on the way ({} steps)", s.callsign.text(), steps.len()));
            inner.record(Entry { short: Some(note.clone()), ..Entry::said(config::random_hex(8), &s, Some(Kind::Fyi), note) });
            inner.tell();
        }
    }
}

/// The time of day as the log stamps it.
fn clock() -> String {
    let s = now_secs();
    format!("{:02}:{:02}:{:02}Z", s / 3600 % 24, s / 60 % 60, s % 60)
}
fn log(msg: impl AsRef<str>) {
    println!("{} {}", clock(), msg.as_ref());
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // signal 0 only asks; "not permitted" still means it exists
    unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}
#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true // not checked on this platform yet: an agent leaves when it signs off or a delivery fails
}

/// Ask a process to end: asked, not killed, so it can finish what it must and pass the word on to its own.
/// Nothing on a system with no such way of asking.
fn terminate(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// The tower's state, held to change it. When it is let go, what was changed is written down, and whoever
/// waits on the tower is told if `tell` was asked. So no path through a change can leave without saving:
/// there is nothing to remember to call.
struct Changing<'a> {
    app: &'a App,
    inner: std::sync::MutexGuard<'a, Inner>,
    changed: bool,
    tell: bool,
}

impl Changing<'_> {
    /// Those waiting on the tower (a phone holding a request open, the pump) are woken once this is let go.
    fn tell(&mut self) {
        self.tell = true;
    }
    /// The state, to change something that is not kept (what an agent is doing this minute, a call's place
    /// in the queue): changing it this way alone writes nothing. For the few places that look often and
    /// change what is kept only now and then; they say `keep` when they have.
    fn unkept(&mut self) -> &mut Inner {
        &mut self.inner
    }
    /// What was changed through `unkept` is to be written after all.
    fn keep(&mut self) {
        self.changed = true;
    }
}
impl std::ops::Deref for Changing<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.inner
    }
}
impl std::ops::DerefMut for Changing<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        self.changed = true;
        &mut self.inner
    }
}
impl Drop for Changing<'_> {
    fn drop(&mut self) {
        if self.changed {
            self.app.save(&self.inner);
        }
        if self.tell {
            self.app.kick.notify_one();
        }
    }
}

/// The tower's state, held for something that is not kept: who is on a call, what a phone is in the middle
/// of saying, counts for this run. Nothing is written when it is let go.
struct Passing<'a>(std::sync::MutexGuard<'a, Inner>);
impl std::ops::Deref for Passing<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}
impl std::ops::DerefMut for Passing<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        &mut self.0
    }
}

/// The tower's state, held to look at.
struct Reading<'a>(std::sync::MutexGuard<'a, Inner>);
impl std::ops::Deref for Reading<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl App {
    /// Look at the tower's state.
    fn read(&self) -> Reading<'_> {
        Reading(self.inner.lock().unwrap())
    }
    /// Change the tower's state: written down when the hold is let go, if anything was changed.
    fn change(&self) -> Changing<'_> {
        Changing { app: self, inner: self.inner.lock().unwrap(), changed: false, tell: false }
    }
    /// Change what is not kept from one run of the tower to the next. Nothing is written.
    fn passing(&self) -> Passing<'_> {
        Passing(self.inner.lock().unwrap())
    }
    /// Write what has changed: new and updated messages, and who is on the radio.
    fn save(&self, inner: &Inner) {
        let touched: HashSet<String> = inner.unsaved.borrow_mut().drain().collect();
        let entries: Vec<Entry> = inner.log.iter().filter(|e| touched.contains(&e.id)).cloned().collect();
        self.store.put_messages(&entries);
        self.store.save_state(&Saved { sessions: inner.sessions.clone(), projects: inner.projects.clone(), colours: inner.colours.clone(), held: inner.held.clone(), menus: inner.menus.clone(), tools: inner.tools.clone(), placed: inner.placed.clone() });
    }
}

pub fn triage(cfg: Config) -> Result<(), String> {
    let mut full = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut full).map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let item = Item { id: String::new(), notice: false, full: full.trim().to_string(), kind: None, say: None, asked: None };
    let j = rt.block_on(prepare(&cfg, &item));
    println!("{}\t{}\t{}", serde_json::to_value(j.kind).unwrap().as_str().unwrap_or(""), j.say, j.options.iter().enumerate().map(|(i, o)| if j.recommended == Some(i as u32) { format!("[{o}]") } else { o.clone() }).collect::<Vec<_>>().join(" | "));
    Ok(())
}

/// Does the model that reads for the tower answer? For `airband setup`.
pub fn try_model(cfg: &Config) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    // a model on this computer is fetched the first time: wait through that, and say so
    local::listen_after(cfg.port);
    local::wait_for_it(true);
    if judge::local_model(&cfg.summarizer).is_some() {
        println!("  Fetching the model and starting it. The first time this is a few gigabytes and takes a while.");
    }
    println!("  Asking it something…");
    let answered = rt.block_on(judge::ask_model(cfg, "Reply with the one word: ready".into())).map(|_| "It answers.".into());
    // a model this command started on this computer is stopped again: the tower starts its own
    local::stop();
    answered
}

/// `airband progress`: the labels of an agent's steps on standard input, one to a line; prints the note he
/// would be given for them, or "none". For trying the judgement by hand.
pub fn progress(cfg: Config) -> Result<(), String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| e.to_string())?;
    let steps: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    println!("{}", rt.block_on(judge::progress(&cfg, &steps)).unwrap_or_else(|| "none".into()));
    Ok(())
}

pub fn run(cfg: Config) -> Result<(), String> {
    if let Some(why) = config::keyless() {
        return Err(format!("airband's keys could not be read: {why}"));
    }
    // Keys a run without the keychain had to seal in a file go into the keychain when this one can use it.
    if crate::vault::carry_asked().exists() {
        // asked for from a shell that could not read the keychain: this run can
        let _ = std::fs::remove_file(crate::vault::carry_asked());
        match crate::vault::carry() {
            Ok(true) => log("the keys are out of the keychain and sealed in a file, ready for a copy signed with another certificate"),
            Ok(false) => {}
            Err(e) => log(format!("the keys could not be carried out of the keychain: {e}")),
        }
    } else if crate::vault::settle() {
        log("the keys are now in the keychain");
    }
    // A tower with one part dead answers nothing and looks alive. Anything that goes wrong badly enough to
    // stop a thread stops the tower, which is then started again (launchd, systemd), whole.
    let usual = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        usual(info);
        log(format!("the tower stopped itself after an internal failure: {info}"));
        std::process::exit(101);
    }));
    // a newer catalogue, now and twice a day: what a phone is offered for this computer comes from it
    std::thread::spawn(|| loop {
        let (changed, lines) = crate::catalog::refresh();
        lines.into_iter().for_each(log);
        if changed {
            log(format!("catalogue: now rev {}", airband_core::catalog::current().rev));
        }
        std::thread::sleep(Duration::from_secs(12 * 3600));
    });
    // a model on this computer is started now, so it is ready when the first message comes; until it is, a
    // message is given as written
    local::listen_after(cfg.port);
    if let Some((model, server)) = judge::local_model(&cfg.summarizer) {
        std::thread::spawn(move || {
            local::wait_for_it(true);
            match local::ensure(&model, &server) {
                Ok(_) => log("local model: ready"),
                Err(why) => log(format!("local model: {why}")),
            }
            local::wait_for_it(false);
        });
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let store = Store::open(&config::dir().join("airband.db"))?;
        let (saved, entries) = store.load(LOG_KEEP);
        let (seq, rev) = store.last_counts();
        let mut inner = Inner { sessions: saved.sessions, projects: saved.projects, colours: saved.colours, held: saved.held, menus: saved.menus, tools: saved.tools, placed: saved.placed, seq, rev, log: entries, ..Inner::default() };
        inner.links = links::Book::load();
        inner.reap();
        inner.close_requests_from_before();
        // credentials said before the chat masked them: masked now, and sent to every radio again as they are
        let leaked: Vec<String> = inner.log.iter_mut().filter_map(|e| e.scrub().then(|| e.id.clone())).collect();
        if !leaked.is_empty() {
            log(format!("masked a credential in {} kept message{}", leaked.len(), if leaked.len() == 1 { "" } else { "s" }));
            leaked.iter().for_each(|id| inner.touch(id));
        }
        pictures::prune();
        attachments::prune();
        inner.settle_reading();
        let port = cfg.port;
        let push = push::Wake::load(&store);
        let app = Arc::new(App { push, cfg, store, inner: Mutex::new(inner), kick: Notify::new(), said: Mutex::new(HashMap::new()), accounts: accounts::Kept::default() });
        // what was changed while starting (a credential masked in a kept message) is written now, not at the
        // next thing an agent says
        {
            let inner = app.read();
            app.save(&inner);
        }
        let router = http::router(app.clone());

        tokio::spawn(pump_loop(app.clone()));
        tokio::spawn(accounts::keep_alive(app.clone()));
        tokio::spawn(progress_loop(app.clone()));
        tokio::spawn(activity_loop(app.clone()));
        held::carry_on(&app).await;
        // tools that have no hooks and are watched (a server of the tool's own): each adapter that watches is
        // started, and tells the tower what it sees the same way an event over HTTP is taken
        {
            let (app, runtime) = (app.clone(), tokio::runtime::Handle::current());
            let tower = move || {
                let (taking, runtime, holding, having) = (app.clone(), runtime.clone(), app.clone(), app.clone());
                let (finding, entering, hooking) = (app.clone(), runtime.clone(), app.clone());
                crate::adapters::Tower {
                    event: Box::new(move |event| {
                        let _here = runtime.enter();
                        taking.store.event(&serde_json::to_value(&event).unwrap_or_default());
                        http::take_event(&taking, event).is_ok()
                    }),
                    holds: Box::new(move |session| held::holds(&holding.read(), session)),
                    found: Box::new(move |agent, idle| {
                        let _here = entering.enter();
                        let id = agent.id.clone();
                        let new = finding.read().find(&id).is_none();
                        let event = AgentEvent { schema: agent::SCHEMA.into(), agent, event: Event::Checkin, via: None };
                        finding.store.event(&serde_json::to_value(&event).unwrap_or_default());
                        if http::take_event(&finding, event).is_err() {
                            return false;
                        }
                        // asked every few seconds for every open session: only one that is to be marked is changed
                        if !finding.read().find(&id).is_some_and(|s| !s.listed || (new && idle)) {
                            return true;
                        }
                        let mut inner = finding.change();
                        if let Some(s) = inner.session(&id) {
                            s.listed = true;
                            // one that is new to the tower and doing nothing is waiting for him, not working
                            s.waiting = s.waiting || (new && idle);
                        }
                        true
                    }),
                    // one the tower runs itself is reported by its own driver, whatever its tool lists
                    hooked: Box::new(move |session| hooking.read().find(session).is_some_and(|s| s.hooked)),
                    listed: Box::new(move |harness| having.read().sessions.iter().filter(|s| s.agent.harness == harness && s.listed && s.agent.deliver != Some(Deliver::Held)).map(|s| s.agent.id.clone()).collect()),
                }
            };
            crate::adapters::watch_all(&tower);
            // which of each tool's sessions are open now, by one look for all of them
            std::thread::spawn(move || crate::adapters::find_all(tower()));
        }
        // the folders his coding tools have had sessions in, read now and every minute: a new agent can be
        // started in any of them by name
        {
            let app = app.clone();
            std::thread::spawn(move || loop {
                http::read_worked(&app);
                std::thread::sleep(Duration::from_secs(60));
            });
        }
        // what an agent's tool could not hand over while the tower was away is taken now, and whenever more is
        // left: read from the folder and taken as any event is, with no request to itself
        {
            let (app, runtime) = (app.clone(), tokio::runtime::Handle::current());
            std::thread::spawn(move || loop {
                crate::adapters::drain_spool(|event| {
                    let _here = runtime.enter();
                    app.store.event(&serde_json::to_value(&event).unwrap_or_default());
                    http::take_event(&app, event).is_ok()
                });
                std::thread::sleep(Duration::from_secs(5));
            });
        }
        relay_link::start(app.clone(), router.clone(), tokio::runtime::Handle::current());
        {
            let app = app.clone();
            tokio::spawn(async move {
                let mut ticks = 0u32;
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    ticks += 1;
                    // old copies of pictures go once an hour, not only when the tower starts: it runs for weeks
                    if ticks.is_multiple_of(120) {
                        tokio::task::spawn_blocking(pictures::prune);
                        let app = app.clone();
                        tokio::task::spawn_blocking(move || app.store.prune());
                    }
                    let mut inner = app.change();
                    // looked at often; written only when someone has gone
                    if inner.unkept().reap() {
                        inner.keep();
                    }
                }
            });
        }

        let local = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e} (is airband already running?)"))?;
        log(format!("listening on 127.0.0.1:{port}"));
        // every address also takes sealed requests from a paired radio that can reach it without the relay
        let router = router.merge(relay_link::sealed_router());
        {
            let router = router.clone();
            tokio::spawn(async move { axum::serve(local, router).await });
        }
        // A private network may come up after the tower does, or change address; so may the local network. On the local
        // network, when that is allowed (`airband lan`), only sealed requests are served: nothing there is
        // trusted with a token.
        let lan = app.cfg.lan;
        let mut bound = HashSet::new();
        loop {
            let wanted = config::overlay_addrs().into_iter().map(|a| (a, false)).chain(config::lan_addrs().into_iter().filter(|_| lan).map(|a| (a, true)));
            for (addr, sealed_only) in wanted {
                if bound.contains(&addr) {
                    continue;
                }
                match tokio::net::TcpListener::bind((addr, port)).await {
                    Ok(l) => {
                        log(format!("listening on {addr}:{port}{}", if sealed_only { ", for a paired radio on this network" } else { "" }));
                        bound.insert(addr);
                        let router = if sealed_only { relay_link::sealed_router() } else { router.clone() };
                        tokio::spawn(async move { axum::serve(l, router).await });
                    }
                    Err(e) => log(format!("cannot listen on {addr}:{port}: {e}")),
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    })
}
