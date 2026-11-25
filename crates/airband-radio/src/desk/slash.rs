//! The box he types in, when it starts with `/`: the commands and skills of the agent his words would go to, as
//! its tool lists them, narrowed to the names that hold what he has typed after the `/`. Picking one puts
//! `/name ` in the box for him to add what it is given. Only the ones that run from his phone are offered.
//!
//! The list is already here when he types `/`: it is asked of the agent's computer as soon as his words would go
//! to that agent (he opens its chat, or picks one of its messages), again when its tool lists them anew (the
//! roster's count for it changes), and again, behind the one it shows, each time the list opens, which is when its
//! computer looks at whether the files they come from changed.
use super::*;
use airband_core::commands::{Command, Commands, NEUTRAL, Runs, Terms, matching, naming, typed};

/// What the box holds and what is known of the list it opens.
#[derive(Default)]
pub(crate) struct Slash {
    /// What is in the box now, and the one agent his words would go to (none when they would not go to one).
    text: String,
    agent: Option<String>,
    /// The agent the list is open for, while it is open (`Some(None)`: open, for no one agent).
    open_for: Option<Option<String>>,
    /// Each agent's list as its computer last gave it, by the agent's id.
    got: HashMap<String, Got>,
    /// How many times each agent's list has been asked for: only the answer to the latest ask is taken, so
    /// an older answer that arrives late never stands over a newer one.
    asks: HashMap<String, u64>,
    /// The roster's count of its tool's lists, for each agent, when its list was last asked for.
    asked_at: HashMap<String, u64>,
}

enum Got {
    Asking,
    Have(Vec<Command>),
    Failed(String),
}

/// The list as the box shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CommandsView {
    /// What the agent's tool calls them, as a heading.
    pub title: String,
    pub items: Vec<CommandOffer>,
    /// One line instead of a list, when there is nothing to pick and something to say why.
    pub note: Option<String>,
    /// The list is being asked for and none is here yet.
    pub asking: bool,
}

/// One he can pick.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CommandOffer {
    pub name: String,
    /// What it does, in its own words.
    pub about: String,
    /// What it is given after its name, when it says.
    pub hint: Option<String>,
    /// What its tool calls its kind ("skill"), when that is known.
    pub kind: Option<String>,
    /// What the box holds once it is picked.
    pub insert: String,
}

/// What he is told in place of a list.
const NO_ONE: &str = "Choose one agent to see what it can run.";
const NOT_FROM_HERE: &str = "This agent was opened at the computer, so these run only there. Agents airband starts run them from here.";
const NONE_LISTED: &str = "Its tool has not listed any yet.";
const NO_MATCH: &str = "Nothing by that name.";

/// What a tool calls these, by its id, from the catalogue on this phone.
fn terms(tool: &str) -> Option<Terms> {
    airband_core::catalog::current().tools.iter().find(|t| t.id == tool).map(|t| t.commands.clone()).filter(|t| !t.term.is_empty())
}

impl Slash {
    /// The box now holds `text`, for `agent`, whose tool has listed its commands anew `listed` times (the
    /// roster's count). Answers with the agent whose list is to be asked for, and the number of this ask, when
    /// one is due: never on a timer.
    fn typed(&mut self, text: String, agent: Option<String>, listed: u64) -> Option<(String, u64)> {
        let open = naming(&text).is_some();
        let was = self.open_for.clone();
        (self.text, self.agent) = (text, agent.clone());
        self.open_for = open.then(|| agent.clone());
        let id = agent?;
        let opened = open && was != self.open_for;
        let relisted = self.asked_at.get(&id) != Some(&listed);
        // asked once his words would go to it; again when its tool lists them anew, and when the list opens
        // (behind the one it shows, or after one that failed); not again while an ask is out
        let due = match self.got.get(&id) {
            None => true,
            Some(Got::Asking) => relisted,
            Some(Got::Have(_) | Got::Failed(_)) => opened || relisted,
        };
        if !due {
            return None;
        }
        let held = matches!(self.got.get(&id), Some(Got::Have(_)));
        if !held {
            self.got.insert(id.clone(), Got::Asking);
        }
        self.asked_at.insert(id.clone(), listed);
        let ask = self.asks.entry(id.clone()).or_default();
        *ask += 1;
        Some((id, *ask))
    }
    /// An answer to ask number `ask` for this agent's list: taken when it is the latest ask.
    fn settle(&mut self, agent: String, ask: u64, got: Got) {
        if self.asks.get(&agent) == Some(&ask) {
            self.got.insert(agent, got);
        }
    }
}

#[uniffi::export]
impl Desk {
    /// What is in the box he types in, and the one agent his words would go to (the one he is looking at, or the
    /// one whose message he picked); none when they would not go to one. Told whenever either changes, and when
    /// the roster does. While the box holds `/` and the start of a name, the view offers that agent's commands
    /// ([`DeskView::commands`]); its list is fetched before then, as soon as his words would go to it.
    pub fn typing(&self, text: String, agent: Option<String>) {
        box_now(&self.shared, text, agent);
        self.shared.host.changed();
    }
    /// What the commands of the agent his words would go to are called, for the way into the list beside the
    /// box: that agent's tool's word for them, or a word for any tool's when they would not go to one agent.
    pub fn commands_title(&self, agent: Option<String>) -> String {
        let state = self.shared.state.lock().unwrap();
        agent.and_then(|id| state.everyone().into_iter().find(|a| a.id == id).and_then(|a| terms(&a.harness))).map_or_else(|| NEUTRAL.to_string(), |t| t.term)
    }
}

