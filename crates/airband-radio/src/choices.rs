//! What he chooses, in one place: what writes his summaries, what listens and what speaks.
//!
//! Each job has one list ([`Desk::choices`]) and one way to pick from it ([`Desk::choose_for`]). What listens
//! and what speaks are this device's alone: a model fetched to it, or a service reached from it with a key of
//! his own. His computer has no part in either. At most one is chosen for a job at a time, and that rule is
//! here and nowhere else: the modules that do the listening and the speaking (`ears.rs`, `mouth.rs`) are
//! handed the one thing to use, or nothing.
//!
//! What he chose is kept by the core, in `choices.json` in the app's folder. Keys are not: the host keeps
//! them where its system keeps secrets ([`DeskHost::secret`]) and they are never handed back to a screen.
//!
//! A choice that stops working stays chosen and says why ([`ChoiceState::Failed`]). Nothing else is used in
//! its place: moving to another service is his to do.
//!
//! Summaries are written on his computer, so that list is the computer's own (`GET reads`), with the one in
//! use named by its id. Before any computer has been heard from, the list is this build's catalogue and the
//! choice waits until one is paired.
use super::*;
use crate::engines::HostDevice;
use crate::models::{Model, VoiceChoice};
use crate::services::{Service, ServiceAsk};
use airband_core::providers::Job;
use airband_core::radio::{OfferName, ReadsOffered, SetReads};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod desk;
mod list;
mod saved;
pub(crate) use desk::computer_reached;
#[cfg(test)]
pub(crate) use desk::switch_all;
use list::*;
use saved::{Pick, Saved, Waiting};

/// The jobs he chooses something for, in the order a first run asks about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChoiceJob {
    /// What reads his agents' messages and writes the short of them. Done on his computer.
    Reads,
    /// What turns his voice into words.
    Ears,
    /// What speaks to him.
    Voice,
}

impl ChoiceJob {
    fn job(self) -> Job {
        match self {
            ChoiceJob::Reads => Job::Summary,
            ChoiceJob::Ears => Job::Ears,
            ChoiceJob::Voice => Job::Voice,
        }
    }
}

/// Where a choice does its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ChoicePlace {
    /// A model on this device.
    Phone,
    /// A service, with a key or a sign-in of his own.
    Service,
}

/// How a choice stands.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ChoiceState {
    /// It can be used as it is.
    Ready,
    /// It takes a key, and none is kept for it.
    NeedsKey,
    /// Its model is not on this device yet.
    NeedsDownload { megabytes: u32 },
    /// Its model is here and works, and a newer version of it can be fetched.
    Stale { megabytes: u32 },
    /// Something is being done about it now. `fraction` is how far along, from 0 to 1, when that can be said.
    Working { label: String, fraction: Option<f64> },
    /// It is chosen and waits for a computer to be reached, which is where it is put to use.
    Waiting,
    /// It is chosen and did not do its job the last time it was tried, and why. It stays chosen.
    Failed { why: String },
}

/// One thing he can choose for a job.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Choice {
    /// What to pass back to [`Desk::choose_for`]. It says nothing by itself: what it stands for is `place`,
    /// `provider` and `offer`.
    pub id: String,
    pub place: ChoicePlace,
    /// For a service: its provider's id, which is also what its key is kept under (one key serves everything
    /// a provider offers). For a model on this device: the model's kind. Empty for his computer's choice.
    pub provider: String,
    /// For a service: which of its provider's offers this is. Empty otherwise.
    pub offer: String,
    /// What to call it.
    pub title: String,
    /// One line under that: where it runs, what it needs from him, what it costs.
    pub line: String,
    /// It costs him nothing to use: say so.
    pub free: bool,
    /// Not one of the few shown first: offered behind "More services", unless it is the one chosen.
    pub more: bool,
    /// It is the one chosen for this job.
    pub chosen: bool,
    pub state: ChoiceState,
    /// It takes a key.
    pub key: bool,
    /// A key is kept for it. Only that there is one: a key is never shown again.
    pub has_key: bool,
    /// What its key is called, for asking for it.
    pub key_name: String,
    /// Where its keys are made.
    pub keys_at: String,
    /// What else it can be told. Anything left out is the usual.
    pub asks: Vec<ServiceAsk>,
    /// The voices it can speak in, the usual one first. Empty for what is not a voice.
    pub voices: Vec<VoiceChoice>,
    /// Which of those speaks now, by name.
    pub voice: String,
    /// Who made it and under what licence, where that is to be shown.
    pub credit: String,
    /// Something worth a small line when it is the one chosen: how fast it last was.
    pub note: String,
}

