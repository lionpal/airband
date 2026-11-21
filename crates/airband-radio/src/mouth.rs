//! The device's own voice, in place of the tower's. One of two kinds, whichever he chose:
//!
//! - made on the device itself by a model: no key, no computer, no relay, nothing sent anywhere;
//! - a voice service reached straight from the device with a key of his own, kept on the device. The services
//!   are the tower's own (`voices.rs`, by the rows of `airband_core::providers`), so the phone can use any the
//!   tower can, and his computer is not needed for it.
//!
//! Either way the radio tells the tower it speaks for itself, and the tower sends it words only.
//!
//! A model is fetched and kept by `models.rs`; this runs it. While one is chosen it stays loaded, so a sentence
//! starts at once.
use super::*;
use crate::models::model;
use std::collections::HashMap;

/// The voice of the device's own that he chose, when he chose one. One thing at a time: which one is decided
/// where he chooses (`choices.rs`) and only carried out here.
#[derive(Clone, PartialEq)]
pub(crate) enum Own {
    /// A model on the device: the app's folder, the model's kind, and which of its voices speaks.
    Model { dir: String, kind: String, voice: String },
    /// A voice service reached straight from the device with a key of his own.
    Service(airband_core::providers::Use),
    /// What he chose cannot speak as things are, and why. Nothing else speaks in its place.
    Cannot(String),
}
static USING: Mutex<Option<Own>> = Mutex::new(None);
/// Why what he chose did not speak the last time it was tried. Cleared when it next does.
static FAILED: Mutex<Option<String>> = Mutex::new(None);

fn using() -> Option<Own> {
    USING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Speak with this from now on; nothing is the tower's voice. A model is loaded in the background and stays
/// loaded, so a sentence starts at once; changing only its voice loads nothing.
pub(crate) fn set(own: Option<Own>) {
    let model = matches!(own, Some(Own::Model { .. }));
    *USING.lock().unwrap_or_else(|e| e.into_inner()) = own;
    *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if model {
        thread::spawn(|| {
            let _ = engine::with(|_| ());
        });
    } else {
        // off this thread: the model may be in the middle of a sentence, and this is called from a screen
        thread::spawn(engine::drop_loaded);
    }
}

/// Why what he chose did not speak the last time, if it did not.
pub(crate) fn failed() -> Option<String> {
    FAILED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// What a radio adds when it asks the tower for the next thing to say, or says it is ready for one: whether it
/// makes the speech itself.
pub(crate) fn asking() -> &'static str {
    if ready() { "?voice=own" } else { "" }
}

/// Has he chosen a voice of the device's own? Then the device says everything, and when that voice cannot,
/// nothing is said in its place.
pub(crate) fn ready() -> bool {
    using().is_some()
}

/// A service for one job, as a provider's row offers it: what where he chooses (`choices.rs`) lists.
#[derive(Debug, Clone)]
pub(crate) struct Service {
    /// What to pass back as `kind`. It also names the key: one key serves everything its provider does.
    pub kind: String,
    /// What to show him: the maker and the model.
    pub name: String,
    /// Not one of the few shown first: offered behind "More services".
    pub more: bool,
    /// It costs him nothing to use: say so.
    pub free: bool,
    /// The coding tool whose sign-in on his computer it uses, by that tool's id; empty when it uses none.
    /// Offer it only to someone who uses that tool.
    pub login: String,
    /// What it costs him, in a few words. Empty when free or not known.
    pub price: String,
    /// For a voice service: a few voices to choose from, the usual one first. The `name` of the one chosen
    /// is the setting `voice`; `label` is what to call it.
    pub voices: Vec<crate::models::VoiceChoice>,
    /// One line under that, for one that takes no key: where it runs and what it costs him. Else empty.
    pub about: String,
    /// It takes a key.
    pub key: bool,
    /// What its key is called, for asking for it ("Gemini API key").
    pub key_name: String,
    /// Where its keys are made.
    pub keys_at: String,
    /// What else it can be told. Anything left out is the usual.
    pub asks: Vec<ServiceAsk>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ServiceAsk {
    pub field: String,
    pub question: String,
    pub usual: String,
}

/// The services for a job that this build has the code for, from the provider table, in the order to show.
pub(crate) fn services(job: airband_core::providers::Job) -> Vec<Service> {
    use airband_core::providers::Job;
    airband_core::providers::in_order(job)
        .into_iter()
        // what this build has the code for. Reading is his computer's job: every one is listed, and the
        // computer says no to one it cannot use.
        .filter(|(_, o)| match job {
            Job::Voice => crate::voices::speaks(o),
            Job::Ears => crate::hears::listens(o),
            Job::Summary => true,
        })
        .map(|(p, o)| Service {
            kind: p.id.clone(),
            more: !o.preferred,
            free: o.local(),
            login: o.login.clone(),
            price: o.price.clone(),
            voices: o.voices.iter().map(|v| crate::models::VoiceChoice { name: v.id.clone(), label: v.name.clone(), about: v.about.clone(), preview: v.sample.clone() }).collect(),
            name: o.label.clone(),
            about: o.about.clone(),
            key: p.key,
            key_name: p.key_name.clone(),
            keys_at: p.keys_at.clone(),
            asks: o.asks.iter().map(|a| ServiceAsk { field: a.field.clone(), question: a.question.clone(), usual: a.usual.clone() }).collect(),
        })
        .collect()
}

/// A coding tool airband works with, for where he picks the ones he uses.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CodingTool {
    /// What its agents are known by (`DeskMember::harness`), and what a service's `login` names.
    pub id: String,
    pub name: String,
    pub maker: String,
    /// What he has to do once before its agents are heard; empty when there is nothing.
    pub first: String,
}

/// The coding tools airband works with, in the order to show them.
#[uniffi::export]
pub fn coding_tools() -> Vec<CodingTool> {
    airband_core::catalog::current().tools.iter().map(|t| CodingTool { id: t.id.into(), name: t.name.into(), maker: t.maker.into(), first: t.first.into() }).collect()
}

pub(crate) fn service_of(kind: &str, key: &str, settings: HashMap<String, String>) -> airband_core::providers::Use {
    airband_core::providers::Use { kind: kind.into(), key: key.trim().into(), set: settings.into_iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k, serde_json::Value::String(v.trim().to_string()))).collect() }
}

