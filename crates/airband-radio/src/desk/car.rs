//! What the car's screen shows, decided here and only drawn by the host: his agents, waiting on him first;
//! his teams; his computers; one agent with its latest lines and what he can do about them; and the voice
//! screen's words and buttons.
//!
//! Apple's rules for an app whose main way in is the voice shape it. The voice screen comes up first. At
//! most three screens are up at once, the first one counted ([`CAR_DEPTH`]): the tabs, one agent pushed on
//! them, and the voice screen over either. Nothing is typed, and no line is long.
use super::*;

/// The most screens the car may show at once, the first one counted: Apple's limit for this kind of app.
pub const CAR_DEPTH: u32 = 3;
/// How many of an agent's latest lines its screen shows.
const LATEST: usize = 3;
/// How long a line on the car's screen may be, in characters.
const LINE: usize = 80;

/// A screen the car can show. The tabs are the first; an agent is pushed on them; the voice screen, or an
/// alert in its place, is put over whichever is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CarScreen {
    Tabs,
    Agent,
    Voice,
    /// What could not be done, said with one button: put up where the voice screen would have been.
    Alert,
}

/// Whether the car may show these screens, first to last: the tabs first and only first, then an agent at
/// most once, then the voice screen or an alert at most once and last; no more than [`CAR_DEPTH`] in all.
#[uniffi::export]
pub fn car_fits(screens: Vec<CarScreen>) -> bool {
    use CarScreen::*;
    let over = |s: &CarScreen| matches!(s, Voice | Alert);
    let shape = match screens.as_slice() {
        [Tabs] | [Tabs, Agent] => true,
        [Tabs, last] | [Tabs, Agent, last] => over(last),
        _ => false,
    };
    shape && screens.len() as u32 <= CAR_DEPTH
}

/// What a row or a button on the car does.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CarAct {
    /// Show this agent's screen.
    Open { agent: String },
    /// Put the voice screen up with the conversation on: for this agent when one is named, for everyone
    /// otherwise; what he says answers `re` when it is set. With `listen`, the microphone opens at once.
    Talk { agent: Option<String>, re: Option<String>, listen: bool },
    /// Put the voice screen up and read this message aloud: who said it, its short form, and the answers it offers.
    Read { agent: String, message: String },
    /// Answer this question with one of the answers it offers.
    Tap { message: String, tapped: Tapped },
}

impl CarAct {
    /// The screen it puts up, if any.
    pub(crate) fn opens(&self) -> Option<CarScreen> {
        match self {
            CarAct::Open { .. } => Some(CarScreen::Agent),
            CarAct::Talk { .. } | CarAct::Read { .. } => Some(CarScreen::Voice),
            CarAct::Tap { .. } => None,
        }
    }
}

/// The screen an act puts up, if any: what a host checks with [`car_fits`] before it shows it.
#[uniffi::export]
pub fn car_opens(act: CarAct) -> Option<CarScreen> {
    act.opens()
}

/// How an agent stands, for the car, in the order they are listed: the ones waiting on him first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum CarState {
    Waiting,
    Stopped,
    Working,
    Done,
}

impl CarState {
    fn of(state: AgentState) -> CarState {
        match state {
            AgentState::Asking => CarState::Waiting,
            AgentState::Stalled => CarState::Stopped,
            AgentState::Working => CarState::Working,
            AgentState::Idle => CarState::Done,
        }
    }
    fn words(self) -> &'static str {
        match self {
            CarState::Waiting => "Waiting on you",
            CarState::Stopped => "Stopped",
            CarState::Working => "Working",
            CarState::Done => "Done",
        }
    }
}

/// The picture beside a row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CarPicture {
    /// An agent's two characters on its team's colour, as the phone draws them.
    Badge { badge: String, colour: Option<u8> },
    /// A computer, drawn as it is on the phone.
    Computer { picture: ComputerPicture },
    /// A mark from the platform's symbol set, by its name.
    Symbol { name: String },
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarRow {
    pub text: String,
    /// A second line, only where the driver needs it.
    pub detail: Option<String>,
    pub picture: Option<CarPicture>,
    /// What a tap does; nothing for a row that only says something.
    pub act: Option<CarAct>,
}

