//! What he says is kept whatever the live listening does: against a stand-in service on this machine that
//! answers, drops the line, or never gives its final words.
use super::*;
use crate::ears_stream::{Socket, Wire};
use crate::takes::Stage;
use crate::takes::tests::{Folder, speech};
use std::collections::VecDeque;
use std::net::{TcpListener, TcpStream};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

/// The stand-in's own way of talking: sound goes as binary; it answers words it has not finished as text and
/// its final words as binary, which ends its turn. The port it is at is the pass's token.
const WIRE: Wire = Wire { connect, audio: |_, bytes| Message::binary(bytes.to_vec()), end: |_| Message::text("end"), read, header: false, reopens: false };
const LISTENER: Listener = Listener { is: Interface { name: "stand-in", can: &[], told: &[] }, wire: &WIRE, pass: |_, _, _, _| Err(String::new()), once: true };

fn connect(pass: &Pass, _: &str) -> Result<Socket, ()> {
    let tcp = TcpStream::connect(("127.0.0.1", pass.token.parse::<u16>().map_err(|_| ())?)).map_err(|_| ())?;
    let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{}/", pass.token), MaybeTlsStream::Plain(tcp)).map_err(|_| ())?;
    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        tcp.set_read_timeout(Some(Duration::from_millis(12))).ok();
    }
    Ok(socket)
}
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool) -> Option<String> {
    match m {
        Message::Text(t) => Some(t.to_string()),
        Message::Binary(b) => {
            kept.push(String::from_utf8_lossy(b).into_owned());
            *ended = true;
            None
        }
        _ => None,
    }
}

/// What the stand-in does on its one connection, once it is made.
fn stand_in(serve: impl FnOnce(&mut tungstenite::WebSocket<TcpStream>) + Send + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        // it takes no more connections: one made again is refused
        drop(listener);
        let mut ws = tungstenite::accept(tcp).unwrap();
        serve(&mut ws);
    });
    port
}

/// A microphone that gives this sound a tenth of a second at a time, `pause` apart, then stops: he tapped send.
struct Mic {
    left: Mutex<VecDeque<Vec<u8>>>,
    pause: Duration,
    stopped: Mutex<bool>,
}
impl Mic {
    fn of(sound: &[u8], pause: Duration) -> Mic {
        Mic { left: Mutex::new(sound.chunks(3200).map(<[u8]>::to_vec).collect()), pause, stopped: Mutex::new(false) }
    }
}
impl RadioHost for Mic {
    fn record_start(&self, _: u32) -> bool {
        true
    }
    fn record_read(&self) -> Vec<u8> {
        thread::sleep(self.pause);
        self.left.lock().unwrap().pop_front().unwrap_or_default()
    }
    fn record_stop(&self) {
        *self.stopped.lock().unwrap() = true;
    }
    fn told(&self, _: String) {}
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn status(&self, _: RadioStatus) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
    fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
    fn hearing(&self, _: String) {}
}

fn ears_in(dir: &Folder) -> Ears {
    Ears::new(Arc::new(Senses { takes: crate::takes::Takes::open(&dir.path()).0, ..Senses::default() }))
}
fn by_stand_in(port: u16) -> super::Mic {
    let by: By = serde_json::from_value(serde_json::json!({ "interface": "stand-in" })).unwrap();
    let grant = Pass { dialect: "stand-in".into(), token: port.to_string(), set: BTreeMap::new(), phrases: Vec::new(), by, until: None };
    super::Mic::Live { with: &LISTENER, pass: Arc::new(move || Ok(grant.clone())) }
}
fn kept(ears: &Ears, dir: &Folder, id: &str) -> (Option<Stage>, Option<usize>) {
    let stage = ears.takes().all().into_iter().find(|t| t.id == id).map(|t| t.stage);
    (stage, std::fs::read(std::path::Path::new(&crate::takes::takes_folder(dir.path())).join(format!("{id}.pcm"))).ok().map(|b| b.len()))
}

#[test]
fn a_line_that_drops_mid_sentence_stops_neither_the_recording_nor_takes_the_words_it_had() {
    // it hears a little, says some words it has not finished, and the line goes
    let port = stand_in(|ws| {
        for _ in 0..3 {
            ws.read().unwrap();
        }
        ws.send(Message::text("ship it and")).unwrap();
    });
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let sound = speech(3);
    let mic = Mic::of(&sound, Duration::from_millis(15));
    let heard = ears.hear_by(&mic, by_stand_in(port), true, Aim::default());
    let Listened::Unheard { take } = heard else { panic!("it was taken as heard") };
    // every sample he said is kept, to the end, after the line went
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why: Unheard::Dropped }), Some(sound.len())));
    assert!(*mic.stopped.lock().unwrap());
}

#[test]
fn tapping_send_with_sound_still_to_go_sends_only_the_final_words_for_all_of_it() {
    // words it has not finished come back as the sound arrives; its final words only once it has all of it
    let port = stand_in(|ws| {
        loop {
            match ws.read() {
                Ok(Message::Binary(_)) => ws.send(Message::text("ship")).unwrap(),
                Ok(Message::Text(_)) => {
                    ws.send(Message::text("ship it and")).unwrap();
                    ws.send(Message::binary(b"ship it and deploy".to_vec())).unwrap();
                }
                _ => return,
            }
        }
    });
    let dir = Folder::new();
    let ears = ears_in(&dir);
    // he taps send while most of what he said has not reached the service yet
    let sound = speech(2);
    let heard = ears.hear_by(&Mic::of(&sound, Duration::ZERO), by_stand_in(port), true, Aim::default());
    let Listened::Said { text, take: Some(take) } = heard else { panic!("not heard") };
    assert_eq!(text, "ship it and deploy");
    // heard, and kept until his computer has it
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Heard { text: "ship it and deploy".into() }), Some(sound.len())));
}

#[test]
fn final_words_that_never_come_leave_the_take_unheard_with_its_sound_and_nothing_sent() {
    // it says words it has not finished, and never its final ones
    let port = stand_in(|ws| {
        while let Ok(m) = ws.read() {
            if m.is_binary() {
                ws.send(Message::text("ship it")).unwrap();
            }
        }
    });
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let sound = speech(1);
    let heard = ears.hear_by(&Mic::of(&sound, Duration::from_millis(5)), by_stand_in(port), true, Aim::default());
    let Listened::Unheard { take } = heard else { panic!("words it had not finished were taken for what he said") };
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why: Unheard::TimedOut }), Some(sound.len())));
}

#[test]
fn a_take_not_heard_is_heard_again_whole_from_its_recording() {
    let dir = Folder::new();
    let ears = ears_in(&dir);
    // the first time, nothing can be reached
    let sound = speech(2);
    let Listened::Unheard { take } = ears.hear_by(&Mic::of(&sound, Duration::ZERO), by_stand_in(1), true, Aim::default()) else { panic!() };
    // the second, the service has all of the recording before it answers
    let port = stand_in(|ws| {
        let mut got = 0;
        loop {
            match ws.read() {
                Ok(Message::Binary(b)) => got += b.len(),
                Ok(Message::Text(_)) => ws.send(Message::binary(format!("{got} bytes").into_bytes())).unwrap(),
                _ => return,
            }
        }
    });
    let side = Mic::of(&[], Duration::ZERO);
    ears.takes().again(&take, false);
    let mic = by_stand_in(port);
    let got = ears.by(&crate::takes::Replay::of(&side, &ears.takes().replay(&take).unwrap()), mic, true);
    let Listened::Said { text, .. } = ears.takes().heard_again(&take, got) else { panic!() };
    assert_eq!(text, format!("{} bytes", sound.len()));
}
