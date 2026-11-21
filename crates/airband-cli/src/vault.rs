//! The tower's keys, kept encrypted and out of the settings file.
//!
//! `config.json` holds every setting. The secret ones (the tower's tokens, each pairing's secret, the relay's
//! admission key, the API keys for voice, listening and summaries) are taken out of it and kept together,
//! encrypted, in one of two places. The file keeps a mark in each place a secret was.
//!
//! - The Mac's login keychain, which the system encrypts with his login password. The keychain lets an item
//!   be read without asking only by the program that made it, and it knows a program by its signature. A
//!   build signed with a developer certificate is the same program to it from one build to the next; an
//!   unsigned one is a stranger every time and would be asked about, on a computer where there may be nobody
//!   to answer. So only a signed build uses it, and reading never brings up a question.
//! - Everywhere else (Linux, Windows, an unsigned build on a Mac, a shell with no keychain): a sealed file,
//!   `keys.sealed`, encrypted with a key made from this machine's own id and a salt kept beside it. A copy of
//!   the folder on another machine, in a backup or in a synced folder, opens nothing. It does not keep the
//!   keys from a program running as him on this machine; nothing kept on a disk without a password can.
//!
//! One thing stays in the settings file: the token the hooks and commands on this computer show the tower,
//! over the loopback address only. It is made here, opens nothing from outside, and has to be readable where
//! the keychain is not (a shell over SSH cannot use the login keychain), or agents started there would go
//! unseen. A command run where the keys cannot be read still works for what needs no key; the tower itself
//! will not start without them.
use serde_json::{Map, Value};

/// What a secret's place in the file holds while the secret is in the keychain.
pub const MARK: &str = "in-keychain";
#[cfg(target_os = "macos")]
const SERVICE: &str = "dev.airband.tower";
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "keys";

/// Every place in the settings that holds a secret, with a name for it that does not change.
fn each(doc: &mut Value, f: &mut dyn FnMut(String, &mut Value)) {
    for name in ["radio_token", "relay_admission"] {
        if let Some(v) = doc.get_mut(name).filter(|v| v.is_string()) {
            f(name.to_string(), v);
        }
    }
    for part in ["voice", "ears", "summarizer", "summarizer_fallback"] {
        // a key written by its name is not a secret: the key it names is kept below
        if let Some(v) = doc.get_mut(part).and_then(|p| p.get_mut("key")).filter(|v| v.as_str().is_some_and(|k| !k.starts_with("key:"))) {
            f(format!("{part}.key"), v);
        }
    }
    if let Some(keys) = doc.get_mut("keys").and_then(Value::as_object_mut) {
        for (name, k) in keys {
            if let Some(v) = k.get_mut("secret").filter(|v| v.is_string()) {
                f(format!("key:{name}"), v);
            }
        }
    }
    if let Some(pairings) = doc.get_mut("pairings").and_then(Value::as_array_mut) {
        for p in pairings {
            let name = format!("pairing:{}", p["name"].as_str().unwrap_or(""));
            if let Some(v) = p.get_mut("secret").filter(|v| v.is_string()) {
                f(name, v);
            }
        }
    }
}

/// Take the secrets out of the settings, leaving the mark. A place that already holds the mark is filled from
/// `kept` (what the keychain has), so nothing is lost by sealing twice.
pub fn seal(doc: &mut Value, kept: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    each(doc, &mut |name, v| {
        let secret = if v.as_str() == Some(MARK) { kept.get(&name).cloned() } else { Some(v.clone()) };
        if let Some(secret) = secret.filter(|s| s.as_str().is_some_and(|s| !s.is_empty())) {
            out.insert(name, secret);
            *v = Value::String(MARK.into());
        }
    });
    out
}

/// Put the secrets back where the marks are. Err names the first one the keychain does not have.
pub fn open(doc: &mut Value, kept: &Map<String, Value>) -> Result<(), String> {
    let mut missing = None;
    each(doc, &mut |name, v| {
        if v.as_str() == Some(MARK) {
            match kept.get(&name) {
                Some(secret) => *v = secret.clone(),
                None => missing = missing.take().or(Some(name)),
            }
        }
    });
    missing.map_or(Ok(()), |name| Err(format!("the keychain has no \"{name}\"")))
}

