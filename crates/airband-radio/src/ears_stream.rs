//! Listening while he talks: one WebSocket to a live service, opened as the microphone opens so it is ready by
//! his first word, fed as he speaks, with the words coming back while he is still talking.
//!
//! Everything here is the same for every live service: the microphone, when he has finished, what is shown
//! as he talks, carrying on after the service ends a turn. How one service is spoken to is a [`Wire`], and
//! each has a file of its own under `ears/`.
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use airband_core::radio::EarsGrant;
use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

pub(crate) type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// The same for any moment, in milliseconds since 1970.
pub(crate) fn stamp_at(ms: u64) -> String {
    let (days, rest) = ((ms / 86_400_000) as i64, ms % 86_400_000);
    // the date for a count of days since 1970
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000)
}

/// A WebSocket to `host`, over the same TLS as every other request this library makes, with the same roots.
/// Reads and writes give up after six seconds until the caller says otherwise.
pub(crate) fn dial(host: &str, req: tungstenite::handshake::client::Request) -> Result<Socket, ()> {
    let address = (host, 443).to_socket_addrs().map_err(|_| ())?.next().ok_or(())?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(6)).map_err(|_| ())?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(6)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(6)));
    // the same TLS as every other request this library makes, with the same roots
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|_| ())?.with_root_certificates(roots).with_no_client_auth();
    let (socket, _) = tungstenite::client_tls_with_config(req, tcp, None, Some(Connector::Rustls(Arc::new(tls)))).map_err(|_| ())?;
    Ok(socket)
}

/// From here on a read waits only a moment, so audio keeps going out while answers are looked for.
pub(crate) fn read_briefly(socket: &Socket) {
    if let MaybeTlsStream::Rustls(stream) = socket.get_ref() {
        let _ = stream.get_ref().set_read_timeout(Some(Duration::from_millis(12)));
    }
}

/// How one live service is spoken to. Everything else about listening live (the microphone, when he has
/// finished, what is shown as he talks, carrying on after the service ends a turn) is the same for all.
pub(crate) struct Wire {
    /// Open the connection and say what is coming.
    pub connect: fn(&EarsGrant, &str) -> Result<Socket, ()>,
    /// One piece of the microphone's sound.
    pub audio: fn(&str, &[u8]) -> Message,
    /// No more sound is coming: say what you have.
    pub end: fn(&str) -> Message,
    /// What one of its messages says: the unfinished stretch if it changed. Finished stretches go into the
    /// list; the flag is set when it has ended its turn.
    pub read: fn(&Message, &mut Vec<String>, &mut bool) -> Option<String>,
    /// A recording's header goes first, before any sound.
    pub header: bool,
    /// When it ends a turn while he still has the microphone open, a new connection is needed to go on.
    pub reopens: bool,
}

