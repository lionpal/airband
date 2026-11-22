//! The copy of the catalogue kept on this device, and fetching a newer one.
use super::*;

/// The name a copy of the catalogue is kept under, by the form of the document.
pub(super) const KEPT: &str = "catalog.json";

/// What to write in a log about a copy of the catalogue that was just looked at: what it holds that this
/// build cannot use, so an offer that vanished is never a mystery. `speaks` says whether this program has the
/// code for an offer.
pub(super) fn news(taken: &Taken, speaks: &dyn Fn(&crate::providers::Offer) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = taken.note.iter().map(|n| format!("catalogue: {n}")).collect();
    if taken.newer {
        let now = current();
        let unspoken: Vec<String> = now.providers.iter().flat_map(|p| p.offers.iter().filter(|o| !speaks(o)).map(move |o| format!("{} by \"{}\"", p.id, o.by.interface))).collect();
        if !taken.left_out.is_empty() {
            lines.push(format!("catalogue rev {}: left out {} rows this build cannot read ({})", now.rev, taken.left_out.len(), taken.left_out.join("; ")));
        }
        if !unspoken.is_empty() {
            lines.push(format!("catalogue rev {}: {} offers are for a newer build and are not offered here ({})", now.rev, unspoken.len(), unspoken.join(", ")));
        }
    }
    lines
}

/// Use the newest catalogue kept in `dir`, if it is newer than the one built in. Nothing is fetched. Returns
/// what is worth a line in a log. The one place a kept catalogue is read, for the app and the tower alike.
pub fn load_kept(dir: &Path, speaks: &dyn Fn(&crate::providers::Offer) -> bool) -> Vec<String> {
    let at = dir.join(KEPT);
    match (std::fs::read(&at), std::fs::read_to_string(at.with_extension("json.sig"))) {
        (Ok(doc), Ok(sig)) => match accept(&doc, &sig) {
            Ok(taken) => news(&taken, speaks),
            Err(why) => vec![format!("catalogue: the copy kept here was not used ({why})")],
        },
        _ => Vec::new(),
    }
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// it is now the one in use, with what is worth a line in a log. Blocks for a few seconds at most; does
/// nothing with no network. The one place it is fetched, for the app and the tower alike.
pub fn refresh(dir: &Path, speaks: &dyn Fn(&crate::providers::Offer) -> bool) -> (bool, Vec<String>) {
    let agent = crate::net::agent(Duration::from_secs(6));
    let get = |url: &str| agent.get(url).call().ok().filter(|r| r.status().as_u16() == 200).and_then(|mut r| r.body_mut().read_to_vec().ok());
    let address = ADDRESS;
    let (Some(doc), Some(sig)) = (get(address), get(&format!("{address}.sig"))) else { return (false, Vec::new()) };
    let sig = String::from_utf8_lossy(&sig).to_string();
    match accept(&doc, &sig) {
        Ok(taken) => {
            let mut lines = news(&taken, speaks);
            if taken.newer {
                // each written whole; a pair that could not both be written is said (the next start then reads
                // the one built in, as the signature no longer fits)
                let at = dir.join(KEPT);
                let kept = crate::kept::write_atomic(&at, &doc).and_then(|_| crate::kept::write_atomic(&at.with_extension("json.sig"), sig));
                if let Err(e) = kept {
                    lines.push(format!("catalogue: the copy taken could not be kept for next time ({e})"));
                }
            }
            (taken.newer, lines)
        }
        Err(why) => (false, vec![format!("catalogue: the published copy was not taken ({why})")]),
    }
}
