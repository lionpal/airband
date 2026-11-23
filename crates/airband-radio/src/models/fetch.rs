//! Fetching a model onto this device, a file at a time, each checked against its hash before it takes its name.
#![expect(clippy::disallowed_methods, reason = "writes a model to this device a part at a time as it downloads: airband_io writes files whole")]
use super::*;
use std::sync::atomic::AtomicU64;

/// How far the fetch now running has got: bytes so far, and bytes in all.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());

/// How far the fetch now running has got, in hundredths. For a progress bar, asked a few times a second.
pub(crate) fn local_model_progress() -> u32 {
    let of = OF.load(Ordering::SeqCst);
    (GOT.load(Ordering::SeqCst).min(of) * 100).checked_div(of).unwrap_or(0) as u32
}

/// Fetch a model. Blocks until it is all here: call it off the main thread. Files already here are kept, so
/// one that was cut short goes on from the last whole file.
pub(crate) fn fetch_local_model(dir: String, m: &Model) -> Result<(), RadioError> {
    let told = crate::choices::told;
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = version_folder(&dir, m);
    GOT.store(0, Ordering::SeqCst);
    OF.store(m.files.iter().map(|f| f.1).sum(), Ordering::SeqCst);
    // no limit on the whole of it (a model is tens or hundreds of megabytes on whatever connection he has),
    // only on a stall
    let client = crate::net::Client::download(Duration::from_secs(15), Duration::from_secs(30));
    for (to, size, sha) in &m.files {
        let path = folder.join(to);
        if std::fs::metadata(&path).is_ok_and(|f| f.len() == *size) {
            GOT.fetch_add(*size, Ordering::SeqCst);
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|_| told("Could not make room for the model on this device."))?;
        let part = path.with_extension("part");
        let mut res = client.send(crate::net::Ask::get(format!("{}/{to}", m.base))).map_err(|_| told("Could not reach the download. Check the connection and try again."))?;
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
        let got: String = airband_core::encode::hex(&sha2::Digest::finalize(hash));
        if got != *sha {
            std::fs::remove_file(&part).ok();
            GOT.store(before, Ordering::SeqCst);
            return Err(told("What came down was not the model. Check the connection and try again."));
        }
        std::fs::rename(&part, &path).map_err(|_| told("Could not write the model to this device."))?;
    }
    // This version is whole: say so, and only now let go of the one that was here before it, which was in use
    // until this moment.
    std::fs::write(folder.join(WHOLE), "").map_err(|_| told("Could not write the model to this device."))?;
    for other in std::fs::read_dir(home(&dir, m)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| *p != folder) {
        if other.is_dir() { std::fs::remove_dir_all(&other) } else { std::fs::remove_file(&other) }.ok();
    }
    Ok(())
}

