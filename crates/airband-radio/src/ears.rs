//! Inbound: turning what he says into text. The host only hands over raw microphone audio; deciding when he has
//! finished (airband_core::audio::Endpointer) and talking to the service are done once, for every platform.
//!
//! Which service is a row of the catalogue, and there are two ways to listen with one:
//!
//! - live, while he talks, by a dialect (a file under `ears/` and a line of [`LISTENERS`]): the phone talks to
//!   the service itself, with a short-lived pass. The words come back as he speaks.
//! - by upload (`hears.rs`): the phone records until he stops and the recording is sent off whole, straight
//!   to the service, with the key of his that is on the phone.
//!
//! What listens is what he chose on this device and nothing else: a model here, or a service with a key of
//! his kept here. His computer has no part in it. With nothing chosen, nothing listens and he is told where to
//! choose. Nothing here or in the app names a service.
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use airband_core::providers::{By, Interface, Job, Offer, Provider, Use};
use std::collections::BTreeMap;
use std::sync::mpsc::sync_channel;
use std::time::Instant;

const RATE: u32 = 16000;
/// A pass is asked for again after this long (the shortest-lived of them lasts ten minutes).
const PASS_LIFE: Duration = Duration::from_secs(300);

mod bidi;
mod chunks;
mod usp;
mod pass;
pub(crate) use pass::*;

/// Said when he opens the microphone with nothing chosen to listen with.
pub(crate) const NOTHING_CHOSEN: &str = "Nothing is chosen to listen with on this device. Choose it in Settings, under Listening.";

/// How a dialect gets its pass: from the row, the service's name, his settings and his key.
type AskPass = fn(&By, &Provider, &BTreeMap<String, String>, &str) -> Result<String, String>;

/// A live listening dialect: how one service is listened with while he talks. Each is a file under `ears/`
/// and a line of [`LISTENERS`].
pub(crate) struct Listener {
    /// Its name: what a provider's row names (`by.interface`), and what a pass calls it (`Pass::dialect`).
    pub is: Interface,
    /// How its live connection is spoken to.
    pub wire: &'static crate::ears_stream::Wire,
    /// A short-lived pass for a key: what the service's row tells the dialect, what the service is called, his
    /// settings, and the key. Err says why not, in words for him.
    pass: AskPass,
    /// Its pass is good for one connection only: a new one is asked for each time, and none is kept.
    once: bool,
}

/// Every live dialect this build speaks.
pub(crate) const LISTENERS: &[Listener] = &[usp::LISTENER, bidi::LISTENER, chunks::LISTENER];

/// A pass for this dialect is used up by one connection, so it is never kept for the next.
pub(crate) fn once(kind: &str) -> bool {
    listener(kind).is_some_and(|l| l.once)
}

pub(crate) fn listener(kind: &str) -> Option<&'static Listener> {
    LISTENERS.iter().find(|l| l.is.name == kind)
}

/// The live dialect an offer names, when this build speaks it and can do what the offer needs.
pub(crate) fn live(offer: &Offer) -> Option<&'static Listener> {
    LISTENERS.iter().find(|l| offer.job == Job::Ears && l.is.fits(offer))
}

/// What he chose to listen with on the device itself, when he chose that. One thing at a time: which one is
/// decided where he chooses (`choices.rs`) and only carried out here.
#[derive(Clone)]
pub(crate) enum Own {
    /// A model on the device: nothing he says leaves it.
    Model(Arc<crate::engines::Held>),
    /// A listening service reached straight from the device with a key of his own. The key is never sent
    /// anywhere but to the service it is for.
    Service(Use),
    /// What he chose cannot be used as things are, and why. Nothing else listens in its place.
    Cannot(String),
    /// The host takes a line he types: a radio in a terminal. This is what such a radio is started with
    /// ([`Radio::start`]); it is never put in the place of something else.
    Host,
}

