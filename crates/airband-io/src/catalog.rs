//! The copy of the catalogue kept on this device, and fetching a newer one: the one place either is done, for
//! the app and the tower alike. Whether a copy is airband's and newer, and what is worth saying about it, are
//! core's (`airband_core::catalog::accept`, `news`).
use airband_core::catalog::{ADDRESS, KEPT, accept, news};
use airband_core::providers::Offer;
use std::path::Path;
use std::time::Duration;

/// Why nothing was fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotFetched {
    /// No answer at all: no network, or the address could not be reached. Nothing to say.
    Unreachable,
    /// The address answered, with something other than 200: worth a line, since the catalogue should be there.
    Status(u16),
}

/// Where a catalogue comes from: the published one, or a test's.
pub trait Fetch {
    /// The bytes at `address`, when it answered 200.
    fn get(&self, address: &str) -> Result<Vec<u8>, NotFetched>;
}

/// The published catalogue, over the network: a few seconds at most.
pub struct Published;
impl Fetch for Published {
    fn get(&self, address: &str) -> Result<Vec<u8>, NotFetched> {
        let mut res = crate::http::Client::new(Duration::from_secs(6)).send(crate::http::Ask::get(address)).map_err(|_| NotFetched::Unreachable)?;
        match res.status().as_u16() {
            200 => res.body_mut().read_to_vec().map_err(|_| NotFetched::Unreachable),
            status => Err(NotFetched::Status(status)),
        }
    }
}

/// Use the newest catalogue kept in `dir`, if it is newer than the one built in. Nothing is fetched. Returns
/// what is worth a line in a log. `speaks` says whether this program has the code for an offer.
pub fn load_kept(dir: &Path, speaks: &dyn Fn(&Offer) -> bool) -> Vec<String> {
    let at = dir.join(KEPT);
    let (doc, sig) = (crate::files::read_bytes(&at), crate::files::read_text(&at.with_extension("json.sig")));
    match (doc, sig) {
        (Ok(Some(doc)), Ok(Some(sig))) => match accept(&doc, &sig) {
            Ok(taken) => news(&taken, speaks),
            Err(why) => vec![format!("catalog: the copy kept here was not used ({why})")],
        },
        // none kept yet: nothing to say
        (Ok(None), Ok(None)) => Vec::new(),
        (Err(e), _) | (_, Err(e)) => vec![format!("catalog: the copy kept here could not be read, so the one built in is used ({e})")],
        (Ok(_), Ok(_)) => vec!["catalog: the copy kept here has no signature beside it (or a signature and no copy), so the one built in is used".to_string()],
    }
}

/// Fetch the published catalogue and, if it is airband's and newer, use it and keep it in `dir`. True when
/// it is now the one in use, with what is worth a line in a log. Does nothing with no network.
pub fn refresh(dir: &Path, speaks: &dyn Fn(&Offer) -> bool) -> (bool, Vec<String>) {
    refresh_from(&Published, dir, speaks)
}

/// [`refresh`], from `from`.
pub fn refresh_from(from: &dyn Fetch, dir: &Path, speaks: &dyn Fn(&Offer) -> bool) -> (bool, Vec<String>) {
    let (doc, sig) = match (from.get(ADDRESS), from.get(&format!("{ADDRESS}.sig"))) {
        (Ok(doc), Ok(sig)) => (doc, sig),
        (Err(NotFetched::Status(status)), _) | (_, Err(NotFetched::Status(status))) => return (false, vec![format!("catalog: the published copy could not be had (the address answered {status})")]),
        _ => return (false, Vec::new()),
    };
    let sig = String::from_utf8_lossy(&sig).to_string();
    match accept(&doc, &sig) {
        Ok(taken) => {
            let mut lines = news(&taken, speaks);
            if taken.newer {
                // each written whole; a pair that could not both be written is said (the next start then reads
                // the one built in, as the signature no longer fits)
                let at = dir.join(KEPT);
                let kept = crate::files::write_atomic(&at, &doc).and_then(|_| crate::files::write_atomic(&at.with_extension("json.sig"), sig));
                if let Err(e) = kept {
                    lines.push(format!("catalog: the copy taken could not be kept for next time ({e})"));
                }
            }
            (taken.newer, lines)
        }
        Err(why) => (false, vec![format!("catalog: the published copy was not taken ({why})")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::folder;

    struct Nothing;
    impl Fetch for Nothing {
        fn get(&self, _: &str) -> Result<Vec<u8>, NotFetched> {
            Err(NotFetched::Unreachable)
        }
    }
    struct Missing;
    impl Fetch for Missing {
        fn get(&self, _: &str) -> Result<Vec<u8>, NotFetched> {
            Err(NotFetched::Status(404))
        }
    }
    struct Forged;
    impl Fetch for Forged {
        fn get(&self, address: &str) -> Result<Vec<u8>, NotFetched> {
            Ok(if address.ends_with(".sig") { b"not a signature".to_vec() } else { b"{}".to_vec() })
        }
    }

    #[test]
    fn a_kept_copy_that_cannot_be_read_is_said_and_one_not_signed_is_never_kept() {
        let dir = folder();
        // nothing kept: nothing said
        assert!(load_kept(&dir, &|_| true).is_empty());
        // a copy kept without its signature: said, never passed over unseen
        std::fs::write(dir.join(KEPT), b"{}").unwrap();
        assert_eq!(load_kept(&dir, &|_| true).len(), 1);
        std::fs::remove_file(dir.join(KEPT)).unwrap();
        // no network: nothing, quietly; a forged copy: said, and nothing kept
        assert_eq!(refresh_from(&Nothing, &dir, &|_| true), (false, Vec::new()));
        // K-N1: an address that answers, but not with the catalogue, is said
        let (taken, said) = refresh_from(&Missing, &dir, &|_| true);
        assert!(!taken && said.len() == 1 && said[0].contains("404"), "{said:?}");
        let (taken, said) = refresh_from(&Forged, &dir, &|_| true);
        assert!(!taken && said.len() == 1, "{said:?}");
        assert!(!dir.join(KEPT).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_published_copy_is_its_bytes_on_200_the_status_otherwise_and_nothing_where_nothing_answers() {
        use std::io::{Read, Write};
        let serve = |answer: &'static str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let at = format!("http://{}/catalog.json", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                let (mut conn, _) = listener.accept().unwrap();
                assert!(conn.read(&mut [0u8; 2048]).unwrap() > 0);
                conn.write_all(answer.as_bytes()).unwrap();
            });
            at
        };
        assert_eq!(Published.get(&serve("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")), Ok(b"{}".to_vec()));
        assert_eq!(Published.get(&serve("HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")), Err(NotFetched::Status(404)));
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert_eq!(Published.get(&format!("http://{free}/catalog.json")), Err(NotFetched::Unreachable));
    }
}
