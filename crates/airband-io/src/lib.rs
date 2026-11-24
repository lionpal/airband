//! The edges every airband process shares, so that `airband-core` touches neither the disk nor the network:
//! - [`files`]: writing a file whole or not at all, with the permissions it should have
//! - [`kept`]: a file of ours that is read once and saved again and again, and what happens when it is damaged,
//!   decided here once for the tower, the phone and the relay
//! - [`foreign`]: a file of another tool's, which is never set aside, never taken for empty and never written
//!   over when it cannot be read
//! - [`http`]: the one HTTP client, and reading what a service answered
//! - [`catalog`]: the published catalogue, fetched and kept
//! - [`child`]: every program started, handed its standard input, output and error and nothing else
#![warn(clippy::let_underscore_must_use)]
// test code may touch files and build JSON by hand to stand in for the outside
#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_macros, clippy::let_underscore_must_use))]
// this crate is where files and the network are reached, for every other crate: each module that does says so
// at its top, so a module of this crate is held to the same rules as any other
pub mod catalog;
pub mod child;
pub mod files;
pub mod foreign;
pub mod http;
pub mod kept;
