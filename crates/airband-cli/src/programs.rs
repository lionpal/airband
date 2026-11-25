//! The programs the tower starts by name on this computer, and whether each can be found where the tower looks
//! for it: on its PATH, as the system finds a program to start (`execvp`), which is how every program the tower
//! starts by name is found (`airband_io::child::command`).
//!
//! The tower started at login has the PATH `airband install` wrote for it, and that PATH is his login shell's
//! (`login_path`): never the PATH of whatever ran `airband install`, which over ssh or from a script leaves out
//! the folders his own shell adds.
use crate::config::{self, Config};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// What a program is run for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Use {
    /// A coding tool's agents, by the tool's id: its launchers start them, and its adapter runs its program.
    Tool { tool: String },
    /// Summaries: the command of the model he chose to read his agents' messages.
    Summaries,
}

impl Use {
    /// As he is told it.
    fn words(&self) -> String {
        match self {
            Use::Tool { tool } => Some(crate::adapters::called(tool).0).filter(|n| !n.is_empty()).unwrap_or_else(|| tool.clone()),
            Use::Summaries => "summaries".into(),
        }
    }
}

/// A program the tower is set up to run, by the name it is started by, and what it is run for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Program {
    pub(crate) program: String,
    pub(crate) uses: Vec<Use>,
}

impl Program {
    /// "claude (for Claude Code and summaries)".
    pub(crate) fn words(&self) -> String {
        let uses: Vec<String> = self.uses.iter().map(Use::words).collect();
        let uses = match uses.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        format!("{} (for {uses})", self.program)
    }
}

/// Every program the tower is set up to run on this computer, each once:
/// - for each coding tool used here (its own folder is on this computer, as its row places it) and each tool
///   one of his own launchers starts: its row's program, which its adapter runs, and what its launchers start.
///   Of a tool's launchers, his usual one alone when it is one of them, as only that one is then ever picked
///   (`launch::which_launcher`); else each of them, as he may be asked to pick any.
/// - the command of the model that reads his agents' messages, when it is one that runs a program here.
pub(crate) fn needed(cfg: &Config) -> Vec<Program> {
    let built = airband_core::catalog::built();
    let built_in = |name: &str| built.tools.iter().any(|t| t.launchers.iter().any(|l| l.name == name));
    let mut tools: Vec<String> = built.tools.iter().filter(|t| used_here(t)).map(|t| t.id.clone()).collect();
    for (_, l) in cfg.launchers.iter().filter(|(name, _)| !built_in(name)) {
        if !tools.contains(&l.harness) {
            tools.push(l.harness.clone());
        }
    }
    let mut wanted: Vec<(String, Use)> = Vec::new();
    for tool in &tools {
        let by = Use::Tool { tool: tool.clone() };
        if let Some(row) = crate::adapters::tool_row(tool).map(|row| row.told("program")).filter(|p| !p.is_empty()) {
            wanted.push((row.to_string(), by.clone()));
        }
        let its: Vec<(&String, &config::Launcher)> = cfg.launchers.iter().filter(|(_, l)| l.harness == *tool).collect();
        let usual: Vec<&config::Launcher> = its.iter().filter(|(name, _)| cfg.launcher.as_ref().is_some_and(|u| u.as_str() == name.as_str())).map(|(_, l)| *l).collect();
        let picked = if usual.is_empty() { its.iter().map(|(_, l)| *l).collect() } else { usual };
        wanted.extend(picked.into_iter().filter_map(starts).map(|p| (p.to_string(), by.clone())));
    }
    if let Some(program) = crate::tower::judge::summary_program(&cfg.summarizer) {
        wanted.push((program, Use::Summaries));
    }
    let mut out: Vec<Program> = Vec::new();
    for (program, by) in wanted {
        match out.iter_mut().find(|p| p.program == program) {
            Some(p) if !p.uses.contains(&by) => p.uses.push(by),
            Some(_) => {}
            None => out.push(Program { program, uses: vec![by] }),
        }
    }
    out
}

