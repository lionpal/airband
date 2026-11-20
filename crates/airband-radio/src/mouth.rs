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
use crate::models::{fetched, model};
use std::collections::HashMap;

/// The voice model in use, if any: the app's folder and the kind.
static CHOSEN: Mutex<Option<(String, String)>> = Mutex::new(None);
/// Which of that model's voices speaks.
static VOICE: Mutex<String> = Mutex::new(String::new());

/// Speak with this voice model from now on, in the voice named (one of `local_voice_choices`; anything else
/// is its usual one); an empty `kind` goes back to the tower's. The host remembers the choice and calls this
/// at launch. The model is loaded in the background and stays loaded; changing only the voice loads nothing.
#[uniffi::export]
pub fn set_local_voice(dir: String, kind: String, voice: String) {
    let found = model(&kind).filter(|m| m.job == airband_core::providers::Job::Voice && fetched(&dir, m));
    if let Some(m) = found {
        let named = m.voices.iter().map(|(name, _)| *name).find(|name| *name == voice).or(m.voices.first().map(|(name, _)| *name)).unwrap_or_default();
        *VOICE.lock().unwrap_or_else(|e| e.into_inner()) = named.to_string();
    }
    let chosen = found.map(|_| (dir, kind));
    let on = chosen.is_some();
    *CHOSEN.lock().unwrap_or_else(|e| e.into_inner()) = chosen;
    if on {
        thread::spawn(|| {
            let _ = engine::with(|_| ());
        });
    } else {
        // off this thread: the model may be in the middle of a sentence, and this is called from a screen
        thread::spawn(engine::drop_loaded);
    }
}

/// The model of this kind is being taken off the device: if it is the voice in use, it is no longer.
pub(crate) fn stop_using(kind: &str) {
    let mut chosen = CHOSEN.lock().unwrap_or_else(|e| e.into_inner());
    if chosen.as_ref().is_some_and(|(_, k)| k == kind) {
        *chosen = None;
        thread::spawn(engine::drop_loaded);
    }
}

/// The chosen voice could not speak: it is no longer the one in use, so the tower is asked for its own again.
pub(crate) fn give_up() {
    *SERVICE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *CHOSEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    thread::spawn(engine::drop_loaded);
}

/// What a radio adds when it asks the tower for the next thing to say, or says it is ready for one: whether it
/// makes the speech itself.
pub(crate) fn asking() -> &'static str {
    if ready() { "?voice=own" } else { "" }
}

/// Is a voice of the device's own chosen and usable?
pub(crate) fn ready() -> bool {
    service().is_some() || CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// A service he can reach from the device with a key of his own, for a settings screen: a voice, or a
/// listening service.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Service {
    /// What to pass back as `kind`. It also names the key: one key serves everything its provider does.
    pub kind: String,
    /// What to show him: the maker and the model.
    pub name: String,
    /// Not one of the few shown first: offered behind "More services".
    pub more: bool,
    /// It costs him nothing to use: say so.
    pub free: bool,
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

#[derive(Debug, Clone, uniffi::Record)]
pub struct ServiceAsk {
    pub field: String,
    pub question: String,
    pub usual: String,
}

fn services(job: airband_core::providers::Job) -> Vec<Service> {
    airband_core::providers::in_order(job)
        .into_iter()
        .filter(|(p, _)| p.in_app)
        .map(|(p, o)| Service {
            kind: p.id.into(),
            more: !o.preferred,
            free: o.local(),
            price: o.price.into(),
            voices: o.voices.iter().map(|v| crate::models::VoiceChoice { name: v.id.into(), label: v.name.into(), about: v.about.into(), preview: v.sample.into() }).collect(),
            name: o.label.into(),
            about: o.about.into(),
            key: p.key,
            key_name: p.key_name.into(),
            keys_at: p.keys_at.into(),
            asks: o.asks.iter().map(|a| ServiceAsk { field: a.field.into(), question: a.question.into(), usual: a.usual.into() }).collect(),
        })
        .collect()
}

/// The voice services this build can reach, straight from the provider table. A screen offers these and names
/// none itself.
#[uniffi::export]
pub fn voice_services() -> Vec<Service> {
    services(airband_core::providers::Job::Voice)
}

/// What can read the agents' messages on his computer, for choosing it from the device.
#[uniffi::export]
pub fn reading_services() -> Vec<Service> {
    services(airband_core::providers::Job::Summary)
}

/// And the listening services.
#[uniffi::export]
pub fn listening_services() -> Vec<Service> {
    services(airband_core::providers::Job::Ears)
}

/// The service in use, if he chose one.
static SERVICE: Mutex<Option<airband_core::providers::Use>> = Mutex::new(None);

fn service() -> Option<airband_core::providers::Use> {
    SERVICE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(crate) fn service_of(kind: &str, key: &str, settings: HashMap<String, String>) -> airband_core::providers::Use {
    airband_core::providers::Use { kind: kind.into(), key: key.trim().into(), set: settings.into_iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k, serde_json::Value::String(v.trim().to_string()))).collect() }
}

