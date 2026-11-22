//! How long each thing on the desk lives: a computer for as long as he is paired with it, a connection for
//! one try at reaching it. Each against stand-in towers ([`super::fake::FakeTower`]).
use super::fake::FakeTower;
use super::*;
use airband_core::radio::{Feed, wire::at};

fn link(n: u8) -> String {
    format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t{n}")
}
/// A stand-in that answers its feed as the computer called `name`.
fn tower(name: &str) -> Arc<FakeTower> {
    let t = Arc::new(FakeTower::default());
    t.answers::<at::Feed>(&Feed { station: name.into(), ..Feed::default() });
    t
}
/// A desk reaching each of these links through its own stand-in.
fn desk_with(towers: Vec<(String, Arc<FakeTower>)>) -> Desk {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk::through("", host, move |l| towers.iter().find(|(k, _)| k == l).map(|(_, t)| t.clone()).unwrap_or_default())
}
fn radio_of(desk: &Desk, link: &str) -> Arc<Radio> {
    desk.shared.state.lock().unwrap().computers.iter().find(|s| s.link == link).map(|s| s.radio.clone()).unwrap()
}
/// Wait until the computer on this link has answered and said its name.
fn answered(desk: &Desk, link: &str) -> bool {
    for _ in 0..200 {
        if desk.shared.state.lock().unwrap().computers.iter().any(|s| s.link == link && s.online && !s.name.is_empty()) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn a_computer_that_stays_paired_keeps_its_connection_when_another_is_paired() {
    let desk = desk_with(vec![(link(1), tower("studio")), (link(2), tower("laptop"))]);
    desk.set_links(vec![link(1)]);
    let first = radio_of(&desk, &link(1));
    desk.set_links(vec![link(1), link(2)]);
    let still = radio_of(&desk, &link(1));
    let count = desk.shared.state.lock().unwrap().computers.len();
    desk.stop();
    assert!(Arc::ptr_eq(&first, &still), "pairing another computer restarted the connection to this one");
    assert_eq!(count, 2);
}

#[test]
fn asking_a_computer_afresh_replaces_only_its_connection() {
    let desk = desk_with(vec![(link(1), tower("studio"))]);
    desk.set_links(vec![link(1)]);
    assert!(answered(&desk, &link(1)), "the stand-in tower was never heard");
    let before = radio_of(&desk, &link(1));
    let (old_epoch, station) = desk.shared.state.lock().unwrap().computers.first().map(|s| (s.epoch, s.id())).unwrap();
    desk.reconnect();
    let (online, name, epoch) = desk.shared.state.lock().unwrap().computers.first().map(|s| (s.online, s.name.clone(), s.epoch)).unwrap();
    let after = radio_of(&desk, &link(1));
    // the old connection's host still speaks for the outbox both connections share: a message it found no one
    // for is put to him, never dropped (audit 6, P-B1)
    let old = ComputerHost { shared: desk.shared.clone(), station };
    old.unplaced(Outgoing::of("late".into()), None, vec![]);
    let asking = desk.shared.state.lock().unwrap().asking.as_ref().map(|a| a.message.text.clone());
    desk.stop();
    assert!(!Arc::ptr_eq(&before, &after), "it was not asked afresh");
    // one record of how requests went, for both connections: one writer of its file (audit 6, P-B10)
    assert!(Arc::ptr_eq(&before.link.trail, &after.link.trail), "each connection kept a trail file of its own");
    assert_ne!(epoch, old_epoch);
    assert!(online && name == "studio", "what was seen of it was dropped with the old connection");
    assert_eq!(asking.as_deref(), Some("late"), "a message the tower could not place vanished after a reconnect");
}

#[test]
fn coming_back_to_the_app_asks_afresh_only_a_computer_that_has_gone_quiet() {
    let quiet = Arc::new(FakeTower::default());
    let desk = desk_with(vec![(link(1), tower("studio")), (link(2), quiet)]);
    desk.set_links(vec![link(1), link(2)]);
    assert!(answered(&desk, &link(1)));
    let (talking, silent) = (radio_of(&desk, &link(1)), radio_of(&desk, &link(2)));
    desk.wake();
    let (talking_now, silent_now) = (radio_of(&desk, &link(1)), radio_of(&desk, &link(2)));
    desk.stop();
    assert!(Arc::ptr_eq(&talking, &talking_now), "a computer that answered a moment ago was asked afresh");
    assert!(!Arc::ptr_eq(&silent, &silent_now), "a computer that never answered was not asked afresh");
}

#[test]
fn a_computer_he_unpairs_is_let_go_and_the_others_are_left_as_they_are() {
    let desk = desk_with(vec![(link(1), tower("studio")), (link(2), tower("laptop"))]);
    desk.set_links(vec![link(1), link(2)]);
    let laptop = radio_of(&desk, &link(2));
    let studio = radio_of(&desk, &link(1));
    desk.set_links(vec![link(2)]);
    let left: Vec<String> = desk.shared.state.lock().unwrap().computers.iter().map(|s| s.link.clone()).collect();
    let still = radio_of(&desk, &link(2));
    desk.stop();
    assert_eq!(left, [link(2)]);
    assert!(Arc::ptr_eq(&laptop, &still));
    assert!(studio.stop.load(Ordering::SeqCst), "the unpaired computer's connection was not stopped");
}

#[test]
fn a_computer_that_refuses_this_phone_is_shown_as_out_of_reach_and_the_others_are_not() {
    let refusing = Arc::new(FakeTower::default());
    refusing.refuses::<at::Feed>(426, &airband_core::radio::wire::Refusal::OtherVersion { speaks: "airband.radio/9".into() });
    let desk = desk_with(vec![(link(1), tower("studio")), (link(2), refusing.clone())]);
    desk.set_links(vec![link(1), link(2)]);
    assert!(answered(&desk, &link(1)));
    let mut away = Vec::new();
    for _ in 0..300 {
        away = looked(&desk, None, None).away;
        if !away.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let asked = refusing.asked_on::<at::Feed>();
    desk.stop();
    assert!(asked >= 2, "the refusing computer was not asked through its own connection");
    assert_eq!(away, ["127.0.0.1"], "only the computer that refused is out of reach");
}

#[test]
fn a_team_that_has_gone_is_let_go_of_and_one_just_made_stays_open_until_its_first_agent() {
    let desk = desk_with(vec![(link(1), tower("studio"))]);
    desk.set_links(vec![link(1)]);
    assert!(answered(&desk, &link(1)), "the stand-in tower was never heard");
    desk.look(Some("gone-room".into()), Some("gone-agent".into()));
    desk.shown();
    let gone = desk.view().focus;
    desk.shared.state.lock().unwrap().focus = Focus { room: Some("made-room".into()), agent: None, made: Some("made-room".into()) };
    desk.shown();
    let made = desk.view().focus;
    desk.look(None, None);
    desk.shown();
    let after = desk.view().focus;
    let still_made = desk.shared.state.lock().unwrap().focus.made.clone();
    desk.stop();
    assert_eq!(gone, DeskFocus::default(), "a team no computer has is let go of");
    assert_eq!(made.room.as_deref(), Some("made-room"), "a team he has just made stays open while it waits for its first agent");
    assert_eq!(after, DeskFocus::default());
    assert_eq!(still_made.as_deref(), Some("made-room"), "looking elsewhere does not forget the team waiting for its agent");
}
