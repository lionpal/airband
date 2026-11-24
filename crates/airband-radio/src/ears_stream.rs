//! Listening while he talks: one WebSocket to a live service, opened as the microphone opens so it is ready by
//! his first word, fed as he speaks, with the words coming back while he is still talking.
//!
//! Everything here is the same for every live service: the microphone, when he has finished, what is shown
//! as he talks, carrying on after the service ends a turn. How one service is spoken to is a [`Wire`], and
//! each has a file of its own under `ears/`.
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use crate::ears::Pass;
use crate::takes::Unheard;
use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};
#[cfg(feature = "stand-ins")]
use tungstenite::client::IntoClientRequest;

pub(crate) type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// A WebSocket to `host`, over the same TLS as every other request this library makes, with the same roots.
/// Reads and writes give up after six seconds until the caller says otherwise.
pub(crate) fn dial(host: &str, req: tungstenite::handshake::client::Request) -> Result<Socket, ()> {
    // a build for the end-to-end suites reaches a stand-in on this machine in the service's place
    #[cfg(feature = "stand-ins")]
    if let Some(to) = airband_io::http::stand_in(&req.uri().to_string()) {
        let mut there = to.as_str().into_client_request().map_err(|_| ())?;
        *there.headers_mut() = req.headers().clone();
        let at = there.uri().authority().map(|a| a.to_string()).ok_or(())?;
        there.headers_mut().insert("host", at.parse().map_err(|_| ())?);
        let tcp = TcpStream::connect(at.as_str()).map_err(|_| ())?;
        tcp.set_read_timeout(Some(Duration::from_secs(6))).ok();
        let (socket, _) = tungstenite::client(there, MaybeTlsStream::Plain(tcp)).map_err(|_| ())?;
        return Ok(socket);
    }
    let address = (host, 443).to_socket_addrs().map_err(|_| ())?.next().ok_or(())?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(6)).map_err(|_| ())?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(6))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(6))).ok();
    // the same TLS as every other request this library makes, with the same roots
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|_| ())?.with_root_certificates(roots).with_no_client_auth();
    let (socket, _) = tungstenite::client_tls_with_config(req, tcp, None, Some(Connector::Rustls(Arc::new(tls)))).map_err(|_| ())?;
    Ok(socket)
}

/// From here on a read waits only a moment, so audio keeps going out while answers are looked for.
pub(crate) fn read_briefly(socket: &Socket) {
    match socket.get_ref() {
        MaybeTlsStream::Rustls(stream) => {
            stream.get_ref().set_read_timeout(Some(Duration::from_millis(12))).ok();
        }
        MaybeTlsStream::Plain(tcp) => {
            tcp.set_read_timeout(Some(Duration::from_millis(12))).ok();
        }
        _ => {}
    }
}

/// How one live service is spoken to. Everything else about listening live (the microphone, when he has
/// finished, what is shown as he talks, carrying on after the service ends a turn) is the same for all.
pub(crate) struct Wire {
    /// Open the connection and say what is coming.
    pub connect: fn(&Pass, &str) -> Result<Socket, ()>,
    /// One piece of the microphone's sound.
    pub audio: fn(&str, &[u8]) -> Message,
    /// No more sound is coming: say what you have.
    pub end: fn(&str) -> Message,
    /// What one of its messages says: the unfinished stretch if it changed, given the one so far (a service
    /// that sends only what is new adds it to that). Finished stretches go into the list; the flag is set when
    /// it has ended its turn.
    pub read: fn(&Message, &mut Vec<String>, &mut bool, &str) -> Option<String>,
    /// A recording's header goes first, before any sound.
    pub header: bool,
    /// When it ends a turn while he still has the microphone open, a new connection is needed to go on.
    pub reopens: bool,
}