/// Speak through this service from now on, with his key; an empty `kind` stops. The host keeps the key
/// somewhere safe and calls this at launch.
#[uniffi::export]
pub fn set_service_voice(kind: String, key: String, settings: HashMap<String, String>) {
    let chosen = (!kind.is_empty()).then(|| service_of(&kind, &key, settings)).filter(|u| crate::voices::synth_for(u).is_some());
    *SERVICE.lock().unwrap_or_else(|e| e.into_inner()) = chosen;
}

/// Does the service speak with this key and these settings? One short real call. Blocks: call it off the main
/// thread.
#[uniffi::export]
pub fn check_service_voice(kind: String, key: String, settings: HashMap<String, String>) -> Result<(), RadioError> {
    let told = |reason: String| RadioError::Told { reason };
    let synth = crate::voices::synth_for(&service_of(&kind, &key, settings)).ok_or_else(|| told("This app has no such voice service.".into()))?;
    let mut bytes = 0usize;
    synth.speak("Ready.", &mut |pcm| bytes += pcm.len(), &mut |_, _| {}).map_err(|e| told(format!("The voice service did not speak: {e}.")))?;
    if bytes == 0 { Err(told("The voice service answered with no sound.".into())) } else { Ok(()) }
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
    Ok(VoiceSample { rate, pcm })
}

/// Make `text` into sound with whichever voice of the device's own is set: the service if he chose one, else
/// the model. Returns the rate of what was handed to `sink`.
fn speak(text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32), stop: &dyn Fn() -> bool) -> Result<u32, String> {
    match service().and_then(|u| crate::voices::synth_for(&u)) {
        // a service is read to its end; what comes after he says stop is dropped
        Some(synth) => synth.speak(text, &mut |pcm| if !stop() { sink(pcm) }, mark).map(|_| synth.rate()),
        None => engine::speak(text, sink, stop).map(|_| RATE),
    }
}

/// The rate every model here speaks at.
const RATE: u32 = 24000;

/// Say `text` in the device's own voice, until it is over or `stop` says to leave off. False if not a sound
/// was made, so the caller can do something else. Sound is handed to the host about a quarter of a second at a
/// time, so a slow moment in the making is not heard; a service that says when each word is spoken has that
/// passed on, for showing the words as they are said.
pub(crate) fn say(host: &dyn RadioHost, text: &str, stop: &dyn Fn() -> bool) -> bool {
    use std::cell::RefCell;
    // what is waiting to be handed over, whether anything has been, and word timings not yet passed on
    let state = RefCell::new((Vec::<u8>::new(), false, Vec::<(u32, u32)>::new(), false));
    let rate = service().and_then(|u| crate::voices::synth_for(&u)).map(|s| s.rate()).unwrap_or(RATE);
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
        return true;
    }
    let (held, mut started, _, _) = state.into_inner();
    if made.is_ok() && !held.is_empty() {
        started = true;
    }
    if started {
        // the rest, and wait until it has all been heard
        host.play(rate, held, true);
    }
    started
}

