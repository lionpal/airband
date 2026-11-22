//! What his words take with them and where an answer from a notification goes, against stand-in towers
//! ([`super::fake::FakeTower`]).
use super::fake::FakeTower;
use super::*;
use airband_core::radio::wire::{Refusal, at};

fn link(n: u8) -> String {
    format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t{n}")
}
fn desk_with(towers: Vec<(String, Arc<FakeTower>)>) -> Desk {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk::through("", host, move |l| towers.iter().find(|(k, _)| k == l).map(|(_, t)| t.clone()).unwrap_or_default())
}
fn radio_of(desk: &Desk, link: &str) -> Arc<Radio> {
    desk.shared.state.lock().unwrap().computers.iter().find(|s| s.link == link).map(|s| s.radio.clone()).unwrap()
}

// N27: only his own next words take the files he attached and the question he was asked; a button, a command
// or words sent on to another agent go as they are and leave both for him.
#[test]
fn only_his_own_next_words_take_his_files_and_his_question() {
    let desk = desk_with(vec![(link(1), Arc::default())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    desk.stop();
    radio.session.arm("f1".into());
    radio.session.asked(&Reply { question: Some("q1".into()), ..Reply::default() });
    radio.queue_as(Outgoing::of("Keep going.".into()), Carry::Nothing);
    let first = radio.unsent().last().cloned().unwrap();
    assert!(first.attachments.is_empty() && first.answers.is_none(), "a command took his files or his question");
    radio.queue("yes".into(), None, None, None);
    let second = radio.unsent().last().cloned().unwrap();
    assert_eq!((second.attachments, second.answers), (vec!["f1".to_string()], Some("q1".to_string())));
}

// N31, D-6: an answer from a notification goes to the computer the notification came from, by its id, and
// nowhere else; one whose computer is gone is told as that, and the question stays waiting on him.
#[test]
fn a_notification_answer_goes_only_to_the_computer_it_came_from() {
    let (one, two) = (Arc::new(FakeTower::default()), Arc::new(FakeTower::default()));
    two.refuses::<at::Utterance>(409, &Refusal::Told { reason: "That question was closed.".into() });
    let desk = desk_with(vec![(link(1), one.clone()), (link(2), two.clone())]);
    desk.set_links(vec![link(1), link(2)]);
    let got = desk.answer_now(station_id(&link(2)).to_string(), "m1".into(), "yes".into(), None);
    let gone = desk.answer_now("not-a-computer".into(), "m2".into(), "yes".into(), None);
    let answered = desk.shared.state.lock().unwrap().answered.clone();
    desk.stop();
    assert_eq!(got, AnswerOutcome::Refused { why: "That question was closed.".into() });
    assert_eq!(one.asked_on::<at::Utterance>(), 0, "it went to another computer");
    assert!(matches!(gone, AnswerOutcome::NotPaired { .. }));
    assert!(!answered.contains("m1") && !answered.contains("m2"), "an answer that never went was taken as given");
}

// Words waiting for his files to reach another computer go when the last file has got there or failed, not
// on a clock.
#[test]
fn words_waiting_for_files_go_once_no_file_is_still_on_its_way() {
    let mut state = State::default();
    let upload = |state: UploadState| Upload { shown: DeskUpload { id: "f1".into(), name: "a.jpg".into(), sent: 0, total: 1, state }, station: "s".into(), dropped: Arc::default(), file_path: String::new(), name: "a.jpg".into() };
    state.uploads.push(upload(UploadState::Sending));
    state.after_files = Some(AfterFiles { text: "look".into(), label: None, re: None, room: None, agent: None });
    assert!(settled(&mut state).is_none(), "the words went before the file got there");
    state.uploads[0].shown.state = UploadState::Failed;
    assert_eq!(settled(&mut state).map(|w| w.text).as_deref(), Some("look"));
    assert!(state.after_files.is_none());
}