/// Does the service speak with this key and these settings? One short real call. Blocks.
pub(crate) fn check(with: &airband_core::providers::Use) -> Result<(), String> {
    let synth = crate::voices::synth(with)?;
    let mut bytes = 0usize;
    synth.speak("Ready.", &mut |pcm| bytes += pcm.len(), &mut |_, _| {}).map_err(|e| format!("The voice service did not speak: {e}."))?;
    if bytes == 0 { Err("The voice service answered with no sound.".into()) } else { Ok(()) }
}

/// A clip of sound: 16-bit little-endian mono PCM at `rate` samples a second.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VoiceSample {
    pub rate: u32,
    pub pcm: Vec<u8>,
}

/// A sentence in the device's own voice as it is now set, for a "hear it" button. Blocks while it is made.
#[uniffi::export]
pub fn own_voice_sample() -> Result<VoiceSample, RadioError> {
    const SAMPLE: &str = "Shop two is waiting. The build passed, and it wants to know whether to push.";
    let mut pcm = Vec::new();
    let rate = speak(SAMPLE, &mut |part| pcm.extend_from_slice(part), &mut |_, _| {}, &|| false).map_err(|reason| RadioError::Told { reason })?;
    if pcm.is_empty() {
        return Err(RadioError::Told { reason: "The voice made no sound.".into() });
    }
    Ok(VoiceSample { rate, pcm })
}

/// Make `text` into sound with the voice of the device's own that he chose. Returns the rate of what was
/// handed to `sink`. Err says why it could not, in words for him.
fn speak(text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32), stop: &dyn Fn() -> bool) -> Result<u32, String> {
    match using() {
        // a service is read to its end; what comes after he says stop is dropped
        Some(Own::Service(with)) => {
            let synth = crate::voices::synth(&with)?;
            synth.speak(text, &mut |pcm| if !stop() { sink(pcm) }, mark).map(|_| synth.rate())
        }
        Some(Own::Model { .. }) => engine::speak(text, sink, stop),
        Some(Own::Cannot(why)) => Err(why),
        None => Err("No voice is chosen on this device.".into()),
    }
}

/// The rate what he chose speaks at: its service's, or what its model's row says.
fn rate() -> Option<u32> {
    match using()? {
        Own::Service(with) => crate::voices::synth_for(&with).map(|s| s.rate()),
        Own::Model { kind, .. } => model(&kind).and_then(|m| m.told("rate").parse().ok()),
        Own::Cannot(_) => None,
    }
}

/// Say `text` in the device's own voice, until it is over or `stop` says to leave off. Err, with why in words
/// for him, if not a sound was made: the voice stays the one he chose, and that is kept for him to see. Sound is handed to the host about a quarter of a second at a
/// time, so a slow moment in the making is not heard; a service that says when each word is spoken has that
/// passed on, for showing the words as they are said.
pub(crate) fn say(host: &dyn RadioHost, text: &str, stop: &dyn Fn() -> bool) -> Result<(), String> {
    use std::cell::RefCell;
    // what is waiting to be handed over, whether anything has been, and word timings not yet passed on
    let state = RefCell::new((Vec::<u8>::new(), false, Vec::<(u32, u32)>::new(), false));
    let fail = |why: String| {
        *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.clone());
        Err(why)
    };
    let Some(rate) = rate() else {
        return fail(match using() {
            Some(Own::Cannot(why)) => why,
            _ => "The voice you chose does not say what rate it speaks at.".into(),
        });
    };
    let hand = rate as usize / 4 * 2;
    let made = speak(
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
        *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return Ok(());
    }
    fail(match made {
        Err(why) => format!("The voice you chose did not speak: {why}"),
        Ok(_) => "The voice you chose made no sound.".into(),
    })
}

