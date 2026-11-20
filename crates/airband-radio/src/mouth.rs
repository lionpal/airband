//! The device's own voice, in place of the tower's. One of two kinds, whichever he chose:
//!
//! - made on the device itself by a model: no key, no computer, no relay, nothing sent anywhere;
//! - a voice service reached straight from the device with a key of his own, kept on the device. The services
//!   are the tower's own (`voices.rs`, by the rows of `airband_core::providers`), so the phone can use any the
//!   tower can, and his computer is not needed for it.
//!
//! Either way the radio tells the tower it speaks for itself, and the tower sends it words only.
//!
//! The model:
//!
//! The program that runs a voice model is part of the app; the model is not. He asks for it on a settings
//! screen and it is fetched once into the app's own folder. While one is chosen it stays loaded, so a sentence
//! starts at once. A radio with a voice of its own tells the tower so, and the tower sends it words only.
//!
//! Each model is a row in [`MODELS`]: what to show, what to fetch and where it goes. A host offers what
//! [`local_voices`] lists and names none itself.
use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

struct Model {
    kind: &'static str,
    name: &'static str,
    /// Each file: where it is fetched from, where it goes under the model's folder, its size, and its SHA-256.
    /// A file that does not match is not kept.
    files: &'static [(&'static str, &'static str, u64, &'static str)],
    /// The voice it speaks in, by the name its file has under `voices/`.
    #[cfg_attr(not(feature = "phonon"), allow(dead_code))]
    voice: &'static str,
}

/// Gradium's Phonon, 31 million parameters, English, in eight-bit weights. Model and voices are under Apache
/// 2.0. Pinned to a commit each, so every phone has the same files.
#[cfg(feature = "phonon")]
const MODELS: &[Model] = &[Model {
    kind: "phonon",
    name: "Phonon",
    files: &[
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/config.json", "config.json", 4_634, "b5d0f88353638370d94035b7d73d7d6577c0554d78f8d7ecfd40490aae39ae85"),
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/tokenizer.json", "tokenizer.json", 277_967, "691a00068fa7b989f2f3e5ae5daa2d1a5bb319f2d5360deafcfb19a8bb6e7936"),
        ("https://huggingface.co/gradium/phonon-voices/resolve/da6c9ec0d47be8df5c87c730a3aa97187f3229c3/voices/NbpkqMVS3CJeq2j8.safetensors", "voices/Zoey.safetensors", 256_136, "97c7f8c253413574406ab80dd23f49c3ebb2517163b00a9bf86b9c4cf43e7e23"),
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/model.q8.gguf", "model.q8.gguf", 64_512_352, "a7d9aad2cd6dc66473f24a08bc6d2939030ef4d751ef82349361fbef3272dbae"),
    ],
    voice: "Zoey",
}];
#[cfg(not(feature = "phonon"))]
const MODELS: &[Model] = &[];

/// A voice the device can make itself, for a settings screen.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LocalVoice {
    /// What to pass back as `kind`.
    pub kind: String,
    /// What to show him.
    pub name: String,
    /// How much is fetched, in megabytes.
    pub megabytes: u32,
    /// Its model is on this device.
    pub fetched: bool,
}

fn model(kind: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.kind == kind)
}
fn folder(dir: &str, m: &Model) -> PathBuf {
    Path::new(dir).join("voices").join(m.kind)
}
fn fetched(dir: &str, m: &Model) -> bool {
    // whole files only: each was checked against its hash before it took its name
    m.files.iter().all(|(_, to, size, _)| std::fs::metadata(folder(dir, m).join(to)).is_ok_and(|f| f.is_file() && f.len() == *size))
}

/// The voices this build can make on the device, and whether each one's model is here. `dir` is a folder of
/// the app's own, the same every time.
#[uniffi::export]
pub fn local_voices(dir: String) -> Vec<LocalVoice> {
    MODELS.iter().map(|m| LocalVoice { kind: m.kind.into(), name: m.name.into(), megabytes: (m.files.iter().map(|f| f.2).sum::<u64>() as f64 / 1e6).round() as u32, fetched: fetched(&dir, m) }).collect()
}

