//! The phone's own listening model, heard live on this Mac's GPU or Neural Engine: the app's
//! NeuralEngineListening (built for the Mac by `tools/phonon-live`, loaded here in place of the app's
//! `run_model`) under the core's own listening ([`Ears::hear_by`], with its words shown as he talks), fed a
//! recording of speech as fast as it was spoken. The recordings are heard one after another by one model, got
//! ready once, as on a phone: Core ML on a Mac keeps what it compiled for a program only while it runs. Run it with `tools/phonon-live.sh`, which builds the library
//! and makes the recording; by hand:
//!   PHONON_LIVE_LIB=<libPhononHost.dylib> PHONON_LIVE_UNITS=gpu|ane AIRBAND_MODEL=phonon-2.5-en \
//!   AIRBAND_MODEL_DIR=<the model's folder> SPEECH_WAV=<16 kHz 16-bit mono>[:<another>…] \
//!   cargo test --release -p airband-radio phonon_live -- --ignored --nocapture
use super::*;
use crate::takes::tests::Folder;
use std::collections::VecDeque;
use std::ffi::{CString, c_char, c_int, c_void};
use std::time::Instant;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;

type Open = unsafe extern "C" fn(i32, *const c_char) -> i32;
type Hear = unsafe extern "C" fn(*const c_char, *const u8, usize, *mut usize) -> *mut u8;
type Free = unsafe extern "C" fn(*mut u8);

/// The app's listening, in the library `tools/phonon-live` builds.
struct Phonon {
    hear: Hear,
    free: Free,
}

impl Phonon {
    /// The library at `lib`, set up for the model in `folder` on `units` (1 the GPU, 2 the Neural Engine).
    fn load(lib: &str, units: i32, folder: &str) -> Phonon {
        let path = CString::new(lib).unwrap();
        // SAFETY: the library is ours (tools/phonon-live) and the symbols have the types its @_cdecl give
        unsafe {
            let handle = dlopen(path.as_ptr(), RTLD_NOW);
            assert!(!handle.is_null(), "{lib} could not be loaded");
            let symbol = |name: &str| {
                let at = dlsym(handle, CString::new(name).unwrap().as_ptr());
                assert!(!at.is_null(), "{lib} has no {name}");
                at
            };
            let open = std::mem::transmute::<*mut c_void, Open>(symbol("phonon_live_open"));
            let phonon = Phonon { hear: std::mem::transmute::<*mut c_void, Hear>(symbol("phonon_live_hear")), free: std::mem::transmute::<*mut c_void, Free>(symbol("phonon_live_free")) };
            let folder = CString::new(folder).unwrap();
            assert_eq!(open(units, folder.as_ptr()), 0, "the listening could not be set up");
            phonon
        }
    }

    /// What the app's `run_model` gives the core for a recording.
    fn run(&self, folder: &str, pcm: &[u8]) -> Option<Vec<u8>> {
        let folder = CString::new(folder).unwrap();
        let mut len = 0usize;
        // SAFETY: as above; what it hands back is `len` bytes it allocated, freed by its own function
        unsafe {
            let words = (self.hear)(folder.as_ptr(), pcm.as_ptr(), pcm.len(), &mut len);
            if words.is_null() {
                return None;
            }
            let out = std::slice::from_raw_parts(words, len).to_vec();
            (self.free)(words);
            Some(out)
        }
    }
}

/// One run of the model: when it started and ended (milliseconds from the first sound), on how much sound, and
/// whether it gave words back.
struct Reading {
    began: u128,
    ended: u128,
    sound_ms: usize,
    ran: bool,
}

/// A microphone that gives the recording a tenth of a second at a time, each piece when it has been spoken
/// (from when it was opened), and stops at its end: he tapped send as he finished.
struct Spoken<'a> {
    phonon: &'a Phonon,
    left: Mutex<VecDeque<Vec<u8>>>,
    read: Mutex<u32>,
    opened: Mutex<Option<Instant>>,
    shown: Mutex<Vec<(u128, String)>>,
    readings: Mutex<Vec<Reading>>,
}

