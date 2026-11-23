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
use crate::takes::{Aim, Unheard};

pub(crate) const RATE: u32 = 16000;

/// Said after why he was not heard: nothing of it is lost.
pub(crate) const KEPT: &str = "Your recording is in the chat to try again.";

mod bidi;
mod chunks;
mod usp;
mod pass;
pub(crate) use pass::*;

/// What he is told when the listening service he chose is not one this build knows.
pub(crate) fn unknown_service() -> String {
    crate::senses::Cannot::UnknownService(crate::senses::Sense::Listening).told()
}
/// What he is told when what he chose to listen with is a service this build cannot listen through this way:
/// the one wording ([`crate::senses::Cannot::ServiceNotUsed`]).
pub(crate) fn cannot_listen() -> String {
    crate::senses::Cannot::ServiceNotUsed(crate::senses::Sense::Listening).told()
}
/// Said when he opens the microphone with nothing chosen to listen with.
pub(crate) fn nothing_chosen() -> String {
    crate::senses::Cannot::NothingChosen(crate::senses::Sense::Listening).told()
}

/// How a dialect gets its pass: from the row, the service's name, his settings and his key.
type AskPass = fn(&By, &Provider, &BTreeMap<String, String>, &str) -> Result<Ticket, String>;

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
    Cannot(crate::senses::Cannot),
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
    /// The names he is likely to say: his agents and their projects, as the chat last listed them, and the
    /// words of the trade the catalogue lists. For a recogniser that can be told what to expect.
    phrases: Mutex<Vec<String>>,
    /// Goes up each time he chooses again, with `moved` told: the warmer of passes looks again then.
    turn: Mutex<u64>,
    moved: std::sync::Condvar,
}

impl Chosen {
    fn using(&self) -> Option<Own> {
        self.using.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Listen with this from now on; nothing is nothing.
    pub(crate) fn set(&self, own: Option<Own>) {
        *self.using.lock().unwrap_or_else(|e| e.into_inner()) = own;
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.turn.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.moved.notify_all();
    }

    /// Which choice this is: it changes each time he chooses again.
    pub(crate) fn turn(&self) -> u64 {
        *self.turn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wait until he chooses again after `turn`, or until `until` (ms) when it is given: the two facts a
    /// pass fetched ahead waits on.
    pub(crate) fn wait_past(&self, turn: u64, until: Option<u64>) {
        let mut now = self.turn.lock().unwrap_or_else(|e| e.into_inner());
        while *now == turn {
            match until.map(|u| u.saturating_sub(now_ms())) {
                Some(0) => return,
                Some(ms) => now = self.moved.wait_timeout(now, Duration::from_millis(ms)).unwrap_or_else(|e| e.into_inner()).0,
                None => now = self.moved.wait(now).unwrap_or_else(|e| e.into_inner()),
            }
        }
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
            None => Some(nothing_chosen()),
            Some(Own::Cannot(why)) => Some(why.told()),
            Some(_) => None,
        }
    }

    /// Why what he chose did not listen the last time, if it did not.
    pub(crate) fn failed(&self) -> Option<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// What he chose could not listen. It stays chosen, and he is told: moving to something else is his to do.
    fn fail(&self, host: &dyn RadioHost, why: String) -> Listened {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.clone());
        host.told(why);
        Listened::Cannot
    }
}

/// A new pass from the live service `with` names, for his key, with the names he is likely to say: the one
/// way one is asked for. The dialect comes with it, so whoever asked knows whether it may be kept. Err says
/// why not, in words for him.
pub(crate) fn pass_for(with: &Use, phrases: Vec<String>) -> Result<(Pass, &'static Listener), String> {
    let (provider, offer) = with.offer(Job::Ears).ok_or_else(unknown_service)?;
    let l = live(&offer).ok_or_else(cannot_listen)?;
    let set = with.settings(Job::Ears);
    let Ticket { token, until } = (l.pass)(&offer.by, &provider, &set, &with.key)?;
    Ok((Pass { dialect: l.is.name.into(), token, set, phrases, by: offer.by.clone(), until }, l))
}

