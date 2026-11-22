//! A model the tower runs itself, on this computer, for a provider whose row says so (the `on-this-computer`
//! interface).
//! Nothing a message says leaves the machine.
//!
//! The model is run by llama.cpp's server, a separate program the tower fetches once (a pinned build, checked
//! against its hash: a row of `catalog/servers/`) and starts beside itself. The server fetches the model the first time it is asked for
//! it. The tower then speaks to it over the loopback address by the same chat interface as any other service,
//! held to the shape of the answer it wants, so everything else in the tower is unchanged.
//!
//! One server, for one model, for as long as that model is what reads. It holds the model in memory the whole
//! time: about as much as the model's download.
#![expect(clippy::disallowed_methods, reason = "fetches the local model program over the network")]
use super::*;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::OnceLock;

/// Where the server listens: ten ports up from the tower's own, on the loopback address only. Set once, as the
/// tower starts.
static PORT: OnceLock<u16> = OnceLock::new();
/// The server this run started, and the model it has.
static RUNNING: Mutex<Option<(String, Child)>> = Mutex::new(None);

pub(crate) fn listen_after(tower_port: u16) {
    PORT.set(tower_port.saturating_add(10)).ok();
}

fn port() -> u16 {
    PORT.get().copied().unwrap_or(config::DEFAULT_PORT + 10)
}

fn dir() -> PathBuf {
    config::dir().join("local")
}
/// The program that runs a model, by the name a provider's row gives it: a row of the catalogue
/// (`catalog/servers/`), always as compiled into this build. Which build of it, where its archive is and what
/// that hashes to are the row's.
fn server_row(id: &str) -> Result<&'static airband_core::catalog::Server, String> {
    airband_core::catalog::built().servers.iter().find(|s| s.id == id).ok_or_else(|| format!("This version of airband has no program called \"{id}\" to run a model with."))
}
fn program(server: &airband_core::catalog::Server) -> PathBuf {
    dir().join(&server.program)
}
fn address() -> String {
    format!("http://127.0.0.1:{}", port())
}

/// Fetch the server program, if it is not here yet. Blocks while it downloads (about 17 MB).
fn fetch(server: &airband_core::catalog::Server) -> Result<PathBuf, String> {
    let program = program(server);
    if program.is_file() {
        return Ok(program);
    }
    let archive = server.archives.iter().find(|a| a.os == std::env::consts::OS && a.arch == std::env::consts::ARCH).ok_or("A model cannot be run on this kind of computer yet. Choose another way to read.")?;
    airband_io::files::make_folder(&dir()).map_err(|e| e.to_string())?;
    let url = format!("{}/{}", server.base, archive.file);
    log(format!("local model: fetching the program that runs it ({})", archive.file));
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    let mut res = agent.get(&url).call().map_err(|e| format!("the program that runs the model could not be fetched: {e}"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut res.body_mut().as_reader(), &mut bytes).map_err(|e| format!("the download stopped part way: {e}"))?;
    let got: String = airband_core::encode::hex(&<sha2::Sha256 as sha2::Digest>::digest(&bytes));
    if got != archive.sha256 {
        return Err("what came down was not the program that runs the model; nothing was kept".into());
    }
    let packed = dir().join(&archive.file);
    airband_io::files::write_atomic(&packed, &bytes).map_err(|e| e.to_string())?;
    let unpacked = Command::new(crate::platform::TAR).arg("-xzf").arg(&packed).arg("-C").arg(dir()).status().map_err(|e| format!("could not unpack it: {e}"))?;
    airband_io::files::remove(&packed).ok();
    if !unpacked.success() || !program.is_file() {
        return Err("the program that runs the model could not be unpacked".into());
    }
    Ok(program)
}

fn healthy() -> bool {
    let agent = airband_io::http::agent(Duration::from_secs(2));
    agent.get(format!("{}/health", address())).call().is_ok_and(|r| r.status().as_u16() == 200)
}

/// When a process started, as the system says it: what tells it from another that was later given its number.
fn started(pid: u32) -> Option<String> {
    let out = Command::new(crate::platform::PS).args(["-p", &pid.to_string(), "-o", "lstart="]).env("LC_ALL", "C").env("TZ", "UTC").output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty())
}