impl Spoken<'_> {
    fn since(&self) -> u128 {
        self.opened.lock().unwrap().expect("the microphone was opened").elapsed().as_millis()
    }
}

impl RadioHost for Spoken<'_> {
    fn record_start(&self, _: u32) -> bool {
        *self.opened.lock().unwrap() = Some(Instant::now());
        true
    }
    fn record_read(&self) -> Vec<u8> {
        let Some(piece) = self.left.lock().unwrap().pop_front() else { return Vec::new() };
        let mut read = self.read.lock().unwrap();
        *read += 1;
        // a microphone hands over a piece once it has been said
        let due = self.opened.lock().unwrap().unwrap() + Duration::from_millis(100) * *read;
        thread::sleep(due.saturating_duration_since(Instant::now()));
        piece
    }
    fn record_stop(&self) {}
    fn told(&self, text: String) {
        println!("told: {text}");
    }
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn status(&self, _: RadioStatus) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
    fn run_model(&self, _: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        assert_eq!(job, ChoiceJob::Ears);
        let began = self.since();
        let words = self.phonon.run(&folder, &input);
        self.readings.lock().unwrap().push(Reading { began, ended: self.since(), sound_ms: input.len() / 32, ran: words.is_some() });
        words
    }
    fn hearing(&self, text: String) {
        let at = self.since();
        println!("{at:>6} ms  {text}");
        self.shown.lock().unwrap().push((at, text));
    }
}

/// The sound of a WAV file of 16 kHz 16-bit mono: what its `data` chunk holds.
fn wav_pcm(wav: &[u8]) -> Vec<u8> {
    assert!(wav.len() >= 12 && &wav[0..4] == b"RIFF" && &wav[8..12] == b"WAVE", "not a WAV file");
    let mut at = 12;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &wav[at + 8..(at + 8 + size).min(wav.len())];
        if id == b"fmt " {
            let (format, channels, rate, bits) = (u16::from_le_bytes([body[0], body[1]]), u16::from_le_bytes([body[2], body[3]]), u32::from_le_bytes(body[4..8].try_into().unwrap()), u16::from_le_bytes([body[14], body[15]]));
            assert_eq!((format, channels, rate, bits), (1, 1, RATE, 16), "the recording is to be 16 kHz 16-bit mono PCM");
        }
        if id == b"data" {
            return body.to_vec();
        }
        at += 8 + size + (size & 1);
    }
    panic!("the WAV file has no sound in it");
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is to be set: see the top of this file"))
}

#[test]
#[ignore]
fn phonon_live() {
    let units = match env("PHONON_LIVE_UNITS").as_str() {
        "gpu" => 1,
        "ane" => 2,
        other => panic!("PHONON_LIVE_UNITS is gpu or ane, not {other}"),
    };
    let folder = env("AIRBAND_MODEL_DIR");
    let device = crate::engines::HostDevice { engines: vec!["phonon-coreml".into()], os: "ios".into(), os_version: 18, os_build: String::new(), team_colours: 0 };
    let m = crate::models::model(&env("AIRBAND_MODEL"), &device).expect("a listening model of the catalogue's that the phone runs");
    assert_eq!(m.job, Job::Ears);
    // Getting it ready, as the core does once after download (`choices.rs`, run_once): a second of quiet,
    // run with every encoder loaded. The first time on this Mac for these units it is compiled for them.
    let readying = Instant::now();
    let phonon = Phonon::load(&env("PHONON_LIVE_LIB"), units, &folder);
    phonon.run(&folder, &[0u8; 32000]).expect("the model was got ready");
    let ready_ms = readying.elapsed().as_millis();
    // and once more, as the quiet load at launch is
    let warm = Instant::now();
    phonon.run(&folder, &[0u8; 32000]).expect("the model runs");
    let warm_ms = warm.elapsed().as_millis();
    // a recording at the edge of one window, as his words pass 15 s, is heard like any other
    for samples in [239_680, 239_840, 240_000, 240_160] {
        let tone: Vec<u8> = (0..samples).flat_map(|i| ((((i as f32) * 0.07).sin() * 300.0) as i16).to_le_bytes()).collect();
        assert!(phonon.run(&folder, &tone).is_some(), "a recording of {samples} samples was not heard");
    }
    println!("units {}: getting it ready {ready_ms} ms, then a load {warm_ms} ms", env("PHONON_LIVE_UNITS"));
    for wav in env("SPEECH_WAV").split(':') {
        dictate(&phonon, &m, &folder, wav);
    }
}

