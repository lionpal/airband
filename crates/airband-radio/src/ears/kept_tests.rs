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
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool, _: &str) -> Option<String> {
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
    /// How many times he was told something went wrong: the tone, and the banner.
    told: Mutex<usize>,
    /// What the host's model engine gives back (nothing: it could not), and how many times it was asked.
    model: Mutex<Option<Vec<u8>>>,
    asked: Mutex<usize>,
    /// What was shown of his words as he spoke.
    shown: Mutex<Vec<String>>,
}
impl Mic {
    fn of(sound: &[u8], pause: Duration) -> Mic {
        Mic { left: Mutex::new(sound.chunks(3200).map(<[u8]>::to_vec).collect()), pause, stopped: Mutex::new(false), told: Mutex::new(0), model: Mutex::new(None), asked: Mutex::new(0), shown: Mutex::new(Vec::new()) }
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
    fn told(&self, _: String) {
        *self.told.lock().unwrap() += 1;
    }
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
        *self.asked.lock().unwrap() += 1;
        self.model.lock().unwrap().clone()
    }
    fn hearing(&self, text: String) {
        self.shown.lock().unwrap().push(text);
    }
}

/// A listening model the host runs, whose row says the phone gets it ready before first use.
fn host_model() -> Arc<crate::engines::Held> {
    let m = crate::models::Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Ears, engine: "host-only".into(), version: "1".into(), base: "https://airband.dev/models/tiny".into(), files: Vec::new(), voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with: [(crate::models::field::PREPARE.to_string(), serde_json::json!("Getting it ready"))].into() };
    crate::engines::Held::of(&m, "/models/tiny/1".into())
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
    let Listened::Unheard { take, why } = heard else { panic!("it was taken as heard") };
    assert_eq!(why, Unheard::Dropped);
    // every sample he said is kept, to the end, after the line went
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why: Unheard::Dropped }), Some(sound.len())));
    assert!(*mic.stopped.lock().unwrap());
    // and he is told, once: he may believe it went
    assert_eq!(*mic.told.lock().unwrap(), 1);
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
    let Listened::Unheard { take, .. } = heard else { panic!("words it had not finished were taken for what he said") };
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why: Unheard::TimedOut }), Some(sound.len())));
}

#[test]
fn a_take_not_heard_is_heard_again_whole_from_its_recording() {
    let dir = Folder::new();
    let ears = ears_in(&dir);
    // the first time, nothing can be reached
    let sound = speech(2);
    let Listened::Unheard { take, .. } = ears.hear_by(&Mic::of(&sound, Duration::ZERO), by_stand_in(1), true, Aim::default()) else { panic!() };
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
    let got = ears.by(&crate::takes::Replay::of(&side, &ears.takes().replay(&take).unwrap(), false), mic, true);
    let Listened::Said { text, .. } = ears.takes().heard_again(&take, got) else { panic!() };
    assert_eq!(text, format!("{} bytes", sound.len()));
}

#[test]
fn a_recording_heard_again_by_a_live_service_is_given_no_faster_than_it_was_spoken() {
    let side = Mic::of(&[], Duration::ZERO);
    let sound = speech(2);
    let replay = crate::takes::Replay::of(&side, &sound, true);
    let began = std::time::Instant::now();
    while !replay.record_read().is_empty() {}
    // two seconds of sound: the second is given a second after the first
    assert!(began.elapsed() >= Duration::from_millis(950), "{:?}", began.elapsed());
    let fast = crate::takes::Replay::of(&side, &sound, false);
    let began = std::time::Instant::now();
    while !fast.record_read().is_empty() {}
    assert!(began.elapsed() < Duration::from_millis(500));
}

#[test]
fn an_upload_closes_the_microphone_when_he_stops_and_not_after_the_upload() {
    // a recording read by a reader that only records when the microphone was stopped before it was asked
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let mic = Mic::of(&speech(1), Duration::ZERO);
    let tape = ears.takes().tape(&mic, Aim::default());
    assert!(record(&tape, true).is_some());
    tape.record_stop();
    tape.record_stop();
    assert!(*mic.stopped.lock().unwrap());
    // stopped once, whoever asks: nothing more is read from it
    assert!(tape.record_read().is_empty());
}

