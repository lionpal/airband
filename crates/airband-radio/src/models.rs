//! Models that run on the device itself: a voice, and a listening model. No key, no computer, no relay, and
//! nothing he says or hears leaves the device.
//!
//! The program that runs a model is part of the app; the model is not. He chooses one on a settings screen and
//! only then is it fetched, once, into the app's own folder. The files come from airband's own address,
//! already in the form the device runs, each checked against its hash before it takes its name.
//!
//! Each model is a row of [`MODELS`]: what job it does, what to show, what to fetch and where it goes. A host
//! offers what [`local_models`] lists and names none itself.
use super::*;
use airband_core::providers::Job;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

pub(crate) struct Model {
    pub kind: &'static str,
    pub name: &'static str,
    pub job: Job,
    /// Where its files are kept: each is fetched from here by its path.
    base: &'static str,
    /// Each file: its path under the model's folder, its size, and its SHA-256. A file that does not match is
    /// not kept.
    files: &'static [(&'static str, u64, &'static str)],
    /// For a voice: the voice it speaks in, by the name its file has under `voices/`.
    #[cfg_attr(not(feature = "phonon"), allow(dead_code))]
    pub voice: &'static str,
    /// Who made it and under what licence, for the screen that offers it. Its licence asks to be credited.
    pub credit: &'static str,
}

pub(crate) const MODELS: &[Model] = &[
    // Gradium's Phonon, 31 million parameters, English, in eight-bit weights, with the voice Zoey.
    #[cfg(feature = "phonon")]
    Model {
        kind: "phonon",
        name: "Phonon",
        job: Job::Voice,
        base: "https://airband.dev/models/phonon-31m-en/1",
        files: &[
            ("config.json", 4_634, "b5d0f88353638370d94035b7d73d7d6577c0554d78f8d7ecfd40490aae39ae85"),
            ("tokenizer.json", 277_967, "691a00068fa7b989f2f3e5ae5daa2d1a5bb319f2d5360deafcfb19a8bb6e7936"),
            ("voices/Zoey.safetensors", 256_136, "97c7f8c253413574406ab80dd23f49c3ebb2517163b00a9bf86b9c4cf43e7e23"),
            ("model.q8.gguf", 64_512_352, "a7d9aad2cd6dc66473f24a08bc6d2939030ef4d751ef82349361fbef3272dbae"),
        ],
        voice: "Zoey",
        credit: "Phonon by Gradium, Apache 2.0.",
    },
    // Fermion Research's Phonon-2.5 English, compiled for the Neural Engine. The host runs it (`transcribe`).
    Model {
        kind: "phonon-2.5-en",
        name: "Phonon 2.5",
        job: Job::Ears,
        base: "https://airband.dev/models/phonon-2.5-en/1",
        files: &[
            ("manifest.json", 597, "4682560886674dfa082ee26133e2797ac404ac519dd76704f81d21529ee4e6ad"),
            ("NOTICE.txt", 1_901, "33c1c49cbad0fc9dd8da7a9b4f7a17f0e86a41c793ae792942e79b48da2255ea"),
            ("decoder.bin", 13_301_279, "0bc5d72c61d48f3ca731fae0824bd415278c5fb7c0fba6dc41941f9f2951d4ee"),
            ("Phonon-2.5-English.mlmodelc/analytics/coremldata.bin", 243, "c7916daab8e2af5e111473da8f9f86d222e8dbeed48e19c21e13e33097b6cefb"),
            ("Phonon-2.5-English.mlmodelc/coremldata.bin", 750, "9e8514ea3128326850181e1b8056d965ab521bbc89dae3ab115e5462752b14d9"),
            ("Phonon-2.5-English.mlmodelc/metadata.json", 15_619, "26729c24178409c1b6b2d566695fe4bb68d35e36ed5b363e57c23440073cce7d"),
            ("Phonon-2.5-English.mlmodelc/model.mil", 2_713_692, "fc6b8c27b168f326f1c006bce0e84a7da11787955edd2b09a4af6304d473ff93"),
            ("Phonon-2.5-English.mlmodelc/weights/weight.bin", 329_432_768, "597947905f317ee9bfed5efb84b60880217584046d5cb193a910c9959fe6e1bb"),
        ],
        voice: "",
        credit: "Phonon-2.5 English by Fermion Research, from NVIDIA's Parakeet TDT 0.6B v3. CC BY 4.0.",
    },
];

/// A model the device can run itself, for a settings screen.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LocalModel {
    /// What to pass back as `kind`.
    pub kind: String,
    /// What to show him.
    pub name: String,
    /// How much is fetched, in megabytes.
    pub megabytes: u32,
    /// It is on this device, whole.
    pub fetched: bool,
    /// Who made it and under what licence: shown where it is offered.
    pub credit: String,
}

pub(crate) fn model(kind: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.kind == kind)
}
/// Where a model's files are, under the app's folder.
pub(crate) fn folder(dir: &str, m: &Model) -> PathBuf {
    Path::new(dir).join("models").join(m.kind)
}
pub(crate) fn fetched(dir: &str, m: &Model) -> bool {
    // whole files only: each was checked against its hash before it took its name
    m.files.iter().all(|(to, size, _)| std::fs::metadata(folder(dir, m).join(to)).is_ok_and(|f| f.is_file() && f.len() == *size))
}

