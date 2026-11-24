//! Opened from a notification: the chat is followed at least as far as the notification before the message is
//! shown, and a message that is not there says why. Against a stand-in tower ([`super::fake::FakeTower`]).
use super::fake::FakeTower;
use super::*;
use airband_core::radio::{Feed, Message, wire::at};

const LINK: &str = "airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t1&name=studio";

/// A line of the tower's chat, as it sends it: `rev` is its change count.
fn line(id: &str, seq: u64, rev: u64, text: &str) -> Message {
    serde_json::from_value(serde_json::json!({ "seq": seq, "id": id, "at": 1_000 + seq, "from": "shop 1", "group": "shop", "agent": "a1", "kind": "report", "text": text, "rev": rev })).unwrap()
}

/// Until the view says so, for up to a few seconds.
fn until(desk: &Desk, fact: impl Fn(&DeskView) -> bool) -> DeskView {
    for _ in 0..300 {
        let view = desk.view();
        if fact(&view) {
            return view;
        }
        thread::sleep(Duration::from_millis(10));
    }
    desk.view()
}

#[test]
fn a_message_opened_from_its_notification_is_shown_only_once_the_chat_has_caught_up_to_it() {
    let tower = Arc::new(FakeTower::default());
    // the phone's copy of the chat is behind: what it was last given is older than the notification
    tower.first::<at::Feed>(&Feed { station: "studio".into(), rev: 3, ..Feed::default() });
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let t = tower.clone();
    let desk = Desk::through("", host, move |_| t.clone());
    // the computer does not answer until he has opened the notification
    tower.away.store(true, std::sync::atomic::Ordering::SeqCst);
    desk.set_links(vec![LINK.into()]);
    let station = desk.shared.state.lock().unwrap().computers[0].id().to_string();
    desk.open_message(station.clone(), "m1".into(), 9, String::new(), "a1".into());
    let view = desk.view();
    assert_eq!(view.opening, Some(DeskOpening { message: "m1".into(), state: OpeningState::CatchingUp }), "shown before the chat has it");
    assert_eq!(view.focus.agent.as_deref(), Some("a1"), "on the agent that said it");
    // reached, and still behind it: it is still being caught up to, whatever the feed holds meanwhile
    tower.away.store(false, std::sync::atomic::Ordering::SeqCst);
    let behind = until(&desk, |v| v.online);
    assert_eq!(behind.opening.map(|o| o.state), Some(OpeningState::CatchingUp));
    // the tower's next answer reaches the notification's change count, with the line as the notification had it
    tower.answers::<at::Feed>(&Feed { station: "studio".into(), rev: 9, seq: 2, messages: vec![line("m0", 1, 2, "Started."), line("m1", 2, 9, "The checker passes on all twelve units.")], ..Feed::default() });
    let here = until(&desk, |v| v.opening.as_ref().is_some_and(|o| o.state == OpeningState::Here));
    assert_eq!(here.opening.map(|o| o.state), Some(OpeningState::Here));
    assert_eq!(here.feed.iter().find(|m| m.id == "m1").map(|m| m.text.as_str()), Some("The checker passes on all twelve units."), "the words the notification showed");
    // he looks somewhere else: it is let go
    desk.look(None, None);
    assert_eq!(desk.view().opening, None);
    // caught up past one the computer no longer has: it says so, and why
    desk.open_message(station, "gone".into(), 5, String::new(), String::new());
    let gone = desk.view().opening.unwrap();
    assert!(matches!(gone.state, OpeningState::Gone { why: OpeningGone::NotKept, .. }), "{gone:?}");
    desk.close_opening();
    assert_eq!(desk.view().opening, None);
    // from a computer no longer paired here
    desk.open_message("another".into(), "m1".into(), 1, String::new(), String::new());
    let unpaired = desk.view().opening.unwrap();
    desk.stop();
    assert!(matches!(unpaired.state, OpeningState::Gone { why: OpeningGone::NotPaired, .. }), "{unpaired:?}");
}
