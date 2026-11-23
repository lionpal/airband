//! The programs that run a model on the device, by the name a model's row gives as its `engine`.
//!
//! An engine is either part of this build (a row of `HERE`, one module each under `engines/`) or the host's
//! own (the host says which it has, and is asked to run it through `run_model`). Which it is makes no
//! difference to what uses a model: a voice and a listening model are both a [`Loaded`], whoever runs them.
//!
//! A new model for an engine that is here is a row of the catalogue. A new engine in this build is one module
//! and one row of `HERE`; a new engine in a host is that host saying its name and answering `run_model` for it.
#![expect(clippy::disallowed_macros, reason = "the host engine's input")]
use super::*;
use crate::models::Model;
use std::path::{Path, PathBuf};

#[cfg(feature = "phonon")]
mod ptts;

/// How an engine the host has is asked to run a model: the engine's name, the folder the model's files are
/// in, the job, and what to work on. For listening that is a recording (16-bit little-endian mono PCM at 16
/// kHz) and the answer is the words, as UTF-8. For a voice it is JSON, `{"text": the words, "voice": the
/// voice's name}`, and the answer is the sound (16-bit little-endian mono PCM at the rate the model's row
/// gives). Nothing when the host could not.
pub(crate) type ByHost<'a> = &'a dyn Fn(&str, &str, ChoiceJob, Vec<u8>) -> Option<Vec<u8>>;

/// A model, loaded and ready to run.
pub(crate) trait Loaded: Send {
    /// Say `text` in `voice`, handing the sound to `sink` as it is made, until it is over or `stop` says to
    /// leave off.
    fn speak(&mut self, host: ByHost, voice: &str, text: &str, sink: &mut dyn FnMut(&[u8]), stop: &dyn Fn() -> bool) -> Result<(), String>;
    /// The words in a recording: 16-bit little-endian mono PCM at 16 kHz.
    fn hear(&mut self, host: ByHost, pcm: Vec<u8>) -> Result<String, String>;
}

/// A program in this build that runs models.
pub(crate) trait Engine: Sync {
    /// Load the model whose files are in `folder`, as its row describes it.
    fn load(&self, folder: &Path, m: &Model) -> Result<Box<dyn Loaded>, String>;
}

/// The engines in this build, by the name a row gives.
const HERE: &[(&str, &dyn Engine)] = &[
    #[cfg(feature = "phonon")]
    ("ptts", &ptts::Ptts),
];

/// What the device this runs on can do, as its host says.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct HostDevice {
    /// The engines the host runs itself, by the name a model's row gives.
    pub engines: Vec<String>,
    /// The operating system, as a model's row names it under `min_os` ("ios"), and its major version.
    pub os: String,
    pub os_version: u32,
    /// How many team colours the host draws with: the phone hands out places in that list, and no more.
    pub team_colours: u8,
}

/// Can this build, on this device, run the model? Its engine is here or the host's, and where the row says
/// which version of an operating system it needs, this is that system at that version or later.
pub(crate) fn runs(m: &Model, device: &HostDevice) -> bool {
    let engine = HERE.iter().any(|(name, _)| *name == m.engine) || device.engines.contains(&m.engine);
    let system = match m.with.get(crate::models::field::MIN_OS) {
        None => true,
        Some(needs) => needs.get(&device.os).and_then(|v| v.as_u64()).is_some_and(|least| u64::from(device.os_version) >= least),
    };
    engine && system
}

/// A model run by an engine the host has: nothing is loaded here, the host keeps it.
struct Hosted {
    engine: String,
    folder: String,
}

impl Loaded for Hosted {
    fn speak(&mut self, host: ByHost, voice: &str, text: &str, sink: &mut dyn FnMut(&[u8]), _: &dyn Fn() -> bool) -> Result<(), String> {
        let pcm = host(&self.engine, &self.folder, ChoiceJob::Voice, serde_json::to_vec(&serde_json::json!({ "text": text, "voice": voice })).expect("words write as JSON")).ok_or("this device could not run it")?;
        sink(&pcm);
        Ok(())
    }
    fn hear(&mut self, host: ByHost, pcm: Vec<u8>) -> Result<String, String> {
        let said = host(&self.engine, &self.folder, ChoiceJob::Ears, pcm).ok_or("this device could not run it")?;
        String::from_utf8(said).map_err(|_| "what it heard was not words".into())
    }
}

/// A model he chose, with what it takes to run it. It is loaded when first used and stays loaded for as long
/// as this is kept, so a sentence starts at once.
pub(crate) struct Held {
    pub(crate) m: Model,
    folder: PathBuf,
    loaded: Mutex<Option<Box<dyn Loaded>>>,
}

impl Held {
    pub(crate) fn of(m: &Model, folder: PathBuf) -> Arc<Held> {
        Arc::new(Held { m: m.clone(), folder, loaded: Mutex::new(None) })
    }

