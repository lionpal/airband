//! `airband hold`: keeps one agent's program running and its input open, apart from the tower.
//!
//! The tower starts one of these for each agent it runs itself, and then only ever talks to it through a
//! socket. So the agent does not end when the tower does: a tower that is updated or restarted finds the socket
//! again and carries on, and whatever the agent was in the middle of goes on undisturbed.
//!
//!   airband hold --sock <path> --cwd <folder> -- <program> [arguments...]
//!
//! Whatever is written to the socket is written to the program's standard input. One message is special: a
//! zero byte followed by `end` closes the program's input, which is how it is asked to finish. When the program
//! ends, so does this, and the socket goes with it. What the program prints is read and dropped.
#![cfg(unix)]
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// What closes the program's input when written to the socket.
pub const END: &[u8] = b"\0end\n";

pub fn run(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let (mut sock, mut cwd, mut argv) = (None, None, Vec::new());
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sock" => sock = it.next().cloned(),
            "--cwd" => cwd = it.next().cloned(),
            "--" => argv = it.by_ref().cloned().collect(),
            other => return Err(format!("airband hold does not take \"{other}\"")),
        }
    }
    let sock = sock.ok_or("airband hold needs --sock")?;
    let (program, rest) = argv.split_first().ok_or("airband hold needs a program after --")?;
    // a terminal closing, or the tower that started this going away, must not take the agent with it
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).map_err(|e| format!("cannot open {sock}: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));
    }
    let mut command = Command::new(program);
    command.args(rest).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(cwd) = &cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|e| {
        let _ = std::fs::remove_file(&sock);
        format!("{program} did not start: {e}")
    })?;
    let input = Arc::new(Mutex::new(child.stdin.take()));
    if let Some(mut output) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut sink = [0u8; 16 * 1024];
            while matches!(output.read(&mut sink), Ok(n) if n > 0) {}
        });
    }
    {
        let input = input.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                let mut said = Vec::new();
                if conn.read_to_end(&mut said).is_err() || said.is_empty() {
                    continue;
                }
                let mut input = input.lock().unwrap();
                if said == END {
                    *input = None;
                } else if let Some(to) = input.as_mut() {
                    let _ = to.write_all(&said).and_then(|()| to.flush());
                }
            }
        });
    }
    let _ = child.wait();
    let _ = std::fs::remove_file(&sock);
    Ok(())
}