/// A coding tool is used on this computer when the folder it keeps its own files in is here, as its row places
/// it: its home (moved or usual), and the folder in it its row names, when it names one.
fn used_here(tool: &airband_core::catalog::Tool) -> bool {
    let home = crate::adapters::tool_home(&tool.id);
    match tool.told("state") {
        "" => home.is_dir(),
        state => home.join(state).is_dir(),
    }
}

/// The program a launcher starts: the one its Agent Client Protocol server is, or the first word of its command.
fn starts(l: &config::Launcher) -> Option<&str> {
    match &l.held {
        Some(config::Held::Acp { program, .. }) => program.first(),
        _ => l.argv.first(),
    }
    .map(String::as_str)
}

/// Of the programs the tower is set up to run, the ones `path` does not lead to.
pub(crate) fn unfound(cfg: &Config, path: &OsStr) -> Vec<Program> {
    needed(cfg).into_iter().filter(|p| found(&p.program, path).is_none()).collect()
}

/// Where the system finds `program` to start it, given this PATH: a name with a `/` in it is that file; any
/// other is looked for in each folder of the PATH in turn (an empty one is the current folder), and the first
/// file there that may be run is it. As `execvp` does.
#[cfg(unix)]
pub(crate) fn found(program: &str, path: &OsStr) -> Option<PathBuf> {
    let runs = |p: &Path| {
        let Ok(c) = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(p.as_os_str())) else { return false };
        // SAFETY: the path ends in a nul, and access only reads it
        airband_io::files::metadata(p).is_ok_and(|m| m.is_file()) && unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0
    };
    if program.contains('/') {
        return Some(PathBuf::from(program)).filter(|p| runs(p));
    }
    if program.is_empty() {
        return None;
    }
    std::env::split_paths(path).map(|dir| if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir }).map(|dir| dir.join(program)).find(|p| runs(p))
}

/// The same on Windows: a name with a folder in it is that file; any other is looked for in each folder of the
/// PATH, as it is and with each ending the system runs (PATHEXT).
#[cfg(windows)]
pub(crate) fn found(program: &str, path: &OsStr) -> Option<PathBuf> {
    let endings: Vec<String> = std::env::var("PATHEXT").map(|e| e.split(';').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();
    let tries = |p: PathBuf| std::iter::once(p.clone()).chain(endings.iter().map(move |e| PathBuf::from(format!("{}{e}", p.display()))));
    if program.contains(['/', '\\']) {
        return tries(PathBuf::from(program)).find(|p| p.is_file());
    }
    std::env::split_paths(path).flat_map(|dir| tries(dir.join(program))).find(|p| p.is_file())
}

/// The PATH this process finds programs on: its own, or with none set, the one the system then uses.
pub(crate) fn own_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_else(system_path)
}

/// The PATH the system gives a program that is given none (`_CS_PATH`), which a fresh login starts from.
#[cfg(unix)]
fn system_path() -> OsString {
    let mut buf = vec![0u8; 1024];
    // SAFETY: confstr writes at most the length it is given, and says how long the whole value is
    let len = unsafe { libc::confstr(libc::_CS_PATH, buf.as_mut_ptr().cast(), buf.len()) };
    buf.truncate(len.saturating_sub(1).min(buf.len()));
    std::os::unix::ffi::OsStringExt::from_vec(buf)
}
#[cfg(not(unix))]
fn system_path() -> OsString {
    OsString::new()
}

/// His login shell, as this computer's record of his account has it.
#[cfg(unix)]
pub(crate) fn login_shell() -> Result<PathBuf, String> {
    // SAFETY: getpwuid returns this process's user's record or null, and it is read at once
    let shell = unsafe {
        let me = libc::getpwuid(libc::getuid());
        if me.is_null() || (*me).pw_shell.is_null() {
            return Err("airband could not find your login shell in this computer's record of your account.".into());
        }
        std::ffi::CStr::from_ptr((*me).pw_shell).to_bytes().to_vec()
    };
    let shell = PathBuf::from(<OsString as std::os::unix::ffi::OsStringExt>::from_vec(shell));
    if shell.as_os_str().is_empty() {
        return Err("Your account has no login shell set, so airband cannot learn your PATH.".into());
    }
    Ok(shell)
}

