//! Models that run on the device itself: a voice, and a listening model. No key, no computer, no relay, and
//! nothing he says or hears leaves the device.
//!
//! The program that runs a model is part of the app; the model is not. He chooses one on a settings screen and
//! only then is it fetched, once, into the app's own folder. The files come from airband's own address,
//! already in the form the device runs, each checked against its hash before it takes its name.
//!
//! Each model is a row of the catalogue (`catalog/models/`, see `airband_core::catalog`): what job it does,
//! what to show, what to fetch, where it goes and what its engine is told about it. Where he chooses
//! (`choices.rs`) lists them beside the services; a host names none itself.
use super::*;
use airband_core::providers::{Job, Offer};
use crate::engines::HostDevice;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

pub(crate) use airband_core::catalog::PhoneModel as Model;

/// The models in the catalogue that this build can run on this device (`engines::runs`). Each says which
/// version of its files it is: that is the folder they are kept in.
pub(crate) fn models(device: &HostDevice) -> impl Iterator<Item = Arc<Model>> + '_ {
    airband_core::catalog::current().models.clone().into_iter().filter(move |m| crate::engines::runs(m, device) && !m.version.is_empty())
}

/// The name a copy of the catalogue is kept under, by the form of the document.
const KEPT: &str = "catalog.json";

