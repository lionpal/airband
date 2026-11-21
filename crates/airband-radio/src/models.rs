//! Models that run on the device itself: a voice, and a listening model. No key, no computer, no relay, and
//! nothing he says or hears leaves the device.
//!
//! The program that runs a model is part of the app; the model is not. He chooses one on a settings screen and
//! only then is it fetched, once, into the app's own folder. The files come from airband's own address,
//! already in the form the device runs, each checked against its hash before it takes its name.
//!
//! Each model is a row of the catalogue (`catalog/models/`, see `airband_core::catalog`): what job it does, what to show, what to fetch and where it goes. A host
//! offers what [`local_models`] lists and names none itself.
use super::*;
use airband_core::providers::Job;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

pub(crate) use airband_core::catalog::PhoneModel as Model;

/// The programs in this build that run a model, by the name a row gives its `engine`. `ptts` is the voice
/// engine compiled in with the `phonon` feature; `host-transcribe` is the host's own (`DeskHost::transcribe`).
const ENGINES: &[&str] = &[
    #[cfg(feature = "phonon")]
    "ptts",
    "host-transcribe",
];

/// The models in the catalogue that this build can run.
pub(crate) fn models() -> impl Iterator<Item = &'static Model> {
    airband_core::catalog::current().models.iter().filter(|m| ENGINES.contains(&m.engine))
}

/// Use the newest catalogue this device has kept (in `dir`, a folder of the app's own), if it is newer than
/// the one built in. Nothing is fetched. Call once at the start.
#[uniffi::export]
pub fn load_catalog(dir: String) {
    let at = Path::new(&dir).join("catalog.json");
    if let (Ok(doc), Ok(sig)) = (std::fs::read(&at), std::fs::read_to_string(at.with_extension("json.sig"))) {
        let _ = airband_core::catalog::accept(&doc, &sig);
    }
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// what a chooser lists may have changed. Blocks for a few seconds at most; does nothing with no network.
#[uniffi::export]
pub fn refresh_catalog(dir: String) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(6))).build().into();
    let get = |url: &str| agent.get(url).call().ok().and_then(|mut r| r.body_mut().read_to_vec().ok());
    let address = airband_core::catalog::ADDRESS;
    let (Some(doc), Some(sig)) = (get(address), get(&format!("{address}.sig"))) else { return false };
    let sig = String::from_utf8_lossy(&sig).to_string();
    if airband_core::catalog::accept(&doc, &sig) != Ok(true) {
        return false;
    }
    let at = Path::new(&dir).join("catalog.json");
    let _ = std::fs::write(&at, &doc);
    let _ = std::fs::write(at.with_extension("json.sig"), sig);
    true
}

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
    models().find(|m| m.kind == kind)
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
    models().filter(|m| m.job == job).map(|m| LocalModel { kind: m.kind.into(), name: m.name.into(), megabytes: (m.files.iter().map(|f| f.1).sum::<u64>() as f64 / 1e6).round() as u32, fetched: fetched(dir, m), credit: m.credit.into() }).collect()
}

/// The voices this build can make on the device, and whether each one's model is here. `dir` is a folder of
/// the app's own, the same every time.
#[uniffi::export]
pub fn local_voices(dir: String) -> Vec<LocalModel> {
    listed(&dir, Job::Voice)
}

/// One of the voices a voice model can speak in.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VoiceChoice {
    /// What to pass back as the voice.
    pub name: String,
    /// What to call it where he chooses.
    pub label: String,
    /// A few words on how it sounds.
    pub about: String,
    /// A short clip of it, to hear before choosing: an address the host can fetch and play, whether or not
    /// the model is on the device yet.
    pub preview: String,
}

/// The voices a voice model offers, the usual one first. Empty for a model that is not a voice.
#[uniffi::export]
pub fn local_voice_choices(kind: String) -> Vec<VoiceChoice> {
    model(&kind).map(|m| m.voices.iter().map(|(name, about)| VoiceChoice { name: name.to_string(), label: name.to_string(), about: about.to_string(), preview: format!("{}/previews/{name}.m4a", m.base) }).collect()).unwrap_or_default()
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
        for m in models() {
            assert!(!m.files.is_empty() && !m.credit.is_empty() && m.base.starts_with("https://"), "{}", m.kind);
            assert!(m.files.iter().all(|(to, size, sha)| !to.starts_with('/') && *size > 0 && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())), "{}", m.kind);
            assert_eq!(models().filter(|o| o.kind == m.kind).count(), 1, "{} is listed twice", m.kind);
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