/// Rows under one heading, or none.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarSection {
    pub header: Option<String>,
    pub rows: Vec<CarRow>,
}

/// Which tab it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CarTabKind {
    Agents,
    Teams,
    Computers,
}

/// One tab of the first screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarTab {
    pub kind: CarTabKind,
    pub title: String,
    /// Its mark, in the platform's symbol set.
    pub symbol: String,
    /// Something in it is waiting on him: the tab carries the small red mark.
    pub needs_you: bool,
    pub sections: Vec<CarSection>,
}

/// One agent's screen: what it asks, what he can do, and its latest lines.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarAgent {
    pub id: String,
    pub title: String,
    pub sections: Vec<CarSection>,
}

/// A state of the voice screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CarVoiceKind {
    Ready,
    Listening,
    Sending,
    Speaking,
    Offline,
}

/// What a button on the voice screen does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CarVoiceAction {
    /// The talk button: the first press opens the microphone and the second sends.
    Talk,
    /// Take back what he is saying.
    Cancel,
    /// Stop what is being read.
    Quiet,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarVoiceButton {
    pub action: CarVoiceAction,
    pub title: String,
    pub symbol: String,
}

/// One state of the voice screen: its words, longest first (the car shows the longest that fits), its mark,
/// and its buttons, two at most.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarVoiceState {
    pub kind: CarVoiceKind,
    pub titles: Vec<String>,
    pub symbol: String,
    /// Show the badge of the agent he is talking with in place of the mark.
    pub badge: bool,
    pub buttons: Vec<CarVoiceButton>,
}

/// The voice screen for the conversation that is on: with one agent, or with everyone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarVoice {
    /// The agent he is talking with, by id, when it is one.
    pub agent: Option<String>,
    /// What that agent is called; empty with everyone.
    pub with: String,
    pub states: Vec<CarVoiceState>,
    /// The words on the button that ends the conversation.
    pub end: String,
}

/// How much the car can show, as it says: rows in a list, sections in a list, and tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CarRoom {
    pub rows: u32,
    pub sections: u32,
    pub tabs: u32,
}

/// Everything the car's screens show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarView {
    pub paired: bool,
    /// What happens when the car connects: the voice screen, ready to talk. Nothing while he is not paired.
    pub launch: Option<CarAct>,
    pub tabs: Vec<CarTab>,
    /// The agent asked for, while it is still there.
    pub agent: Option<CarAgent>,
    pub voice: CarVoice,
}

/// The voice screen state to show for where the conversation is.
#[uniffi::export]
pub fn car_voice_kind(status: RadioStatus, mic_open: bool) -> CarVoiceKind {
    match status {
        RadioStatus::Offline => CarVoiceKind::Offline,
        RadioStatus::Listening => CarVoiceKind::Listening,
        RadioStatus::Sending => CarVoiceKind::Sending,
        RadioStatus::Speaking { .. } => CarVoiceKind::Speaking,
        RadioStatus::Idle if mic_open => CarVoiceKind::Listening,
        RadioStatus::Idle => CarVoiceKind::Ready,
    }
}

#[uniffi::export]
impl Desk {
    /// What the car shows, with `agent`'s screen when one is asked for, fitted to what the car can show.
    /// Changes nothing: nothing is marked read.
    pub fn car(&self, agent: Option<String>, room: CarRoom) -> CarView {
        let view = self.view();
        let (said, computers) = {
            let state = self.shared.state.lock().unwrap();
            let said: Vec<ChatMessage> = state.merged().into_iter().map(|m| ChatMessage { open: state.is_open(&m.id), ..m }).collect();
            let computers: Vec<(DeskComputer, bool)> = state.computers.iter().map(|c| (state.shown(c), c.online)).collect();
            (said, computers)
        };
        car_view(&view, &said, &computers, agent.as_deref(), room)
    }
}

