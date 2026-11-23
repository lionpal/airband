//! What a computer and a room are known by on this device.
use super::*;

/// What names a computer to this device, from its pairing link: nothing secret, the same every time. One
/// computer paired by two links (through the relay, and straight on its network, made at once with one
/// secret) is two entries in his list, each reconnected and sent to on its own: the two never share a name.
fn station_key(link: &str) -> Option<String> {
    match Pairing::parse(link).ok()? {
        Pairing::Relay { secret, .. } => Keys::derive(&secret).ok().map(|k| k.channel),
        Pairing::Near { secret, .. } => Keys::derive(&secret).ok().map(|k| format!("near-{}", k.channel)),
        Pairing::Direct { .. } => None,
    }
}
/// What a computer is known by in the rooms of his screen: its relay channel (what a notification from it
/// names), the same marked as reached on its network for a link straight to it, or for a plain direct
/// pairing, a hash of its link (the link holds a token, and a room's id is kept on disk).
pub(crate) fn station_id(link: &str) -> StationId {
    StationId::new(station_key(link).unwrap_or_else(|| airband_core::relay::token_hash(link)))
}
/// A room on his screen: one team on one computer, as one id. Only ever compared whole.
pub(crate) fn room_id(station: &StationId, team: &str) -> String {
    if team.is_empty() { String::new() } else { format!("{station}/{team}") }
}
/// A short name that is the same every launch for the same bytes: not a secret and not a check, only a name.
/// `from` is where it starts; each use keeps its own, so the names already on his phone stay the same.
pub(crate) fn same_name(from: u64, bytes: &[u8]) -> String {
    let h = bytes.iter().fold(from, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{h:x}")
}
/// A folder name that is the same every launch for the same link.
pub(super) fn folder(link: &str) -> String {
    same_name(1469598103934665603, link.as_bytes())
}