/// Whatever the service has said since last asked, without waiting for more. `kept` gathers finished stretches;
/// returns the unfinished one, if it changed from `so_far`. Err when the connection has gone.
fn drain(wire: &Wire, socket: &mut Socket, kept: &mut Vec<String>, ended: &mut bool, so_far: &str) -> Result<Option<String>, ()> {
    let mut hypothesis: Option<String> = None;
    loop {
        match socket.read() {
            Ok(m) if m.is_close() => return Err(()),
            Ok(m) if m.is_text() || m.is_binary() => {
                if let Some(now) = (wire.read)(&m, kept, ended, hypothesis.as_deref().unwrap_or(so_far)) {
                    hypothesis = Some(now);
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(hypothesis),
            Err(_) => return Err(()),
        }
    }
}

/// Asks for a pass for the service: Err says why there is none, in the service's words for him.
pub(crate) type PassFor = Arc<dyn Fn() -> Result<Pass, String> + Send + Sync>;

/// Listen over the streaming connection, telling `hearing` what he has said so far as it changes. Without
/// `manual` it is one reply: it ends when he stops talking. With it he opened the microphone himself: all of it
/// is sent, pauses and all, until the host closes the microphone, and if the service ends its turn before then
/// (a long silence) a new one is opened and what he said so far is kept.
/// Ok(None) is silence. Err is why it was not all heard: the service or the line failed, at the start or
/// partway, and could not be had again. What was heard before a failure is not taken for all he said: the
/// recording of it is whole ([`crate::takes`]), and is heard again whole. Nothing else listens in its place.
pub(crate) fn live(wire: &'static Wire, pass: PassFor, host: &dyn RadioHost, rate: u32, header: Vec<u8>, manual: bool) -> Result<Option<String>, Unheard> {
    // what goes first on every connection: a recording's header, for a service that wants one
    let header: Vec<Vec<u8>> = if wire.header { vec![header] } else { Vec::new() };
    let request = random_hex(16);
    // The pass is asked for and the connection opened on a thread of their own, while the microphone is already
    // being read here: he does not wait for either before he can talk.
    let opening = connect(wire, &pass, &request);
    let mut stream = Stream { wire, pass, host, waiting: VecDeque::from(header.clone()), header_unsent: header.len(), header, request, opening, socket: None, kept: Vec::new(), unfinished: String::new(), ended: false, shown: String::new(), sent: 0, rate };
    listen(&mut stream, wire, host, rate, manual)
}

/// The listening itself, on a stream made ready.
fn listen(stream: &mut Stream, wire: &'static Wire, host: &dyn RadioHost, rate: u32, manual: bool) -> Result<Option<String>, Unheard> {
    let mut end = Endpointer::new(rate);
    // a microphone he left open is closed for him once the take is as long as one is ([`crate::takes::Tape`]):
    // what he said is kept and goes as said
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
        stream.keep(bytes, sending);
        if !stream.connected()? {
            // it went partway and could not be had again: the rest of what he says is not heard
            return Err(Unheard::Dropped);
        }
        if sending {
            stream.send()?;
        }
        if manual && stream.ended {
            // the service closed its turn and he has not closed the microphone: carry on, in a new connection
            // if this service needs one
            stream.ended = false;
            if wire.reopens {
                stream.reconnect();
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return Ok(None),
            _ => {}
        }
    }
    if !manual && !end.speaking {
        return Ok(None);
    }
    host.status(RadioStatus::Sending);
    stream.finish()
}

/// A connection, or why there is none: the service's reason when it would not give a pass.
type Opened = Result<Socket, Option<String>>;

/// The pass asked for and the connection opened, on a thread of their own.
fn connect(wire: &'static Wire, pass: &PassFor, request: &str) -> Receiver<Opened> {
    let (tx, rx) = channel();
    let (pass, request) = (pass.clone(), request.to_string());
    thread::spawn(move || {
        // with no pass to be had there is nothing to connect with, and the service's reason is kept
        tx.send(pass().map_err(Some).and_then(|grant| (wire.connect)(&grant, &request).map_err(|()| None))).ok();
    });
    rx
}

/// One listening over the streaming connection: the connection or its opening, what waits to be sent, and what
/// has been heard so far.
struct Stream<'a> {
    wire: &'static Wire,
    pass: PassFor,
    host: &'a dyn RadioHost,
    /// What goes first on every connection.
    header: Vec<Vec<u8>>,
    /// How many of the pieces at the front of `waiting` are the header, not yet sent on this connection.
    header_unsent: usize,
    request: String,
    opening: Receiver<Opened>,
    socket: Option<Socket>,
    /// Audio from just before he was judged to be speaking, and whatever is said before the connection is up.
    waiting: VecDeque<Vec<u8>>,
    kept: Vec<String>,
    /// The stretch it has not finished, as it last said.
    unfinished: String,
    /// The service ended its turn.
    ended: bool,
    shown: String,
    /// Bytes of sound sent on this connection: how long the service may take to finish all of it is bounded
    /// by how long it is.
    sent: u64,
    rate: u32,
}

impl Stream<'_> {
    /// A new connection, under a new request, its header first, then the sound the last one was never sent.
    fn reconnect(&mut self) {
        self.socket = None;
        self.request = random_hex(16);
        self.opening = connect(self.wire, &self.pass, &self.request);
        let unsent = std::mem::take(&mut self.waiting).into_iter().skip(self.header_unsent);
        self.waiting = self.header.iter().cloned().chain(unsent).collect();
        self.header_unsent = self.header.len();
        self.sent = 0;
    }

    /// Send what waits, one piece at a time: a piece leaves the list only once it has gone, so what a failed
    /// send did not get out is still there for the next connection.
    fn send_waiting(&mut self) -> Result<(), ()> {
        let Some(s) = self.socket.as_mut() else { return Ok(()) };
        while let Some(chunk) = self.waiting.front() {
            s.send((self.wire.audio)(&self.request, chunk)).map_err(|_| ())?;
            if self.header_unsent == 0 {
                self.sent += chunk.len() as u64;
            }
            self.waiting.pop_front();
            self.header_unsent = self.header_unsent.saturating_sub(1);
        }
        Ok(())
    }

    /// Sound read from the microphone, kept until it can be sent.
    fn keep(&mut self, bytes: Vec<u8>, sending: bool) {
        let lead = self.header_unsent;
        self.waiting.push_back(bytes);
        // with no connection for a long while, the oldest sound goes: two minutes are kept
        while self.waiting.len() > 1200 {
            self.waiting.remove(lead);
        }
        if !sending {
            // not speaking yet: keep only the header and the last half second
            while self.waiting.len() > 5 + lead {
                self.waiting.remove(lead);
            }
        }
    }

    /// The connection, once it is up: false when it could not be made again after one that went partway. Err
    /// when it could not be made with nothing heard yet. It is not dialled again at once: each try can cost a
    /// new pass, and one that just failed is likely to fail again. A connection that was up and went is made
    /// again ([`Stream::reconnect`]).
    fn connected(&mut self) -> Result<bool, Unheard> {
        if self.socket.is_none() {
            match self.opening.try_recv() {
                Ok(Ok(s)) => self.socket = Some(s),
                Ok(Err(why)) if self.kept.is_empty() => return Err(not_opened(why)),
                Ok(Err(_)) => return Ok(false),
                Err(_) => {}
            }
        }
        Ok(true)
    }

    /// What waits goes, and what has been heard comes back.
    fn send(&mut self) -> Result<(), Unheard> {
        if self.socket.is_none() {
            return Ok(());
        }
        let sent = self.send_waiting();
        let Some(s) = self.socket.as_mut() else { return Ok(()) };
        match sent.and_then(|()| drain(self.wire, s, &mut self.kept, &mut self.ended, &self.unfinished)) {
            Ok(Some(now)) => self.tell(&now),
            Ok(None) => {}
            // it ended its turn and closed: everything it heard is final, and a new connection takes what he
            // says next
            Err(()) if self.ended => {
                self.ended = false;
                self.reconnect();
            }
            // the connection went partway: what it had not finished is never taken for what he said. The
            // recording is whole, and is heard again whole
            Err(()) => return Err(Unheard::Dropped),
        }
        Ok(())
    }

    /// Tell the host what he has said so far, when it has changed: for showing only. What is sent is only ever
    /// what the service finished.
    fn tell(&mut self, now: &str) {
        self.unfinished = now.to_string();
        let all = [self.kept.join(" "), now.to_string()].join(" ").trim().to_string();
        if all != self.shown {
            self.shown = all.clone();
            self.host.hearing(all);
        }
    }

    /// He is done (he closed the microphone, which is the fact of it, or a reply he was asked for ended):
    /// whatever was not sent yet goes now, then the end of the audio, and the service's final words for all of
    /// it are waited for.
    fn finish(&mut self) -> Result<Option<String>, Unheard> {
        let ended = self.last();
        self.settle(ended)
    }

    /// What he said, once the end of the audio went (`ended`) or failed: only what the service finished, once it
    /// has ended its turn on the whole recording. When it did not (it stopped answering, or the line went), the
    /// words it had not finished are never taken for what he said: that is a failure, and the recording is
    /// heard again whole.
    fn settle(&mut self, ended: Result<(), Unheard>) -> Result<Option<String>, Unheard> {
        match ended {
            // it ended its turn before the line went: every word came back
            Err(_) if self.ended => {}
            Err(why) => return Err(why),
            Ok(()) if !self.ended => return Err(Unheard::TimedOut),
            Ok(()) => {}
        }
        let said = self.kept.join(" ");
        Ok((!said.is_empty()).then_some(said))
    }

    fn last(&mut self) -> Result<(), Unheard> {
        let mut s = match self.socket.take() {
            Some(s) => s,
            None => match self.opening.recv_timeout(Duration::from_secs(12)) {
                Ok(Ok(s)) => s,
                Ok(Err(why)) => return Err(not_opened(why)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(Unheard::TimedOut),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(Unheard::Unreachable),
            },
        };
        let dropped = |_| Unheard::Dropped;
        for chunk in self.waiting.drain(..) {
            s.send((self.wire.audio)(&self.request, &chunk)).map_err(dropped)?;
            self.sent += chunk.len() as u64;
        }
        // a turn it ended while he was still talking is not the end that is waited for now
        self.ended = false;
        s.send((self.wire.end)(&self.request)).map_err(dropped)?;
        // The service's own end of its turn is what is waited for, or the line going. A service that says
        // neither is given at most as long as the sound it was sent, and never less than a few seconds, to
        // finish all of it.
        let until = Instant::now() + Duration::from_millis((self.sent * 1000 / (u64::from(self.rate) * 2)).max(5000));
        while !self.ended && Instant::now() < until {
            if let Some(now) = drain(self.wire, &mut s, &mut self.kept, &mut self.ended, &self.unfinished).map_err(|()| Unheard::Dropped)? {
                self.tell(&now);
            }
        }
        s.close(None).ok();
        Ok(())
    }
}

/// Why no connection was made: the service's reason, when it would give no pass; otherwise it was not reached.
fn not_opened(why: Option<String>) -> Unheard {
    match why {
        Some(why) => Unheard::Service { why },
        None => Unheard::Unreachable,
    }
}

impl Stream<'_> {
}

