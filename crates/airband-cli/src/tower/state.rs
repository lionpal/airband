//! What the tower remembers, and the rules about it: who is on the radio and what they are called, the chat log,
//! what is unheard, and each radio's place in a call. No I/O and no model here; everything that changes these
//! goes through a method below, and the rules are tested at the bottom of this file.
//!
//! An agent is known by its id, which never changes. Its callsign is only what it is called: it can change (he
//! names the session), and a later agent can be given a callsign an earlier one had. So a message records the id
//! of the agent that said it, and every question about "which agent" is answered by id.
use super::*;
/// A folder's path as he would read it: "~/Companies/shop" inside his home folder, whole anywhere else.
pub(super) fn shown(at: &std::path::Path) -> String {
    let homes = [config::home().canonicalize().ok(), Some(config::home())];
    match homes.iter().flatten().find_map(|home| at.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => at.display().to_string(),
    }
}
#[derive(Default)]
pub(super) struct Inner {
    /// By the id each delivery of his words went out under, the chat lines it carried. The agent's own record
    /// names that id when it takes the words in: that, and nothing about the words, is how a line is marked read.
    pub(super) receipts: HashMap<String, Vec<String>>,
    /// What is known about the things at the web addresses in the chat.
    pub(super) links: links::Book,
    pub(super) sessions: Vec<Session>,
    pub(super) log: Vec<Entry>,
    /// The last changes to what is read out, as the log has them: for `airband status`.
    pub(super) aloud_trail: std::collections::VecDeque<String>,
    pub(super) seq: u64,
    /// The change count: see [`Entry::rev`].
    pub(super) rev: u64,
    /// Every project an agent has been seen working in, by spoken name, with its folder. A new agent can be
    /// launched in any of these.
    pub(super) projects: BTreeMap<String, String>,
    /// The teams he named himself. One of these keeps its name though a team on another of his computers has it.
    pub(super) named_teams: BTreeSet<String>,
    /// The teams the configuration lists, by name, with their folders. Never kept: read from it at the start.
    pub(super) configured: BTreeMap<String, String>,
    /// This computer's name, as a phone paired with several shows it. Never kept.
    pub(super) here: String,
    /// The folders his coding tools have had sessions in, as last read from their records
    /// (`adapters::worked_all`): the folder, the tools, when. Never kept: it is read again every minute.
    pub(super) worked: Vec<(String, Vec<String>, u64)>,
    /// The agents the tower runs itself.
    pub(super) held: Vec<held::Kept>,
    /// What an agent of each launcher that says so itself last said it can be set to, by the launcher's name
    /// (`held::offers`).
    pub(super) menus: BTreeMap<String, airband_core::tools::Offers>,
    /// The coding tools he uses, by their rows' ids, the one he uses most first: as his phone last said.
    pub(super) tools: Vec<String>,
    /// Names agents go by on his other computers, as a radio last told it. None of this tower's numbered
    /// agents is called any of them.
    pub(super) elsewhere: Vec<Callsign>,
    /// Where each agent that was moved to another team was put, by its id. A session that goes away and comes
    /// back (the computer restarted, he resumed it) is put back there, not in its folder's team again.
    pub(super) placed: BTreeMap<String, (String, Mover)>,
    /// Each radio that has been heard from, by the id it gives.
    pub(super) calls: HashMap<String, Call>,
    /// When each open message was last put in front of him on a call, and how many times: for reminders.
    pub(super) offered: HashMap<String, (u32, Instant)>,
    /// Ids of messages that are new or changed since they were last written to the database.
    pub(super) unsaved: std::cell::RefCell<HashSet<String>>,
    /// Ids of messages whose short version is being made right now, so none is made twice.
    pub(super) judging: HashSet<String>,
    /// Requests for permission that are waiting for his answer, or have just had it, by id.
    pub(super) permits: HashMap<String, Permit>,
    /// Ids of the lines a notification has been decided for, so each is decided once.
    pub(super) pushed: HashSet<String>,
    /// How many requests for the whole chat are open right now: while there is one, it reads what agents say.
    pub(super) readers: u32,
    /// Ids of things he said that this tower is working out an answer to right now.
    pub(super) answering: HashSet<String>,
    /// What has come of the last things he said, for telling the radio that said each in its feed: the change
    /// count it was settled at, the radio, and the outcome under the utterance's id.
    pub(super) settled: VecDeque<(u64, String, airband_core::radio::Settled)>,
}
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Saved {
    pub(super) sessions: Vec<Session>,
    #[serde(default)]
    pub(super) projects: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) named_teams: BTreeSet<String>,
    #[serde(default)]
    pub(super) held: Vec<held::Kept>,
    #[serde(default)]
    pub(super) menus: BTreeMap<String, airband_core::tools::Offers>,
    #[serde(default)]
    pub(super) tools: Vec<String>,
    #[serde(default)]
    pub(super) placed: BTreeMap<String, (String, Mover)>,
}
impl Inner {
    pub(super) fn session(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.agent.id == id)
    }
    pub(super) fn find(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.agent.id == id)
    }
    /// The agent that said a logged message, if it is still on the radio.
    pub(super) fn sender(&self, e: &Entry) -> Option<&Session> {
        e.agent.as_deref().and_then(|id| self.find(id))
    }
    pub(super) fn reap(&mut self) -> bool {
        let dead: Vec<(String, String)> = self.sessions.iter().filter(|s| s.agent.deliver != Some(Deliver::Held) && s.agent.pid.is_some_and(|p| !alive(p))).map(|s| (s.agent.id.clone(), s.callsign.text())).collect();
        for (id, callsign) in &dead {
            log(format!("{callsign} is gone"));
            self.remove(id);
        }
        // a permission request nobody is waiting on any more: its line stops offering an answer
        let over: Vec<String> = self.permits.iter().filter(|(_, p)| !p.live()).map(|(id, _)| id.clone()).collect();
        for id in over {
            self.close_permit(&id, permit::Ended::Gone);
        }
        // a radio not heard from in a day has nothing worth keeping
        self.calls.retain(|_, c| c.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(86_400)));
        !dead.is_empty()
    }
    /// What an agent is doing.
    ///
    /// One part of this is a rule and not a fact: an agent that stopped to wait for background work of its own
    /// counts as working for [`STILL_ITS_OWN_WORK_MS`] and as stalled after. No tool says whether that work is
    /// still running.
    pub(super) fn state_of(&self, s: &Session, open: &HashSet<String>) -> AgentState {
        if !s.waiting {
            return AgentState::Working;
        }
        // It stopped only to wait for something of its own that is still running: that is still work, for a
        // while. If nothing has come of it after twenty minutes it is not coming back by itself: it has
        // stalled, and a poke is offered.
        if let Some(e) = self.log.iter().rev().find(|e| e.agent.as_deref() == Some(s.agent.id.as_str())).filter(|e| e.background) {
            return if now_ms().saturating_sub(e.at) < STILL_ITS_OWN_WORK_MS { AgentState::Working } else { AgentState::Stalled };
        }
        match self.last_said(&s.agent.id) {
            Some(e) if open.contains(&e.id) => AgentState::Asking,
            Some(e) if e.kind == Some(Kind::Stalled) => AgentState::Stalled,
            _ => AgentState::Idle,
        }
    }
    /// A number that changes when anything a radio shows about the agents does: who is there, what each is
    /// called, its state, what it is doing.
    pub(super) fn roster_rev(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.roster_entries()).unwrap_or_default().hash(&mut h);
        // never 0: that is what a radio sends before it has seen one
        h.finish() | 1
    }
    /// The agents that have stopped without finishing or asking: the ones a poke is for.
    pub(super) fn stalled(&self) -> Vec<Session> {
        let open = self.open_ids();
        self.sessions.iter().filter(|s| self.state_of(s, &open) == AgentState::Stalled).cloned().collect()
    }
    pub(super) fn roster_entries(&self) -> Vec<RosterEntry> {
        let open = self.open_ids();
        self.sessions.iter().map(|s| RosterEntry { id: s.agent.id.clone(), callsign: s.callsign.text(), group: s.callsign.project.clone(), number: s.callsign.number.clone(), named: s.callsign.named, harness: s.agent.harness.clone(), waiting: s.waiting, state: self.state_of(s, &open), subagents: s.subagents, reachable: s.agent.deliver.is_some(), doing: s.doing.clone().filter(|_| !s.waiting), held: s.agent.deliver == Some(Deliver::Held), terminal: matches!(s.agent.deliver, Some(Deliver::Keys { .. } | Deliver::Socket { .. })), ends: match &s.agent.deliver { Some(Deliver::Held) => true, Some(Deliver::Harness { name, .. }) => crate::adapters::puts_away(name), _ => false }, model: s.model.clone(), effort: s.effort.clone() }).collect()
    }
    /// His teams that have a folder: what the configuration lists, then wherever agents have been seen.
    pub(super) fn teams(&self) -> BTreeMap<String, String> {
        let mut all = self.projects.clone();
        all.extend(self.configured.clone());
        // a project whose folder has gone is not offered
        all.retain(|_, folder| std::path::Path::new(folder).is_dir());
        all
    }
    /// Where a new agent can be started by name: his teams, then the folders his coding tools have lately
    /// had sessions in, each by the name a team there would have. A team's name is never taken from it.
    pub(super) fn launchable(&self) -> BTreeMap<String, String> {
        let mut all = self.teams();
        for recent in self.recent().into_iter().filter(|r| !r.team && !r.name.is_empty()).take(20) {
            all.entry(recent.name).or_insert(recent.folder);
        }
        all
    }
    /// The folders his coding tools have had sessions in, newest first, with his teams: one that has a team
    /// is called by it, and a team no tool has a record of comes last.
    pub(super) fn recent(&self) -> Vec<airband_core::radio::RecentFolder> {
        let teams = self.teams();
        let mut all: Vec<airband_core::radio::RecentFolder> = self
            .worked
            .iter()
            .take(60)
            .cloned()
            .map(|(folder, tools, at)| {
                let team = self.team_at(&folder);
                airband_core::radio::RecentFolder { shown: shown(std::path::Path::new(&folder)), name: team.clone().or_else(|| self.folder_team(&folder)).unwrap_or_default(), team: team.is_some(), tools, at, folder }
            })
            .collect();
        for (name, folder) in teams {
            if !all.iter().any(|r| real_folder(&r.folder) == real_folder(&folder)) {
                all.push(airband_core::radio::RecentFolder { shown: shown(std::path::Path::new(&folder)), name, team: true, tools: Vec::new(), at: 0, folder });
            }
        }
        all
    }
    pub(super) fn roster(&self) -> String {
        if self.sessions.is_empty() {
            return "No agents are running.".into();
        }
        let names = |waiting: bool| self.sessions.iter().filter(|s| s.waiting == waiting).map(|s| s.callsign.text()).collect::<Vec<_>>().join(", ");
        let (waiting, working) = (names(true), names(false));
        [(!waiting.is_empty()).then(|| format!("Waiting on you: {waiting}.")), (!working.is_empty()).then(|| format!("Working: {working}."))].into_iter().flatten().collect::<Vec<_>>().join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON_CALL: airband_core::radio::Presence = airband_core::radio::Presence { on_call: true, showing_chat: false };

    fn agent(id: &str, cwd: &str, title: Option<&str>) -> Agent {
        Agent { id: id.into(), harness: "test".into(), cwd: Some(cwd.into()), name: None, title: title.map(String::from), pid: None, deliver: None, meta: Default::default() }
    }
    fn says(inner: &mut Inner, agent_id: &str, kind: Kind, text: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        inner.record(Entry::said(id.clone(), &s, Some(kind), text.into()));
        id
    }
    fn names(inner: &Inner) -> Vec<String> {
        inner.sessions.iter().map(|s| s.callsign.text()).collect()
    }

    #[test]
    fn a_new_agent_can_be_started_by_name_in_a_folder_a_coding_tool_has_worked_in() {
        let here = std::env::temp_dir().join(format!("airband-recent-{}", config::random_hex(6)));
        let folder = |name: &str| {
            std::fs::create_dir_all(here.join(name)).unwrap();
            here.join(name).display().to_string()
        };
        let (shop, back, other_shop) = (folder("shop"), folder("back-office"), folder("elsewhere/shop"));
        let mut inner = Inner::default();
        inner.check_in(&agent("a", &shop, None));
        inner.worked = vec![(back.clone(), vec!["codex".into()], 9), (other_shop.clone(), vec!["claude-code".into()], 7), (shop.clone(), vec!["claude-code".into(), "codex".into()], 5)];
        let recent = inner.recent();
        let listed: Vec<(&str, &str, bool, u64)> = recent.iter().map(|r| (r.name.as_str(), r.folder.as_str(), r.team, r.at)).collect();
        // newest first; a folder is called as its team is when it has one, and as a team there would be when not
        // another folder called as his team is would be a team of its own, its parent's name in front
        assert_eq!(listed, [("back office", back.as_str(), false, 9), ("elsewhere shop", other_shop.as_str(), false, 7), ("shop", shop.as_str(), true, 5)]);
        // by name: his team keeps its name, and a folder that is only recent never takes it
        let launchable = inner.launchable();
        assert_eq!(launchable.get("shop").map(|f| real_folder(f)), Some(real_folder(&shop)));
        assert_eq!(launchable.get("elsewhere shop"), Some(&other_shop));
        assert_eq!(launchable.get("back office"), Some(&back));
        assert_eq!(inner.teams().len(), 1);
        // a team no tool has a record of is still listed, last
        inner.worked = vec![(back.clone(), vec!["codex".into()], 9)];
        assert_eq!(inner.recent().iter().map(|r| (r.name.as_str(), r.team, r.at)).collect::<Vec<_>>(), [("back office", false, 9), ("shop", true, 0)]);
        let _ = std::fs::remove_dir_all(&here);
    }

    #[test]
    fn a_team_is_a_folder_and_two_folders_that_read_alike_are_two_teams_called_apart() {
        let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
        assert_eq!(inner.check_in(&agent("a", "/a/work/shop", None)).text(), "shop one");
        // another folder of the same name: its parent's name goes in front, so the two are never said alike
        assert_eq!(inner.check_in(&agent("b", "/b/work/shop", None)).text(), "work shop one");
        // and a third goes up one more
        assert_eq!(inner.check_in(&agent("c", "/c/work/shop", None)).text(), "c work shop one");
        // a second agent in a folder joins that folder's team, however the path is written
        assert_eq!(inner.check_in(&agent("d", "/b/work/shop/", None)).text(), "work shop two");
        assert_eq!(inner.check_in(&agent("e", "/a/work/shop", None)).text(), "shop two");
        assert_eq!(inner.team_at("/b/work/shop").as_deref(), Some("work shop"));
        assert_eq!(inner.folder_team("/d/shop").as_deref(), Some("d shop"));
    }

    #[test]
    fn a_folder_whose_name_another_computer_has_takes_this_computers_name_in_front() {
        let theirs = |team: &str| Callsign { project: team.into(), number: "one".into(), named: false };
        let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
        // told first: a new folder's team keeps off the name
        inner.taken_elsewhere(&[theirs("shop")]);
        assert_eq!(inner.check_in(&agent("a", "/x/shop", None)).text(), "laptop shop one");
        // told after: a team here that has the name is renamed, and its agents go with it
        assert_eq!(inner.check_in(&agent("b", "/x/garden", None)).text(), "garden one");
        assert!(inner.taken_elsewhere(&[theirs("shop"), theirs("garden")]));
        assert_eq!(names(&inner), ["laptop shop one", "laptop garden one"]);
        assert_eq!(inner.team_at("/x/garden").as_deref(), Some("laptop garden"));
        // a name he gave a team is his: it stays, and only numbers that clash move
        inner.rename_team("laptop garden", "garden").unwrap();
        assert!(inner.taken_elsewhere(&[theirs("shop"), theirs("garden")]));
        assert_eq!(names(&inner), ["laptop shop one", "garden two"]);
    }

    #[test]
    fn an_agent_taken_over_keeps_its_name_and_the_radios_are_told() {
        let mut inner = Inner::default();
        let first = inner.check_in(&agent("a", "/x/shop", None));
        let s = inner.find("a").cloned().unwrap();
        inner.record(Entry::said("m1".into(), &s, None, "done".into()));
        // back from a new process under another number, as checking in again can give it
        inner.session("a").unwrap().callsign.number = "two".into();
        let rev = inner.rev;
        assert!(inner.keep_name("a", first.clone()));
        assert_eq!(inner.find("a").unwrap().callsign, first);
        assert!(inner.rev > rev && inner.log.iter().find(|e| e.id == "m1").unwrap().rev > rev);
        // a name someone else has taken meanwhile is not taken from them
        inner.check_in(&agent("b", "/x/shop", None));
        let b = inner.find("b").unwrap().callsign.clone();
        assert!(!inner.keep_name("a", b));
    }

    #[test]
    fn renaming_a_team_changes_only_what_it_is_called() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", Some("refunds")));
        inner.check_in(&agent("b", "/x/shop", None));
        assert_eq!(inner.rename_team("shop", "Store Front"), Ok("store front".to_string()));
        assert_eq!(names(&inner), ["store front refunds", "store front one"]);
        // its folder is still its folder: a new agent there joins it
        assert_eq!(inner.check_in(&agent("c", "/x/shop", None)).text(), "store front two");
        // a name another team has, or none, is refused in words
        inner.check_in(&agent("d", "/x/garden", None));
        assert_eq!(inner.rename_team("garden", "store front"), Err("There is already a team called store front.".to_string()));
        assert!(inner.rename_team("garden", "!!").is_err());
        assert!(inner.rename_team("nobody", "x").is_err());
    }

    #[test]
    fn kept_agents_from_two_folders_in_one_team_go_back_to_their_own_folders_teams() {
        // as kept before teams went by folder: one name, two folders
        let session = |id: &str, cwd: &str, number: &str| Session { agent: agent(id, cwd, None), callsign: Callsign { project: "shop".into(), number: number.into(), named: false }, waiting: false, last_heard: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0, queue: Vec::new(), listed: false, hooked: false, turn: None };
        let mut inner = Inner { sessions: vec![session("a", "/a/work/shop", "one"), session("b", "/b/work/shop", "two")], projects: [("shop".to_string(), "/a/work/shop".to_string())].into(), ..Inner::default() };
        inner.settle_teams();
        assert_eq!(names(&inner), ["shop one", "work shop one"]);
        // once is enough: doing it again changes nothing
        inner.settle_teams();
        assert_eq!(names(&inner), ["shop one", "work shop one"]);
    }

    #[test]
    fn one_found_by_its_tools_list_goes_to_the_team_it_was_started_under_when_it_says_so() {
        let mut inner = Inner::default();
        // first heard of from its tool's list: no name, so it is in its folder's team
        assert_eq!(inner.check_in(&agent("a", "/x/shop", None)).text(), "shop one");
        // its own first report has the name it was started under
        let mut named = agent("a", "/x/shop", None);
        named.name = Some("garden".into());
        assert_eq!(inner.check_in(&named).text(), "garden one");
        // and stays there on every report after
        assert_eq!(inner.check_in(&named).text(), "garden one");
        // one he moved himself is left where he put it
        inner.check_in(&agent("b", "/x/shop", None));
        inner.move_to("b", "orchard", Mover::Him);
        let mut late = agent("b", "/x/shop", None);
        late.name = Some("garden".into());
        assert_eq!(inner.check_in(&late).project, "orchard");
    }

    /// A request for leave from `agent_id`, as `permit::ask` records it.
    fn asks_leave(inner: &mut Inner, agent_id: &str, permit: &str, what: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        let request = permit::Request { what: Some(what.into()), at_keyboard: false };
        let line = request.line().unwrap();
        inner.record(Entry { short: Some(line.clone()), options: vec![permit::ALLOW.into(), permit::DENY.into()], permit: Some(permit.into()), leave: Some(request), ..Entry::said(id.clone(), &s, Some(Kind::NeedsYou), line) });
        inner.permits.insert(permit.into(), Permit { agent_id: agent_id.into(), entry: id.clone(), decided: None, asked: Instant::now(), polled: Instant::now(), question: false });
        id
    }
    fn card(inner: &Inner, id: &str) -> airband_core::radio::Message {
        inner.message(inner.log.iter().find(|e| e.id == id).unwrap())
    }

    #[test]
    fn a_request_for_leave_offers_an_answer_only_while_the_harness_waits() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let line = asks_leave(&mut inner, "a", "p1", "run `make`");
        assert!(card(&inner, &line).permission && card(&inner, &line).options.len() == 2);
        // the harness stopped asking after it: the next sweep closes it, and the line says where it went
        inner.permits.get_mut("p1").unwrap().polled = Instant::now().checked_sub(Duration::from_secs(61)).unwrap();
        inner.reap();
        let over = card(&inner, &line);
        assert!(!over.permission && over.options.is_empty() && inner.permits.is_empty());
        assert_eq!(over.text, permit::Request { what: Some("run `make`".into()), at_keyboard: true }.line().unwrap());
        // it is still known as a request for leave, so a late answer is not taken for words to the agent
        assert!(inner.log.iter().any(|e| e.id == line && e.leave.is_some()));
    }

    #[test]
    fn every_way_a_request_ends_closes_its_line_and_says_whether_the_agent_waits() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let waits = |inner: &mut Inner| inner.session("a").unwrap().waiting;
        let open = |inner: &mut Inner, permit: &str| {
            let line = asks_leave(inner, "a", permit, "run `make`");
            inner.set_aloud(&line, Aloud::Waiting);
            inner.session("a").unwrap().waiting = true;
            line
        };
        // he answered: his answer waits to be collected, the line offers nothing more, and the agent goes on
        let line = open(&mut inner, "p1");
        inner.close_permit("p1", permit::Ended::Answered(true, String::new()));
        assert_eq!(inner.permits["p1"].decided, Some((true, String::new())));
        assert!(card(&inner, &line).options.is_empty() && !card(&inner, &line).permission && inner.unheard().next().is_none() && !waits(&mut inner));
        assert_eq!(card(&inner, &line).text, permit::Request { what: Some("run `make`".into()), at_keyboard: false }.line().unwrap());
        // the harness gave up waiting and asks at its keyboard: the line says so, and the agent still waits
        let line = open(&mut inner, "p2");
        inner.close_permit("p2", permit::Ended::AtKeyboard);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p2") && inner.unheard().next().is_none() && waits(&mut inner));
        assert_eq!(card(&inner, &line).text, permit::Request { what: Some("run `make`".into()), at_keyboard: true }.line().unwrap());
        // it asked something newer: the older line is closed
        let line = open(&mut inner, "p3");
        inner.close_permit("p3", permit::Ended::Replaced);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p3") && inner.unheard().next().is_none());
        // nobody asks after it any more: it was answered at the keyboard, or the agent has gone
        let line = open(&mut inner, "p4");
        inner.close_permit("p4", permit::Ended::Gone);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p4") && inner.unheard().next().is_none() && !waits(&mut inner));
        // a request that is already over is not ended twice
        inner.close_permit("p4", permit::Ended::Answered(true, String::new()));
        assert!(!inner.permits.contains_key("p4"));
    }

    #[test]
    fn a_request_from_before_the_tower_started_is_over() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let line = asks_leave(&mut inner, "a", "p1", "run `make`");
        // what a restart leaves: the line as saved, and no record of anyone waiting
        inner.permits.clear();
        inner.log.iter_mut().for_each(|e| e.permit = None);
        inner.close_requests_from_before();
        let c = card(&inner, &line);
        assert!(!c.permission && c.options.is_empty(), "{c:?}");
        assert!(inner.log.iter().any(|e| e.id == line && e.leave.is_some()));
    }

    #[test]
    fn numbers_in_order_and_a_name_he_gave_takes_its_number() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/shop", None));
        assert_eq!(names(&inner), ["shop one", "shop two"]);
        // he named a third session "shop 2": it takes two, and the one that had it by arrival moves on
        inner.check_in(&agent("c", "/x/shop", Some("shop 2")));
        assert_eq!(names(&inner), ["shop one", "shop three", "shop two"]);
        // the same name again changes nothing; a new name renames
        inner.check_in(&agent("c", "/x/shop", Some("shop 2")));
        inner.check_in(&agent("c", "/x/shop", Some("questions")));
        assert_eq!(names(&inner), ["shop one", "shop three", "shop questions"]);
        // a name another session was given by him is not taken from it
        inner.check_in(&agent("d", "/x/shop", Some("shop questions")));
        assert_eq!(names(&inner)[3], "shop two");
    }

    #[test]
    fn a_question_stays_open_through_a_rename_and_follows_the_agent() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/backoffice", None));
        let q = says(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        let before = inner.rev;
        inner.check_in(&agent("a", "/x/backoffice", Some("backoffice login fix")));
        assert!(inner.open_ids().contains(&q));
        let m = inner.message(&inner.log[0]);
        // the room is the project, however many words the name has
        assert_eq!((m.from.as_str(), m.group.as_str()), ("backoffice login fix", "backoffice"));
        // and radios are told the line changed
        assert!(inner.log[0].rev > before);
    }

    #[test]
    fn a_dead_agents_question_does_not_pass_to_whoever_takes_its_callsign() {
        let mut inner = Inner::default();
        inner.check_in(&agent("old", "/x/shop", None));
        let q = says(&mut inner, "old", Kind::NeedsYou, "Rewrite them?");
        inner.remove("old");
        inner.check_in(&agent("new", "/x/shop", None));
        assert_eq!(names(&inner), ["shop one"]);
        assert!(inner.open_ids().is_empty());
        // the old line still says who asked, and is nobody's now
        assert!(inner.sender(&inner.log.iter().find(|e| e.id == q).unwrap().clone()).is_none());
    }

    #[test]
    fn open_means_asked_and_not_yet_answered() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        let qa = says(&mut inner, "a", Kind::NeedsYou, "Rewrite them?");
        let qb = says(&mut inner, "b", Kind::NeedsYou, "Ship it?");
        says(&mut inner, "a", Kind::Fyi, "Still here.");
        assert_eq!(inner.open_ids(), HashSet::from([qa.clone(), qb.clone()]));
        // he answers one
        let to = vec![inner.find("a").unwrap().clone()];
        inner.record(Entry::his("r1".into(), &to, "rewrite".into()));
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        assert_eq!(inner.last_instruction("a").as_deref(), Some("rewrite"));
        // the other goes on to report something: its question is still his to answer
        says(&mut inner, "b", Kind::Report, "Also fixed the footer.");
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        // but once it has spoken past it three times it is let go: it has decided for itself by then
        says(&mut inner, "b", Kind::Report, "And the header.");
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        says(&mut inner, "b", Kind::Report, "And the sidebar.");
        assert!(inner.open_ids().is_empty());
        // a newer question of its own is open again
        let qb2 = says(&mut inner, "b", Kind::NeedsYou, "Ship it now, or after the weekend?");
        assert_eq!(inner.open_ids(), HashSet::from([qb2]));
    }

    #[test]
    fn what_an_agent_is_doing() {
        let mut inner = Inner::default();
        for id in ["w", "q", "s", "d"] {
            inner.check_in(&agent(id, &format!("/x/{id}"), None));
        }
        let q = says(&mut inner, "q", Kind::NeedsYou, "Ship it?");
        says(&mut inner, "s", Kind::Stalled, "Two done. I'll do the third next.");
        says(&mut inner, "d", Kind::Report, "Shipped.");
        for id in ["q", "s", "d"] {
            inner.session(id).unwrap().waiting = true;
        }
        let open = inner.open_ids();
        assert_eq!(open, HashSet::from([q]));
        let state = |id: &str| inner.state_of(inner.find(id).unwrap(), &open);
        assert_eq!([state("w"), state("q"), state("s"), state("d")], [AgentState::Working, AgentState::Asking, AgentState::Stalled, AgentState::Idle]);
        assert_eq!(inner.stalled().iter().map(|s| s.agent.id.as_str()).collect::<Vec<_>>(), ["s"]);
    }

    #[test]
    fn each_radio_has_its_own_call() {
        let mut inner = Inner::default();
        for radio in ["phone", "desk"] {
            let c = inner.call(radio);
            c.presence.says(ON_CALL);
            c.ready = true;
        }
        inner.call("desk").held = true;
        assert_eq!(inner.free_radio().as_deref(), Some("phone"));
        inner.check_in(&agent("a", "/x/shop", None));
        let t = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        inner.call("phone").outbox = Some(Transmission { id: t.clone(), callsign: "shop one".into(), kind: Kind::NeedsYou, say: String::new(), waiting: 0 });
        inner.call("phone").ready = false;
        // the agent goes back to work: what the phone was about to say is withdrawn, and only there
        inner.no_longer_waiting("a");
        assert!(inner.call("phone").outbox.is_none() && inner.call("phone").ready);
    }

    /// Something an agent said that is in the reading queue, as `intake::enqueue` leaves it once judged.
    fn queued(inner: &mut Inner, agent_id: &str, kind: Kind, text: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        inner.record(Entry { aloud: Aloud::Waiting, short: Some(text.into()), ..Entry::said(id.clone(), &s, Some(kind), text.into()) });
        id
    }
    fn aloud(inner: &Inner, id: &str) -> Aloud {
        inner.log.iter().find(|e| e.id == id).unwrap().aloud.clone()
    }
    fn dropped(why: Dropped) -> Aloud {
        Aloud::dropped(why)
    }

    #[test]
    fn a_report_he_has_not_heard_is_kept_when_he_speaks_to_its_agent_and_its_question_is_not() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        let question = queued(&mut inner, "a", Kind::NeedsYou, "Ship the next one?");
        inner.no_longer_waiting("a");
        assert_eq!((aloud(&inner, &report), aloud(&inner, &question)), (Aloud::Waiting, dropped(Dropped::Answered)));
        // something newer from the same agent still takes the place of what it said before
        inner.supersede("a", false);
        assert_eq!(aloud(&inner, &report), dropped(Dropped::Superseded));
        assert_eq!(inner.unheard().count(), 0);
    }

    #[test]
    fn every_way_out_of_the_reading_queue_is_written_down_with_its_reason() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        // routine: the judge found nothing for his ears
        let routine = queued(&mut inner, "a", Kind::Report, "Still going.");
        inner.fill(&routine, &judge::Judged { kind: Kind::Fyi, say: "Still going.".into(), options: Vec::new(), recommended: None, read: None, background: false, own_session: false });
        assert_eq!(aloud(&inner, &routine), dropped(Dropped::Routine));
        // dismissed, and a request for leave that is over
        let leave = asks_leave(&mut inner, "a", "p1", "run `make`");
        inner.set_aloud(&leave, Aloud::Waiting);
        inner.close_permit("p1", permit::Ended::Gone);
        assert_eq!(aloud(&inner, &leave), dropped(Dropped::PermissionEnded));
        // its agent has gone: the question goes, the report is still his news
        let report = queued(&mut inner, "b", Kind::Report, "Refunds sent.");
        let question = queued(&mut inner, "b", Kind::NeedsYou, "Send the rest?");
        inner.remove("b");
        assert_eq!((aloud(&inner, &report), aloud(&inner, &question)), (Aloud::Waiting, dropped(Dropped::AgentGone)));
        // said long before he turned reading aloud on
        inner.log.iter_mut().find(|e| e.id == report).unwrap().at = 1;
        inner.leave_old_news();
        assert_eq!(aloud(&inner, &report), dropped(Dropped::Stale));
        // each of those is in the trail `airband status` shows, with what it was and what it became
        let trail: Vec<&String> = inner.aloud_trail.iter().collect();
        for reason in ["routine", "permission ended", "agent gone", "stale"] {
            assert!(trail.iter().any(|l| l.contains(&format!("waiting → dropped ({reason})"))), "{reason}: {trail:?}");
        }
        assert!(inner.unheard().next().is_none());
    }

    #[test]
    fn a_line_dropped_while_he_was_listening_is_marked_as_missed_with_its_reason() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        // nobody on a call: a report replaced by a newer one was never going to be read, and is not marked
        let early = queued(&mut inner, "a", Kind::Report, "Half done.");
        inner.set_aloud(&early, Aloud::dropped(Dropped::Superseded));
        assert_eq!(inner.message(inner.log.iter().find(|e| e.id == early).unwrap()).not_read, None);
        // a radio on a call: the same thing is something he was listening for and did not hear
        inner.radio_says("car", ON_CALL);
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        let before = inner.rev;
        inner.set_aloud(&report, Aloud::dropped(Dropped::Superseded));
        let line = inner.log.iter().find(|e| e.id == report).unwrap();
        assert_eq!(inner.message(line).not_read, Some(Dropped::Superseded));
        // and the line counts as changed, so a radio showing the chat is sent it again with the mark
        assert!(line.rev > before);
        // what was never for his ears, or was said before he turned reading on, is not something he missed
        for why in [Dropped::Routine, Dropped::Stale] {
            let id = queued(&mut inner, "a", Kind::Report, "Still going.");
            inner.set_aloud(&id, Aloud::dropped(why));
            assert_eq!(inner.message(inner.log.iter().find(|e| e.id == id).unwrap()).not_read, None, "{why:?}");
        }
        // one he heard and that is then let go was not missed
        let heard = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        inner.set_aloud(&heard, Aloud::Heard { radio: "car".into(), at: 1 });
        inner.set_aloud(&heard, Aloud::dropped(Dropped::Dismissed));
        assert_eq!(inner.message(inner.log.iter().find(|e| e.id == heard).unwrap()).not_read, None);
    }

    #[test]
    fn a_radio_is_where_it_says_it_is() {
        use airband_core::radio::Presence as Said;
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        // it says it is showing the chat and not on a call: there is someone to ask, and nobody to read to
        inner.radio_says("phone", Said { on_call: false, showing_chat: true });
        assert!(inner.following() && !inner.connected() && inner.call("phone").presence.showing_chat());
        assert_eq!(inner.free_radio(), None);
        // it says it is on a call: it is free to be handed something, from that moment
        inner.radio_says("phone", ON_CALL);
        assert!(inner.connected() && !inner.call("phone").presence.showing_chat());
        assert_eq!(inner.free_radio().as_deref(), Some("phone"));
        // what it took and never said it heard goes back in the queue when it comes on a call again
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        inner.set_aloud(&report, Aloud::Handed { radio: "phone".into(), at: 1 });
        inner.radio_says("phone", Said::default());
        assert!(!inner.connected());
        inner.radio_says("phone", ON_CALL);
        assert_eq!(aloud(&inner, &report), Aloud::Waiting);
    }

    #[test]
    fn what_came_of_what_he_said_is_told_to_the_radio_that_said_it() {
        use airband_core::radio::Outcome;
        let mut inner = Inner::default();
        let before = inner.rev;
        inner.settle("phone", "u1", Outcome::Answered { reply: Reply { say: "Sent to shop one.".into(), ..Reply::default() } });
        inner.settle("desk", "u2", Outcome::Rejected { why: airband_core::radio::Rejected::TooOld });
        // each radio is told its own, once it asks from before the outcome
        assert_eq!(inner.settled_for("phone", Some(before)).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["u1"]);
        assert_eq!(inner.settled_for("desk", None).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["u2"]);
        // and not again once it has asked from after it
        assert!(inner.settled_for("phone", Some(inner.rev)).is_empty());
        // only so many are kept; the database has the rest for a radio that asks again by id
        (0..SETTLED_KEEP + 5).for_each(|n| inner.settle("phone", &format!("n{n}"), Outcome::Taken));
        assert_eq!(inner.settled.len(), SETTLED_KEEP);
    }

    #[test]
    fn what_a_radio_took_and_did_not_hear_goes_back_in_the_queue() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        inner.set_aloud(&report, Aloud::Handed { radio: "phone".into(), at: 1 });
        inner.call("phone").handed = Some(Transmission { id: report.clone(), callsign: "shop one".into(), kind: Kind::Report, say: String::new(), waiting: 0 });
        assert_eq!(inner.unheard().count(), 1);
        inner.give_back("phone");
        assert_eq!(aloud(&inner, &report), Aloud::Waiting);
        assert!(inner.call("phone").handed.is_none());
        // heard: it is out of the queue, and stays on record as heard
        inner.set_aloud(&report, Aloud::Heard { radio: "phone".into(), at: 2 });
        inner.give_back("phone");
        assert!(matches!(aloud(&inner, &report), Aloud::Heard { .. }) && inner.unheard().next().is_none());
    }

    #[test]
    fn at_start_a_gone_agents_question_goes_and_its_recent_report_stays() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        let live = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        let report = queued(&mut inner, "b", Kind::Report, "Refunds sent.");
        let old = queued(&mut inner, "b", Kind::Report, "Started.");
        let question = queued(&mut inner, "b", Kind::NeedsYou, "Send the rest?");
        inner.log.iter_mut().find(|e| e.id == old).unwrap().at = 1;
        inner.set_aloud(&live, Aloud::Handed { radio: "phone".into(), at: 1 });
        inner.sessions.retain(|s| s.agent.id != "b");
        inner.settle_reading();
        assert_eq!([aloud(&inner, &live), aloud(&inner, &report)], [Aloud::Waiting, Aloud::Waiting]);
        assert_eq!((aloud(&inner, &old), aloud(&inner, &question)), (dropped(Dropped::Stale), dropped(Dropped::AgentGone)));
    }

    #[test]
    fn a_radio_that_only_reads_and_types_keeps_who_it_was_talking_to() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        // never on a call: it has not polled for anything to play
        inner.call("phone").focus = Some("a".into());
        inner.reap();
        assert_eq!(inner.calls.get("phone").and_then(|c| c.focus.as_deref()), Some("a"));
    }
}