/// The car's view, from the desk's view, every line said (oldest first), and each computer with whether it
/// can be reached.
pub(crate) fn car_view(view: &DeskView, said: &[ChatMessage], computers: &[(DeskComputer, bool)], agent: Option<&str>, room: CarRoom) -> CarView {
    let voice = voice(view);
    if !view.paired {
        let row = CarRow { text: "Not paired with a computer yet".into(), detail: None, picture: None, act: None };
        let tab = CarTab { kind: CarTabKind::Agents, title: "Agents".into(), symbol: "person.2.fill".into(), needs_you: false, sections: vec![CarSection { header: None, rows: vec![row] }] };
        return CarView { paired: false, launch: None, tabs: vec![tab], agent: None, voice };
    }
    let mut tabs = vec![agents_tab(view), teams_tab(view)];
    // his computers only when he has several: with one, the voice screen already says when it cannot be reached
    if computers.len() > 1 {
        tabs.push(computers_tab(computers));
    }
    tabs.truncate(room.tabs as usize);
    tabs.iter_mut().for_each(|t| fit(&mut t.sections, room));
    let mut agent = agent.and_then(|id| agent_screen(view, said, id));
    if let Some(a) = agent.as_mut() {
        fit(&mut a.sections, room);
    }
    CarView { paired: true, launch: Some(CarAct::Talk { agent: None, re: None, listen: false }), tabs, agent, voice }
}

/// Every agent, in the order the car lists them: by how it stands (waiting on him first), then by name.
fn ordered(view: &DeskView) -> Vec<(&DeskMember, Option<u8>)> {
    let mut all: Vec<(&DeskMember, Option<u8>)> = view.rooms.iter().flat_map(|r| r.members.iter().map(move |m| (m, r.colour))).collect();
    all.sort_by(|(a, _), (b, _)| CarState::of(a.state).cmp(&CarState::of(b.state)).then_with(|| a.callsign.cmp(&b.callsign)));
    all
}

fn agent_row(m: &DeskMember, colour: Option<u8>, detail: bool) -> CarRow {
    CarRow { text: m.callsign.clone(), detail: detail.then(|| CarState::of(m.state).words().to_string()), picture: Some(CarPicture::Badge { badge: m.badge.clone(), colour }), act: Some(CarAct::Open { agent: m.id.clone() }) }
}

/// Talk to everyone, then his agents under how they stand.
fn agents_tab(view: &DeskView) -> CarTab {
    let everyone = CarRow { text: "Talk to everyone".into(), detail: None, picture: Some(CarPicture::Symbol { name: "mic.fill".into() }), act: Some(CarAct::Talk { agent: None, re: None, listen: true }) };
    let mut sections = vec![CarSection { header: None, rows: vec![everyone] }];
    for (m, colour) in ordered(view) {
        let header = CarState::of(m.state).words().to_string();
        match sections.last_mut() {
            Some(last) if last.header.as_deref() == Some(header.as_str()) => last.rows.push(agent_row(m, colour, false)),
            _ => sections.push(CarSection { header: Some(header), rows: vec![agent_row(m, colour, false)] }),
        }
    }
    CarTab { kind: CarTabKind::Agents, title: "Agents".into(), symbol: "person.2.fill".into(), needs_you: waiting(view), sections }
}

/// Each team under its name, the one that most needs him first, with its agents and how each stands.
fn teams_tab(view: &DeskView) -> CarTab {
    let mut rooms: Vec<&DeskRoom> = view.rooms.iter().collect();
    rooms.sort_by(|a, b| CarState::of(a.state).cmp(&CarState::of(b.state)).then_with(|| a.name.cmp(&b.name)));
    let sections = rooms
        .into_iter()
        .map(|r| {
            let mut members: Vec<&DeskMember> = r.members.iter().collect();
            members.sort_by(|a, b| CarState::of(a.state).cmp(&CarState::of(b.state)).then_with(|| a.callsign.cmp(&b.callsign)));
            CarSection { header: Some(r.name.clone()), rows: members.into_iter().map(|m| agent_row(m, r.colour, true)).collect() }
        })
        .collect();
    CarTab { kind: CarTabKind::Teams, title: "Teams".into(), symbol: "square.grid.2x2.fill".into(), needs_you: waiting(view), sections }
}

