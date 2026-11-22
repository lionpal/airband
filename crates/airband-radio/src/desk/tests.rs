use super::*;
use airband_core::radio::SetReads;

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

/// What the desk shows once he looks at a team, an agent, or everyone.
pub(super) fn looked(desk: &Desk, room: Option<String>, agent: Option<String>) -> DeskView {
    desk.look(room, agent);
    // as the app draws a screen: what it shows is marked read, then it is shown
    desk.shown();
    desk.view()
}
fn msg(id: &str, seq: u64, from: &str, group: &str, needs: bool) -> ChatMessage {
    ChatMessage { seq, id: id.into(), at: seq, his: from == "you", from: from.into(), group: group.into(), team: group.into(), room: String::new(), to: Vec::new(), to_agents: Vec::new(), needs_you: needs, fyi: false, unjudged: String::new(), stalled: false, text: id.into(), long: false, queued: false, seen: 0, options: Vec::new(), recommended: None, permission: false, pictures: Vec::new(), links: Vec::new(), agent: Some(from.into()), station: String::new(), computer: String::new(), open: needs, badge: String::new(), colour: None, read: None, own_session: false, not_read: String::new() }
}
fn station(name: &str, port: u16, agents: &[(&str, &str)], messages: Vec<ChatMessage>) -> Computer {
    let link = format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t");
    let radio = Radio::start(link.clone(), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    let mut station = Computer {
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
        agents: agents.iter().map(|(c, g)| (c, g, c.strip_prefix(g).unwrap_or(c).trim().to_string())).map(|(c, g, number)| ChatAgent { id: c.to_string(), callsign: c.to_string(), badge: airband_core::routing::Callsign { project: g.to_string(), number: number.clone(), named: false }.badge(), group: g.to_string(), team: g.to_string(), room: String::new(), number: number.clone(), number_kept: number, named: false, harness: "h".into(), waiting: false, state: AgentState::Working, subagents: 0, doing: None, held: false, terminal: false, ends: false, model: None, effort: None }).collect(),
        open: messages.iter().filter(|m| m.needs_you).map(|m| m.id.clone()).collect(),
        epoch: 0,
        was_waiting: Vec::new(),
        landing: Vec::new(),
        quick: Vec::new(),
        messages,
    };
    station.settle_rooms();
    station
}
/// The room on his screen of a team (by its id) on whichever computer has it.
fn room(state: &State, team: &str) -> String {
    state.computers.iter().flat_map(|s| s.agents.iter()).find(|a| a.team == team).map(|a| a.room.clone()).unwrap()
}
fn two() -> State {
    State {
        computers: vec![
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
    let state = State { computers: vec![station("mini", 1, &[("shop two", "shop")], Vec::new()), station("macbook", 2, &[("shop one", "shop")], Vec::new())], ..State::default() };
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() };
    // a team of the same name on each is two rooms: a new agent goes to the computer of the room it was asked in
    let (on_mini, on_macbook) = { let s = desk.shared.state.lock().unwrap(); (s.computers[0].agents[0].room.clone(), s.computers[1].agents[0].room.clone()) };
    assert_ne!(on_mini, on_macbook);
    desk.new_agent(on_macbook, None);
    // a room no computer has, or a bare team name: nothing happens, and nothing is said
    desk.new_agent("garden".into(), None);
    desk.new_agent("shop".into(), None);
    let unsent = |i: usize| desk.shared.state.lock().unwrap().computers[i].radio.unsent().len();
    assert_eq!((unsent(0), unsent(1)), (0, 1));
    assert!(host.0.lock().unwrap().is_empty());
    // a tap or an answer to a message no computer has now goes to none of them, and he is told why
    desk.tap("gone".into(), Tapped::Allow);
    desk.answer("gone".into(), "yes".into());
    assert_eq!((unsent(0), unsent(1)), (0, 1));
    assert_eq!(*host.0.lock().unwrap(), ["That computer is no longer paired.", "That computer is no longer paired."]);
    // the same for anything else asked of a computer he did not name
    assert!(matches!(desk.station_radio(&StationId::default()), Err(RadioError::Told { .. })));
    assert!(matches!(desk.station_radio(&StationId::from("laptop")), Err(RadioError::Offline)));
}

#[test]
fn a_button_is_sent_as_what_it_stands_for_and_never_as_words_to_be_read() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let mut state = two();
    state.computers[0].messages[0].options = vec!["Rewrite".into(), "Leave".into()];
    state.computers[0].agents[0].state = AgentState::Stalled;
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() };
    let sent = |desk: &Desk| desk.shared.state.lock().unwrap().computers[0].radio.unsent().into_iter().map(|o| (o.text, o.re, o.act, o.to)).collect::<Vec<_>>();
    let act = Some;
    // a yes to a request, an answer he tapped, and one the message does not have (nothing goes)
    desk.tap("q1".into(), Tapped::Allow);
    desk.tap("q1".into(), Tapped::Option { index: 1 });
    desk.tap("q1".into(), Tapped::Option { index: 7 });
    // a poke, and a new agent
    desk.poke(None);
    let shop = room(&desk.shared.state.lock().unwrap(), "shop");
    desk.new_agent(shop.clone(), None);
    // words to the agent he is looking at go to it by its id, with no tag written into them
    desk.send("ship it".into(), None, Some(shop), Some("shop two".into()));
    assert_eq!(
        sent(&desk),
        vec![
            ("Allow".to_string(), Some("q1".to_string()), act(Act::Verdict { allow: true }), vec![]),
            ("Leave".to_string(), Some("q1".to_string()), act(Act::Option { index: 1 }), vec![]),
            ("Keep going.".to_string(), None, act(Act::Control { control: airband_core::routing::Control::Poke }), vec![]),
            ("New agent in shop".to_string(), None, act(Act::Launch(airband_core::radio::Launch { team: "shop".into(), ..Default::default() })), vec![]),
            ("ship it".to_string(), None, None, vec!["shop two".to_string()]),
        ]
    );
}

