//! What a computer and a room are known by on this device.
use super::*;

/// What names a computer to this device, from its pairing link: nothing secret, the same every time.
fn station_key(link: &str) -> Option<String> {
    match Pairing::parse(link).ok()? {
        Pairing::Relay { secret, .. } | Pairing::Near { secret, .. } => Keys::derive(&secret).ok().map(|k| k.channel),
        Pairing::Direct { .. } => None,
    }
}
/// What a computer is known by in the rooms of his screen: its relay channel, or for a plain direct pairing,
/// a hash of its link (the link holds a token, and a room's id is kept on disk).
pub(crate) fn station_id(link: &str) -> String {
    station_key(link).unwrap_or_else(|| airband_core::relay::token_hash(link))
}
/// A room on his screen: one team on one computer, as one id. Only ever compared whole.
pub(crate) fn room_id(station: &str, team: &str) -> String {
    if team.is_empty() { String::new() } else { format!("{station}/{team}") }
}