#[cfg(feature = "phonon")]
mod engine {
    use super::*;
    use ptts::checkpoint::{Checkpoint, ResolveOptions};
    use ptts::preprocess::Lang;
    use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

    /// The model once loaded, with every voice it was fetched with: whose folder and kind it was loaded for.
    static LOADED: Mutex<Option<((String, String), Synth)>> = Mutex::new(None);

    pub(super) fn drop_loaded() {
        *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Do something with the chosen voice's model, loading it first if it is not the one loaded.
    pub(super) fn with<T>(f: impl FnOnce(&mut (String, &mut Synth)) -> T) -> Result<T, String> {
        let chosen = CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("No voice is chosen on this device.")?;
        let m = model(&chosen.1).ok_or("This app has no such voice.")?;
        let voice = VOICE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.as_ref().is_none_or(|(of, _)| *of != chosen) {
            let said = |e: &dyn std::fmt::Display| format!("The voice could not be loaded: {e}");
            let checkpoint = Checkpoint::resolve(crate::models::folder(&chosen.0, m), ResolveOptions { quant: Quant::Q80, weights: None }).map_err(|e| said(&e))?;
            let mut synth = checkpoint.builder(Lang::En).device(DeviceKind::Cpu).build().map_err(|e| said(&e))?;
            checkpoint.register_voices(&mut synth);
            *loaded = Some((chosen, synth));
        }
        let (_, synth) = loaded.as_mut().unwrap();
        Ok(f(&mut (voice, synth)))
    }

    pub(super) fn speak(text: &str, sink: &mut dyn FnMut(&[u8]), stop: &dyn Fn() -> bool) -> Result<(), String> {
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
            Ok(())
        })?
    }
}

#[cfg(not(feature = "phonon"))]
mod engine {
    pub(super) fn drop_loaded() {}
    pub(super) fn with<T>(_: impl FnOnce(&mut ()) -> T) -> Result<T, String> {
        Err("This app has no voice of its own.".into())
    }
    pub(super) fn speak(_: &str, _: &mut dyn FnMut(&[u8]), _: &dyn Fn() -> bool) -> Result<(), String> {
        Err("This app has no voice of its own.".into())
    }
}

#[cfg(all(test, feature = "phonon"))]
mod tests {
    use super::*;
    use crate::models::{fetch_local_model, local_model_progress, local_voices};

    #[test]
    fn choosing_a_voice_that_is_not_here_chooses_nothing() {
        let dir = std::env::temp_dir().join(format!("airband-mouth-{}", random_hex(4)));
        set_local_voice(dir.display().to_string(), "phonon".into(), "Zoey".into());
        assert!(CHOSEN.lock().unwrap().is_none());
    }

    /// The real thing: fetches the model (65 MB) and speaks a sentence. Run by hand with
    /// `AIRBAND_VOICE_DIR=<folder> cargo test --release -p airband-radio fetched -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fetched_it_speaks() {
        let dir = std::env::var("AIRBAND_VOICE_DIR").expect("AIRBAND_VOICE_DIR");
        fetch_local_model(dir.clone(), "phonon".into()).unwrap();
        assert_eq!(local_model_progress(), 100);
        assert!(local_voices(dir.clone())[0].fetched);
        // every voice it offers speaks, and changing voice does not load the model again
        let mut took = Vec::new();
        for choice in crate::models::local_voice_choices("phonon".into()) {
            set_local_voice(dir.clone(), "phonon".into(), choice.name.clone());
            assert!(ready());
            let began = std::time::Instant::now();
            let pcm = own_voice_sample().unwrap().pcm;
            let seconds = pcm.len() as f32 / 2.0 / RATE as f32;
            println!("{}: {seconds:.1} s of sound in {:.2} s", choice.name, began.elapsed().as_secs_f32());
            assert!(seconds > 2.0);
            took.push(pcm.len());
        }
        // different voices make different sound
        took.dedup();
        assert!(took.len() > 1);
    }
}
