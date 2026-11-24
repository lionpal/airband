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
use crate::senses::{Cannot, Sense};
use crate::services::{Service, ServiceAsk};
use airband_core::providers::Job;
use airband_core::radio::{OfferName, ReadsOffered};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

mod contact;
mod desk;
mod list;
mod saved;
mod timings;
pub(crate) use timings::{Did, Timings};
pub use contact::{ContactFrom, ContactNote};
pub(crate) use desk::tell_reads;
use list::*;
use saved::{Pick, Saved};
pub(crate) use saved::ReadsWanted;
pub(crate) use saved::Withheld;

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
    /// Its model is not on this device yet. Chosen, its card offers the download ([`Choice::action`]).
    NeedsDownload { megabytes: u32 },
    /// Its model is here and works, and a newer version of it can be fetched.
    Stale { megabytes: u32 },
    /// Something is being done about it now. `fraction` is how far along, from 0 to 1, when that can be said.
    Working { label: String, fraction: Option<f64> },
    /// It is chosen and waits for a computer to be reached, which is where it is put to use.
    Waiting,
    /// It is chosen and did not do its job the last time it was tried, and why. It stays chosen.
    Failed { why: String },
    /// Its model is here, and this device could not get it ready the last time it tried, and why. It stays
    /// chosen; its card offers to try again ([`Choice::action`]).
    NotReady { why: String },
    /// The model this job starts with, which this device cannot run, and why. It cannot be chosen: he picks
    /// something else.
    CannotRun { why: String },
}

/// One thing he can choose for a job.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Choice {
    /// What to pass back to [`Desk::choose_for`]. It says nothing by itself: what it stands for is `place`,
    /// `provider` and `offer`.
    pub id: String,
    pub place: ChoicePlace,
    /// For a service: its provider's id, which is also what its key is kept under (one key serves everything
    /// a provider offers). For a model on this device: the model's kind.
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
    /// On the card of the one chosen, the words of the one button that gets it ready, when it is not: a
    /// download with its size, an update, or another try at getting it ready. Choosing it again does it.
    pub action: Option<String>,
}

/// Something being done about a choice right now.
#[derive(Clone)]
struct Work {
    job: ChoiceJob,
    id: String,
    /// What is being done, in words. A download's words are made from how far it has got.
    label: String,
    /// A model is being fetched: how far along is known.
    fetching: bool,
}

/// What he chose, kept and put to use.
pub(crate) struct Chooser {
    /// The app's folder: `choices.json` is here, and the models under `models/`.
    dir: String,
    keeper: airband_io::kept::Keeper,
    /// What this device can run, as its host said when the desk was made.
    device: HostDevice,
    saved: Mutex<Saved>,
    working: Mutex<Option<Work>>,
    /// What he is to be told about the file when the desk opens: it could not be read.
    pub(crate) opened: Mutex<Option<String>>,
    /// The host's own way to bring a model's files, which goes on while the app is in the background, when
    /// it has given one (`Desk::use_transfers`).
    pub(crate) transfers: Mutex<Option<Arc<dyn crate::models::Transfers>>>,
    /// Models this device could not get ready the last time it tried since the app opened, by kind, and why.
    unready: Mutex<HashMap<String, String>>,
    /// How long each download and each getting ready took, for the connection record.
    pub(crate) timings: Timings,
    /// One model is got ready at a time.
    preparing: Mutex<()>,
}

/// The key kept on this phone for a provider, when there is one.
pub(crate) fn kept_key(host: &dyn DeskHost, provider: &str) -> Option<String> {
    host.secret(provider.to_string()).filter(|k| !k.is_empty())
}

pub(crate) fn told(reason: impl Into<String>) -> RadioError {
    RadioError::Told { reason: reason.into() }
}

