//! The edges every airband process shares, so that `airband-core` touches neither the disk nor the network:
//! - [`files`]: writing a file whole or not at all, with the permissions it should have
//! - [`kept`]: a file of ours that is read once and saved again and again, and what happens when it is damaged,
//!   decided here once for the tower, the phone and the relay
//! - [`foreign`]: a file of another tool's, which is never set aside, never taken for empty and never written
//!   over when it cannot be read
//! - [`http`]: the one HTTP client, and reading what a service answered
//! - [`catalog`]: the published catalogue, fetched and kept
pub mod catalog;
pub mod files;
pub mod foreign;
pub mod http;
pub mod kept;