/// One dictation of the recording at `wav`, with what was shown as it was said and what was heard.
fn dictate(phonon: &Phonon, m: &crate::models::Model, folder: &str, wav: &str) {
    let pcm = wav_pcm(&airband_io::files::read_bytes(std::path::Path::new(wav)).unwrap().expect("the recording is there"));
    let sound_ms = pcm.len() as u128 / 32;
    println!();
    println!("{wav}");
    let held = crate::engines::Held::of(m, folder.into());
    held.set_ready(true);
    let dir = Folder::new();
    let ears = Ears::new(Arc::new(Senses { takes: crate::takes::Takes::open(&dir.path()).0, ..Senses::default() }));
    let mic = Spoken { phonon, left: Mutex::new(pcm.chunks(3200).map(<[u8]>::to_vec).collect()), read: Mutex::new(0), opened: Mutex::new(None), shown: Mutex::new(Vec::new()), readings: Mutex::new(Vec::new()) };
    // dictation: he opened the microphone himself and taps send as he finishes (`call.rs`)
    let heard = ears.hear_by(&mic, super::Mic::Model(held), true, Aim::default());
    let done = mic.since();
    let Listened::Said { text, .. } = heard else { panic!("it was not heard") };

    let shown = mic.shown.into_inner().unwrap();
    let readings = mic.readings.into_inner().unwrap();
    let during: Vec<&(u128, String)> = shown.iter().filter(|(at, _)| *at <= sound_ms).collect();
    let gaps: Vec<u128> = during.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let (partials, last) = readings.split_at(readings.len() - 1);
    let took = |r: &Reading| r.ended - r.began;
    println!("recording {sound_ms} ms");
    println!("readings while he spoke: {}, each {} ms on average, the longest {} ms (on {} ms of sound)", partials.len(), partials.iter().map(took).sum::<u128>() / partials.len().max(1) as u128, partials.iter().map(took).max().unwrap_or(0), partials.iter().max_by_key(|r| took(r)).map_or(0, |r| r.sound_ms));
    println!("words shown while he spoke: {}; the first at {} ms", during.len(), during.first().map_or(0, |(at, _)| *at));
    println!("between one showing and the next: {} ms on average, the longest {} ms", gaps.iter().sum::<u128>() / gaps.len().max(1) as u128, gaps.iter().max().unwrap_or(&0));
    // what he said had been said by the time it was sent to be read: how old the words are when shown
    let behind: Vec<u128> = partials.iter().map(|r| r.ended.saturating_sub(r.sound_ms as u128)).collect();
    println!("how far the words shown are behind him: {} ms on average, at most {} ms", behind.iter().sum::<u128>() / behind.len().max(1) as u128, behind.iter().max().unwrap_or(&0));
    println!("the last reading: began {} ms after he finished, took {} ms; what he said was had {} ms after he finished", last[0].began.saturating_sub(sound_ms), took(&last[0]), done.saturating_sub(sound_ms));
    println!("FINAL: {text}");
    assert!(!text.is_empty());
    let failed: Vec<usize> = readings.iter().filter(|r| !r.ran).map(|r| r.sound_ms).collect();
    assert!(failed.is_empty(), "the model did not run on these lengths of sound (ms): {failed:?}");
    assert!(during.len() >= 2, "his words should show while he is still speaking");
}
