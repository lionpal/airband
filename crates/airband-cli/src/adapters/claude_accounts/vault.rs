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
    fn delete(&self, service: &str, account: &str);
}

/// The login keychain.
pub(super) struct Keychain;

/// `security`, run as the user it really is: the login keychain is found through the home folder, and with
/// HOME pointed anywhere else `security` finds no keychain and waits for ever on a dialog.
fn security() -> Command {
    let mut c = Command::new("security");
    if let Some(a) = config::account().filter(|a| !a.home.is_empty()) {
        c.env("HOME", a.home);
    }
    c.stdin(Stdio::null());
    c
}

impl Vault for Keychain {
    fn read(&self, service: &str, account: &str) -> Res<Option<String>> {
        let out = security().args(["find-generic-password", "-a", account, "-s", service, "-w"]).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            // `security` exits with 44 when there is no such item (errSecItemNotFound)
            return if out.status.code() == Some(44) { Ok(None) } else { Err(format!("the keychain would not give {service} ({})", out.status)) };
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
        let hex = airband_core::encode::hex(data.as_bytes());
        let line = format!("add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{hex}\"\n");
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
    fn delete(&self, service: &str, account: &str) {
        security().args(["delete-generic-password", "-a", account, "-s", service]).stdout(Stdio::null()).stderr(Stdio::null()).status().ok();
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
    fn delete(&self, service: &str, account: &str) {
        self.0.lock().unwrap().remove(&(service.to_string(), account.to_string()));
    }
}
