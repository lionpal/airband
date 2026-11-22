//! Every rule the tower and the radio share. The network is reached in one place, fetching the published
//! catalogue (`catalog::refresh`), through the one client in [`net`]; `providers::Answering::sent` reads what a
//! service answered a request the caller sent. Files are touched only through [`kept`] and the kept copy of the
//! catalogue (`catalog::load_kept`). Everything else is tested directly.
//!
//! Two versioned protocols meet at the tower (docs/protocols.md):
//! - [`agent`], `airband.agent/1`: how any coding agent, on any harness or model, reports in and is spoken to.
//! - [`radio`], `airband.radio/3`: how a phone, car or desktop app hears transmissions and sends replies.
//! - [`relay`], `airband.relay/1`: how a radio reaches a tower that is behind a home router, through a relay
//!   that can read none of it. [`pairing`] is the link that sets a radio up for either route.
#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

pub mod agent;
pub mod audio;
pub mod catalog;
pub mod clock;
pub mod encode;
pub mod kept;
pub mod links;
pub mod net;
pub mod pairing;
pub mod prose;
pub mod providers;
pub mod radio;
pub mod redact;
pub mod relay;
pub mod routing;
pub mod token;
pub mod tools;