#[test]
fn words_that_only_start_with_an_at_sign_address_nobody() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(two()), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() };
    // no agent is called home: the message he picked still says who it is for
    desk.send("@home is broken".into(), Some("q2".into()), None, None);
    let sent: Vec<(String, Option<String>)> = desk.shared.state.lock().unwrap().computers[1].radio.unsent().into_iter().map(|o| (o.text, o.re)).collect();
    assert_eq!(sent, [("@home is broken".to_string(), Some("q2".to_string()))]);
}

#[test]
fn a_message_sent_again_goes_to_the_agent_picked_by_its_id() {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let mut state = two();
    // an agent's id is not its callsign
    for a in &mut state.computers[0].agents {
        a.id = format!("id-{}", a.id);
    }
    let picked = state.computers[0].agents[1].id.clone();
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() };
    desk.resend("q1".into(), picked.clone());
    let sent: Vec<Vec<String>> = desk.shared.state.lock().unwrap().computers[0].radio.unsent().into_iter().map(|o| o.to).collect();
    assert_eq!(sent, vec![vec![picked]]);
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
    let mut state = State { computers: vec![station("mini", 1, &[], vec![msg("an agent's own words", 1, "shop two", "shop", false)])], ..State::default() };
    state.computers[0].radio = radio.clone();
    state.computers[0].link = link;
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: dir.display().to_string() };
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
    let mut read = ReadMarks::load(&dir.display().to_string());
    read.since = 0;
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(State { read, ..two() }), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: dir.display().to_string() };
    let counts = |v: &DeskView| (v.unread, v.rooms.iter().map(|r| (r.name.clone(), r.unread)).collect::<Vec<_>>());
    // looking at one team: what it said is read, and the other two teams' one thing each is not
    let (shop, ff) = { let s = desk.shared.state.lock().unwrap(); (room(&s, "shop"), room(&s, "ff")) };
    let v = looked(&desk, Some(shop.clone()), None);
    assert_eq!(counts(&v), (2, vec![("backoffice".into(), 1), ("ff".into(), 1), ("shop".into(), 0)]));
    assert_eq!(v.rooms[0].members[0].unread, 1);
    // something new from the team in front of him is read as it comes; from another, it is one more
    {
        let mut state = desk.shared.state.lock().unwrap();
        state.computers[0].messages.push(msg("r2", 5, "shop two", "shop", false));
        state.computers[1].messages.push(msg("r3", 6, "ff one", "ff", false));
        state.computers.iter_mut().for_each(Computer::settle_rooms);
    }
    assert_eq!(counts(&looked(&desk, Some(shop), None)).0, 3);
    // with the chat not on screen nothing is read: one agent of a team opened behind a sheet clears nothing
    host.0.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(looked(&desk, Some(ff.clone()), Some("ff one".into())).unread, 3);
    host.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(counts(&looked(&desk, Some(ff.clone()), Some("ff one".into()))), (1, vec![("backoffice".into(), 1), ("ff".into(), 0), ("shop".into(), 0)]));
    // what was read is kept on the device: a desk opened again on the same folder starts from there
    assert_eq!(ReadMarks::load(&dir.display().to_string()).upto.get("ff one"), Some(&6));
    // everything at once
    assert_eq!(looked(&desk, None, None).unread, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_notification_is_opened_by_the_pairing_that_sealed_it_and_no_other() {
    use airband_core::radio::Notice;
    let link = |secret: &str| Pairing::Relay { relay: "https://relay.example".into(), secret: secret.into() }.link();
    let (mini, laptop, stranger) = (airband_core::relay::new_secret(), airband_core::relay::new_secret(), airband_core::relay::new_secret());
    let keys = Keys::derive(&laptop).unwrap();
    let seal = |n: &Notice| airband_core::encode::base64(&keys.seal(Direction::Push, &serde_json::to_vec(n).unwrap()));
    let question = Notice { id: "m1".into(), from: "shop 3".into(), agent: Some("a3".into()), badge: "S3".into(), group: "shop".into(), team: Some("/work/shop".into()), station: "laptop".into(), text: "Rewrite or leave?".into(), kind: Some(Kind::NeedsYou), options: vec!["Rewrite".into(), "Leave".into()], permission: false };
    let got = open_notice(vec![link(&mini), link(&laptop)], seal(&question), [(format!("{}//work/shop", keys.channel), 2)].into()).unwrap();
    assert_eq!((got.title.as_str(), got.subtitle.as_str(), got.body.as_str(), got.badge.as_str(), got.colour), ("shop 3", "laptop", "Rewrite or leave?", "S3", Some(2)));
    assert_eq!((got.station.as_str(), got.message.as_str(), got.room.as_str(), got.agent.as_str(), got.question), (keys.channel.as_str(), "m1", format!("{}//work/shop", keys.channel).as_str(), "a3", true));
    // a team the phone has given no colour yet is drawn with none, not the first colour
    assert_eq!(open_notice(vec![link(&laptop)], seal(&question), Default::default()).unwrap().colour, None);
    assert_eq!(got.options, ["Rewrite", "Leave"]);
    // one computer: its name says nothing he does not know
    assert_eq!(open_notice(vec![link(&laptop)], seal(&question), Default::default()).unwrap().subtitle, "");
    // a permission card offers its own two answers, not the tower's list
    let card = open_notice(vec![link(&laptop)], seal(&Notice { permission: true, options: vec!["Allow".into(), "Deny".into()], ..question.clone() }), Default::default()).unwrap();
    assert!(card.permission && card.options.is_empty());
    // not by another pairing, not as an ordinary answer from the tower, not when it is not base64 at all
    assert!(open_notice(vec![link(&mini), link(&stranger)], seal(&question), Default::default()).is_none());
    let answer = airband_core::encode::base64(&keys.seal(Direction::ToRadio, &serde_json::to_vec(&question).unwrap()));
    assert!(open_notice(vec![link(&laptop)], answer, Default::default()).is_none());
    assert!(open_notice(vec![link(&laptop)], "not sealed".into(), Default::default()).is_none());
    assert_eq!(station_id(&link(&laptop)), keys.channel);
}

#[test]
fn a_message_goes_to_the_computer_that_has_who_it_is_for() {
    let mut state = two();
    // the message he is answering
    assert_eq!(pick_station(&state, "yes", Some("q2"), None, None), Some(state.computers[1].id()));
    // a tag, in every form the tower accepts
    assert_eq!(pick_station(&state, "@ff1 stop", None, None, None), Some(state.computers[1].id()));
    assert_eq!(pick_station(&state, "@ffone stop", None, None, None), Some(state.computers[1].id()));
    assert_eq!(pick_station(&state, "@shop2 go", None, None, None), Some(state.computers[0].id()));
    assert_eq!(pick_station(&state, "@backoffice ship it", None, None, None), Some(state.computers[0].id()));
    // the room or the agent he is looking at
    assert_eq!(pick_station(&state, "go", None, Some(&room(&state, "ff")), None), Some(state.computers[1].id()));
    assert_eq!(pick_station(&state, "go", None, None, Some("backoffice login fix")), Some(state.computers[0].id()));
    // nothing says, and he has not chosen a main computer: there is no answer, and he is asked
    assert_eq!(pick_station(&state, "go", None, None, None), None);
    state.read.main = Some(state.computers[1].id());
    assert_eq!(pick_station(&state, "go", None, None, None), Some(state.computers[1].id()));
}

#[test]
fn waiting_is_what_is_open_and_not_just_answered() {
    let mut state = two();
    assert!(state.is_open("q1") && state.is_open("q2") && !state.is_open("d1"));
    state.answered.insert("q1".into());
    assert!(!state.is_open("q1"));
    // labelled with its computer, by its id, because there are two
    let (mini, macbook) = (state.computers[0].id().to_string(), state.computers[1].id().to_string());
    assert_eq!(state.merged().iter().map(|m| (m.id.clone(), m.station.clone())).collect::<Vec<_>>(), [("q1".into(), mini.clone()), ("d1".into(), mini), ("q2".into(), macbook)]);
}

#[test]
fn teams_on_two_computers_get_different_colours_that_last() {
    let dir = std::env::temp_dir().join(format!("airband-colours-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut colours = super::colours::Colours::open(dir.to_str().unwrap(), 6);
    // a team called shop on each of two computers: two rooms, by computer and team id, each its own colour
    let _ = colours.see(["mini//w/shop".to_string(), "mini//w/shop".to_string()]);
    let _ = colours.see(["laptop//w/shop".to_string()]);
    assert_ne!(colours.of("mini//w/shop"), colours.of("laptop//w/shop"));
    // a room not seen yet has none
    assert_eq!(colours.of("laptop//w/garden"), None);
    // renaming a team changes neither its room nor its colour, which last
    assert_eq!(super::colours::Colours::open(dir.to_str().unwrap(), 6).of("laptop//w/shop"), colours.of("laptop//w/shop"));
    assert_eq!(super::colours::Colours::open(dir.to_str().unwrap(), 6).of("mini//w/shop"), colours.of("mini//w/shop"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn desk_of(state: State) -> Desk {
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() }
}

#[test]
fn words_sent_again_go_to_the_agent_he_picked_whatever_tag_they_start_with() {
    let mut state = two();
    // what he first said tagged the agent on the other computer
    state.computers[0].messages[0].text = "@ff1 stop that".into();
    let picked = state.computers[0].agents[1].id.clone();
    let desk = desk_of(state);
    desk.resend("q1".into(), picked.clone());
    let state = desk.shared.state.lock().unwrap();
    let here: Vec<Vec<String>> = state.computers[0].radio.unsent().into_iter().map(|o| o.to).collect();
    assert_eq!(here, vec![vec![picked]]);
    assert!(state.computers[1].radio.unsent().is_empty());
}

#[test]
fn unplaced_words_he_places_keep_the_files_they_took() {
    let mut state = two();
    let mut message = Outgoing::new("u1".into(), "look at this".into(), None, None, None, vec!["f1".into()], Vec::new());
    message.label = None;
    state.asking = Some(Asking { message, guesses: Vec::new(), at: state.computers[0].id() });
    let agent = state.computers[0].agents[0].id.clone();
    let desk = desk_of(state);
    desk.choose(agent);
    let sent: Vec<Vec<String>> = desk.shared.state.lock().unwrap().computers[0].radio.unsent().into_iter().map(|o| o.attachments).collect();
    assert_eq!(sent, vec![vec!["f1".to_string()]]);
}

#[test]
fn a_tapped_answer_kept_for_later_is_still_what_the_button_stood_for() {
    // nobody is at this address: the answer cannot go now and is kept
    let radio = Radio::start("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".into(), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    let act = Act::Option { index: 1 };
    assert!(matches!(radio.answer_now("Leave".into(), "q1".into(), Some(act.clone())), AnswerOutcome::KeptForLater { .. }));
    let kept: Vec<(Option<String>, Option<Act>)> = radio.unsent().into_iter().map(|o| (o.re, o.act)).collect();
    assert_eq!(kept, [(Some("q1".to_string()), Some(act))]);
}

/// A computer on this machine that answers every request with 200 and `body`. Its port.
fn tower_answering(body: &'static str) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 65536];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
        }
    });
    port
}
fn temp_dir(what: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("airband-{what}-{}", random_hex(4)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn kept_reads(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(dir.join("choices.json")).unwrap()).unwrap()["reads"].clone()
}

#[test]
fn a_summary_choice_made_offline_waits_for_every_computer() {
    let dir = temp_dir("reads");
    std::fs::write(dir.join("choices.json"), r#"{"reads":{"rev":1,"wanted":{"id":"anthropic","offer":"haiku","his":true}}}"#).unwrap();
    let state = State { computers: vec![station("mini", tower_answering(r#"{"reads":"Haiku","offers":[{"provider":"anthropic","offer":"haiku","label":"Haiku","key":true,"has_key":true}]}"#), &[], vec![]), station("laptop", 9, &[], vec![])], ..State::default() };
    let (mini, epoch) = (state.computers[0].id(), state.computers[0].epoch);
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let shared = Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open(dir.to_str().unwrap(), HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None };
    crate::choices::tell_reads(&shared, &mini, epoch, true);
    // the first computer has it, and it still waits for the other
    let ids: Vec<String> = shared.state.lock().unwrap().computers.iter().map(|s| s.id().to_string()).collect();
    let per = kept_reads(&dir)["per"].clone();
    assert_eq!((per[ids[0].as_str()]["state"].clone(), per.get(&ids[1]).cloned()), (serde_json::json!("done"), None), "{per}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn when_one_computer_refuses_the_others_are_still_asked() {
    let laptop = station("laptop", tower_saying(409, r#"{"refusal":"told","reason":"That service is not offered here."}"#), &[], vec![]);
    let mini = station("mini", tower_answering(r#"{"reads":"Haiku"}"#), &[], vec![]);
    let wanted = SetReads { kind: "anthropic".into(), offer: "haiku".into(), key: String::new(), set: Default::default() };
    let targets = vec![(laptop.id(), laptop.radio.clone()), (mini.id(), mini.radio.clone())];
    let per = super::fan_out(targets, |radio| super::Reach::of(radio.set_reads(&wanted)));
    assert_eq!(per[&mini.id()], super::Reach::Done(()));
    assert_eq!(per[&laptop.id()], super::Reach::Refused("That service is not offered here.".into()));
}

/// A desk with these computers, keeping its choices in `dir`, where his summary service is offered with its
/// key kept on the main computer.
fn desk_choosing(dir: &std::path::Path, computers: Vec<Computer>, reads: serde_json::Value) -> Desk {
    let offered = serde_json::json!({ "reads": reads, "offered": { "offers": [{ "provider": "anthropic", "offer": "haiku", "label": "Haiku", "key": true, "has_key": true, "key_name": "API key", "preferred": true }] } });
    std::fs::write(dir.join("choices.json"), offered.to_string()).unwrap();
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk { shared: Arc::new(Shared { state: Mutex::new(State { computers, ..State::default() }), floor: Floor::default(), host, chooser: crate::choices::Chooser::open(dir.to_str().unwrap(), HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() }
}

#[test]
fn a_summary_choice_some_computers_took_is_remembered_as_taken_there() {
    let dir = temp_dir("partly");
    let mini = station("mini", tower_answering(r#"{"reads":"Haiku","offers":[{"provider":"anthropic","offer":"haiku","label":"Haiku","key":true,"has_key":true}]}"#), &[], vec![]);
    let laptop = station("laptop", tower_saying(409, r#"{"refusal":"told","reason":"Enter your API key."}"#), &[], vec![]);
    let away = station("away", 9, &[], vec![]);
    let ids = [mini.id(), laptop.id()];
    let desk = desk_choosing(&dir, vec![mini, laptop, away], serde_json::Value::Null);
    desk.shared.state.lock().unwrap().computers[2].online = false;
    desk.shared.state.lock().unwrap().read.main = Some(ids[0].clone());
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    let chose = desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false);
    assert!(chose.is_ok(), "{chose:?}");
    // mini has it, laptop said no and is not asked again, and it still waits for the one away
    let per = kept_reads(&dir)["per"].clone();
    assert_eq!(per[ids[0].as_str()], serde_json::json!({ "state": "done" }), "{per}");
    assert_eq!(per[ids[1].as_str()], serde_json::json!({ "state": "refused", "reason": crate::choices::NO_KEY }), "{per}");
    // and he sees laptop's no, by its name, with why
    let view = looked(&desk, None, None);
    let lines: Vec<(String, DeskDelivery)> = view.intents.iter().filter(|i| i.kind == IntentKind::Reads).flat_map(|i| i.computers.iter().map(|c| (c.name.clone(), c.state.clone()))).collect();
    assert_eq!(lines, [("laptop".to_string(), DeskDelivery::Refused { reason: crate::choices::NO_KEY.into() }), ("away".to_string(), DeskDelivery::Waiting)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_computer_that_refuses_a_waiting_summary_choice_does_not_cancel_it_for_the_others() {
    let dir = temp_dir("refused-one");
    let laptop = station("laptop", tower_saying(409, r#"{"refusal":"told","reason":"Enter your API key."}"#), &[], vec![]);
    let mini = station("mini", 9, &[], vec![]);
    let (laptop_id, laptop_epoch, mini_id) = (laptop.id(), laptop.epoch, mini.id());
    let desk = desk_choosing(&dir, vec![laptop, mini], serde_json::json!({ "rev": 1, "wanted": { "id": "anthropic", "offer": "haiku", "his": true } }));
    crate::choices::tell_reads(&desk.shared, &laptop_id, laptop_epoch, true);
    // still waiting for mini
    let reads = kept_reads(&dir);
    assert_eq!(reads["wanted"]["id"], "anthropic", "{reads}");
    assert!(reads["per"].get(mini_id.as_str()).is_none(), "{reads}");
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn a_summary_service_whose_key_his_computer_keeps_is_chosen_with_no_key_on_the_phone() {
    let dir = temp_dir("kept-there");
    let offered = serde_json::json!({ "offered": { "offers": [{ "provider": "anthropic", "offer": "haiku", "label": "Haiku", "key": true, "has_key": true, "key_name": "API key", "preferred": true }] } });
    std::fs::write(dir.join("choices.json"), offered.to_string()).unwrap();
    let state = State { computers: vec![station("mini", tower_answering(r#"{"reads":"Haiku","offers":[{"provider":"anthropic","offer":"haiku","label":"Haiku","key":true,"has_key":true}]}"#), &[], vec![])], ..State::default() };
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open(dir.to_str().unwrap(), HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: String::new() };
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    let chose = desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false);
    assert!(chose.is_ok(), "{chose:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_team_colour_counts_only_teams_on_his_screen() {
    let dir = temp_dir("colours");
    // six rooms kept from before, gone now, each with its own colour but the last
    let gone: std::collections::BTreeMap<String, u8> = (0..5u8).map(|c| (format!("old//w/{c}"), c)).collect();
    std::fs::write(dir.join("colours.json"), serde_json::to_string(&gone).unwrap()).unwrap();
    let mut colours = super::colours::Colours::open(dir.to_str().unwrap(), 6);
    let _ = colours.see(["mini//w/shop".to_string()]);
    let _ = colours.see(["mini//w/garden".to_string()]);
    // nothing on his screen has a colour yet: the first two are the first two colours
    assert_eq!((colours.of("mini//w/shop"), colours.of("mini//w/garden")), (Some(0), Some(1)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lines_kept_on_the_phone_are_in_their_room_before_any_computer_answers() {
    let dir = temp_dir("kept-lines");
    // nobody is at this address
    let link = "airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".to_string();
    let saved = serde_json::json!({ "history": [{ "seq": 1, "id": "m1", "at": 1, "from": "shop two", "group": "shop", "team": "shop", "text": "done" }], "outbox": [], "after": 1 });
    let folder = dir.join(super::folder(&link));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("radio.json"), saved.to_string()).unwrap();
    let desk = Desk::alone(dir.to_str().unwrap(), Arc::new(Watching(std::sync::atomic::AtomicBool::new(true))));
    desk.set_links(vec![link]);
    let state = desk.shared.state.lock().unwrap();
    let rooms: Vec<String> = state.computers[0].messages.iter().map(|m| m.room.clone()).collect();
    assert_eq!(rooms, [room_id(&state.computers[0].id(), "shop")]);
    drop(state);
    desk.shared.state.lock().unwrap().computers.iter().for_each(|s| s.radio.stop());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A radio host that keeps what it was told.
struct Telling(Mutex<Vec<String>>);
impl RadioHost for Telling {
    fn told(&self, text: String) {
        self.0.lock().unwrap().push(text);
    }
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

#[test]
fn a_message_that_cannot_be_fetched_to_read_aloud_says_why() {
    let host = Arc::new(Telling(Mutex::new(Vec::new())));
    let radio = Radio::start("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".into(), String::new(), host.clone()).unwrap();
    radio.stop();
    // a voice is chosen, so it is the fetching that fails
    radio.senses.mouth.set(Some(crate::mouth::Own::Service(crate::services::service_of("gemini", "flash-lite-tts", "k", HashMap::new()))));
    assert!(!radio.read_question("m1"));
    assert_eq!(host.0.lock().unwrap().as_slice(), ["That could not be read aloud: cannot reach your computer."]);
}

// Unix only: it makes the folder unwritable by its permission bits
#[cfg(unix)]
#[test]
fn what_he_said_that_the_phone_could_not_keep_is_told_once() {
    let dir = temp_dir("unkept");
    let host = Arc::new(Telling(Mutex::new(Vec::new())));
    let radio = Radio::start("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".into(), dir.display().to_string(), host.clone()).unwrap();
    radio.stop();
    // the folder the chat is kept in can no longer be written
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    radio.queue("one".into(), None, None, None);
    radio.queue("two".into(), None, None, None);
    let told: Vec<String> = host.0.lock().unwrap().iter().filter(|t| t.starts_with("This phone could not keep it")).cloned().collect();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(told.len(), 1, "{told:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unnamed_computer_can_be_chosen_as_main_by_its_id() {
    let mut state = two();
    // neither computer has said its name
    state.computers.iter_mut().for_each(|s| s.name.clear());
    let desk = desk_of(state);
    let shown = looked(&desk, None, None).stations;
    // never by its place in the list: by where its pairing link reaches it
    assert_eq!(shown.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["127.0.0.1", "127.0.0.1"]);
    desk.set_main_computer(shown[1].id.clone());
    let main = desk.shared.state.lock().unwrap().main().map(Computer::id);
    assert_eq!(main, Some(StationId::from(shown[1].id.as_str())));
    assert_eq!(looked(&desk, None, None).main_computer, Some(shown[1].id.clone()));
}

#[test]
fn what_he_attached_and_the_question_he_was_asked_outlive_a_reconnect() {
    let desk = desk_of(State::default());
    desk.set_links(vec!["airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".into()]);
    let radio = desk.shared.state.lock().unwrap().computers[0].radio.clone();
    radio.arm("f1".into());
    radio.session.asked(&Reply { question: Some("q1".into()), ..Reply::default() });
    desk.reconnect();
    let radio = desk.shared.state.lock().unwrap().computers[0].radio.clone();
    assert_eq!(radio.armed(), ["f1"]);
    assert_eq!(radio.session.take_question().as_deref(), Some("q1"));
    desk.stop();
}

#[test]
fn what_he_sent_that_has_not_gone_is_still_shown_after_the_app_opens_again() {
    let dir = temp_dir("unsent");
    let link = "airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t".to_string();
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    let first = Desk::open(dir.display().to_string(), host.clone());
    first.set_links(vec![link.clone()]);
    first.send("hello".into(), None, None, None);
    first.stop();
    let again = Desk::open(dir.display().to_string(), host);
    again.set_links(vec![link]);
    let shown: Vec<String> = looked(&again, None, None).sending.into_iter().map(|o| o.shown).collect();
    again.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(shown, ["hello"]);
}

#[test]
fn files_on_a_computer_he_unpaired_are_not_sent_with_words_to_the_one_now_in_its_place() {
    let state = two();
    let mini = state.computers[0].id();
    let macbook_link = state.computers[1].link.clone();
    let desk = desk_of(state);
    desk.shared.state.lock().unwrap().uploads.push(Upload { shown: DeskUpload { id: "f1".into(), name: "a.png".into(), sent: 1, total: 1, state: UploadState::Ready }, station: mini, dropped: Arc::default(), file_path: String::new(), name: "a.png".into() });
    // he unpairs the first computer: the second is now the only one
    desk.set_links(vec![macbook_link]);
    desk.send("hi".into(), None, None, None);
    let state = desk.shared.state.lock().unwrap();
    let sent: Vec<(String, Vec<String>)> = state.computers[0].radio.unsent().into_iter().map(|o| (o.text, o.attachments)).collect();
    assert_eq!(sent, [("hi".to_string(), Vec::<String>::new())]);
    assert!(state.uploads.is_empty());
}

#[test]
fn a_button_a_poke_or_words_sent_on_leave_his_files_and_his_question_for_his_own_next_words() {
    let state = two();
    let agent = state.computers[0].agents[0].id.clone();
    let radio = state.computers[0].radio.clone();
    let desk = desk_of(state);
    radio.arm("f1".into());
    radio.session.asked(&Reply { question: Some("q9".into()), ..Reply::default() });
    desk.tap("q1".into(), Tapped::Allow);
    desk.resend("d1".into(), agent.clone());
    desk.shared.state.lock().unwrap().computers[0].agents[0].state = AgentState::Stalled;
    desk.poke(None);
    assert!(radio.unsent().iter().all(|o| o.attachments.is_empty() && o.answers.is_none()), "{:?}", radio.unsent().iter().map(|o| (&o.text, &o.attachments, &o.answers)).collect::<Vec<_>>());
    // his own next words take them
    desk.send("here it is".into(), None, None, Some(agent));
    let last = radio.unsent().pop().unwrap();
    assert_eq!((last.attachments, last.answers), (vec!["f1".to_string()], Some("q9".to_string())));
}

#[test]
fn unplaced_words_he_places_still_answer_what_they_answered() {
    let mut state = two();
    let message = Outgoing { answers: Some("q7".into()), ..Outgoing::new("u1".into(), "yes do it".into(), None, None, None, Vec::new(), Vec::new()) };
    state.asking = Some(Asking { message, guesses: Vec::new(), at: state.computers[0].id() });
    let agent = state.computers[0].agents[0].id.clone();
    let radio = state.computers[0].radio.clone();
    let desk = desk_of(state);
    radio.session.asked(&Reply { question: Some("q8".into()), ..Reply::default() });
    desk.choose(agent);
    assert_eq!(radio.unsent().pop().unwrap().answers.as_deref(), Some("q7"));
    // the newer question is still his to answer
    assert_eq!(radio.session.take_question().as_deref(), Some("q8"));
}

#[test]
fn a_question_he_opens_the_microphone_for_still_waits_until_his_words_go() {
    let desk = desk_of(two());
    desk.talk(Some("q1".into()), None, None);
    desk.cancel_talk();
    assert!(!desk.shared.state.lock().unwrap().answered.contains("q1"));
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
fn radio_at(port: u16) -> Arc<Radio> {
    let radio = Radio::start(format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t"), String::new(), Arc::new(Quiet)).unwrap();
    radio.stop();
    radio
}

#[test]
fn a_refusal_without_words_and_one_for_another_version_are_each_told_as_what_they_are() {
    // a no with no reason is a refusal, never something that goes when it can
    let gone = radio_at(tower_saying(404, r#"{"refusal":"gone"}"#));
    assert!(matches!(gone.recent(), Err(RadioError::Refused)));
    // a computer that speaks another version: he is asked to update
    let other = radio_at(tower_saying(426, r#"{"refusal":"other_version","speaks":"airband.radio/9"}"#));
    assert!(matches!(other.recent(), Err(RadioError::Told { reason }) if reason.contains("another version of airband")));
    // a 4xx that carries no refusal is a no, not an answer and not trouble that passes
    let bare = radio_at(tower_saying(400, "{}"));
    assert!(matches!(bare.recent(), Err(RadioError::Refused)));
}

#[test]
fn trouble_of_his_computers_own_is_not_told_as_a_refusal() {
    let radio = radio_at(tower_saying(500, "{}"));
    let e = radio.recent().unwrap_err();
    assert!(matches!(&e, RadioError::Told { reason } if reason.contains("ran into trouble") && !reason.contains("refused")), "{e:?}");
    // a refusal is read by its tag, never by its status: "not now" is told with its reason
    let busy = radio_at(tower_saying(503, r#"{"refusal":"not_now","reason":"It is still starting."}"#));
    assert!(matches!(busy.recent(), Err(RadioError::Told { reason }) if reason == "It is still starting."));
}

#[test]
fn a_notification_answer_his_computer_cannot_take_just_now_is_kept_not_refused() {
    let radio = radio_at(tower_saying(503, r#"{"refusal":"not_now","reason":"It is still starting."}"#));
    let e = radio.answer_now("yes".into(), "q1".into(), None);
    assert!(matches!(&e, AnswerOutcome::KeptForLater { .. }), "{e:?}");
    assert_eq!(radio.unsent().len(), 1);
}

#[test]
fn every_request_straight_to_the_tower_says_the_protocol_it_speaks_and_nothing_waiting_is_null() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(String::new()));
    let kept = seen.clone();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).unwrap_or(0);
        *kept.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).to_lowercase();
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 4\r\nconnection: close\r\n\r\nnull");
    });
    let radio = radio_at(port);
    assert!(matches!(radio.link.next(false, ""), Ok(None)));
    let request = seen.lock().unwrap().clone();
    assert!(request.contains(&format!("{}: {}", airband_core::radio::SCHEMA_HEADER, airband_core::radio::SCHEMA)), "{request}");
}

#[test]
fn a_no_with_a_reason_is_told_with_its_reason_whatever_its_status() {
    let radio = radio_at(tower_saying(409, r#"{"refusal":"told","reason":"That folder is gone."}"#));
    assert!(matches!(radio.recent(), Err(RadioError::Told { reason }) if reason == "That folder is gone."));
}

#[test]
fn an_answer_kept_after_his_computer_had_trouble_is_told_as_kept() {
    let radio = radio_at(tower_saying(503, "{}"));
    let e = radio.answer_now("yes".into(), "q1".into(), None);
    assert!(matches!(&e, AnswerOutcome::KeptForLater { why } if why.ends_with("It is on this phone and goes when it can.")), "{e:?}");
    assert_eq!(radio.unsent().len(), 1);
}

#[test]
fn a_file_the_phone_cannot_read_is_not_told_as_his_computer_refusing() {
    let radio = radio_at(9);
    let e = radio.upload("f1", "/nonexistent/airband-test.mov", "a.mov", |_, _| true).unwrap_err();
    assert!(matches!(&e, RadioError::Told { reason } if reason == "This phone could not read that file."), "{e:?}");
}

#[test]
fn a_main_computer_the_phone_could_not_keep_is_told_once() {
    struct Noting(Mutex<Vec<String>>);
    impl DeskHost for Noting {
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
            false
        }
        fn device(&self) -> HostDevice {
            HostDevice::default()
        }
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }
    let dir = temp_dir("main-unkept");
    let host = Arc::new(Noting(Mutex::new(Vec::new())));
    let desk = Desk::open(dir.display().to_string(), host.clone());
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    desk.set_main_computer("c1".into());
    desk.set_main_computer("c2".into());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let told: Vec<String> = host.0.lock().unwrap().iter().filter(|t| t.ends_with("are not kept if the app closes.")).cloned().collect();
    assert_eq!(told.len(), 1, "{told:?}");
}

#[test]
fn a_message_names_its_computer_by_the_id_every_other_place_uses() {
    let mut state = two();
    // neither computer has said its name, and two computers could say the same one
    state.computers.iter_mut().for_each(|s| s.name.clear());
    let ids: Vec<String> = state.computers.iter().map(|s| s.id().to_string()).collect();
    let desk = desk_of(state);
    let view = looked(&desk, None, None);
    let q2 = view.feed.iter().find(|m| m.id == "q2").unwrap();
    assert_eq!(q2.station, ids[1]);
    assert!(view.stations.iter().any(|c| c.id == q2.station));
}

#[test]
fn two_computers_with_one_name_are_two_lines_on_the_connection_page() {
    let mut state = two();
    state.computers.iter_mut().for_each(|s| s.name = "Mac".into());
    let desk = desk_of(state);
    let reach = desk.reach();
    assert_ne!(reach[0].id, reach[1].id);
}

#[test]
fn an_account_on_two_computers_is_never_taken_for_one_by_what_it_is_called() {
    let read = station("mini", tower_answering(r#"{"schema":"","accounts":[{"id":"1","harness":"h","label":"me@example.com","limits":[{"name":"week","used":0.5}]}]}"#), &[], vec![]);
    let unread = station("laptop", tower_answering(r#"{"schema":"","accounts":[{"id":"7","harness":"h","label":"me@example.com","problem":"Could not read it."}]}"#), &[], vec![]);
    let desk = desk_of(State { computers: vec![read, unread], ..State::default() });
    let all = desk.accounts(false);
    let laptop = &all[1].accounts[0];
    assert!(laptop.limits.is_empty());
    assert_eq!(laptop.problem.as_deref(), Some("Could not read it."));
}

mod fake;
mod intents;
mod kept_files;
mod lifetimes;
mod scenarios;
mod sending;
