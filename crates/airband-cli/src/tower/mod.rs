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
mod access;
mod loops;
mod probe;
mod serve;
pub use probe::{progress, triage, try_model};
pub use serve::run;
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
use airband_core::radio::{Act, AgentState, Feed, Heard, Kind, Message, More, Reply, RosterEntry, Transmission, Utterance};
use airband_core::routing::{Callsign, Control, parse_utterance};
use airband_core::prose::{opening, plain as spoken};


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