/// Does this service listen with this key and these settings? One real call to it.
pub(crate) fn check(with: &Use) -> Result<(), String> {
    let (provider, offer) = with.offer(Job::Ears).ok_or_else(unknown_service)?;
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
    /// Live, as he speaks, in a dialect this build has. Its pass is asked for once the microphone is open, and
    /// whenever the connection needs a fresh one: [`Ears::pass`], worked out again each time.
    Live { with: &'static Listener, pass: crate::ears_stream::PassFor },
}

/// The passes a live listening service gave, kept once on the device for every computer's radio: one pass
/// serves them all while it is good.
#[derive(Default)]
pub(crate) struct Passes {
    /// The last pass and what he had chosen it for (the service, its settings and his key): it is good until
    /// the moment it says it ends ([`Pass::until`]), and only for that.
    pass: Mutex<Option<(Pass, Use)>>,
    /// Held while a pass is looked for and asked for, so radios that want one at once ask the service once.
    asking: Mutex<()>,
}

#[derive(Clone)]
pub(crate) struct Ears {
    /// What he chose on the device, and the passes kept for it.
    chosen: Arc<Senses>,
}

impl Ears {
    pub(crate) fn new(chosen: Arc<Senses>) -> Ears {
        Ears { chosen }
    }

    /// What he has said, kept until his computer has it.
    pub(crate) fn takes(&self) -> &crate::takes::Takes {
        &self.chosen.takes
    }

    /// The pass kept for exactly this choice, while it is still good. One good for a single connection is
    /// never kept.
    fn kept(&self, with: &Use) -> Option<Pass> {
        let now = now_ms();
        self.chosen.passes.pass.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|(p, made)| made == with && p.until.is_some_and(|until| now < until)).map(|(p, _)| p)
    }
    fn forget(&self) {
        *self.chosen.passes.pass.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn way(&self) -> Way {
        match self.chosen.ears.using() {
            None => Way::Nothing(nothing_chosen()),
            Some(Own::Model(held)) => Way::Mic(Mic::Model(held)),
            Some(Own::Cannot(why)) => Way::Nothing(why.told()),
            Some(Own::Host) => Way::Host,
            Some(Own::Service(with)) => match with.offer(Job::Ears).map(|(_, o)| (live(&o), crate::hears::by_upload(&with))) {
                Some((Some(l), _)) => {
                    let ears = self.clone();
                    Way::Mic(Mic::Live { with: l, pass: Arc::new(move || ears.pass()) })
                }
                Some((None, true)) => Way::Mic(Mic::OwnUpload { phrases: self.chosen.ears.phrases.lock().unwrap().clone(), with }),
                _ => Way::Nothing(cannot_listen()),
            },
        }
    }

    /// A pass for the live service he chose: the one kept for that choice while it is good, or a new one,
    /// asked for now. Err says why there is none, in words for him.
    fn pass(&self) -> Result<Pass, String> {
        let Some(Own::Service(with)) = self.chosen.ears.using() else { return Err(nothing_chosen()) };
        let phrases = self.chosen.ears.phrases.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let _asking = self.chosen.passes.asking.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pass) = self.kept(&with) {
            return Ok(Pass { phrases, ..pass });
        }
        let (pass, l) = pass_for(&with, phrases)?;
        // one that does not say when it ends is not kept: how long it lasts would be a guess
        if !l.once && pass.until.is_some() {
            *self.chosen.passes.pass.lock().unwrap_or_else(|e| e.into_inner()) = Some((pass.clone(), with.clone()));
        }
        Ok(pass)
    }

    /// Ask for the pass now, so it is there before he next opens the microphone. Nothing is asked of anyone
    /// when what he chose needs no pass.
    /// A pass good for one connection only is never asked for ahead: it would be spent for nothing.
    /// Returns when the pass now kept ends, as it says itself: when a new one is to be asked for ahead.
    /// Nothing when none is kept: a pass that cannot be had now is asked for again when he opens the
    /// microphone, and he is told then.
    pub(crate) fn warm(&self) -> Option<u64> {
        let Way::Mic(Mic::Live { with, .. }) = self.way() else { return None };
        if with.once {
            return None;
        }
        self.pass().ok().and(self.chosen.passes.pass.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|(p, _)| p.until))
    }

    /// Open the microphone and listen. The microphone opens first, at once: the pass for the listening service
    /// and the connection to it are made while he is already talking, and what he said meanwhile is sent when
    /// they are ready. With `manual` he opened it himself: it stays open, through any pause, until the host
    /// closes it, and everything he said comes back. Without, it is open for one reply and closes by itself
    /// when he stops talking. `aim` is where his words go, kept with the recording for when they go later.
    ///
    /// Everything read from the microphone is written to a take as it comes ([`crate::takes`]), from the first
    /// sample to the last, whatever the listening does: when it fails partway the recording goes on to his end.
    pub(crate) fn hear(&self, host: &dyn RadioHost, manual: bool, aim: Aim) -> Listened {
        let mic = match self.way() {
            // a radio in a terminal: there is no microphone, and what he says is the line he types
            Way::Host => {
                return host.listen().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map_or(Listened::Nothing, |text| Listened::Said { text, take: None });
            }
            // nothing can listen: say why, and stop there
            Way::Nothing(why) => {
                self.forget();
                return self.chosen.ears.fail(host, why);
            }
            Way::Mic(mic) => mic,
        };
        self.hear_by(host, mic, manual, aim)
    }

    /// The same, with the way of hearing him worked out.
    fn hear_by(&self, host: &dyn RadioHost, mic: Mic, manual: bool, aim: Aim) -> Listened {
        if !host.record_start(RATE) {
            host.told("The microphone could not be opened. Check that airband may use it, in Settings.".into());
            return Listened::Cannot;
        }
        let tape = self.chosen.takes.tape(host, aim);
        let got = self.by(&tape, mic, manual);
        // the listening stopped before he did: the recording goes on to his end all the same
        if matches!(got, Got::Failed(_)) {
            tape.rest(manual);
        }
        tape.record_stop();
        let heard = self.chosen.takes.ended(tape.done(), got);
        self.tell_unheard(host, &heard);
        heard
    }

    /// What listened did not hear him: he is told why, once, with the tone the host makes for anything that
    /// went wrong. The recording waits in the chat.
    fn tell_unheard(&self, host: &dyn RadioHost, heard: &Listened) {
        if let Listened::Unheard { why, .. } = heard {
            host.told(format!("{} {KEPT}", why.told()));
        }
    }

    /// Hear again a take that was not heard, from its recording, by what he has chosen now. What came of it is
    /// kept with the take; nothing when it is not one to hear again.
    pub(crate) fn hear_again(&self, host: &dyn RadioHost, id: &str) -> Listened {
        let takes = &self.chosen.takes;
        let mic = match self.way() {
            Way::Mic(mic) => mic,
            // a terminal has no recordings to hear
            Way::Host => {
                takes.back(id);
                return Listened::Nothing;
            }
            Way::Nothing(why) => {
                takes.back(id);
                return self.chosen.ears.fail(host, why);
            }
        };
        let Some(pcm) = takes.replay(id) else {
            host.told("This phone could not read that recording. Unlock it and try again.".into());
            return Listened::Cannot;
        };
        // A live service is given the recording as fast as it was spoken, as it was given him, and is waited
        // for to end its turn. One that also takes a whole recording is given it whole: no waiting at all.
        let upload = match self.chosen.ears.using() {
            Some(Own::Service(with)) if crate::hears::by_upload(&with) => Some(with),
            _ => None,
        };
        let (mic, paced) = match (mic, upload) {
            (Mic::Live { .. }, Some(with)) => (Mic::OwnUpload { phrases: self.chosen.ears.phrases.lock().unwrap_or_else(|e| e.into_inner()).clone(), with }, false),
            (mic @ Mic::Live { .. }, None) => (mic, true),
            (other, _) => (other, false),
        };
        let got = self.by(&crate::takes::Replay::of(host, &pcm, paced), mic, true);
        let heard = takes.heard_again(id, got);
        self.tell_unheard(host, &heard);
        heard
    }

    /// What one way of hearing made of the microphone `host` gives: the words, or why there are none. Whether
    /// it heard is kept for Settings to show; he is told of a recording that was not heard where it is shown.
    fn by(&self, host: &dyn RadioHost, mic: Mic, manual: bool) -> Got {
        let got = match mic {
            Mic::Model(held) => {
                let name = held.m.name.clone();
                match by_model(host, &held, manual) {
                    Ok(text) => Got::Words(text),
                    // the model would not run: it stays the one he chose
                    Err(fault) => Got::Failed(Unheard::Model { why: format!("{} Nothing else listened in its place.", fault.told(&name)) }),
                }
            }
            // The recording is made here and sent off whole, to the service, with his own key.
            Mic::OwnUpload { with, phrases } => match record(host, manual) {
                None => Got::Words(None),
                Some(pcm) => {
                    // he has finished: the microphone closes now, with its tone, and not after the upload
                    host.record_stop();
                    host.status(RadioStatus::Sending);
                    match crate::hears::transcribe(&with, &crate::hears::wav(RATE, &pcm), &phrases) {
                        Ok(text) => Got::Words(Some(text)),
                        // the service's reason names it already
                        Err(why) => Got::Failed(Unheard::Service { why }),
                    }
                }
            },
            // As he speaks, with the words coming back while he talks. Nothing listens another way in its place.
            Mic::Live { with, pass } => match crate::ears_stream::live(with.wire, pass, host, RATE, crate::hears::wav_header(RATE, None), manual) {
                Ok(text) => Got::Words(text),
                Err(why) => {
                    // the service or the line failed: ask for a fresh pass next time
                    self.forget();
                    Got::Failed(why)
                }
            },
        };
        *self.chosen.ears.failed.lock().unwrap_or_else(|e| e.into_inner()) = match &got {
            Got::Failed(why) => Some(why.told()),
            Got::Words(_) => None,
        };
        got
    }
}

