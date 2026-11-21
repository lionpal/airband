//! Inbound: turning what he says into text. The host only hands over raw microphone audio; deciding when he has
//! finished (airband_core::audio::Endpointer) and talking to the service are done once, for every platform.
//!
//! Which service is a row of the catalogue, and there are two ways to listen with one:
//!
//! - live, while he talks, by a dialect (a file under `ears/` and a line of [`LISTENERS`]): the phone talks to
//!   the service itself, with a short-lived pass. The words come back as he speaks.
//! - by upload (`hears.rs`): the phone records until he stops and the recording is sent off whole. With a key
//!   of his own on the phone it goes straight to the service; with the key on his computer it goes to the
//!   tower, which makes the call.
//!
//! What to listen with comes as an [`EarsGrant`]: from a key of his own on this device first, else from the
//! tower. Nothing here, in the tower or in the app names a
//! service.
use super::*;
use airband_core::audio::{BLOCK_SAMPLES, Encoder, Endpointer, Heard, base64_encode, samples};
use airband_core::providers::{BY_TOWER, Interface, Job, Offer, Use};
use airband_core::radio::{EarsGrant, HearPart, HeardText};
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

/// A live listening dialect: how one service is listened with while he talks. Each is a file under `ears/`
/// and a line of [`LISTENERS`].
pub(crate) struct Listener {
    /// Its name: what a provider's row names (`by.interface`), and what a pass calls it (`EarsGrant::kind`).
    pub is: Interface,
    /// How its live connection is spoken to.
    pub wire: &'static crate::ears_stream::Wire,
    /// The same microphone in one upload to the same service, for when the live connection cannot be made.
    upload: Option<fn(&EarsGrant, &dyn RadioHost, bool) -> Result<Option<String>, ()>>,
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
    /// The names the tower last said he is likely to say: used with a pass of his own, which comes with none.
    phrases: Mutex<Vec<String>>,
}

