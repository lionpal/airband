//! The tower's keys, kept encrypted and out of the settings file.
//!
//! `config.json` holds every setting. The secret ones (the tower's tokens, each pairing's secret, the relay's
//! admission key, the API keys for summaries) are taken out of it and kept together,
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
pub(crate) const MARK: &str = "in-keychain";
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
    for part in crate::config::USERS {
        // a key written by its name is not a secret: the key it names is kept below
        if let Some(v) = doc.get_mut(part).and_then(|p| p.get_mut("key")).filter(|v| v.as_str().is_some_and(|k| !k.starts_with(crate::config::BY_NAME))) {
            f(format!("{part}.key"), v);
        }
    }
    if let Some(keys) = doc.get_mut("keys").and_then(Value::as_object_mut) {
        for (name, k) in keys {
            if let Some(v) = k.get_mut("secret").filter(|v| v.is_string()) {
                f(format!("{}{name}", crate::config::BY_NAME), v);
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
pub(crate) fn seal(doc: &mut Value, kept: &Map<String, Value>) -> Map<String, Value> {
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
pub(crate) fn open(doc: &mut Value, kept: &Map<String, Value>) -> Result<(), String> {
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

/// Do these settings have secrets kept elsewhere: is one of the places that holds a secret marked as kept in the
/// keychain? Read as settings, never by looking for the mark in the text. Text that is not settings has none.
pub(crate) fn sealed(text: &str) -> bool {
    let Ok(mut doc) = serde_json::from_str::<Value>(text) else { return false };
    let mut marked = false;
    each(&mut doc, &mut |_, v| marked |= v.as_str() == Some(MARK));
    marked
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    /// The keychain is only touched in his real home: under a test's made-up one there is no keychain to find.
    fn at_home() -> bool {
        let Some(real) = crate::config::account().map(|a| a.home) else { return false };
        std::env::var("HOME").is_ok_and(|h| h.trim_end_matches('/') == real.trim_end_matches('/'))
    }

    pub(crate) fn get() -> Result<Map<String, Value>, String> {
        if !at_home() {
            return Err("the keychain is not used outside his own home folder".into());
        }
        // never a question on the screen: there may be nobody at it
        let _quiet = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
        let bytes = get_generic_password(SERVICE, ACCOUNT).map_err(|e| format!("the keychain did not give airband its keys ({e}). This copy of airband may not be the signed one that put them there."))?;
        serde_json::from_slice(&bytes).map_err(|_| "what the keychain holds for airband cannot be read".to_string())
    }

    pub(crate) fn put(kept: &Map<String, Value>) -> Result<(), String> {
        if !at_home() {
            return Err("the keychain is not used outside his own home folder".into());
        }
        let _quiet = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
        set_generic_password(SERVICE, ACCOUNT, &serde_json::to_vec(kept).unwrap()).map_err(|e| format!("the keychain did not take airband's keys ({e})"))
    }

    /// Take airband's item out of the keychain.
    pub(crate) fn forget() {
        if at_home()
            && let Err(e) = delete_generic_password(SERVICE, ACCOUNT)
        {
            eprintln!("airband: its keys could not be taken out of the keychain: {e}");
        }
    }

    /// Is this program signed with a developer certificate, and in his own home? Only then is it the same
    /// program to the keychain after the next build, with a keychain to find.
    pub(crate) fn signed() -> bool {
        if !at_home() {
            return false;
        }
        // asked of the system in its own code-requirement language: signed by a certificate Apple issued that
        // names a team (its subject's organisational unit), which an ad-hoc or unsigned build is not
        use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
        let Ok(team) = "anchor apple generic and certificate leaf[subject.OU] exists".parse::<SecRequirement>() else { return false };
        SecCode::for_self(Flags::NONE).is_ok_and(|me| me.check_validity(Flags::NONE, &team).is_ok())
    }
}

/// The sealed file: for wherever there is no keychain this program can use without asking.
mod file {
    use super::*;
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use std::path::PathBuf;

    const BOUND: &[u8] = b"airband.vault/1";

    /// Where the two files are. A test names a folder of its own, and one that names none is stopped: a
    /// test never reads or writes his real keys.
    fn folder() -> PathBuf {
        #[cfg(test)]
        return tests_folder::get().expect("a test keeps its keys in a folder of its own (vault::tests_folder::set)");
        #[cfg(not(test))]
        crate::config::dir()
    }
    fn sealed_at() -> PathBuf {
        folder().join("keys.sealed")
    }
    fn salt_at() -> PathBuf {
        folder().join("keys.salt")
    }

    /// What this machine calls itself to its own system, the same from one start to the next and different on
    /// every other machine. Empty where the system has none to give: nothing is sealed then (`cipher`).
    fn machine() -> String {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                // the hardware's own id, from the system call for it (the IOPlatformUUID), as upper-case hex
                // in the usual 8-4-4-4-12 groups so what was sealed before still opens
                let mut id = [0u8; 16];
                let wait = libc::timespec { tv_sec: 5, tv_nsec: 0 };
                // SAFETY: gethostuuid writes exactly 16 bytes into `id` and reads `wait`; both outlive the call.
                if unsafe { libc::gethostuuid(id.as_mut_ptr(), &wait) } != 0 {
                    return String::new();
                }
                let hex: String = id.iter().map(|b| format!("{b:02X}")).collect();
                format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
            }
            #[cfg(not(target_os = "macos"))]
            if cfg!(windows) {
                // the value alone, as PowerShell's Get-ItemPropertyValue gives it, not read out of a table
                std::process::Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", r"Get-ItemPropertyValue -Path 'HKLM:\SOFTWARE\Microsoft\Cryptography' -Name MachineGuid"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            } else {
                ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| airband_io::files::read_text(std::path::Path::new(p)).ok().flatten()).unwrap_or_default().trim().to_string()
            }
        })
        .clone()
    }

    fn cipher(make_salt: bool) -> Result<ChaCha20Poly1305, String> {
        // the machine's own id is what keeps a copy of the folder from opening anywhere else: without one the
        // salt beside the file would open it, so nothing is sealed
        if machine().is_empty() {
            return Err("this computer gave no id of its own, so airband cannot keep keys on it safely".into());
        }
        // a new salt only where there is none: one that cannot be read now is not written over, or the
        // sealed file it opens could never be opened again
        let salt = match airband_io::files::read_bytes(&salt_at()) {
            Ok(Some(salt)) if salt.len() == 32 => salt,
            Ok(None) if make_salt => {
                let mut salt = vec![0u8; 32];
                getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
                crate::config::write_private(&salt_at(), &salt).map_err(|e| format!("cannot write the keys' salt: {e}"))?;
                salt
            }
            Ok(None) => return Err("the sealed keys have lost their salt and cannot be opened".into()),
            Ok(Some(_)) => return Err("the sealed keys' salt is damaged, so they cannot be opened".into()),
            Err(e) => return Err(format!("cannot read the keys' salt: {e}")),
        };
        let mut key = [0u8; 32];
        hkdf::Hkdf::<sha2::Sha256>::new(Some(&salt), machine().as_bytes()).expand(BOUND, &mut key).map_err(|e| e.to_string())?;
        Ok(ChaCha20Poly1305::new((&key).into()))
    }

    pub(crate) fn has() -> bool {
        sealed_at().is_file()
    }

    pub(crate) fn get() -> Result<Map<String, Value>, String> {
        let sealed = airband_io::files::read_bytes(&sealed_at()).map_err(|e| format!("cannot read airband's sealed keys: {e}"))?.ok_or_else(|| "airband's keys are not on this computer".to_string())?;
        if sealed.len() < 12 + 16 {
            return Err("the sealed keys are cut short".into());
        }
        let (nonce, body) = sealed.split_at(12);
        let plain = cipher(false)?.decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: BOUND }).map_err(|_| "the sealed keys do not open on this machine. They were sealed on another, or the file was changed.".to_string())?;
        serde_json::from_slice(&plain).map_err(|_| "the sealed keys cannot be read".to_string())
    }

    pub(crate) fn put(kept: &Map<String, Value>) -> Result<(), String> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let sealed = cipher(true)?.encrypt(Nonce::from_slice(&nonce), Payload { msg: &serde_json::to_vec(kept).unwrap(), aad: BOUND }).map_err(|_| "the keys could not be sealed".to_string())?;
        crate::config::write_private(&sealed_at(), [nonce.as_slice(), &sealed].concat()).map_err(|e| format!("cannot write the sealed keys: {e}"))
    }

    /// Take the sealed file away. One left behind would be read in place of the keychain (`get` prefers it), so
    /// a file that cannot be removed is an error; the salt only goes with it.
    pub(crate) fn forget() -> Result<(), String> {
        airband_io::files::remove(&sealed_at()).map_err(|e| format!("cannot remove the old sealed keys: {e}"))?;
        airband_io::files::remove(&salt_at()).map_err(|e| format!("cannot remove the old keys' salt: {e}"))
    }
    #[cfg(test)]
    pub(crate) mod tests_folder {
        thread_local!(static DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) });
        pub(crate) fn set(dir: std::path::PathBuf) {
            DIR.with(|d| *d.borrow_mut() = Some(dir));
        }
        pub(crate) fn get() -> Option<std::path::PathBuf> {
            DIR.with(|d| d.borrow().clone())
        }
    }
}

