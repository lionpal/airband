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
use airband_core::providers::{Interface, Job, Offer, Use};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc::{Receiver as Rx, sync_channel};
use std::time::Instant;

const RATE: u32 = 16000;
/// A pass is asked for again after this long (the shortest-lived of them lasts ten minutes).
const PASS_LIFE: Duration = Duration::from_secs(300);

mod azure;
mod elevenlabs;
mod gemini;

/// What the phone needs to listen live with a service: which dialect to speak, a pass from the service that
/// expires in minutes (made here, with his key), and whatever that service has to be told (a region, a
/// language, a model).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pass {
    pub dialect: String,
    pub token: String,
    pub set: BTreeMap<String, String>,
    /// Names he is likely to say (callsigns, projects), for a recogniser that can be told what to expect.
    pub phrases: Vec<String>,
}

impl Pass {
    /// One of the settings, or empty.
    pub fn get(&self, field: &str) -> &str {
        self.set.get(field).map(String::as_str).unwrap_or_default()
    }
}

/// Said when he opens the microphone with nothing chosen to listen with.
pub(crate) const NOTHING_CHOSEN: &str = "Nothing is chosen to listen with on this device. Choose it in Settings, under Listening.";

/// A live listening dialect: how one service is listened with while he talks. Each is a file under `ears/`
/// and a line of [`LISTENERS`].
pub(crate) struct Listener {
    /// Its name: what a provider's row names (`by.interface`), and what a pass calls it (`Pass::dialect`).
    pub is: Interface,
    /// How its live connection is spoken to.
    pub wire: &'static crate::ears_stream::Wire,
    /// The same microphone in one upload to the same service, for when the live connection cannot be made.
    upload: Option<fn(&Pass, &dyn RadioHost, bool) -> Result<Option<String>, ()>>,
    /// A short-lived pass for a key: the service's settings, and the key. Err says why not, in words for him.
    pass: fn(&BTreeMap<String, String>, &str) -> Result<String, String>,
    /// Its pass is good for one connection only: a new one is asked for each time, and none is kept.
    once: bool,
}

/// Every live dialect this build speaks.
pub(crate) const LISTENERS: &[Listener] = &[azure::LISTENER, gemini::LISTENER, elevenlabs::LISTENER];

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
pub(crate) fn pass_for(kind: &str, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let l = listener(kind).ok_or("This version of airband does not know that listening service.")?;
    (l.pass)(set, key)
}