/// Each computer, in the order they were paired, and which cannot be reached.
fn computers_tab(computers: &[(DeskComputer, bool)]) -> CarTab {
    let rows = computers.iter().map(|(c, online)| CarRow { text: c.name.clone(), detail: (!online).then(|| "Can\u{2019}t reach".to_string()), picture: Some(CarPicture::Computer { picture: c.picture }), act: None }).collect();
    CarTab { kind: CarTabKind::Computers, title: "Computers".into(), symbol: "desktopcomputer".into(), needs_you: false, sections: vec![CarSection { header: None, rows }] }
}

fn waiting(view: &DeskView) -> bool {
    view.rooms.iter().flat_map(|r| r.members.iter()).any(|m| m.state == AgentState::Asking)
}

/// One agent: the answers its open question offers, reply and read aloud, then its latest lines, newest first.
/// Nothing for an agent that is no longer there.
fn agent_screen(view: &DeskView, said: &[ChatMessage], id: &str) -> Option<CarAgent> {
    let member = view.rooms.iter().flat_map(|r| r.members.iter()).find(|m| m.id == id)?;
    let lines: Vec<&ChatMessage> = said.iter().rev().filter(|m| !m.his && !m.fyi && m.agent.as_deref() == Some(id)).take(LATEST).collect();
    let question = said.iter().find(|m| !member.question_id.is_empty() && m.id == member.question_id && m.open);
    let mut sections = Vec::new();
    // an answer from here does not count for an agent that takes it only in its own session
    if let Some(q) = question.filter(|q| !q.answer_apart) {
        let rows: Vec<CarRow> = answer_buttons(q.permission, q.options.clone()).into_iter().map(|b| CarRow { text: b.words, detail: None, picture: None, act: Some(CarAct::Tap { message: q.id.clone(), tapped: b.tapped }) }).collect();
        if !rows.is_empty() {
            sections.push(CarSection { header: Some("Answer".into()), rows });
        }
    }
    let reply = CarRow { text: "Reply".into(), detail: None, picture: Some(CarPicture::Symbol { name: "mic.fill".into() }), act: Some(CarAct::Talk { agent: Some(id.to_string()), re: question.map(|q| q.id.clone()), listen: true }) };
    let mut actions = vec![reply];
    // what is read is what it asks while it waits, else the newest thing it said
    if let Some(latest) = question.or(lines.first().copied()) {
        actions.push(CarRow { text: "Read aloud".into(), detail: None, picture: Some(CarPicture::Symbol { name: "speaker.wave.2.fill".into() }), act: Some(CarAct::Read { agent: id.to_string(), message: latest.id.clone() }) });
    }
    sections.push(CarSection { header: None, rows: actions });
    if !lines.is_empty() {
        let rows = lines.into_iter().map(|m| CarRow { text: airband_core::prose::clip(&airband_core::prose::plain(&m.text), LINE), detail: None, picture: None, act: Some(CarAct::Read { agent: id.to_string(), message: m.id.clone() }) }).collect();
        sections.push(CarSection { header: Some("Latest".into()), rows });
    }
    Some(CarAgent { id: id.to_string(), title: member.callsign.clone(), sections })
}

/// Keep to what the car can show: the first sections and rows, in the order they were put.
fn fit(sections: &mut Vec<CarSection>, room: CarRoom) {
    sections.truncate(room.sections as usize);
    let mut left = room.rows as usize;
    for s in sections.iter_mut() {
        s.rows.truncate(left);
        left -= s.rows.len();
    }
    sections.retain(|s| !s.rows.is_empty());
}

