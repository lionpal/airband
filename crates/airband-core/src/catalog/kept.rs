//! What a copy of the catalogue that was looked at is worth saying. Keeping one and fetching a newer one are
//! `airband_io::catalog`'s: core touches neither the disk nor the network.
use super::*;

/// The name a copy of the catalogue is kept under, by the form of the document.
pub const KEPT: &str = "catalog.json";

/// What to write in a log about a copy of the catalogue that was just looked at: what it holds that this
/// build cannot use, so an offer that vanished is never a mystery. `speaks` says whether this program has the
/// code for an offer.
pub fn news(taken: &Taken, speaks: &dyn Fn(&crate::providers::Offer) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = taken.note.iter().map(|n| format!("catalog: {n}")).collect();
    if taken.newer {
        let now = current();
        let unspoken: Vec<String> = now.providers.iter().flat_map(|p| p.offers.iter().filter(|o| !speaks(o)).map(move |o| format!("{} by \"{}\"", p.id, o.by.interface))).collect();
        if !taken.left_out.is_empty() {
            lines.push(format!("catalog rev {}: left out {} rows this build cannot read ({})", now.rev, taken.left_out.len(), taken.left_out.join("; ")));
        }
        if !unspoken.is_empty() {
            lines.push(format!("catalog rev {}: {} offers are for a newer build and are not offered here ({})", now.rev, unspoken.len(), unspoken.join(", ")));
        }
    }
    lines
}