#[cfg(test)]
pub(crate) use file::tests_folder;

/// Can this program use the keychain without anyone being asked? A signed build on a Mac. Never a test's.
#[cfg(target_os = "macos")]
fn keychain() -> bool {
    if cfg!(test) {
        return false;
    }
    static SIGNED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SIGNED.get_or_init(mac::signed)
}

/// Where the keys are kept on this computer, in words for him.
pub(crate) fn place() -> &'static str {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        return "in the keychain";
    }
    "sealed on this computer"
}

/// The keys. From the keychain where this program can use it, else from the sealed file.
pub(crate) fn get() -> Result<Map<String, Value>, String> {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        return mac::get();
    }
    file::get()
}

/// Keep the keys. In the keychain for a signed build on a Mac, and in the sealed file for every other (an
/// unsigned build, another system). Never anywhere in the clear. A keychain that will not take them is an
/// error he is told: they are not quietly put somewhere else.
pub(crate) fn put(kept: &Map<String, Value>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if keychain() {
        return kept_then_forgotten(|kept| mac::put(kept).map_err(|why| format!("{why}. Run this at the computer itself, logged in, and not over SSH.")), kept);
    }
    file::put(kept)
}

/// Keep the keys where `keep` puts them, then take the sealed file away. A sealed file that stays would shadow
/// the newer keys (`get` reads it first, `settle` puts it back), so one that cannot be removed is an error.
#[cfg(any(target_os = "macos", test))]
fn kept_then_forgotten(keep: impl FnOnce(&Map<String, Value>) -> Result<(), String>, kept: &Map<String, Value>) -> Result<(), String> {
    keep(kept)?;
    file::forget().map_err(|why| format!("The keys are kept, but {why}; remove it, or the older keys are read in their place."))
}