/// Does this text have secrets kept elsewhere?
pub fn sealed(text: &str) -> bool {
    text.contains(&format!("\"{MARK}\""))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    /// The keychain is only touched in his real home: under a test's made-up one there is no keychain to find.
    fn at_home() -> bool {
        let real = unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() { return false }
            std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
        };
        std::env::var("HOME").is_ok_and(|h| h.trim_end_matches('/') == real.trim_end_matches('/'))
    }

    pub fn get() -> Result<Map<String, Value>, String> {
        if !at_home() {
            return Err("the keychain is not used outside his own home folder".into());
        }
        // never a question on the screen: there may be nobody at it
        let _quiet = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
        let bytes = get_generic_password(SERVICE, ACCOUNT).map_err(|e| format!("the keychain did not give airband its keys ({e}). This copy of airband may not be the signed one that put them there."))?;
        serde_json::from_slice(&bytes).map_err(|_| "what the keychain holds for airband cannot be read".to_string())
    }

    pub fn put(kept: &Map<String, Value>) -> Result<(), String> {
        if !at_home() {
            return Err("the keychain is not used outside his own home folder".into());
        }
        let _quiet = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
        set_generic_password(SERVICE, ACCOUNT, &serde_json::to_vec(kept).unwrap()).map_err(|e| format!("the keychain did not take airband's keys ({e})"))
    }

    /// Take airband's item out of the keychain.
    pub fn forget() {
        if at_home() {
            let _ = delete_generic_password(SERVICE, ACCOUNT);
        }
    }

    /// Is this program signed with a developer certificate, and in his own home? Only then is it the same
    /// program to the keychain after the next build, with a keychain to find.
    pub fn signed() -> bool {
        if !at_home() {
            return false;
        }
        let Ok(me) = std::env::current_exe() else { return false };
        std::process::Command::new("/usr/bin/codesign").args(["-d", "--verbose=2"]).arg(me).output().is_ok_and(|o| {
            String::from_utf8_lossy(&o.stderr).lines().any(|l| l.strip_prefix("TeamIdentifier=").is_some_and(|team| team != "not set" && !team.is_empty()))
        })
    }
}

/// The sealed file: for wherever there is no keychain this program can use without asking.
mod file {
    use super::*;
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use std::path::PathBuf;

    const BOUND: &[u8] = b"airband.vault/1";

    /// Where the two files are. A test names a folder of its own.
    fn folder() -> PathBuf {
        #[cfg(test)]
        if let Some(dir) = tests_folder::get() {
            return dir;
        }
        crate::config::dir()
    }
    #[cfg(test)]
    pub mod tests_folder {
        thread_local!(static DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) });
        pub fn set(dir: std::path::PathBuf) {
            DIR.with(|d| *d.borrow_mut() = Some(dir));
        }
        pub fn get() -> Option<std::path::PathBuf> {
            DIR.with(|d| d.borrow().clone())
        }
    }
    fn sealed_at() -> PathBuf {
        folder().join("keys.sealed")
    }
    fn salt_at() -> PathBuf {
        folder().join("keys.salt")
    }

    /// What this machine calls itself to its own system, the same from one start to the next and different on
    /// every other machine. Empty where the system has none to give; the salt alone makes the key then.
    fn machine() -> String {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| {
            let said = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            if cfg!(target_os = "macos") {
                said("/usr/sbin/ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"]).lines().find(|l| l.contains("IOPlatformUUID")).and_then(|l| l.split('"').nth(3)).unwrap_or_default().to_string()
            } else if cfg!(windows) {
                said("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"]).split_whitespace().last().unwrap_or_default().to_string()
            } else {
                ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default().trim().to_string()
            }
        })
        .clone()
    }

    fn cipher(make_salt: bool) -> Result<ChaCha20Poly1305, String> {
        let salt = match std::fs::read(salt_at()) {
            Ok(salt) if salt.len() == 32 => salt,
            _ if make_salt => {
                let mut salt = vec![0u8; 32];
                getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
                crate::config::write_private(&salt_at(), &salt).map_err(|e| format!("cannot write the keys' salt: {e}"))?;
                salt
            }
            _ => return Err("the sealed keys have lost their salt and cannot be opened".into()),
        };
        let mut key = [0u8; 32];
        hkdf::Hkdf::<sha2::Sha256>::new(Some(&salt), machine().as_bytes()).expand(BOUND, &mut key).map_err(|e| e.to_string())?;
        Ok(ChaCha20Poly1305::new((&key).into()))
    }

    pub fn has() -> bool {
        sealed_at().is_file()
    }

    pub fn get() -> Result<Map<String, Value>, String> {
        let sealed = std::fs::read(sealed_at()).map_err(|_| "the tower's keys are not on this computer".to_string())?;
        if sealed.len() < 12 + 16 {
            return Err("the sealed keys are cut short".into());
        }
        let (nonce, body) = sealed.split_at(12);
        let plain = cipher(false)?.decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: BOUND }).map_err(|_| "the sealed keys do not open on this machine. They were sealed on another, or the file was changed.".to_string())?;
        serde_json::from_slice(&plain).map_err(|_| "the sealed keys cannot be read".to_string())
    }

    pub fn put(kept: &Map<String, Value>) -> Result<(), String> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let sealed = cipher(true)?.encrypt(Nonce::from_slice(&nonce), Payload { msg: &serde_json::to_vec(kept).unwrap(), aad: BOUND }).map_err(|_| "the keys could not be sealed".to_string())?;
        crate::config::write_private(&sealed_at(), &[nonce.as_slice(), &sealed].concat()).map_err(|e| format!("cannot write the sealed keys: {e}"))
    }

    pub fn forget() {
        let _ = std::fs::remove_file(sealed_at());
        let _ = std::fs::remove_file(salt_at());
    }
}

