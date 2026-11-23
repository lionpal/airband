use super::*;

const ON_CALL: airband_core::radio::Presence = airband_core::radio::Presence { on_call: true, showing_chat: false };

fn agent(id: &str, cwd: &str, title: Option<&str>) -> Agent {
    airband_core::agent::Agent { cwd: Some(cwd.into()), title: title.map(String::from), ..crate::adapters::agent_with(id, "test") }
}
/// An agent reports in, and is taken in.
fn joins(inner: &mut Inner, agent: &Agent) -> Callsign {
    inner.check_in(agent).expect("taken in")
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
    joins(&mut inner, &agent("a", &shop, None));
    inner.worked = vec![(back.clone(), vec!["some-tool".into()], 9), (other_shop.clone(), vec!["other-tool".into()], 7), (shop.clone(), vec!["other-tool".into(), "some-tool".into()], 5)];
    let recent = inner.recent();
    let listed: Vec<(&str, &str, bool, u64)> = recent.iter().map(|r| (r.name.as_str(), r.folder.as_str(), r.team.is_some(), r.at)).collect();
    // the folder with a team carries that team's id
    assert_eq!(recent.iter().find(|r| r.folder == shop).and_then(|r| r.team.clone()), inner.team_at(&shop));
    // newest first; a folder is called as its team is when it has one, and as a team there would be when not
    // another folder called as his team is would be a team of its own, its parent's name in front
    assert_eq!(listed, [("back office", back.as_str(), false, 9), ("elsewhere shop", other_shop.as_str(), false, 7), ("shop", shop.as_str(), true, 5)]);
    // by name: his team keeps its name, and a folder that is only recent never takes it
    let launchable: BTreeMap<String, String> = inner.launchable().into_iter().map(|p| (p.name, p.folder)).collect();
    assert_eq!(launchable.get("shop").map(|f| edges::folders::Folders::real(&edges::folders::Disk, f)), Some(edges::folders::Folders::real(&edges::folders::Disk, &shop)));
    assert_eq!(launchable.get("elsewhere shop"), Some(&other_shop));
    assert_eq!(launchable.get("back office"), Some(&back));
    assert_eq!(inner.teams.len(), 1);
    // a team no tool has a record of is still listed, last
    inner.worked = vec![(back.clone(), vec!["some-tool".into()], 9)];
    assert_eq!(inner.recent().iter().map(|r| (r.name.as_str(), r.team.is_some(), r.at)).collect::<Vec<_>>(), [("back office", false, 9), ("shop", true, 0)]);
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn two_recent_folders_that_would_be_called_alike_are_both_places_to_start_in() {
    let inner = Inner { worked: vec![("/a/x/shop".into(), vec!["some-tool".into()], 9), ("/b/y/shop".into(), vec!["some-tool".into()], 7)], ..Inner::default() };
    let folders: Vec<String> = inner.launchable().into_iter().map(|p| p.folder).collect();
    assert_eq!(folders, ["/a/x/shop", "/b/y/shop"]);
}

#[test]
fn a_team_is_a_folder_and_two_folders_that_read_alike_are_two_teams_called_apart() {
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    assert_eq!(joins(&mut inner, &agent("a", "/a/work/shop", None)).text(), "shop one");
    // another folder of the same name: its parent's name goes in front, so the two are never said alike
    assert_eq!(joins(&mut inner, &agent("b", "/b/work/shop", None)).text(), "work shop one");
    // and a third goes up one more
    assert_eq!(joins(&mut inner, &agent("c", "/c/work/shop", None)).text(), "c work shop one");
    // a second agent in a folder joins that folder's team, however the path is written
    assert_eq!(joins(&mut inner, &agent("d", "/b/work/shop/", None)).text(), "work shop two");
    assert_eq!(joins(&mut inner, &agent("e", "/a/work/shop", None)).text(), "shop two");
    let named = |inner: &Inner, folder: &str| inner.team_name(&inner.team_at(folder).unwrap()).unwrap().to_string();
    assert_eq!(named(&inner, "/b/work/shop"), "work shop");
    let d = inner.folder_team("/d/shop").unwrap();
    assert_eq!((d.as_str(), inner.team_name(&d).unwrap()), ("/d/shop", "d shop"));
}

#[test]
fn a_folder_whose_name_another_computer_has_takes_this_computers_name_in_front() {
    let theirs = |team: &str| Callsign { project: team.into(), number: "one".into(), named: false };
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    // told first: a new folder's team keeps off the name
    inner.taken_elsewhere(&[theirs("shop")]);
    assert_eq!(joins(&mut inner, &agent("a", "/x/shop", None)).text(), "laptop shop one");
    // told after: a team here that has the name is renamed, and its agents go with it
    assert_eq!(joins(&mut inner, &agent("b", "/x/garden", None)).text(), "garden one");
    assert!(inner.taken_elsewhere(&[theirs("shop"), theirs("garden")]));
    assert_eq!(names(&inner), ["laptop shop one", "laptop garden one"]);
    let garden = inner.team_at("/x/garden").unwrap();
    assert_eq!(inner.team_name(&garden), Some("laptop garden"));
    // he cannot give a team a name a team on another of his computers has: names stay distinct
    assert!(inner.rename_team(&garden, "garden").is_err());
    // a name he gave a team is his: it stays, and only numbers that clash move
    inner.rename_team(&garden, "orchard").unwrap();
    assert!(inner.taken_elsewhere(&[theirs("shop"), theirs("garden"), theirs("orchard")]));
    assert_eq!(names(&inner), ["laptop shop one", "orchard two"]);
}

#[test]
fn a_team_whose_every_name_is_taken_keeps_none_that_clashes() {
    let theirs = |team: &str| Callsign { project: team.into(), number: "one".into(), named: false };
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    joins(&mut inner, &agent("a", "/x/shop", None));
    let shop = inner.team_at("/x/shop").unwrap();
    // every name its folder could have, with or without this computer's name, is a team elsewhere
    inner.taken_elsewhere(&[theirs("shop"), theirs("x shop"), theirs("laptop shop"), theirs("laptop x shop")]);
    assert_eq!(inner.team_name(&shop), Some("shop"));
    // no name is free for its folder: none is made up
    assert!(inner.name_for_folder("/x/shop").is_none());
}

#[test]
fn an_agent_taken_over_keeps_its_name_and_the_radios_are_told() {
    let mut inner = Inner::default();
    let first = joins(&mut inner, &agent("a", "/x/shop", None));
    let s = inner.find("a").cloned().unwrap();
    inner.record(Entry::said("m1".into(), &s, None, "done".into()));
    // back from a new process under another number, as checking in again can give it
    inner.session("a").unwrap().callsign.number = "two".into();
    let rev = inner.rev;
    assert!(inner.keep_name("a", first.clone()));
    assert_eq!(inner.find("a").unwrap().callsign, first);
    assert!(inner.rev > rev && inner.log.iter().find(|e| e.id == "m1").unwrap().rev > rev);
    // a name someone else has taken meanwhile is not taken from them
    joins(&mut inner, &agent("b", "/x/shop", None));
    let b = inner.find("b").unwrap().callsign.clone();
    assert!(!inner.keep_name("a", b));
}

#[test]
fn renaming_a_team_changes_only_what_it_is_called() {
    let mut inner = Inner::default();
    joins(&mut inner, &agent("a", "/x/shop", Some("refunds")));
    joins(&mut inner, &agent("b", "/x/shop", None));
    let shop = inner.team_at("/x/shop").unwrap();
    assert_eq!(inner.rename_team(&shop, "Store Front"), Ok(TeamName::new("store front").unwrap()));
    assert_eq!(names(&inner), ["store front refunds", "store front one"]);
    // its folder is still its folder: a new agent there joins it
    assert_eq!(joins(&mut inner, &agent("c", "/x/shop", None)).text(), "store front two");
    // a name another team has, or none, is refused in words
    joins(&mut inner, &agent("d", "/x/garden", None));
    let garden = inner.team_at("/x/garden").unwrap();
    assert_eq!(inner.rename_team(&garden, "store front"), Err("There is already a team called store front, here or on another of your computers.".to_string()));
    assert!(inner.rename_team(&garden, "!!").is_err());
    // a team is found by its id, and never by what it is called
    assert!(inner.rename_team(&TeamId::new("garden"), "x").is_err());
}

#[test]
fn a_renamed_team_keeps_its_lines_its_folder_and_its_id() {
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    joins(&mut inner, &agent("a", "/x/shop", None));
    let shop = inner.team_at("/x/shop").unwrap();
    says(&mut inner, "a", Kind::Report, "done");
    // its agent goes away; what it said stays in the team's view after the team is renamed
    inner.remove("a");
    inner.rename_team(&shop, "store").unwrap();
    let line = inner.log.last().unwrap().clone();
    assert!(inner.in_view(&line, Some(&shop), None));
    let m = inner.message(&line);
    assert_eq!((m.team.as_ref().map(|t| t.as_str()), m.group.as_str()), (Some(shop.as_str()), "store"));
    // renamed by the tower (another computer has the name): the id, and so the phone's room and colour, stay
    joins(&mut inner, &agent("b", "/x/garden", None));
    let garden = inner.team_at("/x/garden").unwrap();
    let theirs = Callsign { project: "garden".into(), number: "one".into(), named: false };
    assert!(inner.taken_elsewhere(&[theirs]));
    let b = inner.find("b").unwrap();
    assert_eq!((b.team.as_str(), b.callsign.project.as_str()), (garden.as_str(), "laptop garden"));
    assert_eq!(inner.roster_entries(&[])[0].team, garden);
}

#[test]
fn a_configured_team_with_no_agents_is_one_team_and_can_be_renamed() {
    let here = std::env::temp_dir().join(format!("airband-configured-{}", config::random_hex(6)));
    std::fs::create_dir_all(here.join("Shop")).unwrap();
    let folder = here.join("Shop").display().to_string();
    let listed: BTreeMap<String, String> = [("Shop".to_string(), folder.clone()), ("!!".to_string(), folder.clone())].into();
    let mut inner = Inner::default();
    inner.configure_teams(&listed);
    // listed once, under its configured name; a name with nothing in it is left out
    assert_eq!(inner.folder_teams().into_iter().collect::<Vec<_>>(), [("shop".to_string(), edges::folders::Folders::real(&edges::folders::Disk, &folder))]);
    let shop = inner.team_at(&folder).unwrap();
    assert_eq!(inner.rename_team(&shop, "store"), Ok(TeamName::new("store").unwrap()));
    assert_eq!(inner.folder_teams().into_keys().collect::<Vec<_>>(), ["store"]);
    // read from the configuration again at the next start: his name stays, and there is still one team
    inner.configure_teams(&listed);
    assert_eq!(inner.folder_teams().into_keys().collect::<Vec<_>>(), ["store"]);
    // an agent that starts there joins it
    assert_eq!(joins(&mut inner, &agent("a", &folder, None)).text(), "store one");
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn one_found_by_its_tools_list_goes_to_the_team_it_was_started_under_when_it_says_so() {
    let mut inner = Inner::default();
    // first heard of from its tool's list: no name, so it is in its folder's team
    assert_eq!(joins(&mut inner, &agent("a", "/x/shop", None)).text(), "shop one");
    // its own first report has the name it was started under
    let mut named = agent("a", "/x/shop", None);
    named.name = Some("garden".into());
    assert_eq!(joins(&mut inner, &named).text(), "garden one");
    // and stays there on every report after
    assert_eq!(joins(&mut inner, &named).text(), "garden one");
    // one he moved himself is left where he put it
    joins(&mut inner, &agent("b", "/x/shop", None));
    let orchard = inner.new_team("orchard", true).unwrap();
    inner.move_to("b", &orchard, Mover::Him);
    let mut late = agent("b", "/x/shop", None);
    late.name = Some("garden".into());
    assert_eq!(joins(&mut inner, &late).project, "orchard");
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    joins(&mut inner, &agent("b", "/x/shop", None));
    assert_eq!(names(&inner), ["shop one", "shop two"]);
    // he named a third session "shop 2": it takes two, and the one that had it by arrival moves on
    joins(&mut inner, &agent("c", "/x/shop", Some("shop 2")));
    assert_eq!(names(&inner), ["shop one", "shop three", "shop two"]);
    // the same name again changes nothing; a new name renames
    joins(&mut inner, &agent("c", "/x/shop", Some("shop 2")));
    joins(&mut inner, &agent("c", "/x/shop", Some("questions")));
    assert_eq!(names(&inner), ["shop one", "shop three", "shop questions"]);
    // a name another session was given by him is not taken from it
    joins(&mut inner, &agent("d", "/x/shop", Some("shop questions")));
    assert_eq!(names(&inner)[3], "shop two");
}

#[test]
fn a_question_stays_open_through_a_rename_and_follows_the_agent() {
    let mut inner = Inner::default();
    joins(&mut inner, &agent("a", "/x/backoffice", None));
    let q = says(&mut inner, "a", Kind::NeedsYou, "Ship it?");
    let before = inner.rev;
    joins(&mut inner, &agent("a", "/x/backoffice", Some("backoffice login fix")));
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
    joins(&mut inner, &agent("old", "/x/shop", None));
    let q = says(&mut inner, "old", Kind::NeedsYou, "Rewrite them?");
    inner.remove("old");
    joins(&mut inner, &agent("new", "/x/shop", None));
    assert_eq!(names(&inner), ["shop one"]);
    assert!(inner.open_ids().is_empty());
    // the old line still says who asked, and is nobody's now
    assert!(inner.sender(&inner.log.iter().find(|e| e.id == q).unwrap().clone()).is_none());
}

#[test]
fn open_means_asked_and_not_yet_answered() {
    let mut inner = Inner::default();
    joins(&mut inner, &agent("a", "/x/shop", None));
    joins(&mut inner, &agent("b", "/x/backoffice", None));
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
        joins(&mut inner, &agent(id, &format!("/x/{id}"), None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    let t = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
    inner.call("phone").outbox = Some(Transmission { id: t.clone(), agent: String::new(), callsign: "shop one".into(), kind: Kind::NeedsYou, say: String::new(), waiting: 0 });
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    joins(&mut inner, &agent("b", "/x/backoffice", None));
    // routine: the judge found nothing for his ears
    let routine = queued(&mut inner, "a", Kind::Report, "Still going.");
    inner.fill(&routine, &judge::Judged { kind: Kind::Fyi, say: "Still going.".into(), cut: false, options: Vec::new(), recommended: None, read: None, background: false, own_session: false });
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
    inner.set_aloud(&report, Aloud::Handed { radio: "phone".into(), at: 1 });
    inner.call("phone").handed = Some(Transmission { id: report.clone(), agent: String::new(), callsign: "shop one".into(), kind: Kind::Report, say: String::new(), waiting: 0 });
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    joins(&mut inner, &agent("b", "/x/backoffice", None));
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
    joins(&mut inner, &agent("a", "/x/shop", None));
    // never on a call: it has not polled for anything to play
    inner.call("phone").focus = Some("a".into());
    inner.reap();
    assert_eq!(inner.calls.get("phone").and_then(|c| c.focus.as_deref()), Some("a"));
}

fn kept_session(id: &str, cwd: Option<&str>, project: &str) -> Session {
    // kept in a team that is no longer there
    Session::new(Agent { cwd: cwd.map(String::from), ..agent(id, "", None) }, Callsign { project: project.into(), number: "one".into(), named: false }, TeamId::new(format!("/gone/{project}")))
}

#[test]
fn an_agent_no_team_can_be_found_for_is_not_taken_in_under_a_made_up_team() {
    let mut inner = Inner::default();
    // no name it was started under, and a folder with no name in its path
    let _ = inner.check_in(&agent("a", "/", None));
    assert!(inner.sessions.is_empty());
    assert!(inner.teams.is_empty());
}

#[test]
fn a_kept_agent_whose_team_is_gone_goes_to_its_folders_team_or_is_let_go() {
    let mut inner = Inner { sessions: vec![kept_session("a", Some("/x/shop"), "gone"), kept_session("b", None, "!!")], ..Inner::default() };
    inner.settle_teams();
    // no team is made from a name nothing has, and nobody is left in no team
    assert_eq!(inner.teams.values().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["shop"]);
    assert_eq!(inner.sessions.iter().map(|s| (s.agent.id.as_str(), s.team.as_str())).collect::<Vec<_>>(), [("a", "/x/shop")]);
}

#[test]
fn an_agent_started_under_a_name_comes_back_to_its_team_after_he_renames_it() {
    let mut inner = Inner::default();
    let mut garden = agent("a", "/x/shop", None);
    garden.name = Some("garden".into());
    assert_eq!(joins(&mut inner, &garden).text(), "garden one");
    let team = inner.team_called("garden").unwrap();
    inner.rename_team(&team, "orchard").unwrap();
    // it goes away and checks in again, still started under the old name
    inner.remove("a");
    assert_eq!(joins(&mut inner, &garden).text(), "orchard one");
    assert_eq!(inner.teams.len(), 1);
}

#[test]
fn a_name_an_agent_is_started_under_keeps_team_names_distinct_and_short() {
    let mut inner = Inner { here: "Laptop".into(), elsewhere: vec![Callsign { project: "garden".into(), number: "one".into(), named: false }], ..Inner::default() };
    let mut a = agent("a", "/x/shop", None);
    a.name = Some("garden".into());
    let _ = inner.check_in(&a);
    let mut b = agent("b", "/x/shop", None);
    b.name = Some("a name far longer than any team may have".into());
    let _ = inner.check_in(&b);
    // neither is a team here: one is the name of a team on another computer, the other is too long
    assert!(inner.teams.values().all(|t| t.name != "garden" && t.name.as_str().chars().count() <= 24), "{:?}", inner.teams);
}

#[test]
fn a_clash_with_another_computer_that_no_name_settles_is_not_said_to_be_a_rename() {
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    let shop = inner.folder_team("/shop").unwrap();
    let theirs = |t: &str| Callsign { project: t.into(), number: "nine".into(), named: false };
    assert!(!inner.taken_elsewhere(&[theirs("shop"), theirs("laptop shop")]));
    assert_eq!(inner.team_name(&shop), Some("shop"));
}

#[test]
fn a_new_team_is_never_an_old_one_by_its_name_and_a_models_name_is_not_his() {
    let mut inner = Inner::default();
    inner.folder_team("/x/shop").unwrap();
    assert!(inner.new_team("shop", false).is_err());
    assert_eq!(inner.teams.len(), 1);
    let orchard = inner.new_team("orchard", false).unwrap();
    assert!(!inner.teams[&orchard].named);
}

#[test]
fn a_start_keeps_the_name_he_gave_a_configured_team_and_makes_no_ghost() {
    let listed: BTreeMap<String, String> = [("shop".to_string(), "/x/shop".to_string())].into();
    let mut inner = Inner::default();
    inner.new_folder_team("/x/shop", TeamName::new("garden").unwrap(), true);
    let mut kept = kept_session("a", Some("/x/shop"), "garden");
    kept.team = TeamId::new("/x/shop");
    inner.sessions.push(kept);
    inner.teams_at_start(&listed);
    assert_eq!(inner.teams.iter().map(|(id, t)| (id.as_str(), t.name.as_str(), t.named)).collect::<Vec<_>>(), [("/x/shop", "garden", true)]);
    assert_eq!(inner.sessions[0].callsign.text(), "garden one");
}

#[test]
fn a_start_keeps_the_name_the_tower_gave_a_configured_team_to_stay_distinct() {
    let listed: BTreeMap<String, String> = [("shop".to_string(), "/x/shop".to_string())].into();
    let mut inner = Inner { here: "Laptop".into(), ..Inner::default() };
    inner.configure_teams(&listed);
    assert!(inner.taken_elsewhere(&[Callsign { project: "shop".into(), number: "one".into(), named: false }]));
    assert_eq!(inner.team_name(&TeamId::new("/x/shop")), Some("laptop shop"));
    // the next start knows nothing of the other computer yet: the name it was given stays
    let mut next = Inner { here: "Laptop".into(), teams: inner.teams.clone(), ..Inner::default() };
    next.configure_teams(&listed);
    assert_eq!(next.team_name(&TeamId::new("/x/shop")), Some("laptop shop"));
}

#[test]
fn a_renamed_teams_agents_never_take_a_callsign_another_agent_has() {
    let mut inner = Inner::default();
    let shop = inner.folder_team("/x/shop").unwrap();
    let garden = inner.folder_team("/x/garden").unwrap();
    let mut a = kept_session("a", Some("/x/shop"), "shop");
    a.team = shop.clone();
    // one in another team still called by a name the renamed team now takes
    let mut b = kept_session("b", Some("/x/garden"), "orchard");
    b.team = garden;
    inner.sessions = vec![a, b];
    inner.rename_team(&shop, "orchard").unwrap();
    assert_eq!(names(&inner), ["orchard two", "orchard one"]);
}

#[test]
fn an_agent_started_under_a_name_no_team_can_have_goes_in_its_folders_team() {
    let mut inner = Inner::default();
    let mut a = agent("a", "/x/shop", None);
    a.name = Some("a name far longer than any team may have".into());
    assert_eq!(inner.check_in(&a).map(|c| c.text()).as_deref(), Some("shop one"));
}

#[test]
fn a_team_is_let_go_only_when_nothing_is_in_it_and_its_folder_is_gone() {
    let here = std::env::temp_dir().join(format!("airband-unused-{}", config::random_hex(6)));
    let folder = |name: &str| {
        std::fs::create_dir_all(here.join(name)).unwrap();
        here.join(name).display().to_string()
    };
    let (kept, gone, talked) = (folder("kept"), folder("gone"), folder("talked"));
    let mut inner = Inner::default();
    let ids: Vec<TeamId> = [&kept, &gone, &talked].iter().map(|f| inner.folder_team(f).unwrap()).collect();
    joins(&mut inner, &agent("a", &talked, None));
    says(&mut inner, "a", Kind::Report, "done");
    inner.remove("a");
    let named = inner.new_team("errands", true).unwrap();
    std::fs::remove_dir_all(here.join("gone")).unwrap();
    std::fs::remove_dir_all(here.join("talked")).unwrap();
    assert!(inner.drop_unused_teams());
    // a folder still there keeps its team; one whose chat lines remain keeps it; a team with no folder and
    // nothing in it goes, as does the one whose folder went
    assert!(inner.teams.contains_key(&ids[0]));
    assert!(!inner.teams.contains_key(&ids[1]));
    assert!(inner.teams.contains_key(&ids[2]));
    assert!(!inner.teams.contains_key(&named));
    assert!(!inner.drop_unused_teams());
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn no_way_of_naming_a_team_gives_it_a_name_another_team_has() {
    let here = std::env::temp_dir().join(format!("airband-samename-{}", config::random_hex(6)));
    let folder = |name: &str| {
        std::fs::create_dir_all(here.join(name)).unwrap();
        here.join(name).display().to_string()
    };
    let (shop, back) = (folder("shop"), folder("back"));
    let mut inner = Inner::default();
    let shop_id = inner.folder_team(&shop).unwrap();
    let back_id = inner.folder_team(&back).unwrap();
    assert_eq!(inner.team_name(&shop_id), Some("shop"));
    assert_eq!(inner.team_name(&back_id), Some("back"));
    assert!(inner.rename_team(&back_id, "shop").is_err());
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn whether_there_is_more_to_read_is_the_verdict_kept_on_the_line_never_a_comparison_of_words() {
    let mut inner = Inner::default();
    joins(&mut inner, &agent("a", "/x/shop", None));
    let id = queued(&mut inner, "a", Kind::Report, "Refunds sent. All forty of them, each checked against the order.");
    let verdict = |say: &str, cut: bool| judge::Judged { kind: Kind::Report, say: say.into(), cut, options: Vec::new(), recommended: None, read: None, background: false, own_session: false };
    // the model cut it down, though its line reads like the start of the message
    inner.fill(&id, &verdict("Refunds sent.", true));
    let e = inner.log.iter().find(|e| e.id == id).unwrap().clone();
    assert!(inner.message(&e).long);
    // the model said it is fine as written, though its words differ from the message's
    inner.fill(&id, &verdict("Refunds sent, all forty checked.", false));
    let e = inner.log.iter().find(|e| e.id == id).unwrap().clone();
    assert!(!inner.message(&e).long);
}

#[test]
fn a_line_not_yet_summarised_has_more_to_read_exactly_when_its_opening_stops_short() {
    let line = |full: &str| Entry { id: "l".into(), full: full.into(), ..Entry::default() };
    assert!(!line("Refunds sent.").more_to_read());
    assert!(!line("Refunds sent. All forty.").more_to_read());
    assert!(line("Refunds sent. All forty. Each checked against the order.").more_to_read());
}

#[test]
fn of_two_free_radios_the_one_he_showed_was_there_last_takes_it_never_the_first_by_name() {
    let mut inner = Inner::default();
    for radio in ["zed", "alpha"] {
        let c = inner.call(radio);
        c.presence.says(ON_CALL);
        c.ready = true;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(inner.free_radio().as_deref(), Some("alpha"));
    // a request from the other shows it is there now
    inner.call("zed");
    assert_eq!(inner.free_radio().as_deref(), Some("zed"));
}
