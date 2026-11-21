//! Every rule the tower and the radio share. No network here, and the only files touched are the ones
//! [`kept`] is asked to write, so all of it is tested directly.
//!
//! Two versioned protocols meet at the tower (docs/protocols.md):
//! - [`agent`], `airband.agent/1`: how any coding agent, on any harness or model, reports in and is spoken to.
//! - [`radio`], `airband.radio/1`: how a phone, car or desktop app hears transmissions and sends replies.
//! - [`relay`], `airband.relay/1`: how a radio reaches a tower that is behind a home router, through a relay
//!   that can read none of it. [`pairing`] is the link that sets a radio up for either route.
pub mod agent;
pub mod audio;
pub mod catalog;
pub mod clock;
pub mod kept;
pub mod pairing;
pub mod providers;
pub mod radio;
pub mod redact;
pub mod relay;
pub mod routing;
pub mod speech;
pub mod tools;