/// How long his shell has to start and say its PATH: some startup files take a few seconds.
const SHELL_SAYS_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// The PATH his login shell sets up, as a new terminal of his has it: the shell started as a login shell and an
/// interactive one, so every startup file of his it reads is read (for zsh, .zprofile and .zshrc both), from
/// what a fresh login is given (his home and name, and the system's PATH), not from whatever runs this.
///
/// What his startup files print is no part of it: the PATH is printed between two markers made new for this
/// run, which no startup file can print, and each is printed in two parts, so a startup file that echoes the
/// commands it runs does not print it either. Standard input is empty and what goes to standard error is not read.
#[cfg(unix)]
pub(crate) fn login_path(shell: &Path, home: &Path) -> Result<OsString, String> {
    use std::io::Read;
    let tag = config::random_hex(16);
    let (open, close) = (format!("airband-path-start-{tag}"), format!("airband-path-end-{tag}"));
    let script = format!("printf '%s%s%s%s%s' 'airband-path-start-' '{tag}' \"$PATH\" 'airband-path-end-' '{tag}'");
    let user = std::env::var_os("USER").or_else(|| std::env::var_os("LOGNAME"));
    let mut cmd = airband_io::child::command(shell);
    cmd.args(["-l", "-i", "-c", &script]).env_clear().env("HOME", home).env("SHELL", shell).env("PATH", system_path()).current_dir(home);
    if let Some(user) = user {
        cmd.env("USER", &user).env("LOGNAME", &user);
    }
    let shown = shell.display();
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("airband could not start your login shell ({shown}) to learn your PATH: {e}"))?;
    let mut out = child.stdout.take().expect("its output was asked for");
    let (said, heard) = std::sync::mpsc::channel();
    // read until the closing marker, never to the end: something a startup file started in the background
    // can hold the output open after the shell has gone
    std::thread::spawn(move || {
        let mut got: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&chunk[..n]),
            }
            if framed(&got, &open, &close).is_some() {
                break;
            }
        }
        said.send(framed(&got, &open, &close)).ok();
    });
    let path = heard.recv_timeout(SHELL_SAYS_WITHIN);
    // the shell is this process's own child: once it has said its PATH, or failed to in time, it is ended
    if !matches!(child.try_wait(), Ok(Some(_))) {
        child.kill().ok();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match path {
        Ok(Some(path)) if !path.is_empty() => Ok(std::os::unix::ffi::OsStringExt::from_vec(path)),
        Ok(Some(_)) => Err(format!("Your login shell ({shown}) has no PATH set.")),
        Ok(None) => Err(format!("Your login shell ({shown}) stopped ({status}) before it said its PATH.")),
        Err(_) => Err(format!("Your login shell ({shown}) did not say its PATH within {} seconds.", SHELL_SAYS_WITHIN.as_secs())),
    }
}

/// What stands between the first `open` and the first `close` after it.
fn framed(got: &[u8], open: &str, close: &str) -> Option<Vec<u8>> {
    let at = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let start = at(got, open.as_bytes())? + open.len();
    let len = at(&got[start..], close.as_bytes())?;
    Some(got[start..start + len].to_vec())
}