#[test]
fn a_take_ends_at_the_longest_one_is() {
    let dir = Folder::new();
    let ears = ears_in(&dir);
    // a microphone left open, giving sound for as long as it is read
    let longest = crate::takes::LONGEST as usize;
    let mic = Mic::of(&vec![1u8; longest + 32000], Duration::ZERO);
    let tape = ears.takes().tape(&mic, Aim::default());
    let mut read = 0;
    loop {
        let got = tape.record_read();
        if got.is_empty() {
            break;
        }
        read += got.len();
    }
    assert_eq!(read, longest);
    let rec = tape.done();
    assert_eq!(kept(&ears, &dir, &rec.id).1, Some(longest));
}

#[test]
fn a_model_not_ready_yet_is_not_run_and_what_he_said_waits_with_why() {
    // Getting a Core ML model ready takes minutes on a phone, on the Neural Engine and the graphics processor
    // alike. Run then, it kept him waiting with no words and no error: now it is not run at all until it is.
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let held = host_model();
    assert!(!held.ready());
    let sound = speech(2);
    let mic = Mic::of(&sound, Duration::ZERO);
    *mic.model.lock().unwrap() = Some(b"ship it".to_vec());
    let Listened::Unheard { take, why } = ears.hear_by(&mic, super::Mic::Model(held.clone()), true, Aim::default()) else { panic!("not kept as unheard") };
    assert_eq!(why, Unheard::NotReady { model: "Tiny".into() });
    assert_eq!(*mic.asked.lock().unwrap(), 0, "the engine was asked while it was not ready");
    // all of it is kept, with the reason the chat shows beside it, and he is told once
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why: why.clone() }), Some(sound.len())));
    assert_eq!(*mic.told.lock().unwrap(), 1);
    assert_eq!(why.told(), "Tiny isn\u{2019}t ready yet.");
    // once it has been got ready, the take is heard again by it
    held.set_ready(true);
    ears.takes().again(&take, false);
    let got = ears.by(&crate::takes::Replay::of(&mic, &ears.takes().replay(&take).unwrap(), false), super::Mic::Model(held), true);
    let Listened::Said { text, .. } = ears.takes().heard_again(&take, got) else { panic!("not heard once ready") };
    assert_eq!(text, "ship it");
}

#[test]
fn a_model_that_gives_back_nothing_leaves_what_he_said_unheard_and_says_so() {
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let held = host_model();
    // it was got ready, and now its engine gives back nothing
    held.set_ready(true);
    let sound = speech(2);
    let mic = Mic::of(&sound, Duration::ZERO);
    let Listened::Unheard { take, why } = ears.hear_by(&mic, super::Mic::Model(held), true, Aim::default()) else { panic!("its silence was taken for his") };
    assert!(matches!(&why, Unheard::Model { why } if why.starts_with("Tiny did not run on this phone")), "{why:?}");
    assert_eq!(kept(&ears, &dir, &take), (Some(Stage::Unheard { why }), Some(sound.len())));
    assert_eq!(*mic.told.lock().unwrap(), 1);
}

#[test]
fn a_model_on_the_phone_shows_his_words_as_he_speaks() {
    let dir = Folder::new();
    let ears = ears_in(&dir);
    let held = host_model();
    held.set_ready(true);
    let mic = Mic::of(&speech(3), Duration::from_millis(100));
    *mic.model.lock().unwrap() = Some(b"ship it".to_vec());
    let Listened::Said { text, .. } = ears.hear_by(&mic, super::Mic::Model(held), false, Aim::default()) else { panic!("not heard") };
    assert_eq!(text, "ship it");
    assert!(!mic.shown.lock().unwrap().is_empty(), "nothing shown as he spoke");
}
