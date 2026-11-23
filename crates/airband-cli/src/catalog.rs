//! The catalogue on this computer: a newer signed copy is fetched now and then and kept beside the settings,
//! and `airband catalog` puts out what is built in, for publishing. See `airband_core::catalog`. Reading a
//! kept copy and fetching a new one are `airband_io`'s (`airband_io::catalog::load_kept`, `refresh`),
//! the same code the app runs.
use crate::config;
use airband_core::catalog;
use std::sync::Mutex;

/// What loading the kept copy had to say, until there is a log to say it in.
static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Use the newest copy this computer has kept, if it is newer than the one built in. Nothing is fetched.
pub(crate) fn load() {
    *SAID.lock().unwrap() = airband_io::catalog::load_kept(&config::dir(), &crate::providers::speaks);
}

/// What loading the kept copy had to say, taken: for a command that fetches nothing.
pub(crate) fn said() -> Vec<String> {
    std::mem::take(&mut *SAID.lock().unwrap())
}

/// Fetch the published copy and, if it is airband's and newer, use it and keep it. Quietly does nothing
/// when there is no network. Blocks for a few seconds at most. True when it is now the one in use, with what
/// is worth a line in a log (rows this build cannot use, a copy that was refused).
pub(crate) fn refresh() -> (bool, Vec<String>) {
    let mut lines = said();
    let (changed, more) = airband_io::catalog::refresh(&config::dir(), &crate::providers::speaks);
    lines.extend(more);
    (changed, lines)
}



const USAGE: &str = "  airband catalog export [--expires-in-days 180]\n  airband catalog sign <key file> <document>\n  airband catalog check <document> [<signature>] [--against <the copy published before>]\n  airband catalog stamp <catalog/rev>";

/// `airband catalog export` prints the catalogue built into this program, as it is published.
/// `sign` prints a document's signature, made with the private key in a file (32 bytes, in hex). `check` says
/// whether a document would be taken by this build. `stamp` gives the rows a
/// new number when they have changed. `scripts/publish-catalog.sh` and `scripts/catalog-stamp.sh` use them.
pub(crate) fn command(args: &[String]) -> Result<(), String> {
    let read = crate::words("catalog", args.get(1..).unwrap_or_default(), &["--expires-in-days", "--against"], &[])?;
    let words = &read.rest;
    match args.first().map(String::as_str) {
        Some("export") => {
            let days: u64 = read.flag("expires-in-days").map(|d| d.parse().map_err(|_| "--expires-in-days takes a number")).transpose()?.unwrap_or(180);
            let now = airband_core::clock::now_secs();
            println!("{}", catalog::built_in(now + days * 86400));
            Ok(())
        }
        Some("sign") => {
            use ed25519_dalek::Signer;
            let (path, doc) = (words.first().ok_or(USAGE)?, words.get(1).ok_or(USAGE)?);
            let hex = text_of(path).map_err(|e| format!("cannot read the key file: {e}"))?;
            let hex = hex.trim();
            let seed = airband_core::encode::unhex(hex).ok_or("that file does not hold a key")?;
            let seed: [u8; 32] = seed.try_into().map_err(|_| "that file does not hold a key")?;
            let doc = bytes_of(doc).map_err(|e| format!("cannot read the document: {e}"))?;
            println!("{}", airband_core::encode::base64(&ed25519_dalek::SigningKey::from_bytes(&seed).sign(&doc).to_bytes()));
            Ok(())
        }
        Some("check") => {
            let doc = bytes_of(words.first().ok_or(USAGE)?)?;
            let text = std::str::from_utf8(&doc).map_err(|e| e.to_string())?;
            if let Some(sig) = words.get(1) {
                catalog::signed(&doc, &text_of(sig)?)?;
            }
            // every row of it is one this source can use: summaries by this program, listening and speaking by
            // the phone's core. What it would leave out is a mistake in a row, or an interface nobody has
            // written yet
            let c = catalog::inspect(text)?;
            let usable = |o: &airband_core::providers::Offer| match o.job {
                airband_core::providers::Job::Summary => crate::providers::speaks(o),
                airband_core::providers::Job::Voice | airband_core::providers::Job::Ears => airband_radio::can_use(o),
            };
            let unspoken: Vec<String> = c.providers.iter().flat_map(|p| p.offers.iter().filter(|o| !usable(o)).map(move |o| format!("{} by \"{}\"", p.id, o.by.interface))).collect();
            if !c.left_out.is_empty() || !unspoken.is_empty() {
                return Err(format!("this build would leave rows of it out: {}", c.left_out.iter().chain(&unspoken).cloned().collect::<Vec<_>>().join("; ")));
            }
            if let Some(before) = read.flag("against") {
                let before = text_of(before)?;
                catalog::would_take(text, &before)?;
                println!("A build that has the copy published before would take it.");
            }
            println!("Number {}: {} providers and {} models for the phone, and this version of airband can use every one.", c.rev, c.providers.len(), c.models.len());
            Ok(())
        }
        Some("stamp") => {
            // the rows changed since the number was given: the next number, and what they hash to now
            let path = words.first().ok_or(USAGE)?;
            let built = catalog::built();
            let stamped = text_of(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            if stamped.lines().nth(1).map(str::trim) == Some(built.hash.as_str()) {
                println!("The rows have not changed since rev {}.", built.rev);
                return Ok(());
            }
            airband_io::files::write_atomic(std::path::Path::new(path), format!("{}\n{}\n", built.rev + 1, built.hash)).map_err(|e| e.to_string())?;
            println!("The rows changed: now rev {}. Build again before publishing.", built.rev + 1);
            Ok(())
        }
        _ => {
            println!("The catalogue in use is rev {}: {} providers, {} models for the phone.\n{USAGE}", catalog::current().rev, catalog::current().providers.len(), catalog::current().models.len());
            Ok(())
        }
    }
}

/// A file named on the command line, whole: one that is not there is an error like any other.
fn text_of(path: &str) -> Result<String, String> {
    airband_io::files::read_text(std::path::Path::new(path)).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is not there"))
}
fn bytes_of(path: &str) -> Result<Vec<u8>, String> {
    airband_io::files::read_bytes(std::path::Path::new(path)).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is not there"))
}