/// Something being done about a choice right now.
#[derive(Clone)]
struct Work {
    job: ChoiceJob,
    id: String,
    label: String,
    /// A model is being fetched: how far along is known.
    fetching: bool,
}

/// What he chose, kept and put to use.
pub(crate) struct Chooser {
    /// The app's folder: `choices.json` is here, and the models under `models/`.
    dir: String,
    /// What this device can run, as its host said when the desk was made.
    device: HostDevice,
    saved: Mutex<Saved>,
    working: Mutex<Option<Work>>,
}

/// What he chose that this build has no code for, said one way: what it cannot do, and where to choose again.
fn cannot(what: &str, job: ChoiceJob) -> String {
    let section = if job == ChoiceJob::Voice { "Voice" } else { "Listening" };
    format!("This version of airband cannot {what}. Choose another in Settings, under {section}.")
}

fn told(reason: impl Into<String>) -> RadioError {
    RadioError::Told { reason: reason.into() }
}

impl Chooser {
    /// What was kept in `dir`, or a new install's choices. With no folder (a test), nothing is kept.
    pub(crate) fn open(dir: &str, device: HostDevice) -> Chooser {
        let kept: Option<Saved> = if dir.is_empty() { None } else { crate::kept::load_or_set_aside(&Path::new(dir).join("choices.json")) };
        let saved = Mutex::new(kept.unwrap_or_else(|| Saved::fresh(&device)));
        Chooser { dir: dir.into(), device, saved, working: Mutex::new(None) }
    }

    /// The coding tools he uses, by id, the one he uses most first.
    pub(crate) fn tools(&self) -> Vec<String> {
        self.saved().tools
    }

    fn saved(&self) -> Saved {
        self.saved.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Change what is kept, and keep it. Err when it could not be kept: it holds until the app closes.
    fn change(&self, with: impl FnOnce(&mut Saved)) -> Result<(), RadioError> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        with(&mut saved);
        if self.dir.is_empty() {
            return Ok(());
        }
        crate::kept::save(&Path::new(&self.dir).join("choices.json"), &*saved).map_err(|why| told(format!("{why} It holds until the app closes.")))
    }

    /// A model this device can run, by its kind.
    fn model(&self, kind: &str) -> Option<Arc<Model>> {
        crate::models::model(kind, &self.device)
    }

    /// Hand `ears.rs` and `mouth.rs` the one thing each is to use, from what he chose. What he chose but
    /// cannot be used as things are is handed over as that, with why: nothing takes its place.
    pub(crate) fn apply(&self, host: &dyn DeskHost, senses: &Senses) {
        let saved = self.saved();
        let key = |id: &str| host.secret(id.to_string()).unwrap_or_default();
        senses.ears.set(saved.pick(ChoiceJob::Ears).map(|ears| match ears.place {
            ChoicePlace::Phone => match self.model(&ears.id).filter(|m| m.job == Job::Ears) {
                None => crate::ears::Own::Cannot(cannot("run the listening model you chose", ChoiceJob::Ears)),
                Some(m) if !crate::models::fetched(&self.dir, &m) => crate::ears::Own::Cannot(format!("{} is not on this phone yet. Download it in Settings, under Listening.", m.name)),
                Some(m) => crate::ears::Own::Model(senses.ears.held(&m, crate::models::folder(&self.dir, &m))),
            },
            ChoicePlace::Service => {
                let with = crate::services::service_of(&ears.id, &ears.offer, &key(&ears.id), ears.set.clone().into_iter().collect());
                if with.offer(Job::Ears).is_some_and(|(_, o)| crate::hears::listens(&o)) { crate::ears::Own::Service(with) } else { crate::ears::Own::Cannot(cannot("listen with the service you chose", ChoiceJob::Ears)) }
            }
        }));
        senses.mouth.set(saved.pick(ChoiceJob::Voice).map(|voice| match voice.place {
            ChoicePlace::Phone => match self.model(&voice.id).filter(|m| m.job == Job::Voice) {
                None => crate::mouth::Own::Cannot(cannot("run the voice you chose", ChoiceJob::Voice)),
                Some(m) if !crate::models::fetched(&self.dir, &m) => crate::mouth::Own::Cannot(format!("{} is not on this phone yet. Download it in Settings, under Voice.", m.name)),
                // none picked yet: the voice the row names as its usual one
                Some(m) => match m.voices.iter().map(|v| &v.0).find(|name| name.as_str() == m.voice(&voice.voice)) {
                    Some(name) => crate::mouth::Own::Model { held: senses.mouth.held(&m, crate::models::folder(&self.dir, &m)), voice: name.clone() },
                    // a voice the model no longer has is not swapped for another of its voices
                    None => crate::mouth::Own::Cannot(format!("{} no longer has the voice {}. Choose another in Settings, under Voice.", m.name, voice.voice)),
                },
            },
            ChoicePlace::Service => {
                let with = crate::services::service_of(&voice.id, &voice.offer, &key(&voice.id), voice.set.clone().into_iter().collect());
                if crate::voices::synth(&with).is_ok() { crate::mouth::Own::Service(with) } else { crate::mouth::Own::Cannot(cannot("speak with the service you chose", ChoiceJob::Voice)) }
            }
        }));
    }

