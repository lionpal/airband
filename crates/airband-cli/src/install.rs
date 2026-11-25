//! `airband install` / `uninstall`: putting the tower in place and starting it at login.
use crate::config;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const LABEL: &str = crate::platform::MAC_NAME;

fn installed_exe() -> PathBuf {
    config::dir().join("bin").join(if cfg!(windows) { "airband.exe" } else { "airband" })
}
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    config::home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

/// The lasting path to this binary when a package manager installed it, given where the binary really is. A
/// package manager keeps each version in a folder of its own and points one folder at the current one, so a path
/// through that one still works after it updates airband. The installer's row says how it is asked for that
/// folder (a run that prints the path alone) and where the binary is under it; the caller keeps the path only
/// when it leads to this very binary.
fn packaged_exe(installer: &airband_core::catalog::Installer, real: &Path) -> Option<PathBuf> {
    let (program, args) = installer.lasting.split_first()?;
    let out = airband_io::child::command(program).args(args).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let lasting = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim_end_matches('\n'));
    Some(in_lasting(installer, &lasting, real))
}
/// This binary's place under the package manager's lasting folder.
fn in_lasting(installer: &airband_core::catalog::Installer, lasting: &Path, real: &Path) -> PathBuf {
    lasting.join(&installer.bin).join(real.file_name().unwrap_or_default())
}

pub(crate) fn install() -> Result<(), String> {
    let cfg = config::load();
    // the PATH the tower is started with, and every program it is set up to run found on it, before anything is
    // put in place: a program it could not find would fail each time it is needed, for as long as it runs
    #[cfg(target_os = "macos")]
    let path = {
        let shell = crate::programs::login_shell()?;
        let path = crate::programs::login_path(&shell, &config::home())?;
        let unfound = crate::programs::unfound(&cfg, &path);
        if !unfound.is_empty() {
            return Err(crate::programs::refusal(&shell, &unfound));
        }
        path.into_string().map_err(|_| "Your PATH has a folder whose name is not text, which airband cannot write down.".to_string())?
    };
    #[cfg(not(target_os = "macos"))]
    let _ = &cfg;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let real = airband_io::files::real_path(&me).unwrap_or_else(|_| me.clone());
    // a package manager's copy is used where it is, so its updates change what the hooks and the tower run
    let packaged = airband_core::catalog::installers().into_iter().find_map(|i| packaged_exe(&i, &real).filter(|lasting| airband_io::files::real_path(lasting).is_ok_and(|to| to == real)).map(|lasting| (i.name, lasting)));
    let exe = match packaged {
        Some((name, lasting)) => {
            println!("Using {name}\u{2019}s copy at {}. After {name} updates airband, run `airband install` again to restart airband.", lasting.display());
            lasting
        }
        None => {
            // a copy of this binary that a rebuild cannot pull out from under running sessions
            let exe = installed_exe();
            airband_io::files::make_folder(exe.parent().unwrap()).map_err(|e| e.to_string())?;
            if me != exe {
                let tmp = exe.with_extension("new");
                airband_io::files::copy(&me, &tmp).map_err(|e| format!("cannot copy the binary to {}: {e}", exe.display()))?;
                airband_io::files::move_to(&tmp, &exe).map_err(|e| e.to_string())?;
            }
            exe
        }
    };
    // each harness that needs something set up on this machine to report to the tower gets it
    for line in crate::adapters::install(&exe)? {
        println!("{line}");
    }
    #[cfg(target_os = "macos")]
    return start_at_login(&exe, &path);
    #[cfg(not(target_os = "macos"))]
    start_at_login(&exe)
}

/// Text as it is written inside an XML element: a folder or PATH with `&` or `<` in it stays what it is.
#[cfg(target_os = "macos")]
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
fn start_at_login(exe: &std::path::Path, path: &str) -> Result<(), String> {
    let log = config::dir().join("airband.log");
    // the tower finds the coding tools where his terminal does: by his login shell's PATH (`programs::login_path`),
    // never the PATH of whatever ran this, which over ssh leaves out the folders his shell adds
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
        exe = xml_text(&exe.display().to_string()),
        log = xml_text(&log.display().to_string()),
        path = xml_text(path),
    );
    airband_io::files::make_folder(plist_path().parent().unwrap()).map_err(|e| e.to_string())?;
    airband_io::files::write_atomic(&plist_path(), plist).map_err(|e| e.to_string())?;
    let uid = unsafe { libc::getuid() };
    airband_io::child::command(crate::platform::LAUNCHCTL).args(["bootout", &format!("gui/{uid}/{LABEL}")]).output().ok();
    // stopping the old one takes a moment, and starting before it has gone fails ("Input/output error"): this left
    // the tower down for some minutes once. Try again until it takes.
    let start = || airband_io::child::command(crate::platform::LAUNCHCTL).args(["bootstrap", &format!("gui/{uid}"), &plist_path().display().to_string()]).output().map_err(|e| e.to_string());
    let mut out = start()?;
    for _ in 0..8 {
        if out.status.success() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        out = start()?;
    }
    if !out.status.success() {
        return Err(format!("airband could not be set to start at login: {}", String::from_utf8_lossy(&out.stderr).trim()));
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
        let service = format!("gui/{uid}/{LABEL}");
        // one not running cannot be stopped, and that is no error: whether it is still loaded after is the fact
        airband_io::child::command(crate::platform::LAUNCHCTL).args(["bootout", &service]).output().map_err(|e| format!("could not run launchctl: {e}"))?;
        let loaded = airband_io::child::command(crate::platform::LAUNCHCTL).args(["print", &service]).output().is_ok_and(|o| o.status.success());
        stopped(loaded, airband_io::files::remove(&plist_path()))?;
    }
    println!("airband is stopped and will not start at login. Your coding tools no longer report to it.");
    Ok(())
}

/// Whether airband can say it is stopped and will not start at login: only when the system no longer has it
/// loaded and its start-at-login file is gone. Either left is said.
#[cfg(target_os = "macos")]
fn stopped(still_loaded: bool, removed: std::io::Result<()>) -> Result<(), String> {
    if still_loaded {
        return Err("airband is still running: the system would not stop it. Try again, or log out and in.".into());
    }
    removed.map_err(|e| format!("airband is stopped, but its start-at-login file could not be removed ({e}): {}", plist_path().display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn stopped_is_said_only_when_it_is_unloaded_and_its_file_is_gone() {
        assert!(stopped(false, Ok(())).is_ok());
        assert!(stopped(true, Ok(())).is_err());
        assert!(stopped(false, Err(std::io::Error::other("busy"))).is_err());
    }

    #[test]
    fn a_packaged_binary_is_used_from_the_path_that_outlives_an_update() {
        let installer = airband_core::catalog::installers().into_iter().next().expect("this build compiles an installer row in");
        assert!(!installer.lasting.is_empty() && !installer.install.is_empty());
        let lasting = Path::new("/pm/opt/airband");
        assert_eq!(in_lasting(&installer, lasting, Path::new("/pm/Cellar/airband/0.1.1/bin/airband")), lasting.join(&installer.bin).join("airband"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod xml_tests {
    #[test]
    fn a_path_with_markup_in_it_is_written_as_text() {
        assert_eq!(super::xml_text("/Users/a&b/<bin>"), "/Users/a&amp;b/&lt;bin&gt;");
    }
}