/// A server left by a tower that was stopped: only the one this tower recorded starting (its process and
/// the moment that process started, on two lines), and only if a process with that number still started at
/// that moment. Its number alone could have been given to another program since.
fn end_the_last_one() {
    let note = dir().join("server.pid");
    let noted = airband_io::files::read_text(&note).ok().flatten().unwrap_or_default();
    let mut lines = noted.lines();
    if let (Some(pid), Some(was)) = (lines.next().and_then(|p| p.trim().parse::<u32>().ok()), lines.next().filter(|p| !p.is_empty()))
        && started(pid).as_deref() == Some(was)
    {
        terminate(pid);
        std::thread::sleep(Duration::from_millis(600));
    }
    airband_io::files::remove(&note).ok();
}

/// Ask the server's keeper to end it, and wait until it has: asked, not killed, so the keeper can pass it on.
fn end(keeper: &mut Child) {
    terminate(keeper.id());
    #[cfg(not(unix))]
    let _ = keeper.kill();
    keeper.wait().ok();
}

/// Stop the server, when something else reads from now on.
pub(crate) fn stop() {
    if let Some((_, mut child)) = RUNNING.lock().unwrap().take() {
        end(&mut child);
    }
    airband_io::files::remove(&dir().join("server.pid")).ok();
}

/// How long a caller waits for a model run on this computer to be ready.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Patience {
    /// Through fetching the program and the model, which takes minutes the first time.
    Long,
    /// A moment: one not ready is told so, and asks again later.
    Moment,
}

/// The address to speak to `model` at, starting its server if it is not running. The first time, the
/// program and then the model are fetched, which takes minutes: a [`Patience::Long`] caller waits through
/// that, and any other is told it is not ready yet. Blocks.
pub(crate) fn ensure(model: &str, server: &str, patience: Patience) -> Result<String, String> {
    let server = server_row(server)?;
    {
        let mut running = RUNNING.lock().unwrap();
        let alive = running.as_mut().is_some_and(|(has, child)| has == model && child.try_wait().is_ok_and(|ended| ended.is_none()));
        if !alive {
            if let Some((_, mut old)) = running.take() {
                end(&mut old);
            }
            end_the_last_one();
            let program = fetch(server)?;
            log(format!("local model: starting {model} (the model is fetched the first time)"));
            let out = std::fs::File::create(dir().join("server.log")).map_err(|e| e.to_string())?;
            // The server is started under a small shell that ends it when this tower is gone, however the tower
            // went: a model left running by itself would hold gigabytes of memory for nobody. The same shell
            // ends it when it is told to stop.
            const KEEPER: &str = "trap 'kill \"$c\" \"$w\" 2>/dev/null' TERM INT; \"$@\" & c=$!; ( while kill -0 \"$AIRBAND_TOWER_PID\" 2>/dev/null; do sleep 2; done; kill \"$c\" 2>/dev/null ) & w=$!; wait \"$c\"";
            let child = Command::new(crate::platform::SH)
                .args(["-c", KEEPER, "sh"])
                .arg(&program)
                .env("AIRBAND_TOWER_PID", std::process::id().to_string())
                // what it is started with and in is its row's to say
                .args(server.args.iter().map(|a| a.replace("{model}", model).replace("{port}", &port().to_string())))
                .envs(server.env.iter().map(|(name, value)| (name.clone(), value.replace("{models}", &dir().join("models").display().to_string()))))
                .stdin(Stdio::null())
                .stdout(out.try_clone().map_err(|e| e.to_string())?)
                .stderr(out)
                .spawn()
                .map_err(|e| format!("the program that runs the model did not start: {e}"))?;
            // with the moment it started; one whose start cannot be read is not noted, and so is never ended later
            if let Some(began) = started(child.id()) {
                // a note that cannot be kept means a later start cannot end this one: said, and the model still runs
                if let Err(e) = airband_io::files::write_atomic(&dir().join("server.pid"), format!("{}\n{began}\n", child.id())) {
                    super::log(format!("the model program's note could not be kept: {e}"));
                }
            }
            *running = Some((model.to_string(), child));
        }
    }
    let until = Instant::now() + if patience == Patience::Long { Duration::from_secs(45 * 60) } else { Duration::from_secs(2) };
    loop {
        if healthy() {
            return Ok(format!("{}/v1", address()));
        }
        if RUNNING.lock().unwrap().as_mut().is_some_and(|(_, child)| child.try_wait().is_ok_and(|ended| ended.is_some())) {
            return Err(format!("the program that runs the model stopped. Its log is {}", dir().join("server.log").display()));
        }
        if Instant::now() >= until {
            return Err("the model on this computer is still getting ready".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