    fn work(&self, job: ChoiceJob, id: &str, label: impl Into<String>, fetching: bool) {
        *self.working.lock().unwrap_or_else(|e| e.into_inner()) = Some(Work { job, id: id.into(), label: label.into(), fetching });
    }
    fn rest(&self) {
        *self.working.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    fn at_work(&self) -> Option<(ChoiceJob, String, String, Option<f64>)> {
        self.working.lock().unwrap_or_else(|e| e.into_inner()).clone().map(|w| (w.job, w.id, w.label, w.fetching.then(|| crate::models::local_model_progress() as f64 / 100.0)))
    }

    fn choices(&self, job: ChoiceJob, host: &dyn DeskHost, senses: &Senses) -> Vec<Choice> {
        let saved = self.saved();
        let facts = Facts {
            models: crate::models::models(&self.device).collect(),
            here: &|m| (crate::models::fetched(&self.dir, m), crate::models::current(&self.dir, m)),
            has_key: &|id| host.secret(id.to_string()).is_some_and(|k| !k.is_empty()),
            failed: match job {
                ChoiceJob::Ears => senses.ears.failed(),
                ChoiceJob::Voice => senses.mouth.failed(),
                ChoiceJob::Reads => None,
            },
            work: self.at_work().filter(|w| w.0 == job).map(|w| (w.1, w.2, w.3)),
            note: match (job, senses.ears.last_heard()) {
                (ChoiceJob::Ears, Some((of, took))) => format!("Last heard {of:.1} s of speech in {took:.2} s"),
                _ => String::new(),
            },
        };
        list(job, &saved, &facts)
    }

    /// Get a model onto this device and ready to run: fetched if the version the catalogue names is not here,
    /// then, for one whose row says the device has work of its own to do first, run once.
    fn make_ready(&self, job: ChoiceJob, m: &Model, host: &dyn DeskHost) -> Result<(), RadioError> {
        if !crate::models::current(&self.dir, m) {
            self.work(job, &m.kind, format!("Downloading {}", m.name), true);
            host.changed();
            crate::models::fetch_local_model(self.dir.clone(), m)?;
        }
        let note = m.told("prepare");
        if !note.is_empty() {
            self.work(job, &m.kind, note, false);
            host.changed();
            // the first run is when the device fits the model to itself: have it do its job once, on nothing
            // much (a second of silence to hear, a word to say)
            let held = crate::engines::Held::of(m, crate::models::folder(&self.dir, m));
            let by: crate::engines::ByHost = &|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input);
            let ran = held.with(|loaded| match m.job {
                // in the voice he picked, or the one its row names as usual: never just the first it lists
                Job::Voice => loaded.speak(by, m.voice(&self.saved().pick(ChoiceJob::Voice).filter(|p| p.id == m.kind).map(|p| p.voice).unwrap_or_default()), "Ready.", &mut |_| {}, &|| false),
                _ => loaded.hear(by, vec![0u8; 32000]).map(|_| ()),
            });
            ran.and_then(|ran| ran).map_err(|why| told(format!("{} did not load on this phone: {why}.", m.name)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(here: &'a dyn Fn(&Model) -> (bool, bool), has_key: &'a dyn Fn(&str) -> bool) -> Facts<'a> {
        Facts { models: crate::models::models(&phone()).collect(), here, has_key, failed: None, work: None, note: String::new() }
    }
    /// A phone whose host runs every engine the catalogue names.
    fn phone() -> HostDevice {
        HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX }
    }
    fn chosen(list: &[Choice]) -> Vec<&str> {
        list.iter().filter(|c| c.chosen).map(|c| c.id.as_str()).collect()
    }
    const NOT_HERE: &dyn Fn(&Model) -> (bool, bool) = &|_| (false, false);
    const HERE: &dyn Fn(&Model) -> (bool, bool) = &|_| (true, true);
    const NO_KEYS: &dyn Fn(&str) -> bool = &|_| false;

    #[test]
    fn one_thing_is_chosen_for_a_job_whatever_was_picked() {
        let service = crate::services::services(Job::Ears).into_iter().find(|s| s.key).expect("a listening service that takes a key");
        let model = crate::models::models(&phone()).find(|m| m.job == Job::Ears).map(|m| m.kind.clone());
        let mut picks = vec![Pick { place: ChoicePlace::Service, id: service.kind.clone(), offer: service.offer.clone(), set: BTreeMap::new(), voice: String::new() }];
        picks.extend(model.clone().map(|id| Pick { place: ChoicePlace::Phone, id, offer: String::new(), set: BTreeMap::new(), voice: String::new() }));
        for pick in picks {
            let saved = Saved { ears: Some(pick.clone()), ..Saved::default() };
            let all = list(ChoiceJob::Ears, &saved, &facts(HERE, &|_| true));
            let want = match pick.place {
                ChoicePlace::Service => service_id(&pick.id, &pick.offer),
                ChoicePlace::Phone => pick.id.clone(),
            };
            assert_eq!(chosen(&all), vec![want.as_str()], "{pick:?}");
            // every id is offered once, and all of it is this device's: a model here or a service with his key
            assert!(all.iter().all(|c| all.iter().filter(|o| o.id == c.id).count() == 1));
            assert!(all.iter().all(|c| matches!(c.place, ChoicePlace::Phone | ChoicePlace::Service)));
        }
        // nothing kept is nothing chosen: his computer is not offered for listening or for the voice
        for job in [ChoiceJob::Ears, ChoiceJob::Voice] {
            assert!(chosen(&list(job, &Saved::default(), &facts(HERE, NO_KEYS))).is_empty());
        }
    }

    #[test]
    fn each_choice_says_how_it_stands() {
        let service = crate::services::services(Job::Voice).into_iter().find(|s| s.key).expect("a voice service that takes a key");
        let saved = Saved { voice: Some(Pick { place: ChoicePlace::Service, id: service.kind.clone(), offer: service.offer.clone(), set: BTreeMap::from([("voice".to_string(), "Kore".to_string())]), voice: String::new() }), ..Saved::default() };
        let service_choice = service_id(&service.kind, &service.offer);
        let of = |all: &[Choice], id: &str| all.iter().find(|c| c.id == id).unwrap().clone();
        // no key kept: it says so, and asks for it by name
        let c = of(&list(ChoiceJob::Voice, &saved, &facts(HERE, NO_KEYS)), &service_choice);
        assert!(c.state == ChoiceState::NeedsKey && !c.has_key && c.line.starts_with("Enter your "), "{c:?}");
        // a key kept: ready, and the voice he picked is the one named
        let c = of(&list(ChoiceJob::Voice, &saved, &facts(HERE, &|_| true)), &service_choice);
        assert!(c.state == ChoiceState::Ready && c.has_key && c.voice == "Kore" && c.line.starts_with("Using your "), "{c:?}");
        // it failed the last time: still chosen, and it says why. What is not chosen is not blamed.
        let failing = Facts { failed: Some("no sound".into()), ..facts(HERE, &|_| true) };
        let all = list(ChoiceJob::Voice, &saved, &failing);
        assert_eq!(of(&all, &service_choice).state, ChoiceState::Failed { why: "no sound".into() });
        assert!(of(&all, &service_choice).chosen && all.iter().filter(|c| matches!(c.state, ChoiceState::Failed { .. })).count() == 1);
        // being worked on
        let busy = Facts { work: Some((service_choice.clone(), "Trying it".into(), None)), ..facts(HERE, &|_| true) };
        assert_eq!(of(&list(ChoiceJob::Voice, &saved, &busy), &service_choice).state, ChoiceState::Working { label: "Trying it".into(), fraction: None });
        // a model: not here, here but behind, here
        if let Some(m) = crate::models::models(&phone()).find(|m| m.job == Job::Voice) {
            let at = |here: &dyn Fn(&Model) -> (bool, bool)| of(&list(ChoiceJob::Voice, &Saved::default(), &facts(here, NO_KEYS)), &m.kind);
            assert!(matches!(at(NOT_HERE).state, ChoiceState::NeedsDownload { megabytes } if megabytes > 0) && at(NOT_HERE).line.contains("MB download"));
            assert!(matches!(at(&|_| (true, false)).state, ChoiceState::Stale { .. }));
            assert!(at(HERE).state == ChoiceState::Ready && at(HERE).free && at(HERE).line == "Runs on this phone");
            // with no voice named, its usual one is the one shown
            assert_eq!(at(HERE).voice, m.voices[0].0);
        }
    }

    #[test]
    fn summaries_are_listed_from_the_computer_and_chosen_by_its_id() {
        use airband_core::radio::ReadsOffer;
        let offer = |id: &str, login: &str, key: bool, has_key: bool| ReadsOffer { provider: id.into(), offer: "x".into(), label: format!("The {id} one"), login: login.into(), key, has_key, key_name: "API key".into(), preferred: true, ..ReadsOffer::default() };
        let named = |provider: &str, offer: &str| OfferName { provider: provider.into(), offer: offer.into() };
        let offered = ReadsOffered { now: Some(named("b", "x")), failed: None, offers: vec![offer("a", "", true, false), offer("b", "", true, true), offer("c", "codex", false, false), offer("d", "opencode", false, false)] };
        let saved = Saved { offered: Some(offered), tools: vec!["codex".into()], ..Saved::default() };
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, NO_KEYS));
        // his tool's sign-in first, another tool's left out, and the label is the computer's own
        assert_eq!(all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["c/x", "a/x", "b/x"]);
        assert_eq!(all[0].title, "The c one");
        // the one in use is the one the computer names, and a key the computer keeps counts as kept
        assert_eq!(chosen(&all), vec!["b/x"]);
        assert!(all[2].has_key && all[2].state == ChoiceState::Ready && all[1].state == ChoiceState::NeedsKey);
        // what he picked while it could not be reached is the one shown, as waiting
        let waiting = Saved { reads: Some(Waiting { id: "a".into(), offer: "x".into(), set: BTreeMap::new(), his: true, taken: Vec::new() }), ..saved };
        let all = list(ChoiceJob::Reads, &waiting, &facts(HERE, &|_| true));
        assert_eq!(chosen(&all), vec!["a/x"]);
        assert_eq!(all[1].state, ChoiceState::Waiting);
    }