#[cfg(feature = "phonon")]
mod engine {
    use super::*;
    use ptts::checkpoint::{Checkpoint, ResolveOptions};
    use ptts::preprocess::Lang;
    use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

    /// What a row calls this engine.
    const NAME: &str = "ptts";

    /// The model once loaded, with every voice it was fetched with: whose folder and kind it was loaded for.
    static LOADED: Mutex<Option<((String, String), Synth)>> = Mutex::new(None);

    pub(super) fn drop_loaded() {
        *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// What a row calls each form of the model's numbers, and each language.
    fn quant(name: &str) -> Option<Quant> {
        [("q80", Quant::Q80)].into_iter().find(|(n, _)| *n == name).map(|(_, q)| q)
    }
    fn lang(name: &str) -> Option<Lang> {
        [("en", Lang::En)].into_iter().find(|(n, _)| *n == name).map(|(_, l)| l)
    }

    /// Do something with the chosen voice's model, loading it first if it is not the one loaded.
    pub(super) fn with<T>(f: impl FnOnce(&mut (String, &mut Synth)) -> T) -> Result<T, String> {
        let Some(Own::Model { dir, kind, voice }) = using() else { return Err("No voice model is chosen on this device.".into()) };
        let m = model(&kind).filter(|m| m.engine == NAME).ok_or("This app cannot run that voice.")?;
        let chosen = (dir, kind);
        let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.as_ref().is_none_or(|(of, _)| *of != chosen) {
            let said = |e: &dyn std::fmt::Display| format!("The voice could not be loaded: {e}");
            // how to load it is the row's to say: a model in another language or form is a row
            let quant = quant(&m.told("quant")).ok_or_else(|| format!("{} does not say what form its files are in.", m.name))?;
            let lang = lang(&m.told("lang")).ok_or_else(|| format!("{} does not say what language it speaks.", m.name))?;
            let checkpoint = Checkpoint::resolve(crate::models::folder(&chosen.0, m), ResolveOptions { quant, weights: None }).map_err(|e| said(&e))?;
            let mut synth = checkpoint.builder(lang).device(DeviceKind::Cpu).build().map_err(|e| said(&e))?;
            checkpoint.register_voices(&mut synth);
            *loaded = Some((chosen, synth));
        }
        let (_, synth) = loaded.as_mut().unwrap();
        Ok(f(&mut (voice, synth)))
    }

    pub(super) fn speak(text: &str, sink: &mut dyn FnMut(&[u8]), stop: &dyn Fn() -> bool) -> Result<u32, String> {
        let rate = super::rate().ok_or("The voice does not say what rate it speaks at.")?;
        with(|(voice, synth)| {
            let stream = synth.stream_with(text, &SpeechOptions::default().voice(voice.clone())).map_err(|e| e.to_string())?;
            for chunk in stream {
                if stop() {
                    break;
                }
                let chunk = chunk.map_err(|e| e.to_string())?;
                let pcm: Vec<u8> = chunk.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
                sink(&pcm);
            }
            Ok(rate)
        })?
    }
}

#[cfg(not(feature = "phonon"))]
mod engine {
    pub(super) fn drop_loaded() {}
    pub(super) fn with<T>(_: impl FnOnce(&mut ()) -> T) -> Result<T, String> {
        Err("This app has no voice of its own.".into())
    }
    pub(super) fn speak(_: &str, _: &mut dyn FnMut(&[u8]), _: &dyn Fn() -> bool) -> Result<u32, String> {
        Err("This app has no voice of its own.".into())
    }
}

#[cfg(all(test, feature = "phonon"))]
mod tests {
    use super::*;
    use crate::models::{fetch_local_model, local_model_progress};

    /// The real thing: fetches the model (65 MB) and speaks a sentence. Run by hand with
    /// `AIRBAND_VOICE_DIR=<folder> cargo test --release -p airband-radio fetched -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fetched_it_speaks() {
        let dir = std::env::var("AIRBAND_VOICE_DIR").expect("AIRBAND_VOICE_DIR");
        fetch_local_model(dir.clone(), "phonon".into()).unwrap();
        assert_eq!(local_model_progress(), 100);
        // every voice it offers speaks, and changing voice does not load the model again
        let mut took = Vec::new();
        for (name, _) in model("phonon").unwrap().voices.clone() {
            set(Some(Own::Model { dir: dir.clone(), kind: "phonon".into(), voice: name.clone() }));
            assert!(ready());
            let began = std::time::Instant::now();
            let pcm = own_voice_sample().unwrap().pcm;
            let seconds = pcm.len() as f32 / 2.0 / rate().unwrap() as f32;
            println!("{name}: {seconds:.1} s of sound in {:.2} s", began.elapsed().as_secs_f32());
            assert!(seconds > 2.0);
            took.push(pcm.len());
        }
        // different voices make different sound
        took.dedup();
        assert!(took.len() > 1);
    }
}
