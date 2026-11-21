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
use super::*;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

/// Where the server listens: ten ports up from the tower's own, on the loopback address only.
static PORT: AtomicU16 = AtomicU16::new(7383);
/// Set by a command a person is watching (`airband setup`, `airband model test`): wait through the first
/// download. The tower itself never waits on it: a message that arrives meanwhile is given as written.
static PATIENT: AtomicBool = AtomicBool::new(false);
/// The server this run started, and the model it has.
static RUNNING: Mutex<Option<(String, Child)>> = Mutex::new(None);

pub(crate) fn listen_after(tower_port: u16) {
    PORT.store(tower_port.saturating_add(10), Ordering::SeqCst);
}
pub(crate) fn wait_for_it(patient: bool) {
    PATIENT.store(patient, Ordering::SeqCst);
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
    format!("http://127.0.0.1:{}", PORT.load(Ordering::SeqCst))
}

/// Fetch the server program, if it is not here yet. Blocks while it downloads (about 17 MB).
fn fetch(server: &airband_core::catalog::Server) -> Result<PathBuf, String> {
    let program = program(server);
    if program.is_file() {
        return Ok(program);
    }
    let archive = server.archives.iter().find(|a| a.os == std::env::consts::OS && a.arch == std::env::consts::ARCH).ok_or("A model cannot be run on this kind of computer yet. Choose another way to read.")?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let url = format!("{}/{}", server.base, archive.file);
    log(format!("local model: fetching the program that runs it ({})", archive.file));
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
    let mut res = agent.get(&url).call().map_err(|e| format!("the program that runs the model could not be fetched: {e}"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut res.body_mut().as_reader(), &mut bytes).map_err(|e| format!("the download stopped part way: {e}"))?;
    let got: String = <sha2::Sha256 as sha2::Digest>::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != archive.sha256 {
        return Err("what came down was not the program that runs the model; nothing was kept".into());
    }
    let packed = dir().join(&archive.file);
    std::fs::write(&packed, &bytes).map_err(|e| e.to_string())?;
    let unpacked = Command::new("tar").arg("-xzf").arg(&packed).arg("-C").arg(dir()).status().map_err(|e| format!("could not unpack it: {e}"))?;
    let _ = std::fs::remove_file(&packed);
    if !unpacked.success() || !program.is_file() {
        return Err("the program that runs the model could not be unpacked".into());
    }
    Ok(program)
}

fn healthy() -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).http_status_as_error(false).build().into();
    agent.get(format!("{}/health", address())).call().is_ok_and(|r| r.status().as_u16() == 200)
}

/// A server left by a tower that was stopped: only the one this tower recorded starting (its process and
/// the program it was, on two lines), and only if that process is still that program.
fn end_the_last_one() {
    let note = dir().join("server.pid");
    let noted = std::fs::read_to_string(&note).unwrap_or_default();
    let mut lines = noted.lines();
    if let (Some(pid), Some(was)) = (lines.next().and_then(|p| p.trim().parse::<u32>().ok()), lines.next().filter(|p| !p.is_empty())) {
        let is_ours = Command::new("ps").args(["-p", &pid.to_string(), "-o", "command="]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(was));
        if is_ours {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(600));
        }
    }
    let _ = std::fs::remove_file(note);
}

/// Ask the server's keeper to end it, and wait until it has: asked, not killed, so the keeper can pass it on.
fn end(keeper: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(keeper.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = keeper.kill();
    let _ = keeper.wait();
}

/// Stop the server, when something else reads from now on.
pub(crate) fn stop() {
    if let Some((_, mut child)) = RUNNING.lock().unwrap().take() {
        end(&mut child);
    }
    let _ = std::fs::remove_file(dir().join("server.pid"));
}

/// The address to speak to `model` at, starting its server if it is not running. The first time, the
/// program and then the model are fetched, which takes minutes: a patient caller waits through that, and any
/// other is told it is not ready yet. Blocks.
pub(crate) fn ensure(model: &str, server: &str) -> Result<String, String> {
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
            let child = Command::new("sh")
                .args(["-c", KEEPER, "sh"])
                .arg(&program)
                .env("AIRBAND_TOWER_PID", std::process::id().to_string())
                .args(["-hf", model, "--no-mmproj", "--reasoning", "off", "--chat-template-kwargs", "{\"enable_thinking\":false}", "--jinja", "-c", "24576", "-np", "3", "--host", "127.0.0.1", "--port", &PORT.load(Ordering::SeqCst).to_string()])
                .env("LLAMA_CACHE", dir().join("models"))
                .stdin(Stdio::null())
                .stdout(out.try_clone().map_err(|e| e.to_string())?)
                .stderr(out)
                .spawn()
                .map_err(|e| format!("the program that runs the model did not start: {e}"))?;
            let _ = std::fs::write(dir().join("server.pid"), format!("{}\n{}\n", child.id(), program.display()));
            *running = Some((model.to_string(), child));
        }
    }
    let until = Instant::now() + if PATIENT.load(Ordering::SeqCst) { Duration::from_secs(45 * 60) } else { Duration::from_secs(2) };
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
