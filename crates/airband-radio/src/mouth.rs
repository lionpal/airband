//! The voice: what speaks on this device. One of two kinds, whichever he chose:
//!
//! - made on the device itself by a model: no key, no computer, no relay, nothing sent anywhere;
//! - a voice service reached straight from the device with a key of his own, kept on the device (`voices.rs`,
//!   by the rows of `airband_core::providers`).
//!
//! His computer sends words only and has no voice: nothing about speaking is set or done there.
//!
//! A model is fetched and kept by `models.rs` and run by its engine (`engines.rs`). While one is chosen it
//! stays loaded, so a sentence starts at once.
use super::*;
use crate::engines::{ByHost, Held};

/// Said when the voice he chose ran and nothing came of it.
const NO_SOUND: &str = "The voice you chose made no sound.";

/// The voice of the device's own that he chose, when he chose one. One thing at a time: which one is decided
/// where he chooses (`choices.rs`) and only carried out here.
#[derive(Clone)]
pub(crate) enum Own {
    /// A model on the device, and which of its voices speaks.
    Model { held: Arc<Held>, voice: String },
    /// A voice service reached straight from the device with a key of his own.
    Service(airband_core::providers::Use),
    /// What he chose cannot speak as things are, and why. Nothing else speaks in its place.
    Cannot(crate::senses::Cannot),
    /// The host says it, its own way, with no sound made here: a radio in a terminal, which prints. This is
    /// what such a radio is started with ([`Radio::start`]); it is never put in the place of something else.
    Host,
}

/// What speaks on this device: one for everything that shares what he chose (a desk and its radios).
#[derive(Default)]
pub(crate) struct Mouth {
    using: Mutex<Option<Own>>,
    /// Why what he chose did not speak the last time it was tried. Cleared when it next does.
    failed: Mutex<Option<String>>,
}

/// Why what is chosen makes no sound: for everything but a service or a model.
fn silent(own: Option<Own>) -> String {
    match own {
        Some(Own::Cannot(why)) => why.told(),
        Some(Own::Host) => "This radio shows what is said and makes no sound.".into(),
        _ => crate::voice::NOTHING_CHOSEN.into(),
    }
}

impl Mouth {
    fn using(&self) -> Option<Own> {
        self.using.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Speak with this from now on; nothing is nothing. A model is loaded in the background and
    /// stays loaded, so a sentence starts at once. The model that was chosen before is let go of, unless it is
    /// this one again (see [`Mouth::held`]).
    pub(crate) fn set(&self, own: Option<Own>) {
        if let Some(Own::Model { held, .. }) = &own {
            let held = held.clone();
            thread::spawn(move || {
                held.with(|_| ()).ok();
            });
        }
        let was = std::mem::replace(&mut *self.using.lock().unwrap_or_else(|e| e.into_inner()), own);
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // off this thread: the model may be in the middle of a sentence, and this is called from a screen
        thread::spawn(move || drop(was));
    }

    /// The model to speak with, from that folder: the one already in use when it is the same, so that
    /// changing only its voice loads nothing.
    pub(crate) fn held(&self, m: &crate::models::Model, folder: std::path::PathBuf) -> Arc<Held> {
        match self.using() {
            Some(Own::Model { held, .. }) if held.is(m, &folder) => held,
            _ => Held::of(m, folder),
        }
    }

    /// Why what he chose did not speak the last time, if it did not.
    pub(crate) fn failed(&self) -> Option<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Has he chosen a voice? When the one he chose cannot speak, nothing is said in its place.
    pub(crate) fn ready(&self) -> bool {
        self.using().is_some()
    }
}

/// Does the service speak with this key and these settings? One short real call. Blocks.
pub(crate) fn check(with: &airband_core::providers::Use) -> Result<(), String> {
    let synth = crate::voices::synth(with)?;
    let mut bytes = 0usize;
    synth.speak("Ready.", &mut |pcm| bytes += pcm.len(), &mut |_, _| {})?;
    if bytes == 0 { Err(synth.service().no_answer()) } else { Ok(()) }
}

/// A clip of sound: 16-bit little-endian mono PCM at `rate` samples a second.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VoiceSample {
    pub rate: u32,
    pub pcm: Vec<u8>,
}

impl Mouth {
    /// A sentence in the device's own voice as it is now set, for a "hear it" button. Blocks while it is made.
    pub(crate) fn sample(&self, host: ByHost) -> Result<VoiceSample, RadioError> {
        const SAMPLE: &str = "Shop two is waiting. The build passed, and it wants to know whether to push.";
        let told = |reason: String| crate::choices::told(reason);
        let rate = self.rate().map_err(told)?;
        let mut pcm = Vec::new();
        self.speak(host, SAMPLE, &mut |part| pcm.extend_from_slice(part), &mut |_, _| {}, &|| false).map_err(told)?;
        if pcm.is_empty() {
            return Err(told(NO_SOUND.into()));
        }
        Ok(VoiceSample { rate, pcm })
    }