/// How far the fetch now running has got: bytes so far, and bytes in all.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());

/// How far the fetch now running has got, in hundredths. For a progress bar, asked a few times a second.
#[uniffi::export]
pub fn local_voice_progress() -> u32 {
    let of = OF.load(Ordering::SeqCst);
    if of == 0 { 0 } else { (GOT.load(Ordering::SeqCst).min(of) * 100 / of) as u32 }
}

/// Fetch a voice's model. Blocks until it is all here: call it off the main thread. Files already here are
/// kept, so one that was cut short goes on from the last whole file.
#[uniffi::export]
pub fn fetch_local_voice(dir: String, kind: String) -> Result<(), RadioError> {
    let told = |reason: &str| RadioError::Told { reason: reason.into() };
    let m = model(&kind).ok_or_else(|| told("This app has no such voice."))?;
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = folder(&dir, m);
    GOT.store(0, Ordering::SeqCst);
    OF.store(m.files.iter().map(|f| f.2).sum(), Ordering::SeqCst);
    // no limit on the whole of it (the model is tens of megabytes on whatever connection he has), only on a stall
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    for (from, to, size, sha) in m.files {
        let path = folder.join(to);
        if std::fs::metadata(&path).is_ok_and(|f| f.len() == *size) {
            GOT.fetch_add(*size, Ordering::SeqCst);
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|_| told("Could not make room for the voice on this device."))?;
        let part = path.with_extension("part");
        let mut res = agent.get(*from).call().map_err(|_| told("Could not reach the voice's download. Check the connection and try again."))?;
        if res.status().as_u16() != 200 {
            return Err(told("The voice's download is not there."));
        }
        let mut file = std::fs::File::create(&part).map_err(|_| told("Could not write the voice to this device."))?;
        let mut body = res.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut hash = <sha2::Sha256 as sha2::Digest>::new();
        loop {
            let n = std::io::Read::read(&mut body, &mut buf).map_err(|_| told("The download stopped part way. Try again."))?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buf[..n]).map_err(|_| told("This device ran out of room for the voice."))?;
            sha2::Digest::update(&mut hash, &buf[..n]);
            GOT.fetch_add(n as u64, Ordering::SeqCst);
        }
        drop(file);
        // a sign-in page, a connection that closed early, a file changed at the source: none of them is the voice
        let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
        if got != *sha {
            let _ = std::fs::remove_file(&part);
            return Err(told("What came down was not the voice. Check the connection and try again."));
        }
        std::fs::rename(&part, &path).map_err(|_| told("Could not write the voice to this device."))?;
    }
    Ok(())
}

/// Take a voice's model off this device. If it is the one in use, it is no longer.
#[uniffi::export]
pub fn remove_local_voice(dir: String, kind: String) {
    if let Some(m) = model(&kind) {
        if CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(_, k)| *k == kind) {
            set_local_voice(dir.clone(), String::new());
        }
        let _ = std::fs::remove_dir_all(folder(&dir, m));
    }
}

/// The voice in use, if any: the app's folder and the kind.
static CHOSEN: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Speak with this voice from now on; an empty `kind` goes back to the tower's. The host remembers the choice
/// and calls this at launch. The model is loaded in the background and stays loaded.
#[uniffi::export]
pub fn set_local_voice(dir: String, kind: String) {
    let chosen = model(&kind).filter(|m| fetched(&dir, m)).map(|_| (dir, kind));
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
    /// What to show him.
    pub name: String,
    /// It takes a key.
    pub key: bool,
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
    airband_core::providers::offering(job)
        .map(|p| Service {
            kind: p.id.into(),
            name: p.name.into(),
            key: p.key,
            keys_at: p.keys_at.into(),
            asks: p.offers.iter().filter(|o| o.job == job).flat_map(|o| o.asks).map(|a| ServiceAsk { field: a.field.into(), question: a.question.into(), usual: a.usual.into() }).collect(),
        })
        .collect()
}

