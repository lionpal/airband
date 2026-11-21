//! The catalogue on this computer: a newer signed copy is fetched now and then and kept beside the settings,
//! and `airband catalog` puts out what is built in, for publishing. See `airband_core::catalog`.
use crate::config;
use airband_core::catalog;
use std::time::Duration;

fn kept() -> (std::path::PathBuf, std::path::PathBuf) {
    (config::dir().join("catalog.json"), config::dir().join("catalog.json.sig"))
}

/// Use the newest copy this computer has kept, if it is newer than the one built in. Nothing is fetched.
pub fn load() {
    let (doc, sig) = kept();
    if let (Ok(doc), Ok(sig)) = (std::fs::read(doc), std::fs::read_to_string(sig)) {
        let _ = catalog::accept(&doc, &sig);
    }
}

/// Fetch the published copy and, if it is airband's and newer, use it and keep it. Quietly does nothing
/// when there is no network. Blocks for a few seconds at most.
pub fn refresh() -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(6))).build().into();
    let get = |url: &str| agent.get(url).call().ok().and_then(|mut r| r.body_mut().read_to_vec().ok());
    let (Some(doc), Some(sig)) = (get(catalog::ADDRESS), get(&format!("{}.sig", catalog::ADDRESS))) else { return false };
    let sig = String::from_utf8_lossy(&sig).to_string();
    match catalog::accept(&doc, &sig) {
        Ok(true) => {
            let (to, to_sig) = kept();
            let _ = std::fs::write(to, &doc);
            let _ = std::fs::write(to_sig, sig);
            true
        }
        _ => false,
    }
}

/// `airband catalog export` prints the catalogue built into this program. `airband catalog sign <key file>`
/// prints its signature, made with the private key in that file (32 bytes, in hex). `airband catalog check
/// <document> <signature>` says whether a pair would be taken. `scripts/publish-catalog.sh` uses all three.
pub fn command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("export") => {
            print!("{}", catalog::built_in());
            Ok(())
        }
        Some("sign") => {
            use ed25519_dalek::Signer;
            let path = args.get(1).ok_or("usage: airband catalog sign <key file>")?;
            let hex = std::fs::read_to_string(path).map_err(|e| format!("cannot read the key file: {e}"))?;
            let hex = hex.trim();
            let seed: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
            let seed: [u8; 32] = seed.try_into().map_err(|_| "that file does not hold a key")?;
            let signature = ed25519_dalek::SigningKey::from_bytes(&seed).sign(catalog::built_in().as_bytes());
            println!("{}", airband_core::audio::base64_encode(&signature.to_bytes()));
            Ok(())
        }
        Some("check") => {
            let (doc, sig) = (args.get(1).ok_or("usage: airband catalog check <document> <signature>")?, args.get(2).ok_or("usage: airband catalog check <document> <signature>")?);
            let doc = std::fs::read(doc).map_err(|e| e.to_string())?;
            let sig = std::fs::read_to_string(sig).map_err(|e| e.to_string())?;
            match catalog::accept(&doc, &sig)? {
                true => println!("Signed by airband, and newer than the one built in (rev {}).", catalog::current().rev),
                false => println!("Signed by airband. Not newer than the one built in (rev {}).", catalog::current().rev),
            }
            Ok(())
        }
        _ => {
            println!("The catalogue in use is rev {}: {} providers, {} models for the phone.\n  airband catalog export | sign <key file> | check <document> <signature>", catalog::current().rev, catalog::current().providers.len(), catalog::current().models.len());
            Ok(())
        }
    }
}