/// Does this service listen with this key and these settings? One real call to it.
pub(crate) fn check(with: &Use) -> Result<(), String> {
    let (_, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    match live(&offer) {
        Some(l) => (l.pass)(&with.settings(Job::Ears), &with.key).map(|_| ()),
        None => crate::hears::check(with),
    }
}

/// How the next thing he says is to be heard: by what he chose on the device. Worked out in one place
/// ([`Ears::way`]) and only carried out by [`Ears::hear`].
#[derive(Clone)]
enum Way {
    /// By the model on the device.
    Model(Arc<crate::engines::Held>),
    /// In one upload to a service, with the key of his own that is already here.
    OwnUpload { with: Use, phrases: Vec<String> },
    /// Live, as he speaks, in a dialect this build has, with a pass made for his key.
    Live { pass: Pass, with: &'static Listener },
    /// Nothing can listen, and why: nothing is chosen, or what he chose cannot. Nothing else is asked.
    Nothing(String),
}

#[derive(Clone)]
pub(crate) struct Ears {
    /// The last pass a live service gave, and when: one is good for a few minutes.
    pass: Arc<Mutex<Option<(Pass, Instant)>>>,
    /// What he chose on the device.
    chosen: Arc<Senses>,
}

impl Ears {
    pub(crate) fn new(chosen: Arc<Senses>) -> Ears {
        Ears { pass: Default::default(), chosen }
    }

    fn kept(&self) -> Option<Pass> {
        self.pass.lock().unwrap().clone().filter(|(p, when)| when.elapsed() < PASS_LIFE && !once(&p.dialect)).map(|(p, _)| p)
    }
    fn keep(&self, pass: &Pass) {
        *self.pass.lock().unwrap() = Some((pass.clone(), Instant::now()));
    }
    fn forget(&self) {
        *self.pass.lock().unwrap() = None;
    }

    fn way(&self) -> Way {
        match self.chosen.ears.using() {
            None => Way::Nothing(NOTHING_CHOSEN.into()),
            Some(Own::Model(held)) => Way::Model(held),
            Some(Own::Cannot(why)) => Way::Nothing(why),
            Some(Own::Service(with)) => {
                let phrases = self.chosen.ears.phrases.lock().unwrap().clone();
                match with.offer(Job::Ears).map(|(_, o)| (live(&o), crate::hears::by_upload(&with))) {
                    Some((Some(l), _)) => {
                        // a pass that is kept is used only for the service and settings it was made for: he
                        // may have chosen another since
                        let set = with.settings(Job::Ears);
                        if let Some(pass) = self.kept().filter(|p| p.dialect == l.is.name && p.set == set) {
                            return Way::Live { pass: Pass { phrases, ..pass }, with: l };
                        }
                        match (l.pass)(&set, &with.key) {
                            Ok(token) => {
                                let pass = Pass { dialect: l.is.name.into(), token, set, phrases };
                                self.keep(&pass);
                                Way::Live { pass, with: l }
                            }
                            Err(why) => Way::Nothing(why),
                        }
                    }
                    Some((None, true)) => Way::OwnUpload { with, phrases },
                    _ => Way::Nothing("This version of airband cannot listen with the service you chose.".into()),
                }
            }
        }
    }

    /// Ask for the pass now, so it is there before he next opens the microphone. Nothing is asked of anyone
    /// when what he chose needs no pass.
    pub(crate) fn warm(&self) {
        if matches!(self.chosen.ears.using(), Some(Own::Service(_))) {
            let _ = self.way();
        }
    }

    /// Open the microphone and listen. The microphone opens first, at once: the pass for the listening service
    /// and the connection to it are made while he is already talking, and what he said meanwhile is sent when
    /// they are ready. With `manual` he opened it himself: it stays open, through any pause, until the host
    /// closes it, and everything he said comes back. Without, it is open for one reply and closes by itself
    /// when he stops talking.
    pub(crate) fn hear(&self, host: &dyn RadioHost, manual: bool) -> Listened {
        let said = |t: Option<String>| t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(Listened::Said).unwrap_or(Listened::Nothing);
        if !host.record_start(RATE) {
            // a host with no microphone to read listens its own way (a terminal takes a typed line). A phone
            // whose microphone did not open is told so.
            if host.own_voice() {
                return said(host.listen());
            }
            host.told("The microphone could not be opened. Check that airband may use it, in Settings.".into());
            return Listened::Cannot;
        }
        let heard_well = || *self.chosen.ears.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let fail = |host: &dyn RadioHost, why: String| self.chosen.ears.fail(host, why);
        match self.way() {
            Way::Model(held) => {
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
            Way::OwnUpload { with, phrases } => {
                let clip = record(host, manual);
                host.record_stop();
                match clip {
                    Ok(None) => Listened::Nothing,
                    Err(()) => Listened::Cannot,
                    Ok(Some(pcm)) => {
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
            Way::Live { with, .. } => {
                // a fresh pass whenever the connection asks for one: the same way, worked out again
                let again: Arc<dyn Fn() -> Option<Pass> + Send + Sync> = {
                    let ears = self.clone();
                    Arc::new(move || match ears.way() {
                        Way::Live { pass, .. } => Some(pass),
                        _ => None,
                    })
                };
                // As he speaks, with the words coming back while he talks. If that connection cannot be made, the
                // same microphone is listened to again the plain way (one upload, the words at the end), by the
                // same service when it has one.
                let heard = crate::ears_stream::live(with.wire, again.clone(), host, RATE, wav_header(), manual);
                host.record_stop();
                let heard = match (heard, with.upload) {
                    (Err(()), Some(upload)) if !manual && host.record_start(RATE) => {
                        let plain = again().ok_or(()).and_then(|pass| upload(&pass, host, manual));
                        host.record_stop();
                        plain
                    }
                    (heard, _) => heard,
                };
                match heard {
                    Ok(text) => {
                        heard_well();
                        said(text)
                    }
                    Err(_) => {
                        // the service or the line failed: ask for a fresh pass next time
                        self.forget();
                        fail(host, "The listening service you chose could not be reached. Nothing else listened in its place.".into())
                    }
                }
            }
            Way::Nothing(why) => {
                host.record_stop();
                self.forget();
                // say why, and stop there
                fail(host, why)
            }
        }
    }
}

/// Lets an HTTP upload read microphone audio as it is captured.
struct Feed(Rx<Vec<u8>>, Vec<u8>);
impl Read for Feed {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.1.is_empty() {
            match self.0.recv() {
                Ok(more) => self.1 = more,
                Err(_) => return Ok(0), // the microphone side hung up: end of the recording
            }
        }
        let n = out.len().min(self.1.len());
        out[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

/// The microphone until he stops, as samples: for a service that takes a finished recording. Ok(None) when he
/// said nothing. A little from before he was judged to be speaking is kept, so the first word is not clipped.
fn record(host: &dyn RadioHost, manual: bool) -> Result<Option<Vec<i16>>, ()> {
    record_with(host, manual, &mut |_| {})
}

/// The same, telling `so_far` what has been kept each time more is, while he is still talking.
fn record_with(host: &dyn RadioHost, manual: bool, so_far: &mut dyn FnMut(&[i16])) -> Result<Option<Vec<i16>>, ()> {
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
            Heard::Silence => return Ok(None),
            _ => {}
        }
    }
    Ok((speaking && !kept.is_empty()).then_some(kept))
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
    })?;
    let Some(pcm) = clip else { return Ok(None) };
    host.status(RadioStatus::Sending);
    let began = Instant::now();
    let heard = read(bytes(&pcm));
    if heard.is_some() {
        *chosen.last.lock().unwrap_or_else(|e| e.into_inner()) = Some((pcm.len() as f32 / RATE as f32, began.elapsed().as_secs_f32()));
    }
    heard.map(|text| Some(text.trim().to_string()).filter(|t| !t.is_empty())).ok_or(())
}

/// A WAV header for a recording whose length is not known yet.
pub(crate) fn wav_header() -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&(RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&0u32.to_le_bytes());
    h
}
