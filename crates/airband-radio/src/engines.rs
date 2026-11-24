//! The programs that run a model on the device, by the name a model's row gives as its `engine`.
//!
//! An engine is either part of this build (a row of `HERE`, one module each under `engines/`) or the host's
//! own (the host says which it has, and is asked to run it through `run_model`). Which it is makes no
//! difference to what uses a model: a voice and a listening model are both a [`Loaded`], whoever runs them.
//!
//! A new model for an engine that is here is a row of the catalogue. A new engine in this build is one module
//! and one row of `HERE`; a new engine in a host is that host saying its name and answering `run_model` for it.
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
    /// The build of that system ("23A341"): what a model got ready for this device was got ready for. Work an
    /// engine keeps for a model (Core ML's compiled models) can be dropped when the system is updated.
    pub os_build: String,
    /// How many team colours the host draws with: the phone hands out places in that list, and no more.
    pub team_colours: u8,
}

/// Can this build, on this device, run the model? Its engine is here or the host's, and where the row says
/// which version of an operating system it needs, this is that system at that version or later.
pub(crate) fn runs(m: &Model, device: &HostDevice) -> bool {
    cannot_run(m, device).is_none()
}

/// Why this device cannot run the model, when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CannotRun {
    /// Its row names a newer version of this device's system than it has.
    OldSystem,
    /// Its row allows other systems, not this one, or nothing here or in the host runs its engine.
    NotHere,
}

impl CannotRun {
    /// In words for him.
    pub(crate) fn said(self) -> &'static str {
        match self {
            CannotRun::OldSystem => "This phone can't run it until its software is updated.",
            CannotRun::NotHere => "This phone can't run it.",
        }
    }
}

pub(crate) fn cannot_run(m: &Model, device: &HostDevice) -> Option<CannotRun> {
    let engine = HERE.iter().any(|(name, _)| *name == m.engine) || device.engines.contains(&m.engine);
    let system = match m.with.get(crate::models::field::MIN_OS) {
        None => Ok(()),
        Some(needs) => match needs.get(&device.os).and_then(|v| v.as_u64()) {
            None => Err(CannotRun::NotHere),
            Some(least) if u64::from(device.os_version) < least => Err(CannotRun::OldSystem),
            Some(_) => Ok(()),
        },
    };
    match (engine, system) {
        (false, _) => Some(CannotRun::NotHere),
        (true, Err(why)) => Some(why),
        (true, Ok(())) => None,
    }
}

/// A model run by an engine the host has: nothing is loaded here, the host keeps it.
struct Hosted {
    engine: String,
    folder: String,
}

impl Loaded for Hosted {
    fn speak(&mut self, host: ByHost, voice: &str, text: &str, sink: &mut dyn FnMut(&[u8]), _: &dyn Fn() -> bool) -> Result<(), String> {
        let pcm = host(&self.engine, &self.folder, ChoiceJob::Voice, serde_json::to_vec(&HostVoice { text, voice }).expect("words write as JSON")).ok_or("it gave back nothing")?;
        sink(&pcm);
        Ok(())
    }
    fn hear(&mut self, host: ByHost, pcm: Vec<u8>) -> Result<String, String> {
        let said = host(&self.engine, &self.folder, ChoiceJob::Ears, pcm).ok_or("it gave back nothing")?;
        String::from_utf8(said).map_err(|_| "what it heard was not words".into())
    }
}

/// A model he chose, with what it takes to run it. It is loaded when first used and stays loaded for as long
/// as this is kept, so a sentence starts at once.
pub(crate) struct Held {
    pub(crate) m: Model,
    folder: PathBuf,
    loaded: Mutex<Option<Box<dyn Loaded>>>,
    /// What its engine needs before first use is done on this device (for a Core ML model, compiling it for
    /// this phone's Neural Engine, which takes minutes): running it now takes moments, or as long as loading
    /// what was compiled, which the run waits for. Set from what is kept ([`crate::models::prepared`]) when it
    /// is made, and once it has run.
    ready: std::sync::atomic::AtomicBool,
}

impl Held {
    pub(crate) fn of(m: &Model, folder: PathBuf) -> Arc<Held> {
        Arc::new(Held { m: m.clone(), folder, loaded: Mutex::new(None), ready: std::sync::atomic::AtomicBool::new(false) })
    }

    /// It can be run now without the engine first doing minutes of work: its row names no such work, or that
    /// work is done.
    pub(crate) fn ready(&self) -> bool {
        !crate::models::needs_preparing(&self.m) || self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// What is kept says that work is done, or its engine could no longer load it.
    pub(crate) fn set_ready(&self, ready: bool) {
        self.ready.store(ready, std::sync::atomic::Ordering::SeqCst);
    }

    /// Is this that model, from that folder? Then it need not be loaded again.
    pub(crate) fn is(&self, m: &Model, folder: &Path) -> bool {
        self.m == *m && self.folder == folder
    }

    /// Do something with the model, loading it first if it is not loaded. One thing at a time for an engine
    /// in this build; a host's engine keeps the model itself, and is asked for each run as it comes (so
    /// getting it ready, which takes minutes, holds up nothing here: the quiet load at launch, getting it ready
    /// and his dictation each wait only on the host's engine).
    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut dyn Loaded) -> T) -> Result<T, Fault> {
        let Some((_, engine)) = HERE.iter().find(|(name, _)| *name == self.m.engine) else {
            return Ok(f(&mut Hosted { engine: self.m.engine.clone(), folder: self.folder.display().to_string() }));
        };
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.is_none() {
            *loaded = Some(engine.load(&self.folder, &self.m).map_err(Fault::Load)?);
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

/// What a host's own voice engine is given, as [`DeskHost::run_model`] says: the words, and the voice's name.
#[derive(serde::Serialize)]
struct HostVoice<'a> {
    text: &'a str,
    voice: &'a str,
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
        let phone = HostDevice { engines: vec!["host-new".into()], os: "ios".into(), os_version: 18, team_colours: 0, os_build: String::new() };
        // an engine nobody here has: not offered
        assert!(!runs(&row("unheard-of", serde_json::json!({})), &phone));
        // the host's own, with nothing said about the system
        assert!(runs(&row("host-new", serde_json::json!({})), &phone));
        assert!(!runs(&row("host-new", serde_json::json!({})), &HostDevice::default()));
        // the row says what it needs: this system at that version or later, and no other system
        assert!(runs(&row("host-new", serde_json::json!({"min_os": {"ios": 18}})), &phone));
        assert!(!runs(&row("host-new", serde_json::json!({"min_os": {"ios": 19}})), &phone));
        assert_eq!(cannot_run(&row("host-new", serde_json::json!({"min_os": {"ios": 19}})), &phone), Some(CannotRun::OldSystem));
        assert_eq!(cannot_run(&row("host-new", serde_json::json!({"min_os": {"android": 12}})), &phone), Some(CannotRun::NotHere));
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
        assert_eq!(held.run(|m| m.hear(&|_, _, _, _| None, vec![0, 0])), Err(Fault::Run("it gave back nothing".into())));
        // said once, naming the model
        assert_eq!(Fault::Run("it gave back nothing".into()).told("Tiny"), "Tiny did not run on this phone: it gave back nothing.");
    }
}