/// Can this program use the keychain without anyone being asked? A signed build on a Mac.
#[cfg(target_os = "macos")]
fn keychain() -> bool {
    static SIGNED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SIGNED.get_or_init(mac::signed)
}

/// Where the keys are kept on this computer, in words for him.
pub fn place() -> &'static str {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        return "in the keychain";
    }
    "sealed on this computer"
}

/// The keys. From the keychain where this program can use it, else from the sealed file.
pub fn get() -> Result<Map<String, Value>, String> {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        return mac::get();
    }
    file::get()
}

/// Keep the keys. In the keychain where this program can use it; where it cannot (an unsigned build, a shell
/// with no keychain, another system) in the sealed file. Never anywhere in the clear.
pub fn put(kept: &Map<String, Value>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if keychain() && mac::put(kept).is_ok() {
        file::forget();
        return Ok(());
    }
    file::put(kept)
}

/// Where a request to carry the keys waits for a tower that can read the keychain (a shell over SSH cannot).
pub fn carry_asked() -> std::path::PathBuf {
    crate::config::dir().join("carry-keys")
}

/// Before this program is replaced by a copy signed with another certificate: move the keys out of the
/// keychain into the sealed file. The keychain knows a program by the certificate that signed it, so the new
/// copy could not read what this one put there. It reads the sealed file instead, and puts the keys back in
/// the keychain as its own (`settle`). Nothing is taken out of the keychain until the file gives the same
/// keys back. Ok(false) when there was nothing in the keychain to move.
pub fn carry() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        let kept = mac::get()?;
        file::put(&kept)?;
        if file::get()? != kept {
            file::forget();
            return Err("the sealed file did not give the keys back; they are still in the keychain".into());
        }
        mac::forget();
        return Ok(true);
    }
    Ok(false)
}

/// Keys sealed in the file by a run that could not use the keychain are moved into it by one that can: the
/// tower, at its start. Says so when it moved them.
pub fn settle() -> bool {
    #[cfg(target_os = "macos")]
    if keychain() && file::has() {
        if let Ok(kept) = file::get() {
            if mac::put(&kept).is_ok() && mac::get().is_ok_and(|back| back == kept) {
                file::forget();
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_sealed_file_opens_here_and_holds_nothing_readable() {
        let dir = std::env::temp_dir().join(format!("airband-vault-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        file::tests_folder::set(dir.clone());
        let kept: Map<String, Value> = serde_json::from_value(json!({"key:eleven": "SECRET-ONE", "pairing:phone": "SECRET-TWO"})).unwrap();
        file::put(&kept).unwrap();
        assert_eq!(file::get().unwrap(), kept);
        let on_disk = std::fs::read(dir.join("keys.sealed")).unwrap();
        assert!(!String::from_utf8_lossy(&on_disk).contains("SECRET"));
        // a changed file does not open, and neither does one whose salt has gone
        let mut bent = on_disk.clone();
        *bent.last_mut().unwrap() ^= 1;
        std::fs::write(dir.join("keys.sealed"), &bent).unwrap();
        assert!(file::get().is_err());
        std::fs::write(dir.join("keys.sealed"), &on_disk).unwrap();
        std::fs::write(dir.join("keys.salt"), [7u8; 32]).unwrap();
        assert!(file::get().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn secrets_leave_the_file_and_come_back_whole() {
        let whole = json!({"port": 7373, "token": "local", "radio_token": "t2", "name": "C", "voice": {"kind": "elevenlabs", "key": "vk", "voice": "v"}, "ears": {"kind": "device"}, "pairings": [{"name": "phone", "secret": "s1"}, {"name": "pad", "secret": "s2"}]});
        let mut doc = whole.clone();
        let kept = seal(&mut doc, &Map::new());
        let text = doc.to_string();
        assert!(sealed(&text));
        assert_eq!(doc["token"], "local");
        for secret in ["t2", "vk", "s1", "s2"] {
            assert!(!text.contains(&format!("\"{secret}\"")), "{secret} is still in the file");
        }
        assert_eq!(kept.len(), 4);
        assert_eq!(doc["voice"]["voice"], "v");
        // sealing what is already sealed loses nothing
        let again = seal(&mut doc.clone(), &kept);
        assert_eq!(again, kept);
        open(&mut doc, &kept).unwrap();
        assert_eq!(doc, whole);
        // a pairing the keychain has no secret for is said, by name
        let mut short = kept.clone();
        short.remove("pairing:pad");
        let mut doc = whole.clone();
        seal(&mut doc, &Map::new());
        assert!(open(&mut doc, &short).unwrap_err().contains("pairing:pad"));
    }
}
