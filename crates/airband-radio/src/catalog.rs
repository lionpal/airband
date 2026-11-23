//! The catalogue this device uses: the newest it has kept, and a newer one fetched when he opens a chooser.
use std::path::Path;

/// The setting a voice service's voice is chosen by, for a host that lays out its own voice picker.
#[uniffi::export]
pub fn voice_setting() -> String {
    crate::models::field::VOICE.to_string()
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

