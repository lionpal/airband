//! Every rule the tower, the relay and the radio share. Core touches neither the disk nor the network: it has no
//! HTTP client to reach it with, and files are `airband-io`'s (kept files, the catalogue fetched and kept, the
//! client). `providers::Answering::status` reads what a service answered. Everything here is tested directly.
//!
//! Three versioned protocols meet at the tower (docs/protocols.md):
//! - [`agent`], `airband.agent/1`: how any coding agent, on any harness or model, reports in and is spoken to.
//! - [`radio`], `airband.radio/4`: how a phone, car or desktop app hears transmissions and sends replies.
//! - [`relay`], `airband.relay/1`: how a radio reaches a tower that is behind a home router, through a relay
//!   that can read none of it. [`pairing`] is the link that sets a radio up for each way
//!   of reaching its tower: through a relay, straight and sealed on his own network, or unsealed on the same machine.
#![warn(clippy::let_underscore_must_use)]
// test code may touch files and build JSON by hand to stand in for the outside
#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_macros, clippy::let_underscore_must_use))]
#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

/// A setting that is on unless it is written off: the one `#[serde(default = "...")]` for one.
pub fn yes() -> bool {
    true
}

pub mod agent;
pub mod audio;
pub mod catalog;
pub mod clock;
pub mod encode;
pub mod identity;
pub mod ids;
pub mod links;
pub mod pairing;
pub mod prose;
pub mod providers;
pub mod radio;
pub mod redact;
pub mod relay;
pub mod routing;
pub mod summary;
pub mod template;
pub mod token;
pub mod tools;

/// How long a request that waits for something new is held open before it is answered with nothing, wherever
/// one is: a tower's pull at the relay, a radio's wait at its tower. One number, so a proxy's idle limit has one
/// thing to be longer than.
pub const HOLD: std::time::Duration = std::time::Duration::from_secs(25);