/// A recording played in place of the microphone, at the speed it was spoken, for the tests: it hears what a
/// phone would send and notes what comes back and when.
#[cfg(test)]
pub(crate) struct Tape {
    left: Mutex<VecDeque<Vec<u8>>>,
    /// What the service said he had said so far, each time that changed, with the milliseconds since the start.
    pub heard: Mutex<Vec<(u128, String)>>,
    pub began: Instant,
    print: bool,
}

#[cfg(test)]
impl Tape {
    /// `pcm` is 16 kHz 16-bit mono. A second and a half of quiet follows it, as after someone stops talking.
    pub(crate) fn of(pcm: &[u8], print: bool) -> Tape {
        let mut parts: VecDeque<Vec<u8>> = pcm.chunks(3200).map(|c| c.to_vec()).collect();
        parts.extend(std::iter::repeat_n(vec![0u8; 3200], 15));
        Tape { left: Mutex::new(parts), heard: Mutex::new(Vec::new()), began: Instant::now(), print }
    }
}

#[cfg(test)]
impl RadioHost for Tape {
    fn told(&self, _: String) {}
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
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
    fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
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

    /// A wire that never connects: what is kept is all there is.
    const NOWHERE: Wire = Wire { connect: |_, _| Err(()), audio: |_, _| Message::text(""), end: |_| Message::text(""), read: |_, _, _, _| None, header: false, reopens: false };

