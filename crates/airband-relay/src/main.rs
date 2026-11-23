//! airband-relay: the meeting point for towers and radios that cannot reach each other directly.
//!
//! It keeps, per channel, the hash of the tower's pull token, the hash of its radio's token and the requests
//! in flight. It holds no keys and sees only sealed bytes (`airband.relay/1`, crates/airband-core/src/relay.rs).
//! Put it behind TLS, on a proxy or load balancer that adds the caller's address to `x-forwarded-for`: the
//! limits per address go by the last entry there, and without one every caller counts as the same address.
//!
//!   PORT                     port to listen on (7474 when not set)
//!   HOST                     address to listen on (0.0.0.0 when not set)
//!   AIRBAND_RELAY_DATA       folder kept in when no database is set (./relay-data when not set): channels.json,
//!                            waitlist.txt, and held/ with the messages kept for towers that are away
//!   DB_HOST and friends      keep all of it in Postgres instead (store.rs)
//!   AIRBAND_RELAY_DYNAMO     keep all of it in Amazon DynamoDB instead: the tables' prefix (store.rs), with AWS_REGION
//!   AIRBAND_RELAY_ADMISSION  if set, a tower must present this to open a channel; without it, anyone may,
//!                            within limits (so many new channels from one address in a day, and in all in an hour)
//!
//! Who may do what. A tower does everything with its channel's pull token, and opening a new channel also
//! takes the admission key. A radio does everything with its channel's radio token. Both tokens are made from
//! the pairing secret, and the relay holds only their hashes. A channel's name alone gets nothing.
//!
//!   PUT    /v1/channels/{c}             tower: open the channel (Bearer pull token; x-airband-radio-hash)
//!   DELETE /v1/channels/{c}             tower: close it, when its radio has been cut off
//!   GET    /v1/channels/{c}/pull        tower: the next sealed request, held up to 25 s (204 when none); one
//!                                       kept while it was away carries x-airband-kept. Each pull carries
//!                                       x-airband-pull (2 or 3) and names in x-airband-got what the one before
//!                                       got; a 3 also names its tower (x-airband-tower) and the kept message it
//!                                       is still answering for (x-airband-answering). 426 or 409 with a
//!                                       PullRefused when it is not taken
//!   POST   /v1/channels/{c}/reply/{id}  tower: the sealed answer to request id; with x-airband-kept, that it
//!                                       took the kept message id, and no body
//!   POST   /v1/channels/{c}/push        tower: a sealed notification for a push service to wake the phone with (push.rs)
//!   POST   /v1/channels/{c}/call        radio: a sealed request; the response is the sealed answer (Bearer radio token)
//!   POST   /v1/channels/{c}/queue       radio: a sealed message to keep for a tower that is away (x-airband-id names it)
//!   POST   /v1/waitlist                 the homepage's sign-up: {"email": "…"} (airband.dev only, by CORS)
//!   OPTIONS /v1/waitlist                the browser's question before that sign-up (CORS)
//!   GET    /healthz
#![warn(clippy::let_underscore_must_use)]
// test code may touch files and build JSON by hand to stand in for the outside
#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_macros, clippy::let_underscore_must_use))]
use airband_core::relay::{ADMISSION_HEADER, ANSWERING_HEADER, CallRefused, GOT_HEADER, ID_HEADER, ORDERED_HEADER, PULL_HEADER, PullRefused, RADIO_HASH_HEADER, REQUEST_HEADER, Route, TOWER_HEADER};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, oneshot};
use airband_core::relay::{HELD_MOST, MAX_MESSAGE, NotPushed, PushAnswer, PushRequest, token_hash};
use airband_core::encode::random_hex;

mod auth;
mod dynamo;
mod handlers;
mod limits;
mod push;
mod state;
mod store;
mod waitlist;
use auth::*;
use handlers::*;
use limits::*;
use state::*;
use waitlist::*;

/// A setting from the environment, trimmed. Nothing when it is not set or says nothing.
fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

async fn relay_from_env() -> Arc<Relay> {
    let store = store::from_env().unwrap_or_else(|e| {
        eprintln!("airband-relay: {e}");
        std::process::exit(2);
    });
    // the database may be waking up or briefly unreachable at start: wait for it rather than start empty
    let mut saved = store.load().await;
    for attempt in 1..=10 {
        if saved.is_ok() {
            break;
        }
        eprintln!("airband-relay: cannot read the channel list from {} (try {attempt} of 10): {}", store.describe(), saved.as_ref().unwrap_err());
        tokio::time::sleep(Duration::from_secs(6)).await;
        saved = store.load().await;
    }
    let saved = saved.unwrap_or_else(|e| {
        eprintln!("airband-relay: giving up: {e}");
        std::process::exit(2);
    });
    println!("airband-relay: channels are kept in {}", store.describe());

    let channels = saved.into_iter().map(|(c, saved)| (c, Channel::from(saved))).collect::<HashMap<_, _>>();
    let admission = var("AIRBAND_RELAY_ADMISSION");
    println!("airband-relay: {} channels known; opening a channel {}", channels.len(), if admission.is_some() { "needs the admission key" } else { "is open to anyone" });
    let push = push::Push::from_env();
    println!("airband-relay: {}", push.describe());
    let relay = Arc::new(Relay::new(push, channels, store, admission));
    // what a tower never came back for is cleared at start and then every hour, not only at start
    let purging = relay.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = purging.store.held_purge().await {
                eprintln!("airband-relay: could not clear old kept messages: {e}");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
    relay
}

fn routes(relay: Arc<Relay>) -> Router {
    Router::new()
        .route(Route::Channel.pattern(), put(open).delete(close))
        .route(Route::Pull.pattern(), get(pull))
        .route(Route::Reply("").pattern(), post(reply))
        .route(Route::Call.pattern(), post(call))
        .route(Route::Queue.pattern(), post(hold))
        .route(Route::Push.pattern(), post(wake_phone))
        .route("/v1/waitlist", post(waitlist).options(waitlist_preflight))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE))
        .with_state(relay)
}

#[tokio::main]
async fn main() {
    let port: u16 = match var("PORT") {
        None => 7474,
        Some(p) => p.parse().unwrap_or_else(|_| {
            eprintln!("airband-relay: PORT \"{p}\" is not a port number");
            std::process::exit(1)
        }),
    };
    let app = routes(relay_from_env().await);
    let host = var("HOST").unwrap_or_else(|| "0.0.0.0".into());
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.expect("cannot listen");
    println!("airband-relay on port {port}");
    axum::serve(listener, app).with_graceful_shutdown(async { tokio::signal::ctrl_c().await.ok(); }).await.expect("the relay stopped");
}