    #[test]
    fn two_offers_of_one_provider_are_two_choices_under_one_key() {
        use airband_core::radio::ReadsOffer;
        // a computer that reads with either of two models of one company
        let offer = |id: &str| ReadsOffer { provider: "maker".into(), offer: id.into(), label: format!("Maker {id}"), key: true, key_name: "Maker key".into(), preferred: true, ..ReadsOffer::default() };
        let offered = ReadsOffered { now: Some(OfferName { provider: "maker".into(), offer: "small".into() }), failed: None, offers: vec![offer("small"), offer("large")] };
        let saved = Saved { offered: Some(offered), ..Saved::default() };
        // with no key kept both ask for the one key, by the same name
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, NO_KEYS));
        assert_eq!(all.iter().map(|c| (c.id.as_str(), c.provider.as_str(), c.offer.as_str())).collect::<Vec<_>>(), vec![("maker/small", "maker", "small"), ("maker/large", "maker", "large")]);
        assert!(all.iter().all(|c| c.state == ChoiceState::NeedsKey && c.key_name == "Maker key"));
        // a key is asked about by the provider's id, once, and then serves both; one of them is chosen
        let asked = Mutex::new(Vec::<String>::new());
        let has = |id: &str| {
            asked.lock().unwrap().push(id.to_string());
            id == "maker"
        };
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, &has));
        assert!(all.iter().all(|c| c.has_key && c.state == ChoiceState::Ready));
        assert!(asked.lock().unwrap().iter().all(|id| id == "maker"));
        assert_eq!(chosen(&all), vec!["maker/small"]);
        // his computer says what reads now could not, the last time it was asked: it stays chosen and says
        // why, and the other one is not blamed
        let mut failing = saved.clone();
        failing.offered.as_mut().unwrap().failed = Some(airband_core::providers::Trouble::WrongKey);
        let all = list(ChoiceJob::Reads, &failing, &facts(HERE, &|id| id == "maker"));
        assert_eq!(chosen(&all), vec!["maker/small"]);
        assert_eq!(all.iter().map(|c| c.state.clone()).collect::<Vec<_>>(), vec![ChoiceState::Failed { why: airband_core::providers::Trouble::WrongKey.said(&all[0].title) }, ChoiceState::Ready]);
        assert!(matches!(&all[0].state, ChoiceState::Failed { why } if why.ends_with("did not accept the key.")));

        // the same through the desk, for a job done on this device: a key typed for one offer is kept under
        // the provider and is the key the other offer finds
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        let host = Arc::new(Host::default());
        let desk = Desk::alone(&dir.display().to_string(), host.clone());
        let reads = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.key).unwrap();
        desk.choose_for(ChoiceJob::Reads, reads.id.clone(), Some("sk-one".into()), HashMap::new(), false).unwrap();
        assert_eq!(host.keys.lock().unwrap().keys().cloned().collect::<Vec<_>>(), vec![reads.provider.clone()]);
        for job in [ChoiceJob::Reads, ChoiceJob::Ears, ChoiceJob::Voice] {
            for c in desk.choices(job).into_iter().filter(|c| c.place == ChoicePlace::Service && c.key) {
                assert_eq!(c.has_key, c.provider == reads.provider, "{job:?} {}", c.id);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn before_any_computer_summaries_follow_the_coding_tool_until_he_picks() {
        let logins: Vec<Service> = crate::services::services(Job::Summary).into_iter().filter(|s| !s.login.is_empty()).collect();
        let (one, two) = (logins[0].clone(), logins.iter().find(|s| s.login != logins[0].login).expect("two tools with a sign-in that reads").clone());
        let mut saved = Saved::default();
        saved.use_tools(vec![one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| (w.id.as_str(), w.his)), Some((one.kind.as_str(), false)));
        // the other tool's sign-in is not offered to him, and his own is the one shown as chosen
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, NO_KEYS));
        assert!(chosen(&all) == vec![service_id(&one.kind, &one.offer).as_str()] && all.iter().all(|c| c.provider != two.kind));
        // he changes the tool: it follows
        saved.use_tools(vec![two.login.clone(), one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| w.id.as_str()), Some(two.kind.as_str()));
        // he picks for himself: a change of tool leaves that alone
        saved.reads = Some(Waiting { id: "anthropic".into(), offer: "haiku".into(), set: BTreeMap::new(), his: true, taken: Vec::new() });
        saved.use_tools(vec![one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| w.id.as_str()), Some("anthropic"));
        // and once a computer has said what it reads with, nothing is made to wait by a change of tool
        let mut said = Saved { offered: Some(ReadsOffered::default()), ..Saved::default() };
        said.use_tools(vec![one.login]);
        assert!(said.reads.is_none());
    }

    #[test]
    fn a_choice_the_phone_could_not_keep_says_so() {
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let chooser = Chooser::open(&dir.display().to_string(), HostDevice::default());
        // the folder choices are kept in can no longer be written
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let went = chooser.change(|s| s.tools = vec!["codex".into()]);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(went, Err(RadioError::Told { .. })), "{went:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_kept_choice_this_build_cannot_read_is_nothing_chosen_and_says_where_to_choose() {
        // kept by a build that let his computer listen and speak for the phone
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("choices.json"), r#"{"ears":{"place":"computer","id":"","offer":"","set":{},"voice":""},"voice":{"place":"computer","id":""},"tools":["codex"]}"#).unwrap();
        let host = Arc::new(Host::default());
        let desk = Desk::alone(&dir.display().to_string(), host.clone());
        // nothing is chosen for either, the rest of what was kept stands, and nothing else took their place
        for job in [ChoiceJob::Ears, ChoiceJob::Voice] {
            assert!(chosen(&desk.choices(job)).is_empty());
            assert_eq!(desk.chosen_label(job), "Nothing chosen");
        }
        assert_eq!(desk.tools(), vec!["codex".to_string()]);
        assert!(!desk.shared.senses.mouth.ready());
        // the microphone, opened: he is told where to choose, and nothing listened
        let tape = crate::ears_stream::Tape::of(&[], false);
        let ears = crate::ears::Ears::new(desk.shared.senses.clone());
        assert!(matches!(ears.hear(&tape, true), Listened::Cannot));
        assert_eq!(desk.shared.senses.ears.failed().as_deref(), Some(crate::ears::NOTHING_CHOSEN));
        // reading aloud, turned on: the same for the voice, and nothing is said
        desk.set_call_mode(true);
        assert_eq!(host.notes.lock().unwrap().last().map(String::as_str), Some(crate::voice::NOTHING_CHOSEN));
        assert!(matches!(crate::voice::say_until(&tape, &desk.shared.senses.mouth, "Shop two is waiting.", &|| false), crate::voice::Said::Nothing(why) if why == crate::voice::NOTHING_CHOSEN));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_install_starts_with_what_needs_nothing_from_him() {
        let saved = Saved::fresh(&phone());
        for (job, pick) in [(Job::Ears, saved.ears.clone()), (Job::Voice, saved.voice.clone())] {
            match crate::models::models(&phone()).find(|m| m.job == job && m.with.get("usual").and_then(|u| u.as_bool()) == Some(true)) {
                Some(m) => assert!(pick.is_some_and(|p| p.place == ChoicePlace::Phone && p.id == m.kind)),
                // with no model that runs here, nothing is chosen: nothing else is put in its place
                None => assert_eq!(pick, None),
            }
        }
        // a device no model runs on starts with nothing chosen for either
        let bare = Saved::fresh(&HostDevice::default());
        assert!(bare.ears.is_none() || crate::models::models(&HostDevice::default()).any(|m| m.job == Job::Ears));
        // which coding tools he uses is his to say
        assert!(saved.tools.is_empty() && saved.reads.is_none());
        // what is kept comes back as it was
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        let at = dir.display().to_string();
        let chooser = Chooser::open(&at, phone());
        chooser.change(|s| s.voice = Some(Pick { place: ChoicePlace::Service, id: "gemini".into(), offer: "flash-lite-tts".into(), set: BTreeMap::from([("voice".to_string(), "Leda".to_string())]), voice: String::new() })).unwrap();
        let again = Chooser::open(&at, phone());
        assert_eq!(again.saved(), chooser.saved());
        // and no key is in it
        assert!(!std::fs::read_to_string(dir.join("choices.json")).unwrap().contains("key"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A host that keeps keys in memory and has no microphone, speaker or model.
    #[derive(Default)]
    struct Host {
        keys: Mutex<HashMap<String, String>>,
        notes: Mutex<Vec<String>>,
    }
    impl DeskHost for Host {
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn changed(&self) {}
        fn status(&self, _: RadioStatus) {}
        fn note(&self, text: String) {
            self.notes.lock().unwrap().push(text);
        }
        fn hearing(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn watching(&self) -> bool {
            false
        }
        fn device(&self) -> HostDevice {
            phone()
        }
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
        fn secret(&self, name: String) -> Option<String> {
            self.keys.lock().unwrap().get(&name).cloned()
        }
        fn keep_secret(&self, name: String, value: Option<String>) {
            let mut keys = self.keys.lock().unwrap();
            match value {
                Some(v) => keys.insert(name, v),
                None => keys.remove(&name),
            };
        }
    }

    #[test]
    fn choosing_keeps_one_thing_and_what_cannot_work_says_so_and_is_not_replaced() {
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        let host = Arc::new(Host::default());
        let desk = Desk::alone(&dir.display().to_string(), host.clone());
        let none = HashMap::new;
        let state = |job, id: &str| desk.choices(job).into_iter().find(|c| c.id == id).map(|c| (c.chosen, c.state)).unwrap();
        // something not on the list is refused, and nothing changes
        assert!(matches!(desk.choose_for(ChoiceJob::Ears, "nothing-like-it".into(), None, none(), false), Err(RadioError::Told { .. })));
        // his computer is not something to listen or speak with
        assert!(matches!(desk.choose_for(ChoiceJob::Voice, "tower".into(), None, none(), false), Err(RadioError::Told { .. })));
        // a model noted for later: one at a time each time
        if let Some(m) = crate::models::models(&phone()).find(|m| m.job == Job::Voice) {
            desk.choose_for(ChoiceJob::Voice, m.kind.clone(), None, none(), true).unwrap();
            assert_eq!(chosen(&desk.choices(ChoiceJob::Voice)), vec![m.kind.as_str()]);
            assert!(matches!(state(ChoiceJob::Voice, &m.kind), (true, ChoiceState::NeedsDownload { .. })));
            // one of its voices, by name
            desk.choose_voice(m.voices[1].0.clone()).unwrap();
            assert_eq!(desk.choices(ChoiceJob::Voice).into_iter().find(|c| c.chosen).unwrap().voice, m.voices[1].0);
            // It is chosen and is not on this device: it is still the one that is to speak. Asked to, it says
            // why it cannot; nothing else speaks in its place, and the card says the same.
            assert!(desk.shared.senses.mouth.ready());
            let tape = crate::ears_stream::Tape::of(&[], false);
            let why = desk.shared.senses.mouth.say(&tape, "Shop two is waiting.", &|| false).unwrap_err();
            assert!(why.contains("is not on this phone yet"), "{why}");
            assert_eq!(state(ChoiceJob::Voice, &m.kind), (true, ChoiceState::Failed { why }));
        }
        // a service whose key is wrong is not taken: the one before it stays, and the key is not kept
        // (his own voice server, at an address where nothing answers: no call leaves this machine)
        let service = crate::services::services(Job::Voice).into_iter().find(|s| s.kind == "local");
        if let Some(service) = service {
            let (service, provider) = (service_id(&service.kind, &service.offer), service.kind);
            let before = chosen(&desk.choices(ChoiceJob::Voice)).iter().map(|s| s.to_string()).collect::<Vec<_>>();
            let refused = desk.choose_for(ChoiceJob::Voice, service.clone(), Some("k".into()), HashMap::from([("url".to_string(), "http://127.0.0.1:9/v1".to_string())]), false);
            assert!(refused.is_err());
            assert_eq!(chosen(&desk.choices(ChoiceJob::Voice)), before.iter().map(String::as_str).collect::<Vec<_>>());
            assert!(host.secret(provider).is_none());
        }
        // summaries, with no computer to ask: it waits, with its key kept by the host and not by the core
        let (reads, provider) = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.key).map(|c| (c.id, c.provider)).unwrap();
        // a key its service does not take is not kept, nothing is chosen with it, and he is told which key
        let refused = desk.choose_for(ChoiceJob::Reads, reads.clone(), Some("sk-broken".into()), none(), false);
        assert!(matches!(&refused, Err(RadioError::Told { reason }) if reason.contains("did not accept the key")), "{refused:?}");
        assert!(host.secret(provider.clone()).is_none() && chosen(&desk.choices(ChoiceJob::Reads)).iter().all(|c| *c != reads));
        // and with no key at all it is asked for, before anything is tried
        assert!(matches!(desk.choose_for(ChoiceJob::Reads, reads.clone(), None, none(), false), Err(RadioError::Told { reason }) if reason.starts_with("Enter your ")));
        desk.choose_for(ChoiceJob::Reads, reads.clone(), Some(" sk-test ".into()), none(), false).unwrap();
        assert_eq!(state(ChoiceJob::Reads, &reads), (true, ChoiceState::Waiting));
        assert_eq!(desk.chosen_label(ChoiceJob::Reads), "Waiting for your computer");
        assert_eq!(host.secret(provider).as_deref(), Some("sk-test"));
        assert!(!std::fs::read_to_string(dir.join("choices.json")).unwrap().contains("sk-test"));
        // opened again, it is all as he left it
        let again = Desk::alone(&dir.display().to_string(), host);
        assert_eq!(again.choices(ChoiceJob::Reads), desk.choices(ChoiceJob::Reads));
        let _ = std::fs::remove_dir_all(dir);
    }
}