    #[test]
    fn only_the_final_words_for_the_whole_recording_are_what_he_said() {
        let tape = Tape::of(&[], false);
        let pass: PassFor = Arc::new(|| Err(String::new()));
        let mut stream = Stream { wire: &NOWHERE, pass, host: &tape, waiting: VecDeque::new(), header: Vec::new(), header_unsent: 0, request: String::new(), opening: channel().1, socket: None, kept: vec!["ship it".into()], unfinished: String::new(), ended: false, shown: String::new(), sent: 0, rate: 16000 };
        // words it had not finished are shown and never kept
        stream.tell("and deploy");
        assert_eq!(stream.kept, ["ship it"]);
        // the end went and it never finished its turn: nothing of it is what he said
        assert_eq!(stream.settle(Ok(())), Err(Unheard::TimedOut));
        // the end could not go: the line's failure, whatever was heard before it
        assert_eq!(stream.settle(Err(Unheard::Dropped)), Err(Unheard::Dropped));
        // with the connection gone at the end and not had again
        assert_eq!(stream.finish(), Err(Unheard::Unreachable));
        // it finished its turn on all of it: its final words are what he said, even if the line went after
        stream.kept = vec!["ship it and deploy".into()];
        stream.ended = true;
        assert_eq!(stream.settle(Ok(())), Ok(Some("ship it and deploy".to_string())));
        assert_eq!(stream.settle(Err(Unheard::Dropped)), Ok(Some("ship it and deploy".to_string())));
    }

