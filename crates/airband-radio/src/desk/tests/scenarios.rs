//! The paths the audits keep finding faults in: what the phone keeps across a reconnect, a relaunch and a
//! computer going, what a send takes with it, and what a refusal or a delay is told as. Each test names the
//! audit-5 finding it covers (scratchpad audit-5.md), so a failure says which one is back.
use super::*;

/// A computer that is never there: nothing listens on port 9. `n` makes each link its own computer.
fn nobody(n: u8) -> String {
    format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t{n}")
}
fn opened(dir: &std::path::Path) -> Arc<Desk> {
    Desk::open(dir.display().to_string(), Arc::new(Watching(std::sync::atomic::AtomicBool::new(true))))
}
fn radio_at(desk: &Desk, i: usize) -> Arc<Radio> {
    desk.shared.state.lock().unwrap().computers[i].radio.clone()
}
fn asking(id: &str) -> Reply {
    Reply { question: Some(airband_core::ids::QuestionId::new(id)), ..Reply::default() }
}
/// A computer on this machine that answers every request with `status` and `body`. Its port.
fn tower_saying(status: u16, body: &'static str) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 65536];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
        }
    });
    port
}

// N24 (and the part of BUG-14 it undoes): the files he attached and the question his computer asked him are
// his until he says something; asking the computer afresh (`reconnect`, or `wake` after 32 seconds) is not that.
#[test]
fn n24_files_and_the_question_he_was_asked_outlast_a_reconnect() {
    let dir = temp_dir("n24");
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1)]);
    let radio = radio_at(&desk, 0);
    radio.arm("f1".into());
    radio.session.asked(&asking("q1"));
    desk.reconnect();
    // what he says next, on the computer as it is now
    radio_at(&desk, 0).queue("yes, that one".into(), None, None, None);
    let sent: Vec<(Vec<String>, Option<String>)> = radio_at(&desk, 0).unsent().into_iter().map(|o| (o.attachments, o.answers)).collect();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(sent, [(vec!["f1".to_string()], Some("q1".to_string()))], "what he said after the reconnect lost the files or the question");
}

// N25: what he typed that has not gone, or was given up on, is on his screen after a relaunch and after a
// reconnect, and he can still send it again or let it go.
#[test]
fn n25_what_did_not_go_is_still_shown_after_a_relaunch_and_can_be_let_go() {
    let dir = temp_dir("n25");
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1)]);
    let radio = radio_at(&desk, 0);
    let waiting = radio.queue("still waiting".into(), None, None, None);
    let given_up = radio.queue("given up on".into(), None, None, None);
    radio.store.change(|s| s.outbox.iter_mut().filter(|o| o.id == given_up).for_each(|o| o.failed = Some("It could not be sent.".into())));
    desk.stop();
    drop(radio);
    // the app is opened again
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1)]);
    let shown: Vec<String> = looked(&desk, None, None).sending.into_iter().map(|o| o.id).collect();
    let kept: Vec<String> = radio_at(&desk, 0).unsent().into_iter().map(|o| o.id).collect();
    let let_go = desk.discard_unsent(given_up.clone());
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(kept, [waiting.clone(), given_up.clone()], "the phone did not keep them across the relaunch");
    assert_eq!(shown, [waiting, given_up], "kept, but not on his screen");
    assert_eq!(let_go.as_deref(), Some("given up on"), "letting go of one that was given up on did nothing");
}

// N26: a file on its way to a computer is known by that computer, never by its place in the list. When that
// computer is unpaired, sending does not crash and does not send to whichever computer now has its place.
#[test]
fn n26_an_attached_file_follows_its_computer_and_not_its_place_in_the_list() {
    let dir = temp_dir("n26");
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1), nobody(2)]);
    {
        let mut state = desk.shared.state.lock().unwrap();
        let shown = DeskUpload { id: "f1".into(), name: "a.jpg".into(), sent: 1, total: 1, state: UploadState::Ready, why: None };
        state.uploads.push(Upload { shown, station: super::super::ids::station_id(&nobody(2)), dropped: Arc::default(), file_path: String::new(), name: "a.jpg".into() });
    }
    // the second computer is unpaired; the first stays
    desk.set_links(vec![nobody(1)]);
    let sent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| desk.send("look at this".into(), None, None, None)));
    let first_took: Vec<Vec<String>> = desk.shared.state.lock().map(|s| s.computers[0].radio.unsent().into_iter().map(|o| o.attachments).collect()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(sent.is_ok(), "sending after the computer with the file was unpaired crashed");
    assert!(first_took.iter().all(|a| a.is_empty()), "the file went to the computer that took the other's place: {first_took:?}");
}

