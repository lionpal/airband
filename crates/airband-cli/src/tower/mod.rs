//! `airband run`: the tower.
//!
//! Agents → him: adapters post `airband.agent/1` events. When an agent finishes a turn, the tower cuts what it
//! wrote down to one or two spoken sentences, puts the agent's callsign in front, and hands it to the radio,
//! one transmission at a time, blocking questions first. Nothing is summarised while no radio is connected.
//! Him → agents: the radio posts what he said as text. A leading callsign picks the agent; otherwise it goes
//! to whoever spoke last. The tower delivers it the way that agent said it can be reached.
//!
//! Radios reach it through a relay (relay_link.rs), sealed end to end, or directly: it listens on 127.0.0.1
//! and this machine's address on a private network laid over the internet, if it has one
//! (`config::overlay_addrs`), where every request needs the token; and with `airband lan` on its local network
//! addresses, which take only sealed requests (`relay_link::sealed_router`), so the pairing key is what opens them.
//!
//! Where things are:
//! - this file: the pieces every module shares; `serve` starts the tower up and runs its background loops
//! - `state`: what the tower remembers, all of it in one place (`Inner`), the tables of it that are kept, and
//!   what is asked of the whole
//! - `sessions`: the agents on the radio, what each is called and which team it is in
//! - `teams`: his teams by id, how each is named, made, renamed and settled at the start, and the places a new
//!   agent can be started in
//! - `entries`: the chat log's lines
//! - `aloud`: where each thing said stands on its way to being heard on a call
//! - `calls`: each radio's place in a call
//! - `intake`: agents' events coming in, the one way in for every source
//! - `judge`: what is worth his attention, and in how few words (asks the model), and the short versions made
//!   behind the requests that want them
//! - `place`: whether an agent is in the right team
//! - `route`: who a reply is for, and the controls the tower answers itself (`understand` also reads his answer
//!   to "Who is that for?")
//! - `deliver`: getting his words to an agent
//! - `edges`: what the tower reaches outside itself through (the model, processes, folders, the ways to an
//!   agent), each a trait
//! - `launch` and `held`: starting a new agent, and the ones the tower runs itself
//! - `pump`: call mode's one-at-a-time queue
//! - `http`: the routes
//! - `permit`: permission an agent's tool asks for, put to him and answered from the phone
//! - `pictures` and `attachments`: copies of the pictures agents look at, and the files he sends
//! - `store`: the database: the chat log, the raw event record, receipts, delivery attempts, the kept tables
//! - `saver`: the one writer of the state to the database, off the lock
//! - `relay_link`: the outbound connection to the relay, one per paired radio
//! - `push`: waking his phone with a sealed notification when a message deserves it and he is not looking
//! - `accounts`, `local`, `links`, `access`, `loops`, `probe`, `status`: a tool's accounts, a model run on this
//!   computer, links in messages, the locks on the tower's state, the timers, the commands that try a model, and
//!   the tower's page about itself
mod accounts;
mod aloud;
mod attachments;
mod calls;
mod deliver;
mod edges;
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
mod saver;
mod store;
mod teams;
mod access;
mod loops;
mod probe;
mod serve;
mod settings;
use settings::Settings;
mod status;
pub(crate) use probe::{progress, triage, try_model};
pub(crate) use serve::run;
pub(crate) use status::{STATUS_PATH, Status};
pub(crate) use relay_link::{RelaySaid, close_channel};
use loops::*;

use aloud::*;
use calls::*;
use deliver::*;
use entries::*;
use judge::*;
use launch::*;
use permit::Permit;
use pump::*;
use route::*;
use sessions::*;
use teams::*;
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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use airband_core::clock::{now_ms, now_secs};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use airband_core::agent::{self, Agent, AgentEvent, Deliver, Event};
use airband_core::radio::{PRESENCE_HEADER, RADIO_HEADER, VIA_HEADER};
use airband_core::radio::{Act, AgentState, Feed, Kind, Message, Reply, RosterEntry, Transmission};
use airband_core::routing::{Callsign, Control, TeamName, parse_utterance};
use airband_core::ids::{AgentId, LauncherId, TeamId};
use airband_core::prose::{opening, plain as spoken};


/// How long `GET next` is held open: the one hold every long wait shares.
use airband_core::HOLD;
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

