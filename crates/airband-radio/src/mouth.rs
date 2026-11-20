//! A voice made on the device itself: no key, no computer, no relay, nothing sent anywhere.
//!
//! The program that runs a voice model is part of the app; the model is not. He asks for it on a settings
//! screen and it is fetched once into the app's own folder. While one is chosen it stays loaded, so a sentence
//! starts at once. A radio with a voice of its own tells the tower so, and the tower sends it words only.
//!
//! Each model is a row in [`MODELS`]: what to show, what to fetch and where it goes. A host offers what
//! [`local_voices`] lists and names none itself.
use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

struct Model {
    kind: &'static str,
    name: &'static str,
    /// Each file: where it is fetched from, where it goes under the model's folder, and its size.
    files: &'static [(&'static str, &'static str, u64)],
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
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/config.json", "config.json", 2_000),
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/tokenizer.json", "tokenizer.json", 277_967),
        ("https://huggingface.co/gradium/phonon-voices/resolve/da6c9ec0d47be8df5c87c730a3aa97187f3229c3/voices/NbpkqMVS3CJeq2j8.safetensors", "voices/Zoey.safetensors", 256_136),
        ("https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae/model.q8.gguf", "model.q8.gguf", 64_512_352),
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
    m.files.iter().all(|(_, to, _)| folder(dir, m).join(to).is_file())
}

/// The voices this build can make on the device, and whether each one's model is here. `dir` is a folder of
/// the app's own, the same every time.
#[uniffi::export]
pub fn local_voices(dir: String) -> Vec<LocalVoice> {
    MODELS.iter().map(|m| LocalVoice { kind: m.kind.into(), name: m.name.into(), megabytes: (m.files.iter().map(|f| f.2).sum::<u64>() / 1_000_000) as u32, fetched: fetched(&dir, m) }).collect()
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
    for (from, to, size) in m.files {
        let path = folder.join(to);
        if path.is_file() {
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
        loop {
            let n = std::io::Read::read(&mut body, &mut buf).map_err(|_| told("The download stopped part way. Try again."))?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buf[..n]).map_err(|_| told("This device ran out of room for the voice."))?;
            GOT.fetch_add(n as u64, Ordering::SeqCst);
        }
        drop(file);
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
        engine::drop_loaded();
    }
}

/// Is a voice of the device's own chosen and here?
pub(crate) fn ready() -> bool {
    CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// A short clip in the chosen voice, as 16-bit little-endian mono PCM at 24000 samples a second, for a "try
/// it" button. Blocks while it is made.
#[uniffi::export]
pub fn local_voice_sample(text: String) -> Result<Vec<u8>, RadioError> {
    let mut pcm = Vec::new();
    engine::speak(&text, &mut |part| pcm.extend_from_slice(part), &|| false).map_err(|reason| RadioError::Told { reason })?;
    Ok(pcm)
}

/// The rate every model here speaks at.
const RATE: u32 = 24000;
/// Sound is handed to the host a quarter of a second at a time, so a slow moment in the making is not heard.
const HAND: usize = RATE as usize / 4 * 2;

/// Say `text` in the device's own voice, until it is over or `stop` says to leave off. False if not a sound
/// was made, so the caller can do something else.
pub(crate) fn say(host: &dyn RadioHost, text: &str, stop: &dyn Fn() -> bool) -> bool {
    let (mut held, mut started) = (Vec::new(), false);
    let made = engine::speak(
        text,
        &mut |pcm| {
            held.extend_from_slice(pcm);
            if held.len() >= HAND {
                host.play(RATE, std::mem::take(&mut held), false);
                started = true;
            }
        },
        stop,
    );
    if stop() {
        return true;
    }
    if made.is_ok() && !held.is_empty() {
        started = true;
    }
    if started {
        // the rest, and wait until it has all been heard
        host.play(RATE, held, true);
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
        let pcm = local_voice_sample("Shop two is waiting. The build passed and it wants to know whether to push.".into()).unwrap();
        let seconds = pcm.len() as f32 / 2.0 / RATE as f32;
        println!("{seconds:.1} s of sound in {:.1} s, loading included", began.elapsed().as_secs_f32());
        let again = std::time::Instant::now();
        local_voice_sample("Shop two is waiting.".into()).unwrap();
        println!("a short one, loaded: {:.2} s", again.elapsed().as_secs_f32());
        assert!(seconds > 2.0);
        std::fs::write(std::path::Path::new(&dir).join("sample.pcm"), pcm).unwrap();
    }
}
