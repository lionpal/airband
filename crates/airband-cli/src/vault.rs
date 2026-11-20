//! The tower's keys, kept in the Mac's keychain and not in a file.
//!
//! `config.json` holds every setting. With the vault on, the secret ones (the tower's tokens, each pairing's
//! secret, the relay's admission key, the API keys for voice, listening and summaries) are taken out of it and
//! kept together as one item in the login keychain, which the system encrypts with his login password. The
//! file keeps a mark in each place a secret was.
//!
//! One thing stays in the file: the token the hooks and commands on this computer show the tower, over the
//! loopback address only. It is made here, opens nothing from outside, and has to be readable where the
//! keychain is not (a shell over SSH cannot use the login keychain), or agents started there would go unseen.
//! A command run where the keychain cannot be read still works for what needs no key; the tower itself will
//! not start without them.
//!
//! The keychain lets an item be read without asking only by the program that made it, and it knows a program
//! by its signature. A build signed with a developer certificate is the same program to it from one build to
//! the next; an unsigned one is a stranger every time and would be asked about, on a computer where there
//! may be nobody to answer. So the vault is only turned on by a signed build (`airband keys lock`, which
//! `airband install` does by itself), reading never brings up a question (it fails with words instead), and
//! an unsigned build, or one on a system with no keychain, keeps the keys in the file, readable by him alone.
use serde_json::{Map, Value};

/// What a secret's place in the file holds while the secret is in the keychain.
pub const MARK: &str = "in-keychain";
#[cfg(target_os = "macos")]
const SERVICE: &str = "dev.airband.tower";
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "keys";

/// Every place in the settings that holds a secret, with a name for it that does not change.
fn each(doc: &mut Value, f: &mut dyn FnMut(String, &mut Value)) {
    // "token" is no longer taken out, but a file from when it was still has its mark to fill
    for name in ["token", "radio_token", "relay_admission"] {
        if name == "token" && doc.get(name).and_then(Value::as_str) != Some(MARK) {
            continue;
        }
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

    pub fn forget() {
        if at_home() {
            let _ = delete_generic_password(SERVICE, ACCOUNT);
        }
    }

    /// Is this program signed with a developer certificate? Only then is it the same program to the keychain
    /// after the next build.
    pub fn signed() -> bool {
        let Ok(me) = std::env::current_exe() else { return false };
        std::process::Command::new("/usr/bin/codesign").args(["-d", "--verbose=2"]).arg(me).output().is_ok_and(|o| {
            String::from_utf8_lossy(&o.stderr).lines().any(|l| l.strip_prefix("TeamIdentifier=").is_some_and(|team| team != "not set" && !team.is_empty()))
        })
    }
}

#[cfg(target_os = "macos")]
pub use mac::{forget, get, put, signed};

#[cfg(not(target_os = "macos"))]
pub fn get() -> Result<Map<String, Value>, String> {
    Err("this system has no keychain airband can use".into())
}
#[cfg(not(target_os = "macos"))]
pub fn put(_: &Map<String, Value>) -> Result<(), String> {
    Err("this system has no keychain airband can use".into())
}
#[cfg(not(target_os = "macos"))]
pub fn forget() {}
#[cfg(not(target_os = "macos"))]
pub fn signed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        // a file from when the local token was taken out too is still opened, and then keeps it in the file
        let mut old = json!({"token": MARK, "radio_token": MARK});
        let had: Map<String, Value> = serde_json::from_value(json!({"token": "local", "radio_token": "t2"})).unwrap();
        open(&mut old, &had).unwrap();
        assert_eq!(old, json!({"token": "local", "radio_token": "t2"}));
        assert_eq!(seal(&mut old, &had).len(), 1);
        assert_eq!(old["token"], "local");
    }
}