/// The voice screen for whoever he is looking at: one agent, or everyone.
fn voice(view: &DeskView) -> CarVoice {
    let member = view.focus.agent.as_ref().and_then(|id| view.rooms.iter().flat_map(|r| r.members.iter()).find(|m| m.id == *id));
    let with = member.map(|m| m.callsign.clone());
    let button = |action, title: &str, symbol: &str| CarVoiceButton { action, title: title.into(), symbol: symbol.into() };
    let talk = button(CarVoiceAction::Talk, "Talk", "mic.fill");
    // longest first, each once: the car shows the longest that fits
    let titles = |long: Option<String>, short: &str| long.into_iter().filter(|t| !t.is_empty() && t != short).chain([short.to_string()]).collect::<Vec<String>>();
    let states = vec![
        CarVoiceState { kind: CarVoiceKind::Ready, titles: titles(Some(with.as_ref().map_or_else(|| "Waiting for your agents".to_string(), |w| format!("Waiting for {w}"))), "Waiting"), symbol: "ellipsis".into(), badge: false, buttons: vec![talk.clone()] },
        CarVoiceState { kind: CarVoiceKind::Listening, titles: titles(with.as_ref().map(|w| format!("Listening, for {w}")), "Listening"), symbol: "mic.fill".into(), badge: false, buttons: vec![button(CarVoiceAction::Talk, "Send", "arrow.up"), button(CarVoiceAction::Cancel, "Cancel", "xmark")] },
        CarVoiceState { kind: CarVoiceKind::Sending, titles: vec!["Sending".into()], symbol: "arrow.up.circle.fill".into(), badge: false, buttons: Vec::new() },
        CarVoiceState { kind: CarVoiceKind::Speaking, titles: titles(with.as_ref().map(|w| format!("{w} is speaking")), "Speaking"), symbol: "speaker.wave.2.fill".into(), badge: with.is_some(), buttons: vec![talk, button(CarVoiceAction::Quiet, "Quiet", "speaker.slash.fill")] },
        CarVoiceState { kind: CarVoiceKind::Offline, titles: titles(Some(view.cannot_reach.clone()), "Offline"), symbol: "wifi.slash".into(), badge: false, buttons: Vec::new() },
    ];
    CarVoice { agent: member.map(|m| m.id.clone()), with: with.unwrap_or_default(), states, end: "End".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, state: AgentState, question: &str) -> DeskMember {
        DeskMember { id: id.into(), callsign: id.into(), state, subagents: 0, doing: None, badge: id[..2].to_uppercase(), model: String::new(), effort: String::new(), number: String::new(), station: String::new(), held: true, terminal: false, ends: false, can_stop: false, move_to: Vec::new(), unread: 0, question: String::new(), question_id: question.into(), harness: "h".into(), tool_name: String::new() }
    }
    fn room(id: &str, members: Vec<DeskMember>) -> DeskRoom {
        let state = members.iter().map(|m| m.state).min_by_key(|s| CarState::of(*s)).unwrap_or(AgentState::Idle);
        DeskRoom { id: id.into(), name: id.into(), state, agents: members.len() as u32, subagents: 0, doing: None, members, colour: Some(1), unread: 0, working: false }
    }
    fn view(rooms: Vec<DeskRoom>, focus: Option<&str>) -> DeskView {
        DeskView { paired: true, online: true, connecting: false, near: false, away: Vec::new(), feed: Vec::new(), rooms, unread: 0, several_computers: false, unread_elsewhere: 0, can_poke: false, quick: Vec::new(), sending: Vec::new(), takes: Vec::new(), stations: Vec::new(), colours: HashMap::new(), main_computer: None, choose_main: false, uploads: Vec::new(), asking: None, intents: Vec::new(), focus: DeskFocus { room: None, agent: focus.map(str::to_string) }, stoppable: Vec::new(), new_agent: NewAgentDefaults::default(), tools: Vec::new(), cannot_reach: "Can\u{2019}t reach your computer.".into() }
    }
    fn said(id: &str, agent: &str, at: u64) -> ChatMessage {
        ChatMessage { seq: at, id: id.into(), at, his: false, from: agent.into(), group: String::new(), team: String::new(), room: String::new(), to: Vec::new(), to_agents: Vec::new(), needs_you: false, fyi: false, unjudged: String::new(), stalled: false, text: format!("**{id}** said"), long: false, queued: false, seen: 0, options: Vec::new(), recommended: None, permission: false, pictures: Vec::new(), links: Vec::new(), agent: Some(agent.into()), station: String::new(), computer: String::new(), open: false, badge: String::new(), colour: None, read: None, own_session: false, answer_apart: false, not_read: String::new() }
    }
    fn computer(id: &str) -> DeskComputer {
        DeskComputer { id: id.into(), name: id.into(), shape: MachineShape::Mini, picture: ComputerPicture::Mini }
    }
    const ROOM: CarRoom = CarRoom { rows: 12, sections: 12, tabs: 5 };
    fn team() -> DeskView {
        view(vec![room("shop", vec![member("shop one", AgentState::Working, ""), member("shop two", AgentState::Asking, "q")]), room("api", vec![member("api one", AgentState::Idle, ""), member("api two", AgentState::Stalled, "")])], None)
    }
    fn texts(s: &CarSection) -> Vec<&str> {
        s.rows.iter().map(|r| r.text.as_str()).collect()
    }
    /// Every act on every screen the view has, with the screens up when it is tapped.
    fn acts(car: &CarView) -> Vec<(Vec<CarScreen>, CarAct)> {
        let on = |screens: Vec<CarScreen>, sections: &[CarSection]| sections.iter().flat_map(|s| s.rows.iter()).filter_map(|r| r.act.clone()).map(|a| (screens.clone(), a)).collect::<Vec<_>>();
        let mut all: Vec<(Vec<CarScreen>, CarAct)> = car.tabs.iter().flat_map(|t| on(vec![CarScreen::Tabs], &t.sections)).collect();
        all.extend(car.launch.clone().map(|a| (vec![CarScreen::Tabs], a)));
        if let Some(a) = &car.agent {
            all.extend(on(vec![CarScreen::Tabs, CarScreen::Agent], &a.sections));
        }
        all
    }

    #[test]
    fn the_agents_waiting_on_him_come_first_then_stopped_working_and_done() {
        let car = car_view(&team(), &[], &[(computer("mini"), true)], None, ROOM);
        let agents = &car.tabs[0];
        assert_eq!(agents.kind, CarTabKind::Agents);
        assert!(agents.needs_you);
        assert_eq!(texts(&agents.sections[0]), ["Talk to everyone"]);
        let headers: Vec<_> = agents.sections[1..].iter().map(|s| s.header.clone().unwrap()).collect();
        assert_eq!(headers, ["Waiting on you", "Stopped", "Working", "Done"]);
        let names: Vec<_> = agents.sections[1..].iter().flat_map(texts).collect();
        assert_eq!(names, ["shop two", "api two", "shop one", "api one"]);
        // under a heading that says how they stand, the row does not say it again
        assert!(agents.sections.iter().flat_map(|s| s.rows.iter()).all(|r| r.detail.is_none()));
        assert_eq!(agents.sections[1].rows[0].picture, Some(CarPicture::Badge { badge: "SH".into(), colour: Some(1) }));
    }

    #[test]
    fn the_team_that_needs_him_comes_first_and_each_agent_says_how_it_stands() {
        let car = car_view(&team(), &[], &[(computer("mini"), true)], None, ROOM);
        let teams = &car.tabs[1];
        assert_eq!(teams.kind, CarTabKind::Teams);
        assert_eq!(teams.sections.iter().map(|s| s.header.clone().unwrap()).collect::<Vec<_>>(), ["shop", "api"]);
        assert_eq!(texts(&teams.sections[0]), ["shop two", "shop one"]);
        assert_eq!(teams.sections[0].rows.iter().map(|r| r.detail.clone().unwrap()).collect::<Vec<_>>(), ["Waiting on you", "Working"]);
    }

    #[test]
    fn computers_have_a_tab_only_when_there_are_several_and_say_which_cannot_be_reached() {
        assert_eq!(car_view(&team(), &[], &[(computer("mini"), true)], None, ROOM).tabs.len(), 2);
        let car = car_view(&team(), &[], &[(computer("mini"), true), (computer("studio"), false)], None, ROOM);
        let tab = &car.tabs[2];
        assert_eq!(tab.kind, CarTabKind::Computers);
        assert_eq!(tab.sections[0].rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>(), [None, Some("Can\u{2019}t reach".into())]);
        assert!(tab.sections[0].rows.iter().all(|r| r.act.is_none()));
    }

    #[test]
    fn it_opens_on_the_voice_screen_ready_to_talk_and_unpaired_it_only_says_so() {
        let car = car_view(&team(), &[], &[], None, ROOM);
        assert_eq!(car.launch, Some(CarAct::Talk { agent: None, re: None, listen: false }));
        let mut none = team();
        none.paired = false;
        let car = car_view(&none, &[], &[], Some("shop one"), ROOM);
        assert_eq!(car.launch, None);
        assert_eq!(car.agent, None);
        assert!(car.tabs.iter().flat_map(|t| t.sections.iter()).flat_map(|s| s.rows.iter()).all(|r| r.act.is_none()));
    }

    #[test]
    fn a_waiting_agent_offers_the_answers_its_question_has_then_reply_and_read_then_its_latest_lines() {
        let mut q = said("q", "shop two", 5);
        (q.needs_you, q.open, q.permission) = (true, true, true);
        let lines = vec![said("a", "shop two", 1), said("b", "shop one", 2), said("c", "shop two", 3), said("d", "shop two", 4), q];
        let car = car_view(&team(), &lines, &[], Some("shop two"), ROOM);
        let agent = car.agent.unwrap();
        assert_eq!(agent.title, "shop two");
        assert_eq!(agent.sections[0].header.as_deref(), Some("Answer"));
        assert_eq!(texts(&agent.sections[0]), ["Allow", "Deny"]);
        assert_eq!(agent.sections[0].rows[1].act, Some(CarAct::Tap { message: "q".into(), tapped: Tapped::Deny }));
        assert_eq!(texts(&agent.sections[1]), ["Reply", "Read aloud"]);
        // his reply answers the question; reading aloud reads it
        assert_eq!(agent.sections[1].rows[0].act, Some(CarAct::Talk { agent: Some("shop two".into()), re: Some("q".into()), listen: true }));
        assert_eq!(agent.sections[1].rows[1].act, Some(CarAct::Read { agent: "shop two".into(), message: "q".into() }));
        // its own lines, newest first, three at most, in plain short words
        assert_eq!(texts(&agent.sections[2]), ["q said", "d said", "c said"]);
    }

    #[test]
    fn a_question_answered_only_in_its_own_session_offers_no_answers_and_options_come_as_offered() {
        let mut q = said("q", "shop two", 5);
        (q.open, q.options, q.answer_apart) = (true, vec!["Ship it".into(), "Wait".into()], true);
        let car = car_view(&team(), std::slice::from_ref(&q), &[], Some("shop two"), ROOM);
        assert_eq!(texts(&car.agent.unwrap().sections[0]), ["Reply", "Read aloud"]);
        q.answer_apart = false;
        let car = car_view(&team(), &[q], &[], Some("shop two"), ROOM);
        let agent = car.agent.unwrap();
        assert_eq!(texts(&agent.sections[0]), ["Ship it", "Wait"]);
        assert_eq!(agent.sections[0].rows[0].act, Some(CarAct::Tap { message: "q".into(), tapped: Tapped::Option { index: 0 } }));
    }

    #[test]
    fn an_agent_with_nothing_said_offers_reply_only_and_one_gone_has_no_screen() {
        let car = car_view(&team(), &[], &[], Some("api one"), ROOM);
        let agent = car.agent.unwrap();
        assert_eq!(agent.sections.len(), 1);
        assert_eq!(texts(&agent.sections[0]), ["Reply"]);
        assert_eq!(car_view(&team(), &[], &[], Some("gone"), ROOM).agent, None);
    }

    #[test]
    fn a_long_line_is_cut_short() {
        let mut long = said("a", "api one", 1);
        long.text = "word ".repeat(60);
        let car = car_view(&team(), &[long], &[], Some("api one"), ROOM);
        let line = &car.agent.unwrap().sections[1].rows[0].text;
        assert!(line.chars().count() <= LINE && line.ends_with('\u{2026}'));
    }

    #[test]
    fn nothing_on_any_screen_puts_up_more_than_three() {
        let mut q = said("q", "shop two", 5);
        (q.open, q.permission) = (true, true);
        let lines = vec![said("a", "shop two", 1), q];
        let several = [(computer("mini"), true), (computer("studio"), false)];
        for agent in [None, Some("shop two"), Some("api one")] {
            let car = car_view(&team(), &lines, &several, agent, ROOM);
            for (mut screens, act) in acts(&car) {
                assert!(car_fits(screens.clone()));
                if let Some(next) = act.opens() {
                    screens.push(next);
                }
                assert!(car_fits(screens.clone()), "{act:?}");
                assert!(screens.len() as u32 <= CAR_DEPTH);
            }
            // an agent's screen opens no other agent's screen
            assert!(car.agent.iter().flat_map(|a| a.sections.iter()).flat_map(|s| s.rows.iter()).all(|r| !matches!(r.act, Some(CarAct::Open { .. }))));
        }
        assert!(!car_fits(vec![CarScreen::Tabs, CarScreen::Agent, CarScreen::Agent]));
        assert!(!car_fits(vec![CarScreen::Tabs, CarScreen::Agent, CarScreen::Voice, CarScreen::Voice]));
        assert!(!car_fits(vec![CarScreen::Tabs, CarScreen::Voice, CarScreen::Agent]));
        assert!(!car_fits(vec![CarScreen::Agent]));
        assert!(car_fits(vec![CarScreen::Tabs, CarScreen::Agent, CarScreen::Alert]));
        assert!(!car_fits(vec![CarScreen::Tabs, CarScreen::Voice, CarScreen::Alert]));
        assert!(!car_fits(Vec::new()));
    }

    #[test]
    fn it_keeps_to_the_rows_the_car_can_show_the_waiting_ones_kept() {
        let car = car_view(&team(), &[], &[], None, CarRoom { rows: 3, sections: 12, tabs: 1 });
        assert_eq!(car.tabs.len(), 1);
        let names: Vec<_> = car.tabs[0].sections.iter().flat_map(texts).collect();
        assert_eq!(names, ["Talk to everyone", "shop two", "api two"]);
    }

    #[test]
    fn the_voice_screen_names_the_agent_he_is_talking_with() {
        let car = car_view(&view(team().rooms, Some("shop two")), &[], &[], None, ROOM);
        assert_eq!(car.voice.with, "shop two");
        assert_eq!(car.voice.states[0].titles, ["Waiting for shop two", "Waiting"]);
        assert!(car.voice.states.iter().all(|s| s.buttons.len() <= 2));
        let car = car_view(&team(), &[], &[], None, ROOM);
        assert_eq!(car.voice.agent, None);
        assert_eq!(car.voice.states[0].titles, ["Waiting for your agents", "Waiting"]);
        assert_eq!(car.voice.states[4].titles, ["Can\u{2019}t reach your computer.", "Offline"]);
        assert_eq!(car_voice_kind(RadioStatus::Idle, true), CarVoiceKind::Listening);
        assert_eq!(car_voice_kind(RadioStatus::Idle, false), CarVoiceKind::Ready);
    }
}