impl Chosen {
    fn using(&self) -> Option<Own> {
        self.using.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Listen with this from now on; nothing is his computer's choice.
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
pub(crate) fn pass_for(kind: &str, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let l = listener(kind).ok_or("This version of airband does not know that listening service.")?;
    (l.pass)(set, key)
}

/// What a listening service makes of a recording, heard the way a phone would send it: live for a live
/// service (with how many times the words on screen changed while it played), in one upload for the others.
/// `pcm` is 16 kHz 16-bit mono. Blocks for as long as the recording is. For trying a service by hand.
pub fn try_listening(with: &Use, pcm: &[u8]) -> Result<(String, usize), String> {
    let (_, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    match live(offer) {
        Some(l) => {
            let set = with.settings(Job::Ears);
            let grant = EarsGrant { kind: l.is.name.into(), token: (l.pass)(&set, &with.key)?, set, phrases: Vec::new() };
            let tape = crate::ears_stream::Tape::of(pcm, false);
            let said = crate::ears_stream::live(l.wire, Arc::new(move || grant.clone()), &tape, RATE, wav_header(), false).map_err(|_| "The live connection to the listening service failed.".to_string())?;
            let changes = tape.heard.lock().unwrap().len();
            Ok((said.unwrap_or_default(), changes))
        }
        None => crate::hears::transcribe(with, &crate::hears::wav(RATE, &samples(pcm)), &[]).map(|said| (said, 0)),
    }
}

/// Does this service listen with this key and these settings? One real call to it.
pub(crate) fn check(with: &Use) -> Result<(), String> {
    let (_, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    match live(offer) {
        Some(l) => (l.pass)(&with.settings(Job::Ears), &with.key).map(|_| ()),
        None => crate::hears::check(with),
    }
}

/// How the next thing he says is to be heard. What he chose on the device comes first; with nothing chosen
/// there, his computer says. Worked out in one place ([`Ears::way`]) and only carried out by [`Ears::hear`].
#[derive(Clone)]
enum Way {
    /// By the model on the device.
    Model(Arc<crate::engines::Held>),
    /// In one upload to a service, with the key of his own that is already here.
    OwnUpload { with: Use, phrases: Vec<String> },
    /// In one upload to his computer, which has the key.
    Tower,
    /// Live, as he speaks, in the dialect the pass names. `own` when the service is the one he chose here.
    Live { pass: EarsGrant, own: bool },
    /// Nothing can listen. With a reason when it is what he chose that cannot: nothing else is asked then.
    Nothing(Option<String>),
}

#[derive(Clone)]
pub(crate) struct Ears {
    /// The last live pass, and when it was given: one is good for a few minutes.
    pass: Arc<Mutex<Option<(EarsGrant, Instant)>>>,
    /// What he chose on the device.
    chosen: Arc<Senses>,
}

impl Ears {
    pub(crate) fn new(chosen: Arc<Senses>) -> Ears {
        Ears { pass: Default::default(), chosen }
    }

    fn kept(&self) -> Option<EarsGrant> {
        self.pass.lock().unwrap().clone().filter(|(g, when)| when.elapsed() < PASS_LIFE && !once(&g.kind)).map(|(g, _)| g)
    }
    fn keep(&self, pass: &EarsGrant) {
        *self.pass.lock().unwrap() = Some((pass.clone(), Instant::now()));
    }
    fn forget(&self) {
        *self.pass.lock().unwrap() = None;
    }

    fn way(&self, link: &Link) -> Way {
        match self.chosen.ears.using() {
            Some(Own::Model(held)) => Way::Model(held),
            Some(Own::Cannot(why)) => Way::Nothing(Some(why)),
            Some(Own::Service(with)) => {
                let phrases = self.chosen.ears.phrases.lock().unwrap().clone();
                match with.offer(Job::Ears).map(|(_, o)| (live(o), crate::hears::by_upload(&with))) {
                    Some((Some(l), _)) => {
                        if let Some(pass) = self.kept() {
                            return Way::Live { pass, own: true };
                        }
                        let set = with.settings(Job::Ears);
                        match (l.pass)(&set, &with.key) {
                            Ok(token) => {
                                let pass = EarsGrant { kind: l.is.name.into(), token, set, phrases };
                                self.keep(&pass);
                                Way::Live { pass, own: true }
                            }
                            Err(why) => Way::Nothing(Some(why)),
                        }
                    }
                    Some((None, true)) => Way::OwnUpload { with, phrases },
                    _ => Way::Nothing(Some("This version of airband cannot listen with the service you chose.".into())),
                }
            }
            // His computer says what to listen with. When it cannot be reached there is nothing to listen
            // with, and he types.
            None => {
                let pass = match self.kept() {
                    Some(pass) => pass,
                    None => {
                        let pass: EarsGrant = match link.request("GET", "/ears", None) {
                            Ok((200, v)) => serde_json::from_value(v).unwrap_or_default(),
                            _ => EarsGrant::default(),
                        };
                        if !pass.phrases.is_empty() {
                            *self.chosen.ears.phrases.lock().unwrap() = pass.phrases.clone();
                        }
                        // only a pass that can be used is kept: with none, the tower is asked again next time
                        if pass.listens() {
                            self.keep(&pass);
                        }
                        pass
                    }
                };
                match pass.kind.as_str() {
                    BY_TOWER => Way::Tower,
                    _ if !pass.listens() => Way::Nothing(None),
                    _ => Way::Live { pass, own: false },
                }
            }
        }
    }

    /// Ask for the pass now, so it is there before he next opens the microphone.
    pub(crate) fn warm(&self, link: &Link) {
        let _ = self.way(link);
        // with a service of his own the tower is still asked, when it is there, for the names he is likely to say
        if matches!(self.chosen.ears.using(), Some(Own::Service(_))) {
            if let Ok((200, v)) = link.request("GET", "/ears", None) {
                let phrases = serde_json::from_value::<EarsGrant>(v).map(|g| g.phrases).unwrap_or_default();
                if !phrases.is_empty() {
                    if let Some((g, _)) = self.pass.lock().unwrap().as_mut() {
                        g.phrases = phrases.clone();
                    }
                    *self.chosen.ears.phrases.lock().unwrap() = phrases;
                }
            }
        }
    }

    /// Open the microphone and listen. The microphone opens first, at once: the pass for the listening service
    /// and the connection to it are made while he is already talking, and what he said meanwhile is sent when
    /// they are ready. With `manual` he opened it himself: it stays open, through any pause, until the host
    /// closes it, and everything he said comes back. Without, it is open for one reply and closes by itself
    /// when he stops talking.
    pub(crate) fn hear(&self, link: &Arc<Link>, host: &dyn RadioHost, manual: bool) -> Listened {
        let said = |t: Option<String>| t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(Listened::Said).unwrap_or(Listened::Nothing);
        if !host.record_start(RATE) {
            // no microphone to read: a terminal's typed line will do; a phone's own recognition will not
            return if host.own_voice() { said(host.listen()) } else { Listened::Cannot };
        }
        let heard_well = || *self.chosen.ears.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let fail = |host: &dyn RadioHost, why: String| self.chosen.ears.fail(host, why);
        match self.way(link) {
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
            // The recording is made here and sent off whole: to the service with his own key, or to his
            // computer, which has the key.
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
            Way::Tower => {
                let clip = record(host, manual);
                host.record_stop();
                match clip {
                    Ok(None) => Listened::Nothing,
                    Err(()) => Listened::Cannot,
                    Ok(Some(pcm)) => {
                        host.status(RadioStatus::Sending);
                        match by_tower(link, &pcm) {
                            Ok(text) => said(Some(text)),
                            Err(()) => {
                                self.forget();
                                Listened::Cannot
                            }
                        }
                    }
                }
            }
            Way::Live { pass, own } => {
                // one this build has no code for cannot be listened with
                let Some(with) = listener(&pass.kind) else {
                    host.record_stop();
                    self.forget();
                    return if host.own_voice() { said(host.listen()) } else { Listened::Cannot };
                };
                // a fresh pass whenever the connection asks for one: the same way, worked out again
                let again: Arc<dyn Fn() -> EarsGrant + Send + Sync> = {
                    let (ears, link) = (self.clone(), link.clone());
                    Arc::new(move || match ears.way(&link) {
                        Way::Live { pass, .. } => pass,
                        _ => EarsGrant::default(),
                    })
                };
                // As he speaks, with the words coming back while he talks. If that connection cannot be made, the
                // same microphone is listened to again the plain way (one upload, the words at the end), by the
                // same service when it has one.
                let heard = crate::ears_stream::live(with.wire, again.clone(), host, RATE, wav_header(), manual);
                host.record_stop();
                let heard = match (heard, with.upload) {
                    (Err(()), Some(upload)) if !manual && host.record_start(RATE) => {
                        let plain = upload(&again(), host, manual);
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
                        if own {
                            return fail(host, "The listening service you chose could not be reached. Nothing else listened in its place.".into());
                        }
                        if host.own_voice() { said(host.listen()) } else { Listened::Cannot }
                    }
                }
            }
            Way::Nothing(why) => {
                host.record_stop();
                self.forget();
                match why {
                    // what he chose on the device cannot listen: say why, and stop there
                    Some(why) => fail(host, why),
                    None if host.own_voice() => said(host.listen()),
                    None => Listened::Cannot,
                }
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

/// Send a recording to the tower, which has the key for the listening service, and take back the words. It goes
/// in parts of ten seconds, each small enough to pass a relay sealed, in the compressed form speech comes in.
fn by_tower(link: &Link, pcm: &[i16]) -> Result<String, ()> {
    let clip = airband_core::relay::random_hex(8);
    let parts: Vec<&[i16]> = pcm.chunks(RATE as usize * 10).collect();
    let mut encoder = Encoder::default();
    for (i, part) in parts.iter().enumerate() {
        let blocks: Vec<u8> = part.chunks(BLOCK_SAMPLES).flat_map(|b| encoder.block(b)).collect();
        let sent = HearPart { clip: clip.clone(), data: base64_encode(&blocks), samples: part.len() as u32, last: i + 1 == parts.len() };
        let answer = link.post("/hear", &sent).map_err(|_| ())?;
        if sent.last {
            return serde_json::from_value::<HeardText>(answer).map(|h| h.text).map_err(|_| ());
        }
    }
    Err(())
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