/// What listens on this device: one for everything that shares what he chose (a desk and its radios).
#[derive(Default)]
pub(crate) struct Chosen {
    using: Mutex<Option<Own>>,
    /// Why what he chose did not listen the last time it was tried. Cleared when it next does.
    failed: Mutex<Option<String>>,
    /// How long the model took over the last thing it heard, and how long that recording was, in seconds.
    last: Mutex<Option<(f32, f32)>>,
    /// The names he is likely to say: his agents and their projects, as the chat last listed them, and the
    /// words of the trade the catalogue lists. For a recogniser that can be told what to expect.
    phrases: Mutex<Vec<String>>,
}

impl Chosen {
    fn using(&self) -> Option<Own> {
        self.using.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Listen with this from now on; nothing is nothing.
    pub(crate) fn set(&self, own: Option<Own>) {
        *self.using.lock().unwrap_or_else(|e| e.into_inner()) = own;
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The model to listen with, from that folder: the one already in use when it is the same.
    pub(crate) fn held(&self, m: &crate::models::Model, folder: std::path::PathBuf) -> Arc<crate::engines::Held> {
        match self.using() {
            Some(Own::Model(held)) if held.is(m, &folder) => held,
            _ => crate::engines::Held::of(m, folder),
        }
    }

    /// The names he is likely to say just now (see `phrases`).
    pub(crate) fn expect(&self, names: impl IntoIterator<Item = String>) {
        let mut all: Vec<String> = names.into_iter().filter(|n| !n.is_empty()).chain(airband_core::catalog::current().phrases.iter().cloned()).collect();
        all.sort();
        all.dedup();
        *self.phrases.lock().unwrap_or_else(|e| e.into_inner()) = all;
    }

    /// Why nothing can listen as things are, before the microphone is opened for it: nothing is chosen, or
    /// what he chose cannot be used. Nothing when something can.
    pub(crate) fn cannot(&self) -> Option<String> {
        match self.using() {
            None => Some(NOTHING_CHOSEN.into()),
            Some(Own::Cannot(why)) => Some(why),
            Some(_) => None,
        }
    }

    /// Why what he chose did not listen the last time, if it did not.
    pub(crate) fn failed(&self) -> Option<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// What the model on the device last took: seconds of speech, and seconds to turn it into words.
    pub(crate) fn last_heard(&self) -> Option<(f32, f32)> {
        *self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What he chose could not listen. It stays chosen, and he is told: moving to something else is his to do.
    fn fail(&self, host: &dyn RadioHost, why: String) -> Listened {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.clone());
        host.told(why);
        Listened::Cannot
    }
}

/// A pass from a live service for a key. Err says why not, in words for him.
#[cfg(test)]
pub(crate) fn pass_for(with: &Use) -> Result<Pass, String> {
    let (provider, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    let l = live(&offer).ok_or("That service does not listen live.")?;
    let set = with.settings(Job::Ears);
    Ok(Pass { dialect: l.is.name.into(), token: (l.pass)(&offer.by, &provider, &set, &with.key)?, set, phrases: Vec::new(), by: offer.by.clone() })
}

/// Does this service listen with this key and these settings? One real call to it.
pub(crate) fn check(with: &Use) -> Result<(), String> {
    let (provider, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    match live(&offer) {
        Some(l) => (l.pass)(&offer.by, &provider, &with.settings(Job::Ears), &with.key).map(|_| ()),
        None => crate::hears::check(with),
    }
}

/// How the next thing he says is to be heard: by what he chose on the device. Worked out in one place
/// ([`Ears::way`]) and only carried out by [`Ears::hear`].
#[derive(Clone)]
enum Way {
    /// Through the microphone, one of these ways.
    Mic(Mic),
    /// A line he types, taken by the host: a radio in a terminal.
    Host,
    /// Nothing can listen, and why: nothing is chosen, or what he chose cannot. Nothing else is asked.
    Nothing(String),
}

/// The ways of hearing him through the microphone.
#[derive(Clone)]
enum Mic {
    /// By the model on the device.
    Model(Arc<crate::engines::Held>),
    /// In one upload to a service, with the key of his own that is already here.
    OwnUpload { with: Use, phrases: Vec<String> },
    /// Live, as he speaks, in a dialect this build has. Its pass is asked for once the microphone is open
    /// ([`Ears::pass`]).
    Live { with: &'static Listener },
}

#[derive(Clone)]
pub(crate) struct Ears {
    /// The last pass a live service gave, when, and what he had chosen it for (the service, its settings and
    /// his key): one is good for a few minutes, and only for that.
    pass: Arc<Mutex<Option<(Pass, Instant, Use)>>>,
    /// Why the last pass could not be had, in words for him: what he is told when listening could not start.
    refused: Arc<Mutex<Option<String>>>,
    /// What he chose on the device.
    chosen: Arc<Senses>,
}

impl Ears {
    pub(crate) fn new(chosen: Arc<Senses>) -> Ears {
        Ears { pass: Default::default(), refused: Default::default(), chosen }
    }

    /// The pass kept for exactly this choice, while it is still good.
    fn kept(&self, with: &Use) -> Option<Pass> {
        self.pass.lock().unwrap().clone().filter(|(p, when, made)| made == with && when.elapsed() < PASS_LIFE && !once(&p.dialect)).map(|(p, _, _)| p)
    }
    fn keep(&self, pass: &Pass, with: &Use) {
        *self.pass.lock().unwrap() = Some((pass.clone(), Instant::now(), with.clone()));
    }
    fn forget(&self) {
        *self.pass.lock().unwrap() = None;
    }

    fn way(&self) -> Way {
        match self.chosen.ears.using() {
            None => Way::Nothing(NOTHING_CHOSEN.into()),
            Some(Own::Model(held)) => Way::Mic(Mic::Model(held)),
            Some(Own::Cannot(why)) => Way::Nothing(why),
            Some(Own::Host) => Way::Host,
            Some(Own::Service(with)) => match with.offer(Job::Ears).map(|(_, o)| (live(&o), crate::hears::by_upload(&with))) {
                Some((Some(l), _)) => Way::Mic(Mic::Live { with: l }),
                Some((None, true)) => Way::Mic(Mic::OwnUpload { phrases: self.chosen.ears.phrases.lock().unwrap().clone(), with }),
                _ => Way::Nothing("This version of airband cannot listen with the service you chose.".into()),
            },
        }
    }

    /// A pass for the live service he chose: the one kept for that choice while it is good, or a new one,
    /// asked for now. None when there is none to be had; why is kept for telling him.
    fn pass(&self) -> Option<Pass> {
        let Some(Own::Service(with)) = self.chosen.ears.using() else { return None };
        let (provider, offer) = with.offer(Job::Ears)?;
        let l = live(&offer)?;
        let phrases = self.chosen.ears.phrases.lock().unwrap().clone();
        if let Some(pass) = self.kept(&with) {
            return Some(Pass { phrases, ..pass });
        }
        let set = with.settings(Job::Ears);
        match (l.pass)(&offer.by, &provider, &set, &with.key) {
            Ok(token) => {
                let pass = Pass { dialect: l.is.name.into(), token, set, phrases, by: offer.by.clone() };
                self.keep(&pass, &with);
                *self.refused.lock().unwrap() = None;
                Some(pass)
            }
            Err(why) => {
                *self.refused.lock().unwrap() = Some(why);
                None
            }
        }
    }

    /// Ask for the pass now, so it is there before he next opens the microphone. Nothing is asked of anyone
    /// when what he chose needs no pass.
    pub(crate) fn warm(&self) {
        if matches!(self.way(), Way::Mic(Mic::Live { .. })) {
            let _ = self.pass();
        }
    }

    /// Open the microphone and listen. The microphone opens first, at once: the pass for the listening service
    /// and the connection to it are made while he is already talking, and what he said meanwhile is sent when
    /// they are ready. With `manual` he opened it himself: it stays open, through any pause, until the host
    /// closes it, and everything he said comes back. Without, it is open for one reply and closes by itself
    /// when he stops talking.
    pub(crate) fn hear(&self, host: &dyn RadioHost, manual: bool) -> Listened {
        let said = |t: Option<String>| t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(Listened::Said).unwrap_or(Listened::Nothing);
        let fail = |host: &dyn RadioHost, why: String| self.chosen.ears.fail(host, why);
        let mic = match self.way() {
            // a radio in a terminal: there is no microphone, and what he says is the line he types
            Way::Host => return said(host.listen()),
            // nothing can listen: say why, and stop there
            Way::Nothing(why) => {
                self.forget();
                return fail(host, why);
            }
            Way::Mic(mic) => mic,
        };
        if !host.record_start(RATE) {
            host.told("The microphone could not be opened. Check that airband may use it, in Settings.".into());
            return Listened::Cannot;
        }
        let heard_well = || *self.chosen.ears.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        match mic {
            Mic::Model(held) => {
                let name = held.m.name.clone();
                let heard = by_model(host, &held, &self.chosen.ears, manual);
                host.record_stop();
                match heard {
                    Ok(text) => {
                        heard_well();
                        said(text)
                    }
                    // the model would not run: it stays the one he chose, and he is told
                    Err(()) => fail(host, format!("{name} did not run on this device. Nothing else listened in its place.")),
                }
            }
            // The recording is made here and sent off whole, to the service, with his own key.
            Mic::OwnUpload { with, phrases } => {
                let clip = record(host, manual);
                host.record_stop();
                match clip {
                    None => Listened::Nothing,
                    Some(pcm) => {
                        host.status(RadioStatus::Sending);
                        match crate::hears::transcribe(&with, &crate::hears::wav(RATE, &pcm), &phrases) {
                            Ok(text) => {
                                heard_well();
                                said(Some(text))
                            }
                            Err(why) => fail(host, format!("The listening service you chose did not take that: {why}")),
                        }
                    }
                }
            }
            Mic::Live { with } => {
                // a fresh pass whenever the connection asks for one: the same way, worked out again
                let again: Arc<dyn Fn() -> Option<Pass> + Send + Sync> = {
                    let ears = self.clone();
                    Arc::new(move || ears.pass())
                };
                // As he speaks, with the words coming back while he talks. When that connection cannot be made
                // he is told, and nothing listens another way in its place.
                let heard = crate::ears_stream::live(with.wire, again, host, RATE, crate::hears::wav_header(RATE, None), manual);
                host.record_stop();
                match heard {
                    Ok(text) => {
                        heard_well();
                        said(text)
                    }
                    Err(_) => {
                        // the service or the line failed: ask for a fresh pass next time. When it was the pass he is
                        // told the service's reason
                        self.forget();
                        let why = self.refused.lock().unwrap().take();
                        fail(host, why.unwrap_or_else(|| "The listening service you chose could not be reached. Nothing else listened in its place.".into()))
                    }
                }
            }
        }
    }
}

/// The microphone until he stops, as samples: for a service that takes a finished recording. None when he
/// said nothing. A little from before he was judged to be speaking is kept, so the first word is not clipped.
fn record(host: &dyn RadioHost, manual: bool) -> Option<Vec<i16>> {
    record_with(host, manual, &mut |_| {})
}

/// The same, telling `so_far` what has been kept each time more is, while he is still talking.
fn record_with(host: &dyn RadioHost, manual: bool, so_far: &mut dyn FnMut(&[i16])) -> Option<Vec<i16>> {
    let mut end = Endpointer::new(RATE);
    let mut lead: std::collections::VecDeque<Vec<i16>> = std::collections::VecDeque::new();
    let mut kept: Vec<i16> = Vec::new();
    let mut speaking = false;
    loop {
        let bytes = host.record_read();
        if bytes.is_empty() {
            break; // the host stopped the microphone
        }
        let heard = samples(&bytes);
        let mut verdict = Heard::Waiting;
        for frame in heard.chunks(320) {
            verdict = end.feed(frame);
            if matches!(verdict, Heard::Finished | Heard::Silence) {
                break;
            }
        }
        // he opened the microphone himself: it is all kept, and only he ends it
        if manual {
            verdict = Heard::Waiting;
        }
        if (end.speaking || manual) && !speaking {
            speaking = true;
            lead.drain(..).for_each(|early| kept.extend(early));
        }
        if speaking {
            kept.extend(heard);
            so_far(&kept);
        } else {
            lead.push_back(heard);
            while lead.len() > 5 {
                lead.pop_front();
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return None,
            _ => {}
        }
    }
    (speaking && !kept.is_empty()).then_some(kept)
}

/// Listen with a model on the device, run by its engine. The model takes a whole recording, so to show his
/// words while he talks the recording so far is read again about twice a second, on a thread of its own: the
/// microphone is never kept waiting, and a reading that is still going when the next is due is let finish
/// and the next one skipped. When he stops, all of it is read once more, and that is what he said.
fn by_model(host: &dyn RadioHost, held: &crate::engines::Held, chosen: &Chosen, manual: bool) -> Result<Option<String>, ()> {
    let read = |pcm: Vec<u8>| held.with(|m| m.hear(&|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input), pcm)).and_then(|heard| heard).ok();
    let bytes = |pcm: &[i16]| pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let clip = thread::scope(|scope| {
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        let read = &read;
        scope.spawn(move || {
            while let Ok(pcm) = rx.recv() {
                if let Some(now) = read(pcm) {
                    host.hearing(now);
                }
            }
        });
        let mut read_at = 0usize;
        // dropping `tx` when this returns ends the thread above, once the reading it is on has finished
        record_with(host, manual, &mut |kept| {
            if kept.len() >= read_at + RATE as usize / 2 && tx.try_send(bytes(kept)).is_ok() {
                read_at = kept.len();
            }
        })
    });
    let Some(pcm) = clip else { return Ok(None) };
    host.status(RadioStatus::Sending);
    let began = Instant::now();
    let heard = read(bytes(&pcm));
    if heard.is_some() {
        *chosen.last.lock().unwrap_or_else(|e| e.into_inner()) = Some((pcm.len() as f32 / RATE as f32, began.elapsed().as_secs_f32()));
    }
    heard.map(|text| Some(text.trim().to_string()).filter(|t| !t.is_empty())).ok_or(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scribe(key: &str) -> Use {
        crate::services::service_of("elevenlabs", "scribe", key, std::collections::HashMap::new())
    }

    #[test]
    fn which_way_he_is_heard_is_worked_out_without_asking_the_service_for_anything() {
        let senses = Arc::new(Senses::default());
        senses.ears.set(Some(Own::Service(scribe("k"))));
        let ears = Ears::new(senses);
        // nothing is asked of the service here: the pass is asked for once the microphone is open
        assert!(matches!(ears.way(), Way::Mic(Mic::Live { .. })));
        assert!(ears.refused.lock().unwrap().is_none());
    }

    #[test]
    fn a_kept_pass_is_used_only_for_the_key_it_was_made_for() {
        let ears = Ears::new(Arc::new(Senses::default()));
        let (a, b) = (scribe("a"), scribe("b"));
        let by = a.offer(Job::Ears).unwrap().1.by.clone();
        ears.keep(&Pass { dialect: "usp".into(), token: "t".into(), set: BTreeMap::new(), phrases: Vec::new(), by }, &a);
        assert!(ears.kept(&a).is_some());
        assert!(ears.kept(&b).is_none());
    }
}