/// One computer, with an agent that has stopped and a question that is open, and its radio stopped so what
/// is queued stays to be looked at.
fn stalled() -> Desk {
    let mut state = State { computers: vec![station("mini", 9, &[("shop two", "shop")], vec![msg("q1", 1, "shop two", "shop", true)])], ..State::default() };
    state.computers[0].agents[0].state = AgentState::Stalled;
    desk_of(state)
}

// N27: only what he says takes his files and answers his computer's question. A button, a poke, words sent
// again and the like leave both for the next thing he says.
#[test]
fn n27_a_poke_a_tap_and_words_sent_again_leave_his_files_and_question() {
    let desk = stalled();
    let radio = radio_at(&desk, 0);
    radio.arm("f1".into());
    radio.session.asked(&asking("q9"));
    desk.poke(None);
    desk.tap("q1".into(), Tapped::Allow);
    desk.resend("q1".into(), "shop two".into()).unwrap();
    let took: Vec<(String, Vec<String>, Option<String>)> = radio.unsent().into_iter().map(|o| (o.text, o.attachments, o.answers)).collect();
    assert!(took.iter().all(|(_, files, answers)| files.is_empty() && answers.is_none()), "{took:?}");
    assert_eq!(radio.armed(), ["f1"]);
    assert_eq!(radio.session.take_question().as_deref(), Some("q9"));
}

// N28: opening the microphone to answer a question is not answering it: until he says something it is still
// waiting.
#[test]
fn n28_a_question_is_still_waiting_while_the_microphone_is_open_for_it() {
    let desk = stalled();
    desk.talk(Some("q1".into()), None, None);
    let hidden = desk.shared.state.lock().unwrap().answered.contains("q1");
    assert!(!hidden, "the question was taken as answered when the microphone opened");
}

// N30: a refusal that gives his computer's reason in words is told as that reason, whatever status it came with.
#[test]
fn n30_a_refusal_with_a_reason_is_told_as_that_reason() {
    let port = tower_saying(409, r#"{"refusal":"told","reason":"That folder is not there."}"#);
    let radio = Radio::start(format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t"), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    let got = radio.folders("/nowhere".into());
    assert!(matches!(&got, Err(RadioError::Told { reason }) if reason == "That folder is not there."), "{got:?}");
}

// N31: an answer from a notification that his computer could not take yet (a 503) is kept and goes later: he
// is told it will go, never that his computer refused it.
#[test]
fn n31_a_notification_answer_kept_for_later_is_not_told_as_refused() {
    let port = tower_saying(503, "{}");
    let radio = Radio::start(format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t"), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    let got = radio.answer_now("Leave".into(), "q1".into(), Some(Act::Option { index: 1 }));
    let kept: Vec<Option<String>> = radio.unsent().into_iter().map(|o| o.re).collect();
    assert_eq!(kept, [Some("q1".to_string())], "it was not kept to go later");
    assert!(matches!(got, AnswerOutcome::KeptForLater { .. }), "kept to go later, but told as {got:?}");
}

// A computer is shown by the name it last said, kept on the phone, from the moment the app opens: never by its
// place in the list.
#[test]
fn a_computer_is_called_what_it_last_said_from_the_moment_the_app_opens() {
    let dir = temp_dir("named");
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1), nobody(2)]);
    radio_at(&desk, 1).store.change(|s| s.computer = "Studio".into());
    desk.stop();
    // the app is opened again, and neither computer has answered yet
    let desk = opened(&dir);
    desk.set_links(vec![nobody(1), nobody(2)]);
    let names: Vec<String> = looked(&desk, None, None).stations.into_iter().map(|c| c.name).collect();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(names[1], "Studio");
    assert!(!names[0].starts_with("Computer"), "{names:?}");
}