/// Why `airband install` stops: what his login shell cannot find, by name, and what each is for.
pub(crate) fn refusal(shell: &Path, unfound: &[Program]) -> String {
    let list: Vec<String> = unfound.iter().map(|p| format!("  {}", p.words())).collect();
    format!(
        "airband was not installed, and nothing was changed. Your login shell ({}) cannot find:\n{}\nInstall what is missing, or add its folder to PATH in your shell's startup files. Then run `airband install` again.",
        shell.display(),
        list.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("airband-programs-{name}-{}-{}", std::process::id(), config::random_hex(4)));
        airband_io::files::make_folder(&home).unwrap();
        home
    }

    /// The launchers that come with airband, as the settings have them once read.
    fn built_launchers() -> std::collections::BTreeMap<String, config::Launcher> {
        airband_core::catalog::built().tools.iter().flat_map(|t| t.launchers.iter()).map(|l| (l.name.clone(), l.clone())).collect()
    }

    #[cfg(unix)]
    fn program(dir: &Path, name: &str) {
        airband_io::files::make_folder(dir).unwrap();
        let p = dir.join(name);
        airband_io::files::write_atomic(&p, "#!/bin/sh\n").unwrap();
        airband_io::files::set_mode(&p, 0o755).unwrap();
    }

    /// Startup files that print, echo what they run, and add to PATH in both the login file and the
    /// interactive one: the PATH comes back whole, with nothing they printed in it.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_path_is_his_login_shells_whatever_its_startup_files_print() {
        let home = home("noisy");
        let zprofile = "echo 'Welcome back'\nprintf 'airband-path-start-0000'\nexport PATH=\"$HOME/from-zprofile:$PATH\"\n";
        let zshrc = "echo \"PATH is $PATH\"\nset -x\nexport PATH=\"$HOME/.local/bin:$PATH\"\necho done >&2\n";
        airband_io::files::write_atomic(&home.join(".zprofile"), zprofile).unwrap();
        airband_io::files::write_atomic(&home.join(".zshrc"), zshrc).unwrap();
        let path = login_path(Path::new("/bin/zsh"), &home).unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs.first(), Some(&home.join(".local/bin")), "{path:?}");
        assert!(dirs.contains(&home.join("from-zprofile")), "{path:?}");
        assert!(dirs.contains(&PathBuf::from("/usr/bin")), "{path:?}");
        let text = path.to_string_lossy();
        for noise in ["Welcome", "PATH is", "airband-path", "done", "\n"] {
            assert!(!text.contains(noise), "{noise:?} in {text:?}");
        }
        airband_io::files::remove_folder(&home).ok();
    }

    /// What runs `airband install` does not give the PATH: a folder only it has is not in what comes back.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_path_of_whatever_runs_install_is_not_used() {
        let home = home("caller");
        let path = login_path(Path::new("/bin/zsh"), &home).unwrap();
        let caller = std::env::var_os("PATH").unwrap_or_default();
        let only_caller: Vec<PathBuf> = std::env::split_paths(&caller).filter(|d| d.starts_with(std::env::temp_dir()) || d.ends_with("target/debug")).collect();
        assert!(std::env::split_paths(&path).all(|d| !only_caller.contains(&d)));
        airband_io::files::remove_folder(&home).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_that_never_says_its_path_is_said() {
        let home = home("silent");
        let shell = home.join("bin/silent");
        airband_io::files::make_folder(shell.parent().unwrap()).unwrap();
        airband_io::files::write_atomic(&shell, "#!/bin/sh\necho hello\nexit 3\n").unwrap();
        airband_io::files::set_mode(&shell, 0o755).unwrap();
        let why = login_path(&shell, &home).unwrap_err();
        assert!(why.contains("stopped") && why.contains("before it said its PATH"), "{why}");
        airband_io::files::remove_folder(&home).ok();
    }

    #[test]
    fn the_path_is_what_stands_between_the_markers() {
        assert_eq!(framed(b"noise airband-path-start-ab/x:/yairband-path-end-ab more", "airband-path-start-ab", "airband-path-end-ab"), Some(b"/x:/y".to_vec()));
        assert_eq!(framed(b"airband-path-start-ab/x", "airband-path-start-ab", "airband-path-end-ab"), None);
        // a marker of another run is noise
        assert_eq!(framed(b"airband-path-start-cd/z airband-path-start-ab/xairband-path-end-ab", "airband-path-start-ab", "airband-path-end-ab"), Some(b"/x".to_vec()));
    }

    /// Found as the system finds a program to start: the first folder that has it as a file that may be run.
    #[cfg(unix)]
    #[test]
    fn a_program_is_found_as_the_system_finds_it() {
        let home = home("found");
        let (a, b) = (home.join("a"), home.join("b"));
        program(&b, "tool");
        airband_io::files::make_folder(&a).unwrap();
        // not one that may be run, and a folder of that name: neither is it
        airband_io::files::write_atomic(&a.join("tool"), "x").unwrap();
        airband_io::files::make_folder(&a.join("other")).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(found("tool", &path), Some(b.join("tool")));
        assert_eq!(found("other", &path), None);
        assert_eq!(found("tool", OsStr::new("/nowhere")), None);
        assert_eq!(found(&b.join("tool").display().to_string(), OsStr::new("")), Some(b.join("tool")));
        airband_io::files::remove_folder(&home).ok();
    }

    /// A coding tool used here whose program the PATH does not lead to is named; one not used here is not asked
    /// for; the summary command is named for summaries.
    #[cfg(unix)]
    #[test]
    fn install_refuses_naming_each_program_the_path_does_not_have() {
        let home = home("refuse");
        config::tests_home::set(home.clone());
        let claude = airband_core::catalog::built().tools.iter().find(|t| t.told("program") == "claude").expect("a row runs claude");
        airband_io::files::make_folder(&crate::adapters::tool_home(&claude.id)).unwrap();
        let mut cfg: Config = serde_json::from_str("{}").unwrap();
        cfg.launchers = built_launchers();
        cfg.summarizer = crate::providers::Use::of("command", "own");
        cfg.summarizer.set.insert("argv".into(), serde_json::Value::from(vec!["my-reader", "--fast"]));
        let bin = home.join("bin");
        airband_io::files::make_folder(&bin).unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        let unfound = unfound(&cfg, &path);
        let names: Vec<&str> = unfound.iter().map(|p| p.program.as_str()).collect();
        assert!(names.contains(&"claude") && names.contains(&"my-reader"), "{names:?}");
        // no other tool's folder is here, so none of theirs is asked for
        for t in airband_core::catalog::built().tools.iter().filter(|t| t.id != claude.id) {
            assert!(unfound.iter().all(|p| p.uses.iter().all(|u| *u != Use::Tool { tool: t.id.clone() })), "{}", t.id);
        }
        let said = refusal(Path::new("/bin/zsh"), &unfound);
        assert!(said.contains("nothing was changed") && said.contains("/bin/zsh") && said.contains(&format!("claude (for {})", crate::adapters::called(&claude.id).0)) && said.contains("my-reader (for summaries)"), "{said}");
        // with every one of them on the PATH, nothing is missing
        for p in &unfound {
            program(&bin, &p.program);
        }
        assert_eq!(super::unfound(&cfg, &path), Vec::new());
        airband_io::files::remove_folder(&home).ok();
    }

    /// His usual launcher, when it is one of a tool's, is the only one of that tool's whose program is needed.
    #[cfg(unix)]
    #[test]
    fn only_his_usual_launcher_of_a_tool_is_needed() {
        let home = home("usual");
        config::tests_home::set(home.clone());
        let claude = airband_core::catalog::built().tools.iter().find(|t| t.told("program") == "claude").unwrap();
        airband_io::files::make_folder(&crate::adapters::tool_home(&claude.id)).unwrap();
        let mut cfg: Config = serde_json::from_str("{}").unwrap();
        cfg.launchers = built_launchers();
        let all: Vec<String> = needed(&cfg).into_iter().map(|p| p.program).collect();
        let (name, held) = claude.launchers.iter().find(|l| l.held.is_some()).map(|l| (l.name.clone(), l.clone())).unwrap();
        cfg.launcher = Some(airband_core::ids::LauncherId::new(name.as_str()));
        let usual: Vec<String> = needed(&cfg).into_iter().map(|p| p.program).collect();
        let others: Vec<&str> = claude.launchers.iter().filter(|l| l.name != name).filter_map(starts).filter(|p| Some(*p) != starts(&held) && *p != claude.told("program")).collect();
        for other in &others {
            assert!(all.iter().any(|p| p == other) && !usual.iter().any(|p| p == other), "{other}");
        }
        airband_io::files::remove_folder(&home).ok();
    }
}