    /// Is this that model, from that folder? Then it need not be loaded again.
    pub(crate) fn is(&self, m: &Model, folder: &Path) -> bool {
        self.m == *m && self.folder == folder
    }

    /// Do something with the model, loading it first if it is not loaded. One thing at a time.
    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut dyn Loaded) -> T) -> Result<T, Fault> {
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.is_none() {
            *loaded = Some(match HERE.iter().find(|(name, _)| *name == self.m.engine) {
                Some((_, engine)) => engine.load(&self.folder, &self.m).map_err(Fault::Load)?,
                None => Box::new(Hosted { engine: self.m.engine.clone(), folder: self.folder.display().to_string() }),
            });
        }
        Ok(f(loaded.as_mut().unwrap().as_mut()))
    }
    /// Run the model at its job, loading it first: what the engine made, or what went wrong.
    pub(crate) fn run<T>(&self, f: impl FnOnce(&mut dyn Loaded) -> Result<T, String>) -> Result<T, Fault> {
        self.with(f).and_then(|ran| ran.map_err(Fault::Run))
    }
}

/// What went wrong with a model on this device, in the engine's own words (no capital, no full stop). It is
/// put in a sentence once, where the job is done ([`Fault::told`]), never wrapped again.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fault {
    Load(String),
    Run(String),
}

impl Fault {
    /// In a sentence for him, naming the model.
    pub(crate) fn told(&self, name: &str) -> String {
        match self {
            Fault::Load(why) => format!("{name} could not be loaded on this phone: {why}."),
            Fault::Run(why) => format!("{name} did not run on this phone: {why}."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::providers::Job;

    fn row(engine: &str, with: serde_json::Value) -> Model {
        Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Ears, engine: engine.into(), version: "1".into(), base: "https://airband.dev/models/tiny".into(), files: Vec::new(), voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with: serde_json::from_value(with).unwrap() }
    }

    #[test]
    fn a_model_is_offered_only_where_its_engine_and_its_system_are() {
        let phone = HostDevice { engines: vec!["host-new".into()], os: "ios".into(), os_version: 18, team_colours: 0 };
        // an engine nobody here has: not offered
        assert!(!runs(&row("unheard-of", serde_json::json!({})), &phone));
        // the host's own, with nothing said about the system
        assert!(runs(&row("host-new", serde_json::json!({})), &phone));
        assert!(!runs(&row("host-new", serde_json::json!({})), &HostDevice::default()));
        // the row says what it needs: this system at that version or later, and no other system
        assert!(runs(&row("host-new", serde_json::json!({"min_os": {"ios": 18}})), &phone));
        assert!(!runs(&row("host-new", serde_json::json!({"min_os": {"ios": 19}})), &phone));
        assert!(!runs(&row("host-new", serde_json::json!({"min_os": {"android": 12}})), &phone));
        // one in this build needs nothing of the host
        #[cfg(feature = "phonon")]
        assert!(runs(&row("ptts", serde_json::json!({})), &HostDevice::default()));
    }

    #[test]
    fn a_hosts_engine_is_asked_by_name_and_folder_for_either_job() {
        let held = Held::of(&row("host-new", serde_json::json!({})), PathBuf::from("/models/tiny/1"));
        let asked = Mutex::new(Vec::new());
        let host = |engine: &str, folder: &str, job: ChoiceJob, input: Vec<u8>| {
            asked.lock().unwrap().push((engine.to_string(), folder.to_string(), job, input));
            Some(match job {
                ChoiceJob::Ears => b"shop two".to_vec(),
                _ => vec![1, 0, 2, 0],
            })
        };
        assert_eq!(held.with(|m| m.hear(&host, vec![0, 0])).unwrap(), Ok("shop two".into()));
        let mut sound = Vec::new();
        held.with(|m| m.speak(&host, "Zoey", "Hello.", &mut |pcm| sound.extend_from_slice(pcm), &|| false)).unwrap().unwrap();
        assert_eq!(sound, [1, 0, 2, 0]);
        let asked = asked.into_inner().unwrap();
        assert_eq!(asked[0], ("host-new".into(), "/models/tiny/1".into(), ChoiceJob::Ears, vec![0, 0]));
        assert_eq!((&asked[1].0, &asked[1].2), (&"host-new".to_string(), &ChoiceJob::Voice));
        // words with a line in them reach the host whole, beside the voice
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&asked[1].3).unwrap(), serde_json::json!({ "text": "Hello.", "voice": "Zoey" }));
        // a host that cannot says so, and nothing is made up in its place
        assert_eq!(held.run(|m| m.hear(&|_, _, _, _| None, vec![0, 0])), Err(Fault::Run("this device could not run it".into())));
        // said once, naming the model (audit 5, B-18)
        assert_eq!(Fault::Run("this device could not run it".into()).told("Tiny"), "Tiny did not run on this phone: this device could not run it.");
    }
}