/// The voice services this build can reach, straight from the provider table. A screen offers these and names
/// none itself.
#[uniffi::export]
pub fn voice_services() -> Vec<Service> {
    services(airband_core::providers::Job::Voice)
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

    /// The model once loaded: whose folder and kind it was loaded for, and the voice's name in it.
    static LOADED: Mutex<Option<((String, String), &'static str, Synth)>> = Mutex::new(None);

    pub(super) fn drop_loaded() {
        *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Do something with the chosen voice's model, loading it first if it is not the one loaded.
    pub(super) fn with<T>(f: impl FnOnce(&mut (&'static str, &mut Synth)) -> T) -> Result<T, String> {
        let chosen = CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("No voice is chosen on this device.")?;
        let m = model(&chosen.1).ok_or("This app has no such voice.")?;
        let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.as_ref().is_none_or(|(of, _, _)| *of != chosen) {
            let said = |e: &dyn std::fmt::Display| format!("The voice could not be loaded: {e}");
            let checkpoint = Checkpoint::resolve(folder(&chosen.0, m), ResolveOptions { quant: Quant::Q80, weights: None }).map_err(|e| said(&e))?;
            let mut synth = checkpoint.builder(Lang::En).device(DeviceKind::Cpu).build().map_err(|e| said(&e))?;
            checkpoint.register_voices(&mut synth);
            *loaded = Some((chosen, m.voice, synth));
        }
        let (_, voice, synth) = loaded.as_mut().unwrap();
        Ok(f(&mut (*voice, synth)))
    }

    pub(super) fn speak(text: &str, sink: &mut dyn FnMut(&[u8]), stop: &dyn Fn() -> bool) -> Result<(), String> {
        with(|(voice, synth)| {
            let stream = synth.stream_with(text, &SpeechOptions::default().voice(voice.to_string())).map_err(|e| e.to_string())?;
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

    #[test]
    fn a_voice_is_offered_with_its_size_and_is_not_here_until_fetched() {
        let dir = std::env::temp_dir().join(format!("airband-mouth-{}", random_hex(4)));
        let offered = local_voices(dir.display().to_string());
        assert_eq!(offered.len(), 1);
        assert_eq!((offered[0].kind.as_str(), offered[0].fetched), ("phonon", false));
        assert!((60..80).contains(&offered[0].megabytes));
        // choosing one that is not here chooses nothing
        set_local_voice(dir.display().to_string(), "phonon".into());
        assert!(!ready());
    }

    /// The real thing: fetches the model (65 MB) and speaks a sentence. Run by hand with
    /// `AIRBAND_VOICE_DIR=<folder> cargo test --release -p airband-radio fetched -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fetched_it_speaks() {
        let dir = std::env::var("AIRBAND_VOICE_DIR").expect("AIRBAND_VOICE_DIR");
        fetch_local_voice(dir.clone(), "phonon".into()).unwrap();
        assert_eq!(local_voice_progress(), 100);
        assert!(local_voices(dir.clone())[0].fetched);
        set_local_voice(dir.clone(), "phonon".into());
        assert!(ready());
        let began = std::time::Instant::now();
        let pcm = own_voice_sample().unwrap().pcm;
        let seconds = pcm.len() as f32 / 2.0 / RATE as f32;
        println!("{seconds:.1} s of sound in {:.1} s, loading included", began.elapsed().as_secs_f32());
        let again = std::time::Instant::now();
        own_voice_sample().unwrap();
        println!("a short one, loaded: {:.2} s", again.elapsed().as_secs_f32());
        assert!(seconds > 2.0);
        std::fs::write(std::path::Path::new(&dir).join("sample.pcm"), pcm).unwrap();
    }
}