/// What he is told when the agent his words or his tap were for has gone.
const NOT_RUNNING: &str = "That agent is no longer running.";
/// The variable an agent airband itself runs (a model on this computer) carries: whatever starts an agent for
/// him takes it out, so the new agent reports to the tower.
const INTERNAL_ENV: &str = "AIRBAND_INTERNAL";

/// An answer that says `s`, and waits for his next words when `listen`.
fn reply(s: impl Into<String>, listen: bool) -> Reply {
    Reply { say: s.into(), listen, ..Reply::default() }
}

/// The first eight characters of an id, for the log. An agent is logged by this, never by its callsign, which
/// carries its team's name.
fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// A team as the log names it. Its id is its folder's path and its name is usually the folder's, so neither is
/// ever logged: a short digest of the id, the same each time, tells its lines apart.
fn team_tag(team: &TeamId) -> String {
    use sha2::{Digest, Sha256};
    airband_core::encode::hex(&Sha256::digest(team.as_str().as_bytes())[..4])
}

/// One radio's line: what it said is worked on in the order it was put here.
type Line = tokio::sync::mpsc::UnboundedSender<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>;

pub(crate) struct App {
    /// The settings while it runs: the only way to them ([`settings::Settings`]).
    pub(crate) settings: settings::Settings,
    store: Arc<Store>,
    /// Writes what changes in the state, off the lock.
    saver: saver::Saver,
    inner: Mutex<Inner>,
    kick: Notify,
    /// Counts every change to the state, once it is let go: what a request held open waits on to look again,
    /// in place of looking every few hundred milliseconds. A waiter looks only, never changes, while it waits.
    changed: tokio::sync::watch::Sender<u64>,
    /// What each radio has said and the tower has yet to answer, in the order it came: one is finished before
    /// the next of the same radio is begun, so an agent gets what he said in the order he said it.
    said: Mutex<HashMap<String, Line>>,
    accounts: accounts::Kept,
    /// Where each paired phone can be woken with a notification.
    push: push::Wake,
    /// Every pairing being served, by its channel.
    served: relay_link::Served,
    /// Held when a held agent's holder is started again, so two deliveries to one whose holder has gone never
    /// start two holders on one session.
    reviving: tokio::sync::Mutex<()>,
    /// The processes on this computer, as the tower asks after them and starts and ends them.
    procs: Arc<dyn edges::procs::Procs + Send>,
    /// The way his words get to agents in place of the real ones ([`edges::deliver::AgentWays`]), when a test
    /// puts one there; the real ones need the tower itself, so they are made from it.
    deliverer: Option<Arc<dyn edges::deliver::Deliverer + Send>>,
    /// The addresses besides 127.0.0.1 the tower is listening on now: what a radio is told it can reach
    /// directly, so it is never told of one that is not served.
    listening: Mutex<Vec<std::net::Ipv4Addr>>,
}

impl App {
    /// The settings now.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.settings.current()
    }
}

/// The time of day as the log stamps it.
fn clock() -> String {
    let s = now_secs();
    format!("{:02}:{:02}:{:02}Z", s / 3600 % 24, s / 60 % 60, s % 60)
}
fn log(msg: impl AsRef<str>) {
    #[cfg(test)]
    logged::push(msg.as_ref());
    println!("{} {}", clock(), msg.as_ref());
}


/// Whether a process on this computer is running ([`edges::procs::Procs::alive`]).
fn alive(pid: u32) -> bool {
    use edges::procs::Procs;
    edges::procs::SystemProcs.alive(pid)
}


/// What was written to the tower's log by any thread (a tokio worker, a blocking task, a link's own thread), for
/// a test to see that something is, or is never, written there. Kept for the whole run and never emptied, so one
/// test taking its lines never takes another's.
#[cfg(test)]
pub(crate) mod logged {
    static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    thread_local!(static READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });
    pub(super) fn push(line: &str) {
        LINES.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
    }
    /// Every line written, by any thread, since this thread last took them (since the run began, the first time).
    pub(crate) fn take() -> Vec<String> {
        let lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
        let from = READ.with(|r| r.replace(lines.len()));
        lines[from.min(lines.len())..].to_vec()
    }

    #[test]
    fn a_line_written_on_another_thread_is_seen() {
        take();
        std::thread::spawn(|| super::log("written off the test thread: k1-off-thread")).join().unwrap();
        assert!(take().iter().any(|l| l == "written off the test thread: k1-off-thread"));
    }
}
