//! Where logins are kept: the keychain, through /usr/bin/security exactly as Claude Code uses it, so Claude Code's
//! own access to its item never changes (an item written another way would make Claude Code ask him for
//! permission). The tests keep them in memory instead, so no real login is ever touched.
use super::Res;
use crate::config;
use std::io::Write;
use std::process::{Command, Stdio};

/// A place that keeps one secret per service and account.
pub(super) trait Vault: Send + Sync {
    /// The secret, or nothing when there is no such item.
    fn read(&self, service: &str, account: &str) -> Res<Option<String>>;
    /// Keep it, and read it back to be sure it was kept.
    fn write(&self, service: &str, account: &str, data: &str) -> Res<()>;
    /// Take an item out. One that is not there is already out.
    fn delete(&self, service: &str, account: &str) -> Res<()>;
}

/// The login keychain.
pub(super) struct Keychain;

/// What `security` ends with when the item asked for is not there (`errSecItemNotFound`).
const ITEM_NOT_FOUND: i32 = 44;

/// `security`, run as the user it really is: the login keychain is found through the home folder, and with
/// HOME pointed anywhere else `security` finds no keychain and waits for ever on a dialog.
/// The keychain file the end-to-end suites make for a run of their own (`AIRBAND_STAND_IN_KEYCHAIN`), in place of
/// his login keychain: read only by a build with the `stand-ins` feature. Nothing for the airband he installs,
/// which always uses his login keychain.
fn stand_in_keychain() -> Option<String> {
    super::super::stand_in("AIRBAND_STAND_IN_KEYCHAIN")
}

fn security() -> Command {
    // by its full path: the tower runs with his terminal's PATH, and every login goes to this program
    let mut c = Command::new(crate::platform::SECURITY);
    if let Some(a) = config::account().filter(|a| !a.home.is_empty()) {
        c.env("HOME", a.home);
    }
    c.stdin(Stdio::null());
    c
}

impl Vault for Keychain {
    fn read(&self, service: &str, account: &str) -> Res<Option<String>> {
        let out = security().args(["find-generic-password", "-a", account, "-s", service, "-w"]).args(stand_in_keychain()).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            // `security` exits with ITEM_NOT_FOUND when there is no such item (errSecItemNotFound)
            return if out.status.code() == Some(ITEM_NOT_FOUND) { Ok(None) } else { Err(format!("the keychain would not give {service} ({})", out.status)) };
        }
        let s = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string();
        // `security -w` prints anything that is not plain text as hex
        if !s.is_empty()
            && let Some(t) = airband_core::encode::unhex(&s).and_then(|b| String::from_utf8(b).ok()).filter(|t| t.starts_with('{'))
        {
            return Ok(Some(t));
        }
        Ok(Some(s))
    }
    fn write(&self, service: &str, account: &str, data: &str) -> Res<()> {
        let line = add_line(service, account, data, stand_in_keychain().as_deref())?;
        // On standard input, so the login is never in the process list. `security -i` takes a line of about 4 KB
        // at most. A longer one could only go as an argument, where any program on this computer can read it
        // while the command runs, so it is not handed over at all.
        if line.len() > 4032 {
            return Err("This login is too long for airband to put in the keychain without other programs on this computer being able to see it for a moment, so nothing was changed.".into());
        }
        let status = {
            let mut child = security().arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
            child.stdin.take().expect("piped").write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?
        };
        if !status.success() {
            return Err(format!("the keychain would not keep {service} ({status})"));
        }
        match self.read(service, account)? {
            Some(back) if back == data => Ok(()),
            _ => Err(format!("the keychain did not keep {service}")),
        }
    }
    fn delete(&self, service: &str, account: &str) -> Res<()> {
        let status = security().args(["delete-generic-password", "-a", account, "-s", service]).args(stand_in_keychain()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| format!("could not ask the keychain: {e}"))?;
        // `security` ends with 44 for an item that is not there
        match status.code() {
            Some(0 | ITEM_NOT_FOUND) => Ok(()),
            _ => Err(format!("the keychain did not delete it ({status})")),
        }
    }
}

/// Logins kept in memory, for the tests.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Memory(pub(super) std::sync::Mutex<std::collections::HashMap<(String, String), String>>);

#[cfg(test)]
impl Vault for Memory {
    fn read(&self, service: &str, account: &str) -> Res<Option<String>> {
        Ok(self.0.lock().unwrap().get(&(service.to_string(), account.to_string())).cloned())
    }
    fn write(&self, service: &str, account: &str, data: &str) -> Res<()> {
        self.0.lock().unwrap().insert((service.to_string(), account.to_string()), data.to_string());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Res<()> {
        self.0.lock().unwrap().remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

/// The line that puts a login in the keychain, for `security -i` on standard input. The account and the service
/// go inside quotes there, which nothing in them may end: one that holds a quote, a backslash or a control
/// character is refused, never put on the line as it is (it could end the quotes and add words of its own).
fn add_line(service: &str, account: &str, data: &str, on: Option<&str>) -> Result<String, String> {
    let quotable = |s: &str| !s.chars().any(|c| c == '"' || c == '\\' || c.is_control());
    if !quotable(account) || !quotable(service) || !on.is_none_or(quotable) {
        return Err("This account's name has a character the keychain's command line cannot take, so nothing was changed.".into());
    }
    let hex = airband_core::encode::hex(data.as_bytes());
    let on = on.map(|k| format!(" \"{k}\"")).unwrap_or_default();
    Ok(format!("add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{hex}\"{on}\n"))
}

#[cfg(test)]
mod line_tests {
    #[test]
    fn an_account_that_could_end_its_quotes_is_refused_never_put_on_the_line() {
        assert!(super::add_line("svc", "a\" -X \"00", "{}", None).is_err());
        assert!(super::add_line("svc", "a\nb", "{}", None).is_err());
        assert!(super::add_line("svc", "a\\b", "{}", None).is_err());
        let line = super::add_line("svc", "me@example.com", "{}", None).unwrap();
        assert!(line.starts_with("add-generic-password -U -a \"me@example.com\" -s \"svc\""));
        // a run's own keychain file is named last, quoted like the rest
        let line = super::add_line("svc", "me@example.com", "{}", Some("/tmp/run.keychain")).unwrap();
        assert!(line.ends_with(" \"/tmp/run.keychain\"\n"), "{line}");
        assert!(super::add_line("svc", "me@example.com", "{}", Some("/tmp/a\" -X \"00")).is_err());
    }
}