/// What one way of hearing made of a recording.
pub(crate) enum Got {
    /// What he said; nothing when it heard no words.
    Words(Option<String>),
    /// It did not hear it, and why.
    Failed(Unheard),
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
fn by_model(host: &dyn RadioHost, held: &crate::engines::Held, manual: bool) -> Result<Option<String>, crate::engines::Fault> {
    let read = |pcm: Vec<u8>| held.run(|m| m.hear(&|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input), pcm));
    let bytes = |pcm: &[i16]| pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let clip = thread::scope(|scope| {
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        let read = &read;
        scope.spawn(move || {
            while let Ok(pcm) = rx.recv() {
                // what he has said so far: a reading that did not work shows nothing new, and the last one says why
                if let Ok(now) = read(pcm) {
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
    // he has finished: the microphone closes now, with its tone, and not after the last reading
    host.record_stop();
    host.status(RadioStatus::Sending);
    read(bytes(&pcm)).map(|text| Some(text.trim().to_string()).filter(|t| !t.is_empty()))
}

#[cfg(test)]
mod kept_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A service of the catalogue's that listens live, with this key.
    fn scribe(key: &str) -> Use {
        let (p, o) = airband_core::providers::in_order(Job::Ears).into_iter().find(|(_, o)| live(o).is_some()).expect("a row that listens live");
        crate::services::service_of(&p.id, &o.id, key, std::collections::HashMap::new())
    }

    #[test]
    fn which_way_he_is_heard_is_worked_out_without_asking_the_service_for_anything() {
        let senses = Arc::new(Senses::default());
        senses.ears.set(Some(Own::Service(scribe("k"))));
        let ears = Ears::new(senses);
        // nothing is asked of the service here: the pass is asked for once the microphone is open
        assert!(matches!(ears.way(), Way::Mic(Mic::Live { .. })));
        assert!(ears.chosen.passes.pass.lock().unwrap().is_none());
    }

    #[test]
    fn a_kept_pass_is_used_only_for_the_key_it_was_made_for() {
        let ears = Ears::new(Arc::new(Senses::default()));
        let (a, b) = (scribe("a"), scribe("b"));
        let by = a.offer(Job::Ears).unwrap().1.by.clone();
        let pass = |until| Pass { dialect: "usp".into(), token: "t".into(), set: BTreeMap::new(), phrases: Vec::new(), by: by.clone(), until };
        *ears.chosen.passes.pass.lock().unwrap() = Some((pass(Some(now_ms() + 60_000)), a.clone()));
        assert!(ears.kept(&a).is_some());
        assert!(ears.kept(&b).is_none());
        // past the moment the pass itself says it ends it is not used, however recently it was had
        *ears.chosen.passes.pass.lock().unwrap() = Some((pass(Some(now_ms() - 1)), a.clone()));
        assert!(ears.kept(&a).is_none());
        *ears.chosen.passes.pass.lock().unwrap() = Some((pass(None), a.clone()));
        assert!(ears.kept(&a).is_none());
    }
}