/// Where a request to carry the keys waits for a tower that can read the keychain (a shell over SSH cannot).
pub(crate) fn carry_asked() -> std::path::PathBuf {
    crate::config::dir().join("carry-keys")
}

/// Before this program is replaced by a copy signed with another certificate: move the keys out of the
/// keychain into the sealed file. The keychain knows a program by the certificate that signed it, so the new
/// copy could not read what this one put there. It reads the sealed file instead, and puts the keys back in
/// the keychain as its own (`settle`). Nothing is taken out of the keychain until the file gives the same
/// keys back. Ok(false) when there was nothing in the keychain to move.
pub(crate) fn carry() -> Result<bool, CarryFailed> {
    #[cfg(target_os = "macos")]
    if keychain() && !file::has() {
        let kept = mac::get().map_err(CarryFailed::Keychain)?;
        sealed_and_checked(&kept, file::get).map_err(CarryFailed::Sealing)?;
        mac::forget();
        return Ok(true);
    }
    Ok(false)
}

/// Why the keys could not be carried out of the keychain: the keychain could not be read from here (a run that
/// can read it does the carrying), or the sealed file could not be made, which no other run would do better.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CarryFailed {
    Keychain(String),
    Sealing(String),
}

impl std::fmt::Display for CarryFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CarryFailed::Keychain(why) => write!(f, "the keychain could not be read ({why})"),
            CarryFailed::Sealing(why) => write!(f, "the keys could not be sealed in a file ({why})"),
        }
    }
}

/// What `airband keys carry` does when carrying failed: one the keychain refused from here is left for the next
/// run that can read it, and said; one whose sealing failed is an error, and nothing is left for later.
pub(crate) fn after_failed_carry(why: &CarryFailed) -> Result<String, String> {
    match why {
        CarryFailed::Keychain(why) => {
            airband_io::files::write_atomic(&carry_asked(), "").map_err(|e| e.to_string())?;
            Ok(format!("The keychain cannot be read from here ({why}).\nairband moves the keys the next time it starts:  airband install"))
        }
        CarryFailed::Sealing(why) => Err(format!("The keys could not be sealed in a file, so they are where they were ({why}).")),
    }
}

