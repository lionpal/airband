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
#![expect(clippy::disallowed_methods, reason = "lists the versions of a model kept on this device, by when each was made whole")]
use super::*;
use crate::engines::HostDevice;
use std::path::{Path, PathBuf};

pub(crate) use airband_core::catalog::PhoneModel as Model;

/// The models in the catalogue that this build can run on this device (`engines::runs`). Each says which
/// version of its files it is: that is the folder they are kept in.
pub(crate) fn models(device: &HostDevice) -> impl Iterator<Item = Arc<Model>> + '_ {
    airband_core::catalog::current().models.clone().into_iter().filter(move |m| crate::engines::runs(m, device) && !m.version.is_empty())
}

/// The model a row marks as the one to start with for a job (`usual`), among those that run on this device.
pub(crate) fn usual(device: &HostDevice, job: airband_core::providers::Job) -> Option<Arc<Model>> {
    models(device).find(|m| m.job == job && m.with.get(field::USUAL).and_then(|u| u.as_bool()) == Some(true))
}

pub(crate) mod field;
mod fetch;
pub(crate) use fetch::{fetch_local_model, local_model_progress};

pub(crate) fn model(kind: &str, device: &HostDevice) -> Option<Arc<Model>> {
    models(device).find(|m| m.kind == kind)
}

/// Everything of one model on this device, under the app's folder: a folder for each version of it.
pub(super) fn home(dir: &str, m: &Model) -> PathBuf {
    Path::new(dir).join("models").join(&m.kind)
}
/// Where the version the catalogue now names is kept.
pub(super) fn version_folder(dir: &str, m: &Model) -> PathBuf {
    home(dir, m).join(&m.version)
}
/// Left in a version's folder when every file of it has been fetched and checked: it says the folder is whole
/// without the catalogue row it was fetched by, which may since have moved on.
pub(super) const WHOLE: &str = ".whole";

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
    // the one made whole last, by when its mark was written: a version's name says nothing about its order
    std::fs::read_dir(home(dir, m)).into_iter().flatten().flatten().map(|e| e.path()).filter_map(|p| std::fs::metadata(p.join(WHOLE)).and_then(|f| f.modified()).ok().map(|at| (at, p))).max_by_key(|(at, _)| *at).map(|(_, p)| p)
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

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::providers::Job;

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

    /// A model like one in the catalogue, with two small files.
    fn small(version: &str) -> Model {
        Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Voice, engine: "ptts".into(), version: version.into(), base: "https://airband.dev/models/tiny".into(), files: vec![("a.bin".into(), 3, "0".repeat(64)), ("voices/b.bin".into(), 2, "0".repeat(64))], voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with: Default::default() }
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

    #[test]
    fn of_two_earlier_versions_the_one_made_whole_last_runs_whatever_their_names() {
        let dir = std::env::temp_dir().join(format!("airband-models-order-{}", random_hex(4)));
        let at = dir.display().to_string();
        // "10" sorts before "9" as text; 10 was made whole after 9
        for v in ["9", "10"] {
            put(&dir.join(format!("models/tiny/{v}")));
            std::fs::write(dir.join(format!("models/tiny/{v}/.whole")), "").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // the catalogue names a version not here yet
        assert_eq!(usable(&at, &small("11")), Some(dir.join("models/tiny/10")));
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
        let device = HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX, team_colours: 0 };
        let m = model(&kind, &device).unwrap();
        fetch_local_model(dir.clone(), &m).unwrap();
        assert!(fetched(&dir, &m) && local_model_progress() == 100);
        println!("{kind}: fetched to {} in {:.0} s", folder(&dir, &m).display(), began.elapsed().as_secs_f32());
    }
}