/// Whatever the service has said since last asked, without waiting for more. `kept` gathers finished stretches;
/// returns the unfinished one, if it changed. Err when the connection has gone.
fn drain(wire: &Wire, socket: &mut Socket, kept: &mut Vec<String>, ended: &mut bool) -> Result<Option<String>, ()> {
    let mut hypothesis = None;
    loop {
        match socket.read() {
            Ok(m) if m.is_close() => return Err(()),
            Ok(m) if m.is_text() || m.is_binary() => {
                if let Some(now) = (wire.read)(&m, kept, ended) {
                    hypothesis = Some(now);
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(hypothesis),
            Err(_) => return Err(()),
        }
    }
}

/// Listen over the streaming connection, telling `hearing` what he has said so far as it changes. Without
/// `manual` it is one reply: it ends when he stops talking. With it he opened the microphone himself: all of it
/// is sent, pauses and all, until the host closes the microphone, and if the service ends its turn before then
/// (a long silence) a new one is opened and what he said so far is kept.
/// Ok(None) is silence. Err is a failure of the service or the line before anything was heard: nothing was lost
/// that the caller cannot get by listening again another way.
pub(crate) fn live(wire: &'static Wire, pass: Arc<dyn Fn() -> EarsGrant + Send + Sync>, host: &dyn RadioHost, rate: u32, header: Vec<u8>, manual: bool) -> Result<Option<String>, ()> {
    // what goes first on every connection: a recording's header, for a service that wants one
    let header: Vec<Vec<u8>> = if wire.header { vec![header] } else { Vec::new() };
    let lead = header.len();
    // The pass is asked for and the connection opened on a thread of their own, while the microphone is already
    // being read here: he does not wait for either before he can talk.
    let open = |request: &str| -> Receiver<Result<Socket, ()>> {
        let (tx, rx) = channel();
        let (pass, request) = (pass.clone(), request.to_string());
        thread::spawn(move || {
            let grant = pass();
            let _ = tx.send((wire.connect)(&grant, &request));
        });
        rx
    };
    let mut request = random_hex(16);
    let mut opening = open(&request);
    let mut socket: Option<Socket> = None;
    let mut end = Endpointer::new(rate);
    // audio from just before he was judged to be speaking, and whatever is said before the connection is up
    let mut waiting: VecDeque<Vec<u8>> = VecDeque::from(header.clone());
    let (mut kept, mut ended, mut shown) = (Vec::<String>::new(), false, String::new());
    let tell = |kept: &[String], now: &str, shown: &mut String| {
        let all = [kept.join(" "), now.to_string()].join(" ").trim().to_string();
        if all != *shown {
            *shown = all.clone();
            host.hearing(all);
        }
    };
    // a microphone he left open is closed for him after this long: what he said is kept, and nothing is sent
    let began = Instant::now();
    const LEFT_OPEN: Duration = Duration::from_secs(240);
    loop {
        let bytes = host.record_read();
        if bytes.is_empty() {
            break; // the host stopped the microphone
        }
        let mut verdict = Heard::Waiting;
        if !manual {
            for frame in samples(&bytes).chunks(320) {
                verdict = end.feed(frame);
                if matches!(verdict, Heard::Finished | Heard::Silence) {
                    break;
                }
            }
        }
        let sending = manual || end.speaking;
        waiting.push_back(bytes);
        // with no connection for a long while, the oldest sound goes: two minutes are kept
        while waiting.len() > 1200 {
            waiting.remove(lead);
        }
        if !sending {
            // not speaking yet: keep only the header and the last half second
            while waiting.len() > 5 + lead {
                waiting.remove(lead);
            }
        }
        if socket.is_none() {
            match opening.try_recv() {
                Ok(Ok(s)) => socket = Some(s),
                // nothing heard yet: the caller can listen another way. Something heard: keep it, and try again
                Ok(Err(())) if kept.is_empty() => return Err(()),
                Ok(Err(())) => opening = open(&request),
                Err(_) => {}
            }
        }
        if let (Some(s), true) = (socket.as_mut(), sending) {
            let sent = waiting.drain(..).try_for_each(|chunk| s.send((wire.audio)(&request, &chunk)).map_err(|_| ()));
            let heard = sent.and_then(|()| drain(wire, s, &mut kept, &mut ended));
            match heard {
                Ok(Some(now)) => tell(&kept, &now, &mut shown),
                Ok(None) => {}
                Err(()) if manual && !kept.is_empty() => ended = true,
                Err(()) => return Err(()),
            }
        }
        if manual && ended {
            // the service closed its turn and he has not closed the microphone: carry on, in a new connection
            // if this service needs one
            ended = false;
            if wire.reopens {
                socket = None;
                request = random_hex(16);
                opening = open(&request);
                waiting = VecDeque::from(header.clone());
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return Ok(None),
            _ => {}
        }
        if manual && began.elapsed() > LEFT_OPEN {
            break;
        }
    }
    if !manual && !end.speaking {
        return Ok(None);
    }
    host.status(RadioStatus::Sending);
    // he has stopped: whatever was not sent yet goes now, then the end of the audio, then the last words come back
    let last = || -> Result<(), ()> {
        let mut s = match socket {
            Some(s) => s,
            None => opening.recv_timeout(Duration::from_secs(12)).map_err(|_| ())??,
        };
        for chunk in waiting.drain(..) {
            s.send((wire.audio)(&request, &chunk)).map_err(|_| ())?;
        }
        // a turn it ended while he was still talking is not the end that is waited for now
        ended = false;
        s.send((wire.end)(&request)).map_err(|_| ())?;
        let until = Instant::now() + Duration::from_secs(5);
        while !ended && Instant::now() < until {
            if let Some(now) = drain(wire, &mut s, &mut kept, &mut ended)? {
                tell(&kept, &now, &mut shown);
            }
        }
        let _ = s.close(None);
        Ok(())
    };
    // with something already heard, a failure at the very end loses only the last words, not all of them
    if last().is_err() && kept.is_empty() {
        return Err(());
    }
    let said = kept.join(" ");
    Ok((!said.is_empty()).then_some(said))
}

/// A recording played in place of the microphone, at the speed it was spoken, for trying a listening service
/// without one: it hears what a phone would send and notes what comes back and when.
pub(crate) struct Tape {
    left: Mutex<VecDeque<Vec<u8>>>,
    /// What the service said he had said so far, each time that changed, with the milliseconds since the start.
    pub heard: Mutex<Vec<(u128, String)>>,
    pub began: Instant,
    print: bool,
}

impl Tape {
    /// `pcm` is 16 kHz 16-bit mono. A second and a half of quiet follows it, as after someone stops talking.
    pub fn of(pcm: &[u8], print: bool) -> Tape {
        let mut parts: VecDeque<Vec<u8>> = pcm.chunks(3200).map(|c| c.to_vec()).collect();
        parts.extend(std::iter::repeat_n(vec![0u8; 3200], 15));
        Tape { left: Mutex::new(parts), heard: Mutex::new(Vec::new()), began: Instant::now(), print }
    }
}

impl RadioHost for Tape {
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn status(&self, _: RadioStatus) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _: u32) -> bool {
        true
    }
    fn record_read(&self) -> Vec<u8> {
        thread::sleep(Duration::from_millis(100));
        self.left.lock().unwrap().pop_front().unwrap_or_default()
    }
    fn record_stop(&self) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn own_voice(&self) -> bool {
        false
    }
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
    fn transcribe(&self, _: String, _: Vec<u8>) -> Option<String> {
        None
    }
    fn hearing(&self, text: String) {
        let at = self.began.elapsed().as_millis();
        if self.print {
            println!("{at:>5} ms  {text}");
        }
        self.heard.lock().unwrap().push((at, text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the real service, with a recording in place of the microphone:
    ///   AZURE_TOKEN=… AZURE_REGION=… SPEECH_WAV=said.wav cargo test -p airband-radio -- --ignored --nocapture streams
    ///   LISTEN_WITH=gemini GEMINI_KEY=… SPEECH_WAV=said.wav cargo test -p airband-radio -- --ignored --nocapture streams
    #[test]
    #[ignore]
    fn streams_to_the_real_service() {
        let wav = std::fs::read(std::env::var("SPEECH_WAV").unwrap()).unwrap();
        let pcm = &wav[44..];
        let tape = Tape::of(pcm, true);
        // LISTEN_WITH names the dialect (azure when left out); the key is turned into a pass the way a tower does it
        let kind = std::env::var("LISTEN_WITH").unwrap_or("azure".into());
        let l = crate::ears::listener(&kind).unwrap();
        let (token, set): (String, std::collections::BTreeMap<String, String>) = match kind.as_str() {
            "gemini" => (crate::ears::pass_for("gemini", &Default::default(), &std::env::var("GEMINI_KEY").unwrap()).unwrap(), [("model".to_string(), std::env::var("GEMINI_MODEL").unwrap_or("gemini-3.5-transcribe-live".into())), ("language".to_string(), "en-US".to_string())].into()),
            "elevenlabs" => (crate::ears::pass_for("elevenlabs", &Default::default(), &std::env::var("ELEVENLABS_KEY").unwrap()).unwrap(), [("model".to_string(), "scribe_v3_live".to_string()), ("language".to_string(), "en".to_string())].into()),
            _ => (std::env::var("AZURE_TOKEN").unwrap(), [("region".to_string(), std::env::var("AZURE_REGION").unwrap()), ("language".to_string(), "en-US".to_string())].into()),
        };
        // WITHOUT_NAMES leaves out the names he is likely to say
        let phrases = if std::env::var("WITHOUT_NAMES").is_ok() { Vec::new() } else { vec!["airband".into(), "shop two".into()] };
        let grant = EarsGrant { kind, token, set, phrases };
        let said = live(l.wire, Arc::new(move || grant.clone()), &tape, 16000, crate::ears::wav_header(), std::env::var("MANUAL").is_ok())
        .unwrap();
        println!("{:>5} ms  FINAL: {said:?} (the recording is {} ms long)", tape.began.elapsed().as_millis(), pcm.len() / 32);
        assert!(said.is_some_and(|s| !s.is_empty()));
        assert!(tape.heard.lock().unwrap().len() >= 2, "words should arrive while he is still speaking");
    }
}