impl Chooser {
    /// What was kept in `dir`, or a new install's choices. With no folder (a test), nothing is kept.
    pub(crate) fn open(dir: &str, device: HostDevice) -> Chooser {
        let (kept, keeper, opened) = crate::kept::open::<Option<Saved>>(crate::kept::file(dir, "choices.json"), "What you chose in Settings");
        let saved = Mutex::new(kept.unwrap_or_else(|| Saved::fresh(&device)));
        Chooser { dir: dir.into(), keeper, device, saved, working: Mutex::new(None), opened: Mutex::new(opened), transfers: Mutex::new(None), unready: Mutex::new(HashMap::new()), timings: Timings::open(dir), preparing: Mutex::new(()) }
    }

    /// The coding tools he uses, by id.
    pub(crate) fn tools(&self) -> Vec<String> {
        self.saved().tools
    }

    /// What writes his summaries, with how each of his computers stands with it.
    pub(crate) fn reads_intent(&self) -> Option<crate::desk::Intent<ReadsWanted>> {
        self.saved().reads
    }

    fn saved(&self) -> Saved {
        self.saved.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Change what is kept, and keep it. Err when it could not be kept: it holds until the app closes.
    fn change(&self, with: impl FnOnce(&mut Saved)) -> Result<(), RadioError> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        crate::kept::again_as::<Option<Saved>, Saved>(&self.keeper, &mut saved, |found| found.flatten().unwrap_or_else(|| Saved::fresh(&self.device)));
        let before = saved.clone();
        with(&mut saved);
        let mut touched = saved.touched;
        touched.add(&before, &saved);
        saved.touched = touched;
        crate::kept::said(self.keeper.save(&*saved)).map_err(|why| told(format!("{why} It holds until the app closes.")))
    }

    /// A model this device can run, by its kind.
    fn model(&self, kind: &str) -> Option<Arc<Model>> {
        crate::models::model(kind, &self.device)
    }

