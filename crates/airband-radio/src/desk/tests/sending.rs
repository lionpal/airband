//! What his words take with them and where an answer from a notification goes, against stand-in towers
//! ([`super::fake::FakeTower`]).
use super::fake::FakeTower;
use super::*;
use airband_core::radio::wire::{Refusal, at};

fn link(n: u8) -> String {
    format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t{n}&name=computer%20{n}")
}
fn desk_with(towers: Vec<(String, Arc<FakeTower>)>) -> Desk {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk::through("", host, move |l| towers.iter().find(|(k, _)| k == l).map(|(_, t)| t.clone()).unwrap_or_default())
}
fn radio_of(desk: &Desk, link: &str) -> Arc<Radio> {
    desk.shared.state.lock().unwrap().computers.iter().find(|s| s.link == link).map(|s| s.radio.clone()).unwrap()
}

// Only his own next words take the files he attached and the question he was asked; a button, a command
// or words sent on to another agent go as they are and leave both for him.
#[test]
fn only_his_own_next_words_take_his_files_and_his_question() {
    let desk = desk_with(vec![(link(1), Arc::default())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    desk.stop();
    radio.session.arm("f1".into());
    radio.session.asked(&Reply { question: Some(airband_core::ids::QuestionId::new("q1")), ..Reply::default() });
    radio.queue_as(Outgoing::of("Keep going.".into()), Carry::Nothing);
    let first = radio.unsent().last().cloned().unwrap();
    assert!(first.attachments.is_empty() && first.answers.is_none(), "a command took his files or his question");
    radio.queue("yes".into(), None, None, None);
    let second = radio.unsent().last().cloned().unwrap();
    assert_eq!((second.attachments, second.answers), (vec!["f1".to_string()], Some("q1".to_string())));
}

// An answer from a notification goes to the computer the notification came from, by its id, and
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
    let upload = |state: UploadState| Upload { shown: DeskUpload { id: "f1".into(), name: "a.jpg".into(), sent: 0, total: 1, state, why: None }, station: StationId::new("s"), dropped: Arc::default(), file_path: String::new(), name: "a.jpg".into() };
    state.uploads.push(upload(UploadState::Sending));
    state.after_files = Some(AfterFiles { text: "look".into(), label: None, re: None, room: None, agent: None, said: None });
    assert!(settled(&mut state).is_none(), "the words went before the file got there");
    state.uploads[0].shown.state = UploadState::Failed;
    assert_eq!(settled(&mut state).map(|w| w.text).as_deref(), Some("look"));
    assert!(state.after_files.is_none());
}

// A message his computer took half an hour ago and keeps answering "taken" for ends, he is told, and it no longer
// holds up what he says after it. One he wrote hours ago that his computer has only now taken (the relay kept it
// while the computer slept) is given its half hour from now.
#[test]
fn a_message_his_computer_only_ever_says_it_has_ends_and_he_is_told() {
    let tower = Arc::new(FakeTower::default());
    tower.answers::<at::Utterance>(&airband_core::radio::Outcome::Taken);
    let desk = desk_with(vec![(link(1), tower.clone())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    let long_ago = now_ms() - crate::store::OUTBOX_EXPIRY_MS - 1000;
    radio.queue_as(Outgoing { at: now_ms(), stage: Stage::Taken, taken_at: Some(long_ago), ..Outgoing::of("old".into()) }, Carry::Nothing);
    radio.queue_as(Outgoing { at: long_ago, stage: Stage::Left, ..Outgoing::of("slept".into()) }, Carry::Nothing);
    let find = |text: &str| radio.unsent().into_iter().find(|o| o.text == text);
    let mut ended = None;
    for _ in 0..300 {
        ended = find("old").and_then(|o| o.failed);
        if ended.is_some() && find("slept").is_some_and(|o| o.stage == Stage::Taken) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    desk.stop();
    assert_eq!(ended.as_deref(), Some(crate::outbox::NEVER_SETTLED));
    let slept = find("slept").expect("still waiting for what came of it");
    assert_eq!((slept.stage, slept.failed), (Stage::Taken, None), "taken just now, so not given up on");
    assert!(slept.taken_at.is_some_and(|t| t > long_ago));
}

// A picture whose parts stop moving forward before the last is only part of one: none
// of it is kept, and he is told.
#[test]
fn a_picture_that_stops_coming_part_way_is_not_kept_as_the_whole() {
    let tower = Arc::new(FakeTower::default());
    tower.answers::<at::Picture>(&airband_core::radio::PicturePart { data: airband_core::encode::base64(b"\xff\xd8 half"), next: 0, done: false });
    let desk = desk_with(vec![(link(1), tower.clone())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    let id = airband_core::encode::random_hex(8);
    let got = radio.picture(id.clone());
    desk.stop();
    assert!(matches!(&got, Err(RadioError::Told { reason }) if reason == "Your computer sent only part of that picture."), "{got:?}");
    assert!(!radio.store.pictures().join(format!("{id}.jpg")).exists(), "part of a picture was kept as the whole");
}

// A file that did not get there says why, for the tray to show.
#[test]
fn a_file_that_did_not_get_there_keeps_why() {
    let tower = Arc::new(FakeTower::default());
    tower.refuses::<at::Attachment>(413, &Refusal::TooBig);
    let desk = desk_with(vec![(link(1), tower.clone())]);
    desk.set_links(vec![link(1)]);
    let file = std::env::temp_dir().join(format!("airband-upload-{}.png", airband_core::encode::random_hex(6)));
    std::fs::write(&file, b"not much").unwrap();
    desk.attach(file.display().to_string(), "a.png".into(), None, None, None);
    let mut shown = None;
    for _ in 0..300 {
        shown = desk.shared.state.lock().unwrap().uploads.first().map(|u| u.shown.clone()).filter(|u| u.state == UploadState::Failed);
        if shown.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    desk.stop();
    std::fs::remove_file(&file).ok();
    assert_eq!(shown.and_then(|u| u.why).as_deref(), Some("It is more than your computer takes in one message."));
}

// Something said aloud that the tower took and then refuses for good ends with the refusal at
// once: it is not asked about for a minute and a half and then given up on in silence.
#[test]
fn a_spoken_message_refused_after_it_was_taken_ends_with_the_refusal() {
    let tower = Arc::new(FakeTower::default());
    tower.first::<at::Utterance>(&airband_core::radio::Outcome::Taken).refuses::<at::Utterance>(409, &Refusal::Told { reason: "That question was closed.".into() });
    let desk = desk_with(vec![(link(1), tower.clone())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    let began = std::time::Instant::now();
    let got = radio.link.say_and_wait(&Outgoing::of("yes".into()).utterance());
    desk.stop();
    assert!(matches!(&got, Err(crate::link::LinkError::Refused(Refusal::Told { .. }))), "{got:?}");
    assert!(began.elapsed() < Duration::from_secs(10), "it kept asking after a refusal for good");
}

// The trail names each request by its endpoint: the id of a picture or a message is never in it.
#[test]
fn the_trail_never_holds_the_id_a_request_names() {
    let tower = Arc::new(FakeTower::default());
    let desk = desk_with(vec![(link(1), tower.clone())]);
    desk.set_links(vec![link(1)]);
    let radio = radio_of(&desk, &link(1));
    let id = airband_core::encode::random_hex(8);
    radio.picture(id.clone()).ok();
    let trail = radio.link_trail();
    desk.stop();
    assert!(trail.iter().any(|l| l.contains("/picture/")), "the failed request is not in the trail: {trail:?}");
    assert!(trail.iter().all(|l| !l.contains(&id)), "{trail:?}");
}