    #[test]
    fn a_service_that_gives_no_pass_is_told_by_its_own_reason_on_this_listening_alone() {
        let tape = Tape::of(&[0u8; 32000], true);
        let refused = live(&NOWHERE, Arc::new(|| Err("Fixture service: that key is not valid.".to_string())), &tape, 16000, Vec::new(), true);
        assert_eq!(refused, Err(Unheard::Service { why: "Fixture service: that key is not valid.".into() }));
        // the next listening, whose pass was had and whose line failed, is not told the last one's reason
        let grant = Pass { dialect: String::new(), token: String::new(), set: Default::default(), phrases: Vec::new(), by: serde_json::from_value(serde_json::json!({ "interface": "x" })).unwrap(), until: None };
        let tape = Tape::of(&[0u8; 32000], true);
        let failed = live(&NOWHERE, Arc::new(move || Ok(grant.clone())), &tape, 16000, Vec::new(), true);
        assert_eq!(failed, Err(Unheard::Unreachable));
    }

    #[test]
    fn sound_a_connection_was_never_sent_goes_on_the_next_one_after_its_header() {
        let tape = Tape::of(&[], false);
        let pass: PassFor = Arc::new(|| Err(String::new()));
        let (h, a, b) = (vec![0u8], vec![1u8], vec![2u8]);
        let mut stream = Stream { wire: &NOWHERE, pass, host: &tape, waiting: VecDeque::from([h.clone(), a.clone(), b.clone()]), header: vec![h.clone()], header_unsent: 1, request: String::new(), opening: channel().1, socket: None, kept: Vec::new(), unfinished: String::new(), ended: false, shown: String::new(), sent: 0, rate: 16000 };
        // the header and the first piece went before the connection failed
        stream.waiting.pop_front();
        stream.header_unsent = 0;
        stream.waiting.pop_front();
        stream.reconnect();
        assert_eq!(Vec::from(stream.waiting.clone()), [h.clone(), b.clone()]);
        // failing again before the header went: it is not sent twice
        stream.reconnect();
        assert_eq!(Vec::from(stream.waiting.clone()), [h, b]);
        // and what is kept while not speaking is trimmed behind the header, not into it
        for _ in 0..10 {
            stream.keep(a.clone(), false);
        }
        assert_eq!(stream.waiting.len(), 6);
        assert_eq!(stream.waiting[0], [0u8]);
    }

    /// Against a real service, with a recording in place of the microphone. LISTEN_WITH is a provider's id in
    /// the catalogue, LISTEN_KEY its key, and LISTEN_SET any of its settings as `field=value,field=value`:
    ///   LISTEN_WITH=… LISTEN_KEY=… LISTEN_SET=region=… SPEECH_WAV=said.wav cargo test -p airband-radio -- --ignored --nocapture streams
    #[test]
    #[ignore]
    fn streams_to_the_real_service() {
        use airband_core::providers::{Job, Use};
        let wav = std::fs::read(std::env::var("SPEECH_WAV").unwrap()).unwrap();
        let pcm = &wav[44..];
        let tape = Tape::of(pcm, true);
        let provider = airband_core::providers::provider(&std::env::var("LISTEN_WITH").unwrap()).expect("a provider in the catalogue");
        let offer = provider.offers.iter().find(|o| o.job == Job::Ears && crate::ears::live(o).is_some()).expect("an offer of its that listens live");
        let mut with = Use::of(&provider.id, &offer.id);
        with.key = std::env::var("LISTEN_KEY").unwrap();
        for (field, value) in std::env::var("LISTEN_SET").unwrap_or_default().split(',').filter_map(|s| s.split_once('=')) {
            with.set.insert(field.into(), serde_json::json!(value));
        }
        let l = crate::ears::live(offer).unwrap();
        // the key is turned into a pass the way the phone does it; WITHOUT_NAMES leaves out the names he is likely to say
        let (mut grant, _) = crate::ears::pass_for(&with, Vec::new()).unwrap();
        if std::env::var("WITHOUT_NAMES").is_err() {
            grant.phrases = vec!["airband".into(), "shop two".into()];
        }
        let said = live(l.wire, Arc::new(move || Ok(grant.clone())), &tape, 16000, crate::hears::wav_header(16000, None), std::env::var("MANUAL").is_ok())
        .unwrap();
        println!("{:>5} ms  FINAL: {said:?} (the recording is {} ms long)", tape.began.elapsed().as_millis(), pcm.len() / 32);
        assert!(said.is_some_and(|s| !s.is_empty()));
        assert!(tape.heard.lock().unwrap().len() >= 2, "words should arrive while he is still speaking");
    }
}