    /// Hand `ears.rs` and `mouth.rs` the one thing each is to use, from what he chose. What he chose but
    /// cannot be used as things are is handed over as that, with why: nothing takes its place.
    pub(crate) fn apply(&self, host: &dyn DeskHost, senses: &Senses) {
        let saved = self.saved();
        // his key for a service, or why it cannot be used: one that takes a key and has none here is never
        // tried with an empty one
        let key = |id: &str, sense: Sense| -> Result<String, Cannot> {
            match (kept_key(host, id), airband_core::providers::provider(id).filter(|p| p.key)) {
                (Some(key), _) => Ok(key),
                (None, None) => Ok(String::new()),
                (None, Some(p)) => Err(Cannot::NoKey { sense, key_name: p.key_name.clone() }),
            }
        };
        senses.ears.set(saved.pick(ChoiceJob::Ears).map(|ears| match ears.place {
            ChoicePlace::Phone => match self.model(&ears.id).filter(|m| m.job == Job::Ears) {
                None => crate::ears::Own::Cannot(Cannot::ModelNotRun(Sense::Listening)),
                Some(m) if !crate::models::fetched(&self.dir, &m) => crate::ears::Own::Cannot(Cannot::NotDownloaded { sense: Sense::Listening, model: m.name.clone() }),
                Some(m) => crate::ears::Own::Model(senses.ears.held(&m, crate::models::folder(&self.dir, &m))),
            },
            ChoicePlace::Service => {
                let key = match key(&ears.id, Sense::Listening) {
                    Ok(key) => key,
                    Err(why) => return crate::ears::Own::Cannot(why),
                };
                let with = crate::services::service_of(&ears.id, &ears.offer, &key, ears.set.clone().into_iter().collect());
                // what cannot listen says which it is: a service this build does not know, or one it cannot use
                match with.offer(Job::Ears) {
                    None => crate::ears::Own::Cannot(Cannot::UnknownService(Sense::Listening)),
                    Some((_, o)) if crate::hears::listens(&o) => crate::ears::Own::Service(with),
                    Some(_) => crate::ears::Own::Cannot(Cannot::ServiceNotUsed(Sense::Listening)),
                }
            }
        }));
        senses.mouth.set(saved.pick(ChoiceJob::Voice).map(|voice| match voice.place {
            ChoicePlace::Phone => match self.model(&voice.id).filter(|m| m.job == Job::Voice) {
                None => crate::mouth::Own::Cannot(Cannot::ModelNotRun(Sense::Voice)),
                Some(m) if !crate::models::fetched(&self.dir, &m) => crate::mouth::Own::Cannot(Cannot::NotDownloaded { sense: Sense::Voice, model: m.name.clone() }),
                Some(m) => match voice_of(&m, &voice.voice) {
                    Ok(name) => crate::mouth::Own::Model { held: senses.mouth.held(&m, crate::models::folder(&self.dir, &m)), voice: name },
                    Err(why) => crate::mouth::Own::Cannot(why),
                },
            },
            ChoicePlace::Service => {
                let key = match key(&voice.id, Sense::Voice) {
                    Ok(key) => key,
                    Err(why) => return crate::mouth::Own::Cannot(why),
                };
                let with = crate::services::service_of(&voice.id, &voice.offer, &key, voice.set.clone().into_iter().collect());
                // why it cannot speak is kept as the voice said it: a rate that is not one is told as that
                match crate::voices::synth(&with) {
                    Ok(_) => crate::mouth::Own::Service(with),
                    Err(why) => crate::mouth::Own::Cannot(Cannot::BadSetting { why }),
                }
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
        self.working.lock().unwrap_or_else(|e| e.into_inner()).clone().map(|w| match w.fetching {
            true => {
                let progress = crate::models::local_model_progress();
                (w.job, w.id, progress.said(), Some(progress.fraction()))
            }
            false => (w.job, w.id, w.label, None),
        })
    }

    fn choices(&self, job: ChoiceJob, host: &dyn DeskHost, senses: &Senses, paired: &[StationId]) -> Vec<Choice> {
        let saved = self.saved();
        let facts = Facts {
            models: crate::models::models(&self.device).collect(),
            here: &|m| (crate::models::fetched(&self.dir, m), crate::models::current(&self.dir, m)),
            unready: &|m| self.unready.lock().unwrap_or_else(|e| e.into_inner()).get(&m.kind).cloned(),
            has_key: &|id| kept_key(host, id).is_some(),
            failed: match job {
                ChoiceJob::Ears => senses.ears.failed(),
                ChoiceJob::Voice => senses.mouth.failed(),
                ChoiceJob::Reads => None,
            },
            work: self.at_work().filter(|w| w.0 == job).map(|w| (w.1, w.2, w.3)),
            paired,
            cannot: crate::models::usual_row(&self.device, job.job()).and_then(|(m, why)| why.map(|why| (m, why))),
        };
        list(job, &saved, &facts)
    }

    /// Get a model onto this device and ready to run: fetched if the version the catalogue names is not here,
    /// then, for one whose row says the device has work of its own to do first, got ready at once.
    fn make_ready(&self, job: ChoiceJob, m: &Model, host: &dyn DeskHost) -> Result<(), RadioError> {
        if !crate::models::current(&self.dir, m) {
            self.work(job, &m.kind, String::new(), true);
            host.changed();
            let began = std::time::Instant::now();
            let went = crate::models::fetch_local_model(self.dir.clone(), m, self.transfers.lock().unwrap_or_else(|e| e.into_inner()).clone());
            let progress = crate::models::local_model_progress();
            self.timings.note(m, match went {
                Ok(()) => Did::Downloaded { bytes: progress.of, ms: began.elapsed().as_millis() as u64 },
                Err(why) => Did::NotDownloaded { bytes: progress.got, ms: began.elapsed().as_millis() as u64, why: why.said() },
            });
            went?;
        }
        self.prepare(job, m, host)
    }

    /// The work the device does with a model before its first use, when its row names some and it is not done
    /// for the version here: have it do its job once, on nothing much (a second of silence to hear, a word to
    /// say). How long it took is kept, and a failure is kept as the model's state until it is tried again.
    fn prepare(&self, job: ChoiceJob, m: &Model, host: &dyn DeskHost) -> Result<(), RadioError> {
        let _one = self.preparing.lock().unwrap_or_else(|e| e.into_inner());
        if crate::models::prepared(&self.dir, m) {
            return Ok(());
        }
        self.work(job, &m.kind, m.told(crate::models::field::PREPARE), false);
        host.changed();
        let began = std::time::Instant::now();
        let went = self.run_once(m, host);
        let ms = began.elapsed().as_millis() as u64;
        let mut unready = self.unready.lock().unwrap_or_else(|e| e.into_inner());
        match &went {
            Ok(()) => {
                unready.remove(&m.kind);
                crate::models::mark_prepared(&self.dir, m);
                self.timings.note(m, Did::Readied { ms });
            }
            Err(why) => {
                unready.insert(m.kind.clone(), why.to_string());
                self.timings.note(m, Did::NotReadied { ms, why: why.to_string() });
            }
        }
        went
    }

    fn run_once(&self, m: &Model, host: &dyn DeskHost) -> Result<(), RadioError> {
        let held = crate::engines::Held::of(m, crate::models::folder(&self.dir, m));
        let by: crate::engines::ByHost = &|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input);
        // in the voice he picked, or the one its row names as usual: never just the first it lists
        let voice = match m.job {
            Job::Voice => voice_of(m, &self.saved().pick(ChoiceJob::Voice).filter(|p| p.id == m.kind).map(|p| p.voice).unwrap_or_default()).map_err(|why| told(why.told()))?,
            _ => String::new(),
        };
        held.run(|loaded| match m.job {
            Job::Voice => loaded.speak(by, &voice, "Ready.", &mut |_| {}, &|| false),
            _ => loaded.hear(by, vec![0u8; 32000]).map(|_| ()),
        })
        .map_err(|fault| told(fault.told(&m.name)))
    }

    /// The models chosen on this device that are here but not yet got ready (a download that finished while
    /// the app was away, or an app opened again before it was done), each with the job it is for.
    fn unprepared(&self) -> Vec<(ChoiceJob, Arc<Model>)> {
        let saved = self.saved();
        [ChoiceJob::Voice, ChoiceJob::Ears]
            .into_iter()
            .filter_map(|job| Some((job, self.model(&saved.pick(job).filter(|p| p.place == ChoicePlace::Phone)?.id)?)))
            .filter(|(_, m)| crate::models::fetched(&self.dir, m) && !crate::models::prepared(&self.dir, m) && !self.unready.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&m.kind))
            .collect()
    }

    /// Get ready, one after another, the models chosen here that are downloaded and not ready yet. Blocks.
    pub(crate) fn prepare_chosen(&self, host: &dyn DeskHost, senses: &Senses) {
        let todo = self.unprepared();
        if todo.is_empty() {
            return;
        }
        for (job, m) in todo {
            // a failure is kept as the model's state, which its card shows with another try, and is timed
            if let Err(why) = self.prepare(job, &m, host) {
                eprintln!("airband: {} could not be got ready: {why}", m.kind);
            }
        }
        self.rest();
        self.apply(host, senses);
        host.changed();
    }

    /// The models chosen on this device that are not downloaded, or not the version the catalogue names: what
    /// a first run asks once to download.
    pub(crate) fn owed(&self) -> Vec<(ChoiceJob, Arc<Model>)> {
        let saved = self.saved();
        [ChoiceJob::Voice, ChoiceJob::Ears].into_iter().filter_map(|job| Some((job, self.model(&saved.pick(job).filter(|p| p.place == ChoicePlace::Phone)?.id)?))).filter(|(_, m)| !crate::models::current(&self.dir, m)).collect()
    }
}

/// The voice a model speaks in: the one he picked, or with none picked the one its row names as usual, when the
/// model has it. A voice it does not have is never swapped for another of its voices.
fn voice_of(m: &Model, picked: &str) -> Result<String, Cannot> {
    let wanted = m.voice(picked);
    m.voices.iter().map(|v| &v.0).find(|name| name.as_str() == wanted).cloned().ok_or_else(|| match picked.is_empty() {
        true => Cannot::NoVoiceNamed { model: m.name.clone() },
        false => Cannot::VoiceGone { model: m.name.clone(), voice: picked.to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(here: &'a dyn Fn(&Model) -> (bool, bool), has_key: &'a dyn Fn(&str) -> bool) -> Facts<'a> {
        Facts { models: crate::models::models(&phone()).collect(), here, unready: &|_| None, has_key, failed: None, work: None, paired: &[], cannot: None }
    }
    /// A phone whose host runs every engine the catalogue names.
    fn phone() -> HostDevice {
        HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX, team_colours: 0 }
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
        let offered = ReadsOffered { now: Some(named("b", "x")), failed: None, offers: vec![offer("a", "", true, false), offer("b", "", true, true), offer("c", "some-tool", false, false), offer("d", "other-tool", false, false)] };
        let saved = Saved { offered: Some(offered), tools: vec!["some-tool".into()], ..Saved::default() };
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, NO_KEYS));
        // his tool's sign-in first, another tool's left out, and the label is the computer's own
        assert_eq!(all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["c/x", "a/x", "b/x"]);
        assert_eq!(all[0].title, "The c one");
        // the one in use is the one the computer names, and a key the computer keeps counts as kept
        assert_eq!(chosen(&all), vec!["b/x"]);
        assert!(all[2].has_key && all[2].state == ChoiceState::Ready && all[1].state == ChoiceState::NeedsKey);
        // what he picked while it could not be reached is the one shown, as waiting
        let waiting = Saved { reads: Some(crate::desk::Intent::after(None, ReadsWanted { id: "a".into(), offer: "x".into(), set: BTreeMap::new(), his: true })), ..saved };
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
    fn before_any_computer_summaries_follow_the_coding_tool_he_names_for_them() {
        // the offer each sign-in's row marks as the one to use
        let logins: Vec<Service> = crate::services::services(Job::Summary).into_iter().filter(|s| !s.login.is_empty() && !s.more).collect();
        let (one, two) = (logins[0].clone(), logins.iter().find(|s| s.login != logins[0].login).expect("two tools with a sign-in that reads").clone());
        let mut saved = Saved::default();
        // one tool: its sign-in, with nothing to ask
        saved.use_tools(vec![one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| (w.wanted.id.as_str(), w.wanted.his)), Some((one.kind.as_str(), false)));
        assert_eq!(saved.summary_tool.as_deref(), Some(one.login.as_str()));
        assert!(!saved.summary_tool_wanted());
        // the other tool's sign-in is not offered to him, and his own is the one shown as chosen
        let all = list(ChoiceJob::Reads, &saved, &facts(HERE, NO_KEYS));
        assert!(chosen(&all) == vec![service_id(&one.kind, &one.offer).as_str()] && all.iter().all(|c| c.provider != two.kind));
        // a second tool, put first: the order of the list decides nothing, and the first stays
        saved.use_tools(vec![two.login.clone(), one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| w.wanted.id.as_str()), Some(one.kind.as_str()));
        // he takes away the one that writes them: the only other one that can is the one
        saved.use_tools(vec![two.login.clone()]);
        assert_eq!(saved.summary_tool.as_deref(), Some(two.login.as_str()));
        // with several that can and none named (the one named was taken away, and more than one is left), he is
        // asked, and nothing is moved for him
        let logins: Vec<String> = logins.iter().map(|s| s.login.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        if logins.len() > 2 {
            saved.use_tools(logins.clone());
            saved.use_tools(logins.iter().filter(|l| **l != two.login).cloned().collect());
        } else {
            saved = Saved::default();
            saved.use_tools(logins.clone());
        }
        assert!(saved.summary_tool_wanted() && saved.summary_tool.is_none() && saved.reads.is_none(), "{:?}", saved.reads);
        // he says which
        assert!(!saved.summaries_by("not-his".into()));
        assert!(saved.summaries_by(one.login.clone()));
        assert_eq!(saved.reads.as_ref().map(|w| w.wanted.id.as_str()), Some(one.kind.as_str()));
        assert!(!saved.summary_tool_wanted());
        // he picks a service for himself: a change of tool leaves that alone
        saved.reads = Some(crate::desk::Intent::after(None, ReadsWanted { id: "fixture-provider".into(), offer: "fixture-offer".into(), set: BTreeMap::new(), his: true }));
        saved.use_tools(vec![one.login.clone()]);
        assert_eq!(saved.reads.as_ref().map(|w| w.wanted.id.as_str()), Some("fixture-provider"));
        // and once a computer has said what it reads with, nothing is made to wait by a change of tool
        let mut said = Saved { offered: Some(ReadsOffered::default()), ..Saved::default() };
        said.use_tools(vec![one.login]);
        assert!(said.reads.is_none());
    }

    #[test]
    fn a_first_run_starts_with_his_tools_sign_in_and_the_two_models_on_the_phone() {
        // listening and speaking: the rows the catalogue marks usual for each, nothing he has to give
        let saved = Saved::fresh(&phone());
        let marked = |job: Job| airband_core::catalog::current().models.iter().filter(|m| m.job == job && m.with.get(crate::models::field::USUAL).and_then(|u| u.as_bool()) == Some(true)).map(|m| m.kind.clone()).collect::<Vec<_>>();
        assert_eq!(saved.ears.as_ref().map(|p| vec![p.id.clone()]), Some(marked(Job::Ears)));
        assert_eq!(saved.voice.as_ref().map(|p| vec![p.id.clone()]), Some(marked(Job::Voice)));
        assert!(saved.ears.iter().chain(saved.voice.iter()).all(|p| p.place == ChoicePlace::Phone));
        // summaries: nothing until he names his coding tool, then that tool's own sign-in, never a key
        assert!(saved.reads.is_none());
        for tool in airband_core::catalog::current().tools.iter() {
            let mut s = saved.clone();
            s.use_tools(vec![tool.id.clone()]);
            let Some(wanted) = s.reads.map(|r| r.wanted) else { continue };
            let service = crate::services::services(Job::Summary).into_iter().find(|x| x.kind == wanted.id && x.offer == wanted.offer).unwrap();
            assert!(service.login == tool.id && !service.key && !wanted.his, "{}", tool.id);
        }
        // a phone whose system is too old for the listening model: nothing is chosen for listening, and the
        // list says why, so he can pick something else
        let (m, _) = crate::models::usual_row(&phone(), Job::Ears).expect("a usual listening model");
        let least = m.with.get(crate::models::field::MIN_OS).and_then(|n| n.get("ios")).and_then(|v| v.as_u64()).expect("the listening model names the system it needs") as u32;
        let old = HostDevice { os_version: least - 1, ..phone() };
        assert!(Saved::fresh(&old).ears.is_none() && Saved::fresh(&old).voice.is_some());
        let chooser = Chooser::open("", old);
        let all = chooser.choices(ChoiceJob::Ears, &Host::default(), &Senses::default(), &[]);
        let row = all.iter().find(|c| c.id == m.kind).expect("the model it cannot run is shown");
        assert_eq!(row.state, ChoiceState::CannotRun { why: crate::engines::CannotRun::OldSystem.said().into() });
        assert!(chosen(&all).is_empty() && !row.chosen);
    }

    // Unix only: it makes the folder unwritable by its permission bits
    #[cfg(unix)]
    #[test]
    fn a_choice_the_phone_could_not_keep_says_so() {
        let dir = std::env::temp_dir().join(format!("airband-choices-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let chooser = Chooser::open(&dir.display().to_string(), HostDevice::default());
        // the folder choices are kept in can no longer be written
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let went = chooser.change(|s| s.tools = vec!["some-tool".into()]);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(went, Err(RadioError::Told { .. })), "{went:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_service_that_takes_a_key_is_never_tried_without_one() {
        let dir = std::env::temp_dir().join(format!("airband-choices-nokey-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        // a row that listens and takes a key
        let (p, o) = airband_core::providers::in_order(Job::Ears).into_iter().find(|(p, _)| p.key).expect("a row that listens with a key");
        std::fs::write(dir.join("choices.json"), serde_json::json!({ "ears": { "place": "service", "id": p.id, "offer": o.id } }).to_string()).unwrap();
        let host = Arc::new(Host::default());
        let desk = Desk::alone(&dir.display().to_string(), host.clone());
        desk.shared.chooser.apply(&*host, &desk.shared.senses);
        let why = desk.shared.senses.ears.cannot();
        let _ = std::fs::remove_dir_all(dir);
        // he is told to enter the key, by its name
        assert_eq!(why, Some(crate::senses::Cannot::NoKey { sense: crate::senses::Sense::Listening, key_name: p.key_name.clone() }.told()));
    }

    #[test]
    fn a_new_install_starts_with_what_needs_nothing_from_him() {
        let saved = Saved::fresh(&phone());
        for (job, pick) in [(Job::Ears, saved.ears.clone()), (Job::Voice, saved.voice.clone())] {
            match crate::models::usual(&phone(), job) {
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
        chooser.change(|s| s.voice = Some(Pick { place: ChoicePlace::Service, id: "fixture-voice-service".into(), offer: "fixture-offer".into(), set: BTreeMap::from([("voice".to_string(), "fixture-voice".to_string())]), voice: String::new() })).unwrap();
        let again = Chooser::open(&at, phone());
        assert_eq!(again.saved(), chooser.saved());
        // and no key is in it
        assert!(!std::fs::read_to_string(dir.join("choices.json")).unwrap().contains("key"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A host that keeps keys in memory and has no microphone, speaker or model.
    #[derive(Default)]
    pub(super) struct Host {
        keys: Mutex<HashMap<String, String>>,
        notes: Mutex<Vec<String>>,
        /// Its engines run a model (giving back nothing much); else they say they could not.
        pub(super) runs: std::sync::atomic::AtomicBool,
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
            self.runs.load(std::sync::atomic::Ordering::SeqCst).then(Vec::new)
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
            // why it cannot; nothing else speaks in its place. Its card offers the download, with its size,
            // and sends him nowhere else.
            assert!(desk.shared.senses.mouth.ready());
            let tape = crate::ears_stream::Tape::of(&[], false);
            let why = desk.shared.senses.mouth.say(&tape, "Shop two is waiting.", &|| false).unwrap_err();
            assert_eq!(why, format!("{} is not on this phone yet.", m.name));
            let card = desk.choices(ChoiceJob::Voice).into_iter().find(|c| c.id == m.kind).unwrap();
            assert!(matches!(card.state, ChoiceState::NeedsDownload { megabytes } if card.action == Some(format!("Download ({megabytes}\u{a0}MB)"))), "{card:?}");
            // and a first run asks once to download it, with its size
            let ask = desk.downloads_wanted().unwrap();
            assert_eq!(ask.button, format!("Download ({}\u{a0}MB)", ask.megabytes));
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

#[cfg(test)]
mod ready_tests {
    use super::tests::Host;
    use super::*;

    /// A model of two small files whose engine is the host's, and whose row says the device gets it ready
    /// before first use; here whole in `dir`.
    fn tiny(dir: &std::path::Path) -> Model {
        let folder = dir.join("models/tiny/1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.bin"), "abc").unwrap();
        std::fs::write(folder.join(crate::models::WHOLE), "").unwrap();
        Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Ears, engine: "host-only".into(), version: "1".into(), base: "https://airband.dev/models/tiny".into(), files: vec![("a.bin".into(), 3, "0".repeat(64))], voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with: [(crate::models::field::PREPARE.to_string(), serde_json::json!("Getting it ready"))].into() }
    }

    #[test]
    fn getting_a_model_ready_ends_ready_or_not_ready_with_why_and_is_timed_either_way() {
        let dir = std::env::temp_dir().join(format!("airband-ready-{}", random_hex(4)));
        let host = Arc::new(Host::default());
        let desk = Desk::alone(&dir.display().to_string(), host.clone());
        let chooser = &desk.shared.chooser;
        let m = tiny(&dir);
        assert!(!crate::models::prepared(&dir.display().to_string(), &m));
        // the engine could not run it: not ready, why, and how long it took
        let failed = chooser.prepare(ChoiceJob::Ears, &m, &*host).unwrap_err().to_string();
        assert_eq!(chooser.unready.lock().unwrap().get("tiny"), Some(&failed));
        assert!(matches!(chooser.timings.all().last().map(|t| &t.did), Some(Did::NotReadied { why, .. }) if *why == failed));
        // so it is not tried again behind his back: the card offers that
        assert!(chooser.unready.lock().unwrap().contains_key("tiny"));
        // tried again, it goes: ready, kept as ready, and timed
        host.runs.store(true, std::sync::atomic::Ordering::SeqCst);
        chooser.prepare(ChoiceJob::Ears, &m, &*host).unwrap();
        assert!(chooser.unready.lock().unwrap().is_empty());
        assert!(crate::models::prepared(&dir.display().to_string(), &m));
        assert!(matches!(chooser.timings.all().last().map(|t| &t.did), Some(Did::Readied { .. })));
        // and is not done again
        let kept = chooser.timings.all().len();
        chooser.prepare(ChoiceJob::Ears, &m, &*host).unwrap();
        assert_eq!(chooser.timings.all().len(), kept);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_chosen_card_says_how_it_stands_and_offers_the_one_button_that_gets_it_ready() {
        let device = HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX, team_colours: 0 };
        let Some(m) = crate::models::models(&device).find(|m| m.job == Job::Ears) else { return };
        let saved = Saved { ears: Some(Pick { place: ChoicePlace::Phone, id: m.kind.clone(), offer: String::new(), set: BTreeMap::new(), voice: String::new() }), ..Saved::default() };
        let card = |here: &dyn Fn(&Model) -> (bool, bool), unready: &dyn Fn(&Model) -> Option<String>, failed: Option<String>, work: Option<(String, String, Option<f64>)>| {
            let facts = Facts { models: vec![m.clone()], here, unready, has_key: &|_| false, failed, work, paired: &[], cannot: None };
            list(ChoiceJob::Ears, &saved, &facts).into_iter().find(|c| c.id == m.kind).unwrap()
        };
        let size = list::megabytes(&m);
        // not here: download, whatever the last try at using it said
        let c = card(&|_| (false, false), &|_| None, Some(format!("{} is not on this phone yet.", m.name)), None);
        assert_eq!((c.state, c.action), (ChoiceState::NeedsDownload { megabytes: size }, Some(format!("Download ({size}\u{a0}MB)"))));
        // downloading: how far, in its own words, and no button
        let c = card(&|_| (false, false), &|_| None, None, Some((m.kind.clone(), "Downloading 23 of 66\u{a0}MB".into(), Some(0.35))));
        assert_eq!((c.state, c.action), (ChoiceState::Working { label: "Downloading 23 of 66\u{a0}MB".into(), fraction: Some(0.35) }, None));
        // here and not got ready: why, and try again
        let c = card(&|_| (true, true), &|_| Some("it could not be loaded".into()), None, None);
        assert_eq!((c.state, c.action), (ChoiceState::NotReady { why: "it could not be loaded".into() }, Some("Try again".into())));
        // a newer version: update, with its size
        let c = card(&|_| (true, false), &|_| None, None, None);
        assert_eq!(c.action, Some(format!("Update ({size}\u{a0}MB)")));
        // ready: nothing to do
        let c = card(&|_| (true, true), &|_| None, None, None);
        assert_eq!((c.state, c.action), (ChoiceState::Ready, None));
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn a_model_whose_row_names_no_usual_voice_is_never_run_with_an_empty_one() {
        let mut m = (*crate::models::models(&HostDevice::default()).chain(airband_core::catalog::current().models.clone()).find(|m| m.job == Job::Voice).unwrap()).clone();
        m.usual_voice = String::new();
        assert_eq!(voice_of(&m, "").unwrap_err(), Cannot::NoVoiceNamed { model: m.name.clone() });
        let first = m.voices[0].0.clone();
        assert_eq!(voice_of(&m, &first).unwrap(), first);
    }
}
