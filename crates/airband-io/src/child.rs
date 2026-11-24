//! Every program an airband process starts: agents it holds, the tools that summarise, deliver and read, a
//! model's server, the system's own commands.
//!
//! A started program is given its standard input, output and error and nothing else the process has open.
//! On macOS a socket is made in two steps (made, then marked as not for programs started later), and a program
//! another thread starts between the two is handed it: it then holds that socket, and the port with it, for as
//! long as it runs, after the tower has let it go. So the program is not trusted to be handed only what was
//! marked: between the moment it is made a process of its own and the moment it becomes the program, every
//! descriptor past the three standard ones is marked to close, whatever its state in the parent was. Linux makes
//! each descriptor marked from the start (std and tokio ask for that in the same call), so nothing is left to do
//! there. Windows hands a program only the handles made to be inherited, and std makes none.
//!
//! [`command`] is the one way a program is started: `std::process::Command::new` and its tokio twin are refused by
//! the linter everywhere else, and a tokio command is made from this one (`tokio::process::Command::from`).
#![expect(clippy::disallowed_methods, reason = "the one place a program is started")]
use std::ffi::OsStr;
use std::process::Command;

/// A command for `program` that hands it only its standard input, output and error.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut, reason = "marked only on the systems that need it")]
    let mut command = Command::new(program);
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::process::CommandExt;
        let past = descriptors_end();
        // SAFETY: the closure runs in the new process before it becomes the program, and calls only fcntl, which
        // is safe to call there: it takes no lock and makes nothing.
        unsafe {
            command.pre_exec(move || {
                only_the_standard_three(past);
                Ok(())
            });
        }
    }
    command
}

/// One past the highest descriptor this process can have open: the system gives each new one the lowest number
/// free below its limit on open files, and nothing in airband lowers that limit once descriptors are open.
#[cfg(all(unix, not(target_os = "linux")))]
fn descriptors_end() -> i32 {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit writes only the struct it is given
    let asked = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    assert_eq!(asked, 0, "the limit on open files is always there to read");
    // and never past the system's own limit for one process, which an unlimited soft limit stands for
    let most = per_process_most();
    i32::try_from(limit.rlim_cur).map_or(most, |soft| soft.min(most))
}

/// The most files one process may have open, whatever its own limit says.
#[cfg(target_os = "macos")]
fn per_process_most() -> i32 {
    let mut most: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name ends in a nul, and the value is written into an int of the size it is told
    let asked = unsafe { libc::sysctlbyname(c"kern.maxfilesperproc".as_ptr(), (&raw mut most).cast(), &mut size, std::ptr::null_mut(), 0) };
    assert_eq!(asked, 0, "the system always says how many files a process may have open");
    most
}

#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
fn per_process_most() -> i32 {
    i32::MAX
}

/// In the new process, before it becomes the program: every descriptor past the standard three is closed as it
/// does, whether or not the parent marked it.
#[cfg(all(unix, not(target_os = "linux")))]
fn only_the_standard_three(past: i32) {
    for fd in 3..past {
        // SAFETY: fcntl on a number that is no descriptor answers an error and changes nothing
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Run by the test below as the program it starts: says how many sockets it was handed.
    const REPORT: &str = "AIRBAND_TEST_REPORT_SOCKETS";

    #[test]
    fn a_program_started_while_sockets_come_and_go_is_handed_none_of_them() {
        if std::env::var_os(REPORT).is_some() {
            return report_sockets();
        }
        // threads that open and close listeners, and pairs of connected sockets, as fast as they can: each is a socket
        // made at a moment a program may be starting
        let stop = Arc::new(AtomicBool::new(false));
        let churn: Vec<_> = (0..4)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut made = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        // the system runs short of room for sockets made this fast now and then: one it could
                        // not make is simply not counted
                        made += u64::from(std::net::TcpListener::bind("127.0.0.1:0").is_ok());
                        made += u64::from(std::os::unix::net::UnixStream::pair().is_ok());
                    }
                    made
                })
            })
            .collect();
        let name = format!("{}::a_program_started_while_sockets_come_and_go_is_handed_none_of_them", module_path!().split_once("::").unwrap().1);
        let mut handed = Vec::new();
        const STARTS: usize = 200;
        for _ in 0..STARTS {
            let out = command(std::env::current_exe().unwrap()).args(["--exact", &name, "--test-threads=1", "--nocapture", "-q"]).env(REPORT, "1").output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            let said = String::from_utf8_lossy(&out.stdout);
            let line = said.lines().find_map(|l| l.strip_prefix("sockets=")).expect("the program said how many sockets it has");
            handed.push(line.parse::<usize>().unwrap());
        }
        stop.store(true, Ordering::Relaxed);
        let made: u64 = churn.into_iter().map(|t| t.join().unwrap()).sum();
        // the sockets really were coming and going all the while
        assert!(made > STARTS as u64 * 10, "only {made} sockets were made");
        assert_eq!(handed.iter().filter(|n| **n > 0).count(), 0, "{} of {STARTS} programs were handed sockets ({made} made meanwhile)", handed.iter().filter(|n| **n > 0).count());
    }

    /// The sockets this process has open, as one line on its output.
    fn report_sockets() {
        let sockets = (0..4096)
            .filter(|fd| {
                let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                // SAFETY: fstat writes only the struct it is given, and answers an error for no descriptor
                unsafe { libc::fstat(*fd, &mut stat) == 0 && stat.st_mode & libc::S_IFMT == libc::S_IFSOCK }
            })
            .count();
        println!("sockets={sockets}");
    }
}