/// What to write in a log about a copy of the catalogue that was just looked at: what it holds that this
/// build cannot use, so an offer that vanished is never a mystery. `speaks` says whether this program has the
/// code for an offer.
fn news(taken: &airband_core::catalog::Taken, speaks: &dyn Fn(&Offer) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = taken.note.iter().map(|n| format!("catalogue: {n}")).collect();
    if taken.newer {
        let now = airband_core::catalog::current();
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
pub fn catalog_load(dir: &Path, speaks: &dyn Fn(&Offer) -> bool) -> Vec<String> {
    let at = dir.join(KEPT);
    match (std::fs::read(&at), std::fs::read_to_string(at.with_extension("json.sig"))) {
        (Ok(doc), Ok(sig)) => match airband_core::catalog::accept(&doc, &sig) {
            Ok(taken) => news(&taken, speaks),
            Err(why) => vec![format!("catalogue: the copy kept here was not used ({why})")],
        },
        _ => Vec::new(),
    }
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// it is now the one in use, with what is worth a line in a log. Blocks for a few seconds at most; does
/// nothing with no network. The one place it is fetched, for the app and the tower alike.
pub fn catalog_refresh(dir: &Path, speaks: &dyn Fn(&Offer) -> bool) -> (bool, Vec<String>) {
    let agent = crate::net::agent(Duration::from_secs(6));
    let get = |url: &str| agent.get(url).call().ok().filter(|r| r.status().as_u16() == 200).and_then(|mut r| r.body_mut().read_to_vec().ok());
    let address = airband_core::catalog::ADDRESS;
    let (Some(doc), Some(sig)) = (get(address), get(&format!("{address}.sig"))) else { return (false, Vec::new()) };
    let sig = String::from_utf8_lossy(&sig).to_string();
    match airband_core::catalog::accept(&doc, &sig) {
        Ok(taken) => {
            if taken.newer {
                let at = dir.join(KEPT);
                let _ = std::fs::write(&at, &doc);
                let _ = std::fs::write(at.with_extension("json.sig"), sig);
            }
            (taken.newer, news(&taken, speaks))
        }
        Err(why) => (false, vec![format!("catalogue: the published copy was not taken ({why})")]),
    }
}

/// What this device does itself is speaking and listening; reading is his computer's.
fn this_device_speaks(offer: &Offer) -> bool {
    match offer.job {
        Job::Voice => crate::voices::speaks(offer),
        Job::Ears => crate::hears::listens(offer),
        Job::Summary => true,
    }
}

/// Use the newest catalogue this device has kept (in `dir`, a folder of the app's own), if it is newer than
/// the one built in. Nothing is fetched. Call once at the start.
#[uniffi::export]
pub fn load_catalog(dir: String) {
    catalog_load(Path::new(&dir), &this_device_speaks).iter().for_each(|line| eprintln!("{line}"));
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// what a chooser lists may have changed. Blocks for a few seconds at most; does nothing with no network.
#[uniffi::export]
pub fn refresh_catalog(dir: String) -> bool {
    let (changed, lines) = catalog_refresh(Path::new(&dir), &this_device_speaks);
    lines.iter().for_each(|line| eprintln!("{line}"));
    changed
}

pub(crate) fn model(kind: &str, device: &HostDevice) -> Option<Arc<Model>> {
    models(device).find(|m| m.kind == kind)
}

/// Everything of one model on this device, under the app's folder: a folder for each version of it.
fn home(dir: &str, m: &Model) -> PathBuf {
    Path::new(dir).join("models").join(&m.kind)
}
/// Where the version the catalogue now names is kept.
fn version_folder(dir: &str, m: &Model) -> PathBuf {
    home(dir, m).join(&m.version)
}
/// Left in a version's folder when every file of it has been fetched and checked: it says the folder is whole
/// without the catalogue row it was fetched by, which may since have moved on.
const WHOLE: &str = ".whole";

/// The version the catalogue names is on this device, whole: each file was checked against its hash before
/// it took its name.
pub(crate) fn current(dir: &str, m: &Model) -> bool {
    m.files.iter().all(|(to, size, _)| std::fs::metadata(version_folder(dir, m).join(to)).is_ok_and(|f| f.is_file() && f.len() == *size))
}

/// Where to run the model from: the version the catalogue names when it is here whole; else a version
/// fetched earlier, which stays in use until the new one is whole. Nothing when none is here.
pub(crate) fn usable(dir: &str, m: &Model) -> Option<PathBuf> {
    if current(dir, m) {
        return Some(version_folder(dir, m));
    }
    let mut earlier: Vec<PathBuf> = std::fs::read_dir(home(dir, m)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join(WHOLE).is_file()).collect();
    earlier.sort();
    earlier.pop()
}

/// Where a model's files are, to run it: see [`usable`]. Before any are here, where they will be.
pub(crate) fn folder(dir: &str, m: &Model) -> PathBuf {
    usable(dir, m).unwrap_or_else(|| version_folder(dir, m))
}
/// The model can be run on this device now.
pub(crate) fn fetched(dir: &str, m: &Model) -> bool {
    usable(dir, m).is_some()
}

/// One of the voices a voice model can speak in.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VoiceChoice {
    /// What to pass back as the voice.
    pub name: String,
    /// What to call it where he chooses.
    pub label: String,
    /// A few words on how it sounds.
    pub about: String,
    /// A short clip of it, to hear before choosing: an address the host can fetch and play, whether or not
    /// the model is on the device yet.
    pub preview: String,
}

/// How far the fetch now running has got: bytes so far, and bytes in all.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());

/// How far the fetch now running has got, in hundredths. For a progress bar, asked a few times a second.
pub(crate) fn local_model_progress() -> u32 {
    let of = OF.load(Ordering::SeqCst);
    (GOT.load(Ordering::SeqCst).min(of) * 100).checked_div(of).unwrap_or(0) as u32
}

/// Fetch a model. Blocks until it is all here: call it off the main thread. Files already here are kept, so
/// one that was cut short goes on from the last whole file.
pub(crate) fn fetch_local_model(dir: String, m: &Model) -> Result<(), RadioError> {
    let told = |reason: &str| RadioError::Told { reason: reason.into() };
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = version_folder(&dir, m);
    GOT.store(0, Ordering::SeqCst);
    OF.store(m.files.iter().map(|f| f.1).sum(), Ordering::SeqCst);
    // no limit on the whole of it (a model is tens or hundreds of megabytes on whatever connection he has),
    // only on a stall
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    for (to, size, sha) in &m.files {
        let path = folder.join(to);
        if std::fs::metadata(&path).is_ok_and(|f| f.len() == *size) {
            GOT.fetch_add(*size, Ordering::SeqCst);
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|_| told("Could not make room for the model on this device."))?;
        let part = path.with_extension("part");
        let mut res = agent.get(format!("{}/{to}", m.base)).call().map_err(|_| told("Could not reach the download. Check the connection and try again."))?;
        if res.status().as_u16() != 200 {
            return Err(told("The download is not there."));
        }
        let mut file = std::fs::File::create(&part).map_err(|_| told("Could not write the model to this device."))?;
        let mut body = res.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut hash = <sha2::Sha256 as sha2::Digest>::new();
        let before = GOT.load(Ordering::SeqCst);
        loop {
            let n = match std::io::Read::read(&mut body, &mut buf) {
                Ok(n) => n,
                Err(_) => {
                    GOT.store(before, Ordering::SeqCst);
                    return Err(told("The download stopped part way. Try again."));
                }
            };
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buf[..n]).map_err(|_| told("This device ran out of room for the model."))?;
            sha2::Digest::update(&mut hash, &buf[..n]);
            GOT.fetch_add(n as u64, Ordering::SeqCst);
        }
        drop(file);
        // a sign-in page, a connection that closed early, a file changed at the source: none of them is the model
        let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
        if got != *sha {
            let _ = std::fs::remove_file(&part);
            GOT.store(before, Ordering::SeqCst);
            return Err(told("What came down was not the model. Check the connection and try again."));
        }
        std::fs::rename(&part, &path).map_err(|_| told("Could not write the model to this device."))?;
    }
    // This version is whole: say so, and only now let go of the one that was here before it, which was in use
    // until this moment.
    let _ = std::fs::write(folder.join(WHOLE), "");
    for other in std::fs::read_dir(home(&dir, m)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| *p != folder) {
        let _ = if other.is_dir() { std::fs::remove_dir_all(&other) } else { std::fs::remove_file(&other) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_says_what_to_fetch_and_who_made_it() {
        // every model in the catalogue, whoever can run it
        let models = || airband_core::catalog::current().models.clone().into_iter();
        for m in models() {
            assert!(!m.files.is_empty() && !m.credit.is_empty() && m.base.starts_with("https://"), "{}", m.kind);
            assert!(m.files.iter().all(|(to, size, sha)| !to.starts_with('/') && *size > 0 && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())), "{}", m.kind);
            assert_eq!(models().filter(|o| o.kind == m.kind).count(), 1, "{} is listed twice", m.kind);
        }
        // nothing is here until it is fetched
        let dir = std::env::temp_dir().join(format!("airband-models-{}", random_hex(4))).display().to_string();
        assert!(models().all(|m| !fetched(&dir, &m)));
    }

    #[test]
    fn what_a_copy_holds_that_this_build_cannot_use_is_said() {
        let taken = airband_core::catalog::Taken { newer: true, left_out: vec!["x: an offer (missing field `label`)".into()], note: None };
        // a build that speaks nothing: every offer in hand is for another build
        let lines = news(&taken, &|_| false);
        assert!(lines.len() == 2 && lines[0].contains("left out 1 rows") && lines[1].contains("for a newer build"), "{lines:?}");
        // one that speaks everything, and a copy with nothing odd in it: nothing to say
        assert!(news(&airband_core::catalog::Taken { newer: true, ..Default::default() }, &|_| true).is_empty());
        // a copy that was not taken says only what is worth knowing about that
        let same = airband_core::catalog::Taken { note: Some("a catalogue with the same rev (5) has different rows; the one in hand is kept".into()), ..Default::default() };
        assert_eq!(news(&same, &|_| false).len(), 1);
    }

    /// A model like one in the catalogue, with two small files.
    fn small(version: &str) -> Model {
        Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Voice, engine: "ptts".into(), version: version.into(), base: "https://airband.dev/models/tiny".into(), files: vec![("a.bin".into(), 3, "0".repeat(64)), ("voices/b.bin".into(), 2, "0".repeat(64))], voices: Vec::new(), credit: String::new(), with: Default::default() }
    }
    fn put(folder: &Path) {
        std::fs::create_dir_all(folder.join("voices")).unwrap();
        std::fs::write(folder.join("a.bin"), "abc").unwrap();
        std::fs::write(folder.join("voices/b.bin"), "de").unwrap();
    }

    #[test]
    fn a_version_here_stays_in_use_until_the_new_one_is_whole() {
        let dir = std::env::temp_dir().join(format!("airband-models-{}", random_hex(4)));
        let at = dir.display().to_string();
        let (one, two) = (small("1"), small("2"));
        // version 1 is here, whole
        put(&dir.join("models/tiny/1"));
        std::fs::write(dir.join("models/tiny/1/.whole"), "").unwrap();
        assert!(fetched(&at, &one) && current(&at, &one));
        assert_eq!(folder(&at, &one), dir.join("models/tiny/1"));
        // the catalogue moves on to version 2: version 1 is still what runs, and it is known to be behind
        assert!(fetched(&at, &two) && !current(&at, &two));
        assert_eq!(folder(&at, &two), dir.join("models/tiny/1"));
        // half of version 2 changes nothing
        std::fs::create_dir_all(dir.join("models/tiny/2")).unwrap();
        std::fs::write(dir.join("models/tiny/2/a.bin"), "abc").unwrap();
        assert_eq!(folder(&at, &two), dir.join("models/tiny/1"));
        // whole: it is the one that runs
        put(&dir.join("models/tiny/2"));
        assert!(current(&at, &two));
        assert_eq!(folder(&at, &two), dir.join("models/tiny/2"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The real download, from where the app gets it, every file checked against its hash. Run by hand:
    /// `AIRBAND_MODEL_DIR=<folder> AIRBAND_MODEL=phonon-2.5-en cargo test --release -p airband-radio fetches -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fetches_a_model_whole() {
        let (dir, kind) = (std::env::var("AIRBAND_MODEL_DIR").unwrap(), std::env::var("AIRBAND_MODEL").unwrap());
        let began = std::time::Instant::now();
        // a phone that has every engine the catalogue names
        let device = HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX };
        let m = model(&kind, &device).unwrap();
        fetch_local_model(dir.clone(), &m).unwrap();
        assert!(fetched(&dir, &m) && local_model_progress() == 100);
        println!("{kind}: fetched to {} in {:.0} s", folder(&dir, &m).display(), began.elapsed().as_secs_f32());
    }
}