fn listed(dir: &str, job: Job) -> Vec<LocalModel> {
    MODELS.iter().filter(|m| m.job == job).map(|m| LocalModel { kind: m.kind.into(), name: m.name.into(), megabytes: (m.files.iter().map(|f| f.1).sum::<u64>() as f64 / 1e6).round() as u32, fetched: fetched(dir, m), credit: m.credit.into() }).collect()
}

/// The voices this build can make on the device, and whether each one's model is here. `dir` is a folder of
/// the app's own, the same every time.
#[uniffi::export]
pub fn local_voices(dir: String) -> Vec<LocalModel> {
    listed(&dir, Job::Voice)
}

/// And the listening models it can run.
#[uniffi::export]
pub fn local_listeners(dir: String) -> Vec<LocalModel> {
    listed(&dir, Job::Ears)
}

/// How far the fetch now running has got: bytes so far, and bytes in all.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());

/// How far the fetch now running has got, in hundredths. For a progress bar, asked a few times a second.
#[uniffi::export]
pub fn local_model_progress() -> u32 {
    let of = OF.load(Ordering::SeqCst);
    if of == 0 { 0 } else { (GOT.load(Ordering::SeqCst).min(of) * 100 / of) as u32 }
}

/// Fetch a model. Blocks until it is all here: call it off the main thread. Files already here are kept, so
/// one that was cut short goes on from the last whole file.
#[uniffi::export]
pub fn fetch_local_model(dir: String, kind: String) -> Result<(), RadioError> {
    let told = |reason: &str| RadioError::Told { reason: reason.into() };
    let m = model(&kind).ok_or_else(|| told("This app has no such model."))?;
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = folder(&dir, m);
    GOT.store(0, Ordering::SeqCst);
    OF.store(m.files.iter().map(|f| f.1).sum(), Ordering::SeqCst);
    // no limit on the whole of it (a model is tens or hundreds of megabytes on whatever connection he has),
    // only on a stall
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    for (to, size, sha) in m.files {
        let path = folder.join(to);
        if std::fs::metadata(&path).is_ok_and(|f| f.len() == *size) {
            GOT.fetch_add(*size, Ordering::SeqCst);
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|_| told("Could not make room for the model on this device."))?;
        let part = path.with_extension("part");
        let mut res = agent.get(format!("{}/{to}", m.base)).call().map_err(|_| told("Could not reach the download. Check the connection and try again."))?;
        if res.status().as_u16() != 200 {
            return Err(told("The download is not there."));
        }
        let mut file = std::fs::File::create(&part).map_err(|_| told("Could not write the model to this device."))?;
        let mut body = res.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut hash = <sha2::Sha256 as sha2::Digest>::new();
        let before = GOT.load(Ordering::SeqCst);
        loop {
            let n = match std::io::Read::read(&mut body, &mut buf) {
                Ok(n) => n,
                Err(_) => {
                    GOT.store(before, Ordering::SeqCst);
                    return Err(told("The download stopped part way. Try again."));
                }
            };
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut file, &buf[..n]).map_err(|_| told("This device ran out of room for the model."))?;
            sha2::Digest::update(&mut hash, &buf[..n]);
            GOT.fetch_add(n as u64, Ordering::SeqCst);
        }
        drop(file);
        // a sign-in page, a connection that closed early, a file changed at the source: none of them is the model
        let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
        if got != *sha {
            let _ = std::fs::remove_file(&part);
            GOT.store(before, Ordering::SeqCst);
            return Err(told("What came down was not the model. Check the connection and try again."));
        }
        std::fs::rename(&part, &path).map_err(|_| told("Could not write the model to this device."))?;
    }
    Ok(())
}

/// Take a model off this device. If it is the one in use, it is no longer.
#[uniffi::export]
pub fn remove_local_model(dir: String, kind: String) {
    if let Some(m) = model(&kind) {
        match m.job {
            Job::Voice => crate::mouth::stop_using(&kind),
            Job::Ears => crate::ears::stop_using(&kind),
            Job::Summary => {}
        }
        let _ = std::fs::remove_dir_all(folder(&dir, m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_says_what_to_fetch_and_who_made_it() {
        for m in MODELS {
            assert!(!m.files.is_empty() && !m.credit.is_empty() && m.base.starts_with("https://"), "{}", m.kind);
            assert!(m.files.iter().all(|(to, size, sha)| !to.starts_with('/') && *size > 0 && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())), "{}", m.kind);
            assert_eq!(MODELS.iter().filter(|o| o.kind == m.kind).count(), 1, "{} is listed twice", m.kind);
        }
        // nothing is here until it is fetched
        let dir = std::env::temp_dir().join(format!("airband-models-{}", random_hex(4))).display().to_string();
        assert!(local_listeners(dir.clone()).iter().chain(local_voices(dir).iter()).all(|m| !m.fetched && m.megabytes > 0));
    }

    /// The real download, from where the app gets it, every file checked against its hash. Run by hand:
    /// `AIRBAND_MODEL_DIR=<folder> AIRBAND_MODEL=phonon-2.5-en cargo test --release -p airband-radio fetches -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fetches_a_model_whole() {
        let (dir, kind) = (std::env::var("AIRBAND_MODEL_DIR").unwrap(), std::env::var("AIRBAND_MODEL").unwrap());
        let began = std::time::Instant::now();
        fetch_local_model(dir.clone(), kind.clone()).unwrap();
        let m = model(&kind).unwrap();
        assert!(fetched(&dir, m) && local_model_progress() == 100);
        println!("{kind}: fetched to {} in {:.0} s", folder(&dir, m).display(), began.elapsed().as_secs_f32());
    }
}