/// The box holds `text`, for `agent`: their list is asked for when one is due.
fn box_now(shared: &Arc<Shared>, text: String, agent: Option<String>) {
    let ask = {
        let mut state = shared.state.lock().unwrap();
        let listed = agent.as_ref().and_then(|id| state.everyone().into_iter().find(|a| a.id == *id)).map_or(0, |a| a.commands);
        let asked = state.slash.typed(text, agent, listed);
        asked.and_then(|(id, n)| state.radio_with_agent(&id).map(|radio| (id, n, radio)))
    };
    if let Some((id, n, radio)) = ask {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let got = match radio.commands(id.clone()) {
                Ok(Commands { items }) => Got::Have(items),
                Err(e) => Got::Failed(e.said()),
            };
            shared.state.lock().unwrap().slash.settle(id, n, got);
            shared.host.changed();
        });
    }
}

/// A computer's roster has come: the list of the agent his words go to is asked for again when its tool has
/// listed them anew since it was.
pub(super) fn roster_came(shared: &Arc<Shared>) {
    let (text, agent) = {
        let state = shared.state.lock().unwrap();
        (state.slash.text.clone(), state.slash.agent.clone())
    };
    box_now(shared, text, agent);
}

/// The list the box shows now; nothing while it is closed.
pub(super) fn view(state: &State) -> Option<CommandsView> {
    let slash = &state.slash;
    let typed_name = naming(&slash.text)?;
    let agent = slash.agent.as_ref().and_then(|id| state.everyone().into_iter().find(|a| a.id == *id));
    let Some(agent) = agent else {
        return Some(CommandsView { title: NEUTRAL.into(), items: Vec::new(), note: Some(NO_ONE.into()), asking: false });
    };
    let terms = terms(&agent.harness).unwrap_or_else(|| Terms { term: NEUTRAL.into(), ..Terms::default() });
    let shown = |items: Vec<CommandOffer>, note: Option<&str>, asking: bool| Some(CommandsView { title: terms.term.clone(), items, note: note.map(String::from), asking });
    match slash.got.get(&agent.id) {
        None | Some(Got::Asking) => shown(Vec::new(), None, true),
        Some(Got::Failed(why)) => shown(Vec::new(), Some(why), false),
        Some(Got::Have(all)) => {
            if all.is_empty() {
                return shown(Vec::new(), Some(NONE_LISTED), false);
            }
            let runs: Vec<Command> = all.iter().filter(|c| c.runs == Runs::Here).cloned().collect();
            if runs.is_empty() {
                return shown(Vec::new(), Some(NOT_FROM_HERE), false);
            }
            let items: Vec<CommandOffer> = matching(&runs, typed_name).into_iter().map(|c| CommandOffer { name: c.name.clone(), about: c.about.clone(), hint: c.hint.clone(), kind: terms.of(c.kind).map(String::from), insert: typed(&c.name) }).collect();
            let none = items.is_empty();
            shown(items, none.then_some(NO_MATCH), false)
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use airband_core::commands::{Kind, Source};

    /// Hand the list an agent's computer gave, as if it had arrived.
    pub(in crate::desk) fn arrived(state: &mut State, agent: &str, items: Vec<Command>) {
        let ask = state.slash.asks.entry(agent.into()).or_default();
        *ask += 1;
        let ask = *ask;
        state.slash.settle(agent.into(), ask, Got::Have(items));
    }

    pub(in crate::desk) fn command(name: &str, runs: bool) -> Command {
        Command { name: name.into(), about: format!("about {name}"), hint: None, kind: Some(Kind::Skill), source: Some(Source::Project), runs: if runs { Runs::Here } else { Runs::AtComputer } }
    }

    #[test]
    fn its_list_is_asked_for_once_his_words_go_to_it_again_as_it_opens_or_is_listed_anew_and_closes_past_the_name() {
        let mut s = Slash::default();
        assert_eq!(s.typed(String::new(), None, 0), None, "his words go to no one agent: nothing to ask");
        assert_eq!(s.typed(String::new(), Some("a".into()), 0), Some(("a".into(), 1)), "his words now go to it: asked for before he types");
        assert_eq!(s.typed("hel".into(), Some("a".into()), 0), None, "being asked: not again");
        s.settle("a".into(), 1, Got::Have(vec![command("review", true)]));
        assert_eq!(s.typed("hello".into(), Some("a".into()), 0), None, "here: not again while he types");
        assert_eq!(s.typed("/".into(), Some("a".into()), 0), Some(("a".into(), 2)), "opened: asked again behind the one shown");
        assert!(matches!(s.got.get("a"), Some(Got::Have(_))), "what it had is shown while it is asked again");
        assert_eq!(s.typed("/re".into(), Some("a".into()), 0), None, "still open: not asked again");
        assert_eq!(s.typed("/review main".into(), Some("a".into()), 0), None, "past the name: closed");
        assert!(s.open_for.is_none());
        assert_eq!(s.typed("/review main".into(), Some("a".into()), 1), Some(("a".into(), 3)), "its tool listed them anew");
        // an answer to an earlier ask arriving after a later one is not taken
        s.settle("a".into(), 2, Got::Failed("late".into()));
        assert!(matches!(s.got.get("a"), Some(Got::Have(_))));
        assert_eq!(s.typed("/".into(), Some("b".into()), 0), Some(("b".into(), 1)), "for another agent");
        assert_eq!(s.typed("/".into(), None, 0), None, "for no one agent: nothing to ask");
    }
}
