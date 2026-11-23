//! The catalogue this device uses: the newest it has kept, and a newer one fetched when he opens a chooser.
use std::path::Path;

/// The setting a voice service's voice is chosen by, for a host that lays out its own voice picker.
#[uniffi::export]
pub fn voice_setting() -> String {
    crate::models::field::VOICE.to_string()
}

/// Who made each model the catalog lists and under what license, each once and as the catalog words it, for
/// the host to show as they are. Every row is credited, whether or not this device can run it.
#[uniffi::export]
pub fn acknowledgments() -> Vec<String> {
    let catalog = airband_core::catalog::current();
    let mut said: Vec<String> = Vec::new();
    for credit in catalog.models.iter().map(|m| &m.credit).filter(|c| !c.is_empty()) {
        if !said.contains(credit) {
            said.push(credit.clone());
        }
    }
    said
}

/// Use the newest catalogue this device has kept (in `dir`, a folder of the app's own), if it is newer than
/// the one built in. Nothing is fetched. Call once at the start.
#[uniffi::export]
pub fn load_catalog(dir: String) {
    airband_io::catalog::load_kept(Path::new(&dir), &crate::can_use).iter().for_each(|line| eprintln!("{line}"));
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// what a chooser lists may have changed. Blocks for a few seconds at most; does nothing with no network.
#[uniffi::export]
pub fn refresh_catalog(dir: String) -> bool {
    let (changed, lines) = airband_io::catalog::refresh(Path::new(&dir), &crate::can_use);
    lines.iter().for_each(|line| eprintln!("{line}"));
    changed
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_credit_the_catalog_carries_is_acknowledged() {
        let catalog = airband_core::catalog::current();
        let listed = acknowledgments();
        let credited: Vec<&String> = catalog.models.iter().map(|m| &m.credit).filter(|c| !c.is_empty()).collect();
        assert!(!credited.is_empty(), "the catalog credits no one");
        for credit in &credited {
            assert_eq!(listed.iter().filter(|l| l == credit).count(), 1, "{credit}");
        }
        // nothing else is listed
        assert!(listed.iter().all(|l| credited.contains(&l)));
    }
}
