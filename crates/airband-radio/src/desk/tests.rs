use super::*;

struct Quiet;
impl RadioHost for Quiet {
    fn told(&self, _: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn status(&self, _: RadioStatus) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn hearing(&self, _: String) {}
    fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
}

fn msg(id: &str, seq: u64, from: &str, group: &str, needs: bool) -> ChatMessage {
    ChatMessage { seq, id: id.into(), at: seq, his: from == "you", from: from.into(), group: group.into(), to: Vec::new(), needs_you: needs, fyi: false, stalled: false, text: id.into(), long: false, queued: false, seen: 0, options: Vec::new(), recommended: None, permission: false, pictures: Vec::new(), links: Vec::new(), agent: Some(from.into()), station: String::new(), open: needs, badge: String::new(), colour: 0, read: None, own_session: false, not_read: String::new() }
}
fn station(name: &str, port: u16, agents: &[(&str, &str)], messages: Vec<ChatMessage>) -> Station {
    let link = format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t");
    let radio = Radio::start(link.clone(), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    Station {
        link,
        radio,
        name: name.into(),
        online: true,
        tried: true,
        answered_at: None,
        fails: 0,
        failing_since: None,
        reads: String::new(),
        reads_failed: String::new(),
        agents: agents.iter().map(|(c, g)| ChatAgent { id: c.to_string(), callsign: c.to_string(), group: g.to_string(), number: c.strip_prefix(g).unwrap_or(c).trim().to_string(), named: false, harness: "h".into(), waiting: false, reachable: true, state: AgentState::Working, subagents: 0, doing: None, held: false, ends: false, model: None, effort: None }).collect(),
        open: messages.iter().filter(|m| m.needs_you).map(|m| m.id.clone()).collect(),
        sending: Vec::new(),
        landing: Vec::new(),
        quick: Vec::new(),
        colours: Default::default(),
        messages,
    }
}
fn two() -> State {
    State {
        stations: vec![
            station("mini", 1, &[("shop two", "shop"), ("backoffice login fix", "backoffice")], vec![msg("q1", 1, "shop two", "shop", true), msg("d1", 2, "backoffice login fix", "backoffice", false)]),
            station("macbook", 2, &[("ff one", "ff")], vec![msg("q2", 3, "ff one", "ff", true)]),
        ],
        ..State::default()
    }
}

struct Watching(std::sync::atomic::AtomicBool);
impl DeskHost for Watching {
    fn secret(&self, _: String) -> Option<String> {
        None
    }
    fn keep_secret(&self, _: String, _: Option<String>) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn changed(&self) {}
    fn status(&self, _: RadioStatus) {}
    fn note(&self, _: String) {}
    fn hearing(&self, _: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn watching(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn device(&self) -> HostDevice {
        HostDevice::default()
    }
    fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn a_computer_is_never_picked_for_him() {
    struct Noted(Mutex<Vec<String>>);
    impl DeskHost for Noted {
        fn secret(&self, _: String) -> Option<String> {
            None
        }
        fn keep_secret(&self, _: String, _: Option<String>) {}
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn changed(&self) {}
        fn status(&self, _: RadioStatus) {}
        fn note(&self, text: String) {
            self.0.lock().unwrap().push(text);
        }
        fn hearing(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn watching(&self) -> bool {
            true
        }
        fn device(&self) -> HostDevice {
            HostDevice::default()
        }
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }
    let host = Arc::new(Noted(Mutex::new(Vec::new())));
    // two computers that each have a team called shop
    let state = State { stations: vec![station("mini", 1, &[("shop two", "shop")], Vec::new()), station("macbook", 2, &[("shop one", "shop")], Vec::new())], ..State::default() };
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: String::new() };
    // with none named he is asked which, and one that has no such team is not swapped for one that has
    desk.new_agent("shop".into(), None, None);
    desk.new_agent("shop".into(), Some("laptop".into()), None);
    // a team no computer has: nothing happens, and nothing is said
    desk.new_agent("garden".into(), None, None);
    let notes = host.0.lock().unwrap().clone();
    assert_eq!(notes, vec!["Which computer should it start on? mini and macbook each have a team called shop.".to_string(), "laptop has no team called shop.".to_string()]);
    // the same for anything else asked of a computer he did not name
    assert!(matches!(desk.station_radio(""), Err(RadioError::Told { .. })));
    assert!(matches!(desk.station_radio("laptop"), Err(RadioError::Offline)));
}

#[test]
fn a_button_is_sent_as_what_it_stands_for_and_never_as_words_to_be_read() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let mut state = two();
    state.stations[0].messages[0].options = vec!["Rewrite".into(), "Leave".into()];
    state.stations[0].agents[0].state = AgentState::Stalled;
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: String::new() };
    let sent = |desk: &Desk| desk.shared.state.lock().unwrap().stations[0].radio.unsent().into_iter().map(|o| (o.text, o.re, o.act, o.to)).collect::<Vec<_>>();
    let act = |a: Act| serde_json::to_string(&a).unwrap();
    // a yes to a request, an answer he tapped, and one the message does not have (nothing goes)
    desk.tap("q1".into(), Tapped::Allow);
    desk.tap("q1".into(), Tapped::Option { index: 1 });
    desk.tap("q1".into(), Tapped::Option { index: 7 });
    // a poke, and a new agent
    desk.poke(None);
    desk.new_agent("shop".into(), None, None);
    // words to the agent he is looking at go to it by its id, with no tag written into them
    desk.send("ship it".into(), None, Some("shop".into()), Some("shop two".into()));
    assert_eq!(
        sent(&desk),
        vec![
            ("Allow".to_string(), Some("q1".to_string()), act(Act::Verdict { allow: true }), vec![]),
            ("Leave".to_string(), Some("q1".to_string()), act(Act::Option { index: 1 }), vec![]),
            ("Keep going.".to_string(), None, act(Act::Control { control: airband_core::routing::Control::Poke }), vec![]),
            ("New agent in shop".to_string(), None, act(Act::Launch(airband_core::radio::Launch { project: "shop".into(), ..Default::default() })), vec![]),
            ("ship it".to_string(), None, String::new(), vec!["shop two".to_string()]),
        ]
    );
}

#[test]
fn the_connection_record_he_can_copy_holds_nothing_said_and_nothing_secret() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let dir = std::env::temp_dir().join(format!("airband-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // paired through a relay nobody is at, so what he says fails and is written down
    let secret = "ef".repeat(32);
    let link = format!("airband://pair?relay=http%3A%2F%2F127.0.0.1%3A9&secret={secret}");
    let radio = Radio::start(link.clone(), dir.display().to_string(), Arc::new(Quiet)).unwrap();
    radio.queue("the door code is 4471".into(), None, None, None);
    for _ in 0..200 {
        if radio.link_trail().iter().any(|l| l.contains("/v1/radio/utterance")) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    radio.seen(vec!["m-private-id".into()]);
    let mut state = State { stations: vec![station("mini", 1, &[], vec![msg("an agent's own words", 1, "shop two", "shop", false)])], ..State::default() };
    state.stations[0].radio = radio.clone();
    state.stations[0].link = link;
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: dir.display().to_string() };
    let record = desk.diagnostics();
    // what is in it: which computer, each try, the way it went and how it ended, and what was told in passing and lost
    assert!(record.starts_with("airband ") && record.contains("\nmini\n"), "{record}");
    assert!(record.contains("POST /v1/radio/utterance by relay: there was no connection to the relay"), "{record}");
    assert!(record.contains("told in passing and not delivered, since the app was opened: heard 1"), "{record}");
    // what is not: anything he or an agent said, an id of a message, the pairing's secret or anything made
    // from it, or an address
    let keys = Keys::derive(&secret).unwrap();
    for private in ["door code", "4471", "an agent's own words", "m-private-id", secret.as_str(), keys.channel.as_str(), keys.radio_token.as_str(), keys.pull_token.as_str(), "127.0.0.1", "http"] {
        assert!(!record.contains(private), "the record holds {private:?}: {record}");
    }
    radio.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_he_is_not_looking_at_stays_unread_and_is_counted() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let dir = std::env::temp_dir().join(format!("airband-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(two()), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: dir.display().to_string() };
    let counts = |v: &DeskView| (v.unread, v.rooms.iter().map(|r| (r.name.clone(), r.unread)).collect::<Vec<_>>());
    // looking at one team: what it said is read, and the other two teams' one thing each is not
    let v = desk.view(Some("shop".into()), None);
    assert_eq!(counts(&v), (2, vec![("backoffice".into(), 1), ("ff".into(), 1), ("shop".into(), 0)]));
    assert_eq!(v.rooms[0].members[0].unread, 1);
    // something new from the team in front of him is read as it comes; from another, it is one more
    {
        let mut state = desk.shared.state.lock().unwrap();
        state.stations[0].messages.push(msg("r2", 5, "shop two", "shop", false));
        state.stations[1].messages.push(msg("r3", 6, "ff one", "ff", false));
    }
    assert_eq!(counts(&desk.view(Some("shop".into()), None)).0, 3);
    // with the chat not on screen nothing is read: one agent of a team opened behind a sheet clears nothing
    host.0.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(desk.view(Some("ff".into()), Some("ff one".into())).unread, 3);
    host.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(counts(&desk.view(Some("ff".into()), Some("ff one".into()))), (1, vec![("backoffice".into(), 1), ("ff".into(), 0), ("shop".into(), 0)]));
    // what was read is kept on the device: a desk opened again on the same folder starts from there
    assert_eq!(ReadMarks::load(&dir.display().to_string()).upto.get("ff one"), Some(&6));
    // everything at once
    assert_eq!(desk.view(None, None).unread, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_notification_is_opened_by_the_pairing_that_sealed_it_and_no_other() {
    use airband_core::radio::Notice;
    let link = |secret: &str| Pairing::Relay { relay: "https://relay.example".into(), secret: secret.into() }.link();
    let (mini, laptop, stranger) = (airband_core::relay::new_secret(), airband_core::relay::new_secret(), airband_core::relay::new_secret());
    let keys = Keys::derive(&laptop).unwrap();
    let seal = |n: &Notice| airband_core::audio::base64_encode(&keys.seal(Direction::Push, &serde_json::to_vec(n).unwrap()));
    let question = Notice { id: "m1".into(), from: "shop three".into(), group: "shop".into(), colour: 2, station: "laptop".into(), text: "Rewrite or leave?".into(), kind: Some(Kind::NeedsYou), options: vec!["Rewrite".into(), "Leave".into()], permission: false };
    let got = open_notice(vec![link(&mini), link(&laptop)], seal(&question)).unwrap();
    assert_eq!((got.title.as_str(), got.subtitle.as_str(), got.body.as_str(), got.badge.as_str(), got.colour), ("shop 3", "laptop", "Rewrite or leave?", "S3", 2));
    assert_eq!((got.station.as_str(), got.message.as_str(), got.room.as_str(), got.agent.as_str(), got.question), (keys.channel.as_str(), "m1", "shop", "shop 3", true));
    assert_eq!(got.options, ["Rewrite", "Leave"]);
    // one computer: its name says nothing he does not know
    assert_eq!(open_notice(vec![link(&laptop)], seal(&question)).unwrap().subtitle, "");
    // a permission card offers its own two answers, not the tower's list
    let card = open_notice(vec![link(&laptop)], seal(&Notice { permission: true, options: vec!["Allow".into(), "Deny".into()], ..question.clone() })).unwrap();
    assert!(card.permission && card.options.is_empty());
    // not by another pairing, not as an ordinary answer from the tower, not when it is not base64 at all
    assert!(open_notice(vec![link(&mini), link(&stranger)], seal(&question)).is_none());
    let answer = airband_core::audio::base64_encode(&keys.seal(Direction::ToRadio, &serde_json::to_vec(&question).unwrap()));
    assert!(open_notice(vec![link(&laptop)], answer).is_none());
    assert!(open_notice(vec![link(&laptop)], "not sealed".into()).is_none());
    assert_eq!(station_key(&link(&laptop)), Some(keys.channel.clone()));
}

#[test]
fn a_message_goes_to_the_computer_that_has_who_it_is_for() {
    let state = two();
    // the message he is answering
    assert_eq!(pick_station(&state, "yes", Some("q2"), None, None), 1);
    // a tag, in every form the tower accepts
    assert_eq!(pick_station(&state, "@ff1 stop", None, None, None), 1);
    assert_eq!(pick_station(&state, "@ffone stop", None, None, None), 1);
    assert_eq!(pick_station(&state, "@shop2 go", None, None, None), 0);
    assert_eq!(pick_station(&state, "@backoffice ship it", None, None, None), 0);
    // the room or the agent he is looking at
    assert_eq!(pick_station(&state, "go", None, Some("ff"), None), 1);
    assert_eq!(pick_station(&state, "go", None, None, Some("backoffice login fix")), 0);
    // nothing says: whoever spoke last
    assert_eq!(pick_station(&state, "go", None, None, None), 1);
}

#[test]
fn waiting_is_what_is_open_and_not_just_answered() {
    let mut state = two();
    assert!(state.is_open("q1") && state.is_open("q2") && !state.is_open("d1"));
    state.answered.insert("q1".into());
    assert!(!state.is_open("q1"));
    // labelled with its computer, because there are two
    assert_eq!(state.merged().iter().map(|m| (m.id.as_str(), m.station.as_str())).collect::<Vec<_>>(), [("q1", "mini"), ("d1", "mini"), ("q2", "macbook")]);
}