    /// Make `text` into sound with the voice of the device's own that he chose. Err says why it could not,
    /// in words for him.
    fn speak(&self, host: ByHost, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32), stop: &dyn Fn() -> bool) -> Result<(), String> {
        match self.using() {
            // a service is read to its end; what comes after he says stop is dropped
            Some(Own::Service(with)) => crate::voices::synth(&with)?.speak(text, &mut |pcm| if !stop() { sink(pcm) }, mark),
            Some(Own::Model { held, voice }) => held.with(|m| m.speak(host, &voice, text, sink, stop))?,
            other => Err(silent(other)),
        }
    }

    /// The rate what he chose speaks at: its service's, or what its model's row says. Err says why there is
    /// none, in words for him.
    fn rate(&self) -> Result<u32, String> {
        match self.using() {
            Some(Own::Service(with)) => crate::voices::synth(&with).map(|s| s.rate()),
            Some(Own::Model { held, .. }) => held.m.told(crate::models::field::RATE).parse().map_err(|_| format!("{} does not say what rate it speaks at.", held.m.name)),
            other => Err(silent(other)),
        }
    }

    /// Say `text` in the device's own voice, until it is over or `stop` says to leave off. Err, with why in words
    /// for him, if not a sound was made: the voice stays the one he chose, and that is kept for him to see.
    /// Sound is handed to the host about a quarter of a second at a time, so a slow moment in the making is not heard; a service that says when each word is spoken has that
    /// passed on, for showing the words as they are said.
    pub(crate) fn say(&self, host: &dyn RadioHost, text: &str, stop: &dyn Fn() -> bool) -> Result<(), String> {
        use std::cell::RefCell;
        if matches!(self.using(), Some(Own::Host)) {
            host.speak(text.to_string());
            return Ok(());
        }
        // what is waiting to be handed over, whether anything has been, and word timings not yet passed on
        let state = RefCell::new((Vec::<u8>::new(), false, Vec::<(u32, u32)>::new(), false));
        let fail = |why: String| {
            *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.clone());
            Err(why)
        };
        let rate = match self.rate() {
            Ok(rate) => rate,
            Err(why) => return fail(why),
        };
        let hand = rate as usize / 4 * 2;
        let made = self.speak(
            &|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input),
            text,
            &mut |pcm| {
                let mut s = state.borrow_mut();
                s.0.extend_from_slice(pcm);
                if s.0.len() >= hand {
                    if !s.2.is_empty() {
                        let marks = std::mem::take(&mut s.2);
                        // the words themselves go with the first timings, and only then
                        host.words(if s.3 { String::new() } else { text.to_string() }, marks.iter().map(|m| m.0).collect(), marks.iter().map(|m| m.1).collect());
                        s.3 = true;
                    }
                    host.play(rate, std::mem::take(&mut s.0), false);
                    s.1 = true;
                }
            },
            &mut |at, ms| state.borrow_mut().2.push((at, ms)),
            stop,
        );
        if stop() {
            return Ok(());
        }
        let (held, mut started, _, _) = state.into_inner();
        if made.is_ok() && !held.is_empty() {
            started = true;
        }
        if started {
            // the rest, and wait until it has all been heard
            host.play(rate, held, true);
            *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Ok(());
        }
        fail(match made {
            Err(why) => format!("The voice you chose did not speak: {why}"),
            Ok(_) => NO_SOUND.into(),
        })
    }
}

#[cfg(all(test, feature = "phonon"))]
mod tests {
    use super::*;
    use crate::engines::HostDevice;
    use crate::models::{fetch_local_model, folder, local_model_progress, model};

    /// The real thing: fetches the model (65 MB) and speaks a sentence. Run by hand with
    /// `AIRBAND_VOICE_DIR=<folder> cargo test --release -p airband-radio fetched -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fetched_it_speaks() {
        let dir = std::env::var("AIRBAND_VOICE_DIR").expect("AIRBAND_VOICE_DIR");
        let device = HostDevice::default();
        let m = model("phonon", &device).unwrap();
        fetch_local_model(dir.clone(), &m).unwrap();
        assert_eq!(local_model_progress(), 100);
        let mouth = Mouth::default();
        // every voice it offers speaks, and changing voice does not load the model again
        let mut took = Vec::new();
        let mut loaded: Option<Arc<Held>> = None;
        for (name, _) in m.voices.clone() {
            let held = mouth.held(&m, folder(&dir, &m));
            assert!(loaded.as_ref().is_none_or(|was| Arc::ptr_eq(was, &held)));
            loaded = Some(held.clone());
            mouth.set(Some(Own::Model { held, voice: name.clone() }));
            assert!(mouth.ready());
            let began = std::time::Instant::now();
            let sample = mouth.sample(&|_, _, _, _| None).unwrap();
            let seconds = sample.pcm.len() as f32 / 2.0 / sample.rate as f32;
            println!("{name}: {seconds:.1} s of sound in {:.2} s", began.elapsed().as_secs_f32());
            assert!(seconds > 2.0);
            took.push(sample.pcm.len());
        }
        // different voices make different sound
        took.dedup();
        assert!(took.len() > 1);
    }
}