/// Seal `kept` in the file and read it back (`read`). However that fails, a written file is taken away again,
/// and one that cannot be is said: left there, it would be read in place of the keychain.
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "carrying the keys out of a keychain is for a Mac"))]
fn sealed_and_checked(kept: &Map<String, Value>, read: impl FnOnce() -> Result<Map<String, Value>, String>) -> Result<(), String> {
    let why = match file::put(kept).and_then(|()| read()) {
        Ok(back) if back == *kept => return Ok(()),
        Ok(_) => "the sealed file did not give the keys back".to_string(),
        Err(why) => why,
    };
    match file::forget() {
        Ok(()) => Err(format!("{why}; they are still in the keychain")),
        Err(left) => Err(format!("{why}, and {left}; remove it, or it is read in place of the keychain")),
    }
}

/// Keys sealed in the file by a run that could not use the keychain are moved into it by one that can: the
/// tower, at its start. Says so when it moved them.
pub(crate) fn settle() -> bool {
    #[cfg(target_os = "macos")]
    if keychain() && file::has()
        && let Ok(kept) = file::get()
            && mac::put(&kept).is_ok() && mac::get().is_ok_and(|back| back == kept) {
                // moved only when the file is gone too: one left would be read again in place of the keychain
                return file::forget().is_ok();
            }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_keychain_that_cannot_be_read_from_here_is_left_for_the_next_run() {
        let home = std::env::temp_dir().join(format!("airband-carry-{}", crate::config::random_hex(4)));
        crate::config::tests_home::set(home.clone());
        std::fs::create_dir_all(crate::config::dir()).unwrap();
        assert!(after_failed_carry(&CarryFailed::Sealing("disk full".into())).unwrap_err().contains("sealed"));
        assert!(!carry_asked().exists(), "a sealing that failed is not blamed on the keychain");
        assert!(after_failed_carry(&CarryFailed::Keychain("locked".into())).unwrap().contains("keychain"));
        assert!(carry_asked().exists());
        let _ = std::fs::remove_dir_all(&home);
    }
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
    fn keys_carried_to_a_file_that_cannot_be_read_back_leave_no_file_to_shadow_the_keychain() {
        let dir = std::env::temp_dir().join(format!("airband-vault-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        file::tests_folder::set(dir.clone());
        let kept: Map<String, Value> = serde_json::from_value(json!({"pairing:phone": "NEW"})).unwrap();
        // the read back fails: no file is left
        assert!(sealed_and_checked(&kept, || Err("it could not be opened".into())).is_err());
        assert!(!file::has());
        // it reads back other keys: no file is left
        assert!(sealed_and_checked(&kept, || Ok(Map::new())).is_err());
        assert!(!file::has());
        // it reads back the same keys: the file stays, to be settled into the next keychain
        assert!(sealed_and_checked(&kept, file::get).is_ok());
        assert!(file::has());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_sealed_file_that_cannot_be_removed_after_the_keys_move_is_an_error_never_left_to_shadow_them() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-vault-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        file::tests_folder::set(dir.clone());
        let kept: Map<String, Value> = serde_json::from_value(json!({"pairing:phone": "OLD"})).unwrap();
        file::put(&kept).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let moved = kept_then_forgotten(|_| Ok(()), &kept);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(moved.is_err(), "the stale file stays, so this must not say the keys are kept");
        assert!(kept_then_forgotten(|_| Ok(()), &kept).is_ok());
        assert!(!file::has());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_salt_that_cannot_be_read_is_never_written_over() {
        let dir = std::env::temp_dir().join(format!("airband-vault-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(dir.join("keys.salt")).unwrap();
        file::tests_folder::set(dir.clone());
        let kept: Map<String, Value> = serde_json::from_value(json!({"pairing:phone": "S"})).unwrap();
        // a folder where the salt goes cannot be read as one: an error, and nothing is made in its place
        assert!(file::put(&kept).is_err());
        assert!(dir.join("keys.salt").is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn secrets_leave_the_file_and_come_back_whole() {
        let whole = json!({"port": 7373, "token": "local", "radio_token": "t2", "name": "C", "summarizer": {"kind": "gemini", "key": "vk", "model": "v"}, "pairings": [{"name": "phone", "secret": "s1"}, {"name": "pad", "secret": "s2"}]});
        let mut doc = whole.clone();
        let kept = seal(&mut doc, &Map::new());
        let text = doc.to_string();
        assert!(sealed(&text));
        // the mark where no secret is kept (his name), or text that is not settings, is not sealed
        assert!(!sealed(&json!({"name": MARK}).to_string()) && !sealed(&format!("\"{MARK}\"")));
        assert_eq!(doc["token"], "local");
        for secret in ["t2", "vk", "s1", "s2"] {
            assert!(!text.contains(&format!("\"{secret}\"")), "{secret} is still in the file");
        }
        assert_eq!(kept.len(), 4);
        assert_eq!(doc["summarizer"]["model"], "v");
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
