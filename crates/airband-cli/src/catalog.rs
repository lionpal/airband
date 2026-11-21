//! The catalogue on this computer: a newer signed copy is fetched now and then and kept beside the settings,
//! and `airband catalog` puts out what is built in, for publishing. See `airband_core::catalog`. Reading a
//! kept copy and fetching a new one are the phone core's (`airband_radio::catalog_load`, `catalog_refresh`),
//! the same code the app runs.
use crate::config;
use airband_core::catalog;
use std::sync::Mutex;

/// What loading the kept copy had to say, until there is a log to say it in.
static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Use the newest copy this computer has kept, if it is newer than the one built in. Nothing is fetched.
pub fn load() {
    *SAID.lock().unwrap() = airband_radio::catalog_load(&config::dir(), &crate::providers::speaks);
}

/// Fetch the published copy and, if it is airband's and newer, use it and keep it. Quietly does nothing
/// when there is no network. Blocks for a few seconds at most. True when it is now the one in use, with what
/// is worth a line in a log (rows this build cannot use, a copy that was refused).
pub fn refresh() -> (bool, Vec<String>) {
    let mut lines = std::mem::take(&mut *SAID.lock().unwrap());
    let (changed, more) = airband_radio::catalog_refresh(&config::dir(), &crate::providers::speaks);
    lines.extend(more);
    (changed, lines)
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// The words after the command that are not an option or its value.
fn plain(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a);
        }
    }
    out
}

const USAGE: &str = "  airband catalog export [--expires-in-days 180]\n  airband catalog sign <key file> <document>\n  airband catalog check <document> [<signature>] [--against <the copy published before>]\n  airband catalog stamp <catalog/rev>";

/// `airband catalog export` prints the catalogue built into this program, as it is published.
/// `sign` prints a document's signature, made with the private key in a file (32 bytes, in hex). `check` says
/// whether a document would be taken by this build. `stamp` gives the rows a
/// new number when they have changed. `scripts/publish-catalog.sh` and `scripts/catalog-stamp.sh` use them.
pub fn command(args: &[String]) -> Result<(), String> {
    let words = plain(args.get(1..).unwrap_or_default());
    match args.first().map(String::as_str) {
        Some("export") => {
            let days: u64 = option(args, "--expires-in-days").map(|d| d.parse().map_err(|_| "--expires-in-days takes a number")).transpose()?.unwrap_or(180);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            println!("{}", catalog::built_in(now + days * 86400));
            Ok(())
        }
        Some("sign") => {
            use ed25519_dalek::Signer;
            let (path, doc) = (words.first().ok_or(USAGE)?, words.get(1).ok_or(USAGE)?);
            let hex = std::fs::read_to_string(path).map_err(|e| format!("cannot read the key file: {e}"))?;
            let hex = hex.trim();
            let seed: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
            let seed: [u8; 32] = seed.try_into().map_err(|_| "that file does not hold a key")?;
            let doc = std::fs::read(doc).map_err(|e| format!("cannot read the document: {e}"))?;
            println!("{}", airband_core::audio::base64_encode(&ed25519_dalek::SigningKey::from_bytes(&seed).sign(&doc).to_bytes()));
            Ok(())
        }
        Some("check") => {
            let doc = std::fs::read(words.first().ok_or(USAGE)?).map_err(|e| e.to_string())?;
            let text = std::str::from_utf8(&doc).map_err(|e| e.to_string())?;
            if let Some(sig) = words.get(1) {
                catalog::signed(&doc, &std::fs::read_to_string(sig).map_err(|e| e.to_string())?)?;
            }
            // every row of it is one this build can use: what it would leave out is a mistake in a row, or
            // an interface nobody has written yet
            let c = catalog::inspect(text)?;
            let unspoken: Vec<String> = c.providers.iter().flat_map(|p| p.offers.iter().filter(|o| !crate::providers::speaks(o)).map(move |o| format!("{} by \"{}\"", p.id, o.by.interface))).collect();
            if !c.left_out.is_empty() || !unspoken.is_empty() {
                return Err(format!("this build would leave rows of it out: {}", c.left_out.iter().chain(&unspoken).cloned().collect::<Vec<_>>().join("; ")));
            }
            if let Some(before) = option(args, "--against") {
                let before = std::fs::read_to_string(before).map_err(|e| e.to_string())?;
                catalog::would_take(text, &before)?;
                println!("A build that has the copy published before would take it.");
            }
            println!("Rev {}: {} providers, {} models for the phone, every row usable by this build.", c.rev, c.providers.len(), c.models.len());
            Ok(())
        }
        Some("stamp") => {
            // the rows changed since the number was given: the next number, and what they hash to now
            let path = words.first().ok_or(USAGE)?;
            let built = catalog::built();
            let stamped = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            if stamped.lines().nth(1).map(str::trim) == Some(built.hash.as_str()) {
                println!("The rows have not changed since rev {}.", built.rev);
                return Ok(());
            }
            std::fs::write(path, format!("{}\n{}\n", built.rev + 1, built.hash)).map_err(|e| e.to_string())?;
            println!("The rows changed: now rev {}. Build again before publishing.", built.rev + 1);
            Ok(())
        }
        _ => {
            println!("The catalogue in use is rev {}: {} providers, {} models for the phone.\n{USAGE}", catalog::current().rev, catalog::current().providers.len(), catalog::current().models.len());
            Ok(())
        }
    }
}
