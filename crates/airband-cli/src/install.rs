//! `airband install` / `uninstall`: putting the tower in place and starting it at login.
use crate::config;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const LABEL: &str = "dev.airband.tower";

fn installed_exe() -> PathBuf {
    config::dir().join("bin").join(if cfg!(windows) { "airband.exe" } else { "airband" })
}
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    config::home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

/// The lasting path to this binary when Homebrew installed it, given where the binary really is. Homebrew keeps
/// each version in a folder of its own and points `<prefix>/opt/airband` at the current one, so that path still
/// works after `brew upgrade`. Homebrew is asked where that is (`brew --prefix airband`, which prints the path
/// alone); the caller keeps it only when it leads to this very binary.
fn homebrew_exe(real: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("brew").args(["--prefix", "airband"]).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let opt = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim_end_matches('\n'));
    Some(in_opt(&opt, real))
}
/// This binary's place under Homebrew's lasting folder for the formula.
fn in_opt(opt: &Path, real: &Path) -> PathBuf {
    opt.join("bin").join(real.file_name().unwrap_or_default())
}

pub(crate) fn install() -> Result<(), String> {
    config::load();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let real = fs::canonicalize(&me).unwrap_or_else(|_| me.clone());
    // Homebrew's copy is used where it is, so `brew upgrade` changes what the hooks and the tower run
    let brewed = homebrew_exe(&real).filter(|lasting| fs::canonicalize(lasting).is_ok_and(|to| to == real));
    let exe = match brewed {
        Some(lasting) => {
            println!("Using the package manager's copy at {}. After it updates airband, run `airband install` again to restart airband.", lasting.display());
            lasting
        }
        None => {
            // a copy of this binary that a rebuild cannot pull out from under running sessions
            let exe = installed_exe();
            fs::create_dir_all(exe.parent().unwrap()).map_err(|e| e.to_string())?;
            if me != exe {
                let tmp = exe.with_extension("new");
                fs::copy(&me, &tmp).map_err(|e| format!("cannot copy the binary to {}: {e}", exe.display()))?;
                fs::rename(&tmp, &exe).map_err(|e| e.to_string())?;
            }
            exe
        }
    };
    // each harness that needs something set up on this machine to report to the tower gets it
    for line in crate::adapters::install(&exe)? {
        println!("{line}");
    }
    start_at_login(&exe)
}

#[cfg(target_os = "macos")]
fn start_at_login(exe: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    let log = config::dir().join("airband.log");
    // the tower finds the coding tools where his terminal does: by the PATH it is installed from, never one
    // written here
    let path = std::env::var("PATH").map_err(|_| "PATH is not set, so airband could not find your coding tools. Run `airband install` from a terminal.".to_string())?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>run</string></array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#,
        exe = exe.display(),
        log = log.display()
    );
    fs::create_dir_all(plist_path().parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(plist_path(), plist).map_err(|e| e.to_string())?;
    let uid = unsafe { libc::getuid() };
    let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{LABEL}")]).output();
    // stopping the old one takes a moment, and starting before it has gone fails ("Input/output error"): this left
    // the tower down for some minutes once. Try again until it takes.
    let start = || Command::new("launchctl").args(["bootstrap", &format!("gui/{uid}"), &plist_path().display().to_string()]).output().map_err(|e| e.to_string());
    let mut out = start()?;
    for _ in 0..8 {
        if out.status.success() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        out = start()?;
    }
    if !out.status.success() {
        return Err(format!("airband could not be started. launchctl said: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    println!("airband is running, and starts again when you log in. Its log: {}", log.display());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn start_at_login(exe: &std::path::Path) -> Result<(), String> {
    println!("airband cannot set itself to start at login on this computer yet. Start it yourself:  \"{}\" run", exe.display());
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    crate::adapters::uninstall()?;
    #[cfg(target_os = "macos")]
    {
        let uid = unsafe { libc::getuid() };
        let _ = std::process::Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{LABEL}")]).output();
        let _ = fs::remove_file(plist_path());
    }
    println!("airband is stopped and will not start at login. Your coding tools no longer report to it.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_homebrew_binary_is_used_from_the_path_that_outlives_an_upgrade() {
        assert_eq!(in_opt(Path::new("/opt/homebrew/opt/airband"), Path::new("/opt/homebrew/Cellar/airband/0.1.1/bin/airband")), PathBuf::from("/opt/homebrew/opt/airband/bin/airband"));
    }
}
