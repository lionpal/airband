//! What every computer is to be told: each computer's answer kept by its id, one that says no changing nothing
//! for the others, and one not reached told when it is. Against stand-in towers ([`super::fake::FakeTower`]).
use super::fake::FakeTower;
use super::*;
use airband_core::radio::wire::{Refusal, ReadsSet, at};
use airband_core::radio::{Feed, ReadsOffer, ReadsOffered};
use airband_core::radio::wire::Done;
use std::collections::BTreeMap;

fn link(n: u8) -> String {
    format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t{n}")
}
/// A stand-in for the computer called `name`, offering one summary service; `has_key` when it keeps the key.
fn tower(name: &str, has_key: bool) -> Arc<FakeTower> {
    let t = Arc::new(FakeTower::default());
    t.answers::<at::Feed>(&Feed { station: name.into(), ..Feed::default() });
    let offer = ReadsOffer { provider: "anthropic".into(), offer: "haiku".into(), label: "Haiku".into(), key: true, has_key, preferred: true, ..ReadsOffer::default() };
    t.answers::<at::ReadsOffered>(&ReadsOffered { offers: vec![offer], ..ReadsOffered::default() });
    t
}
/// A desk keeping its choices in `dir`, where his main computer has said it offers that service with its
/// key kept there.
fn desk_with(dir: &std::path::Path, towers: Vec<(String, Arc<FakeTower>)>) -> Desk {
    let offered = serde_json::json!({ "offered": { "offers": [{ "provider": "anthropic", "offer": "haiku", "label": "Haiku", "key": true, "has_key": true, "preferred": true }] } });
    std::fs::write(dir.join("choices.json"), offered.to_string()).unwrap();
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk::through(dir.to_str().unwrap(), host, move |l| towers.iter().find(|(k, _)| k == l).map(|(_, t)| t.clone()).unwrap_or_default())
}
/// Wait until every computer listed has answered.
fn all_answered(desk: &Desk, n: usize) -> bool {
    for _ in 0..300 {
        if desk.shared.state.lock().unwrap().computers.iter().filter(|s| s.online).count() == n {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}
fn temp(what: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("airband-intents-{what}-{}", random_hex(4)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn reads_lines(desk: &Desk) -> Vec<(String, DeskDelivery)> {
    looked(desk, None, None).intents.into_iter().filter(|i| i.kind == IntentKind::Reads).flat_map(|i| i.computers.into_iter().map(|c| (c.name, c.state))).collect()
}

#[test]
fn a_computer_that_says_no_to_a_summary_choice_is_shown_with_why_and_the_others_keep_it() {
    let dir = temp("partial");
    let (studio, laptop) = (tower("studio", true), tower("laptop", true));
    studio.answers::<at::Reads>(&ReadsSet { reads: "Haiku".into() });
    laptop.refuses::<at::Reads>(409, &Refusal::told("That service is not offered here."));
    let desk = desk_with(&dir, vec![(link(1), studio.clone()), (link(2), laptop.clone())]);
    desk.set_links(vec![link(1), link(2)]);
    assert!(all_answered(&desk, 2));
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    let chose = desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false);
    let lines = reads_lines(&desk);
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(chose.is_ok(), "one computer took it, so the choice stands: {chose:?}");
    assert_eq!(lines, [("laptop".to_string(), DeskDelivery::Refused { reason: "That service is not offered here.".into() })]);
    assert_eq!((studio.asked_on::<at::Reads>(), laptop.asked_on::<at::Reads>()), (1, 1));
}

#[test]
fn a_summary_choice_no_computer_takes_changes_nothing_and_says_why() {
    let dir = temp("none");
    let laptop = tower("laptop", true);
    laptop.refuses::<at::Reads>(409, &Refusal::told("That service is not offered here."));
    let desk = desk_with(&dir, vec![(link(1), laptop)]);
    desk.set_links(vec![link(1)]);
    assert!(all_answered(&desk, 1));
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    let chose = desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false);
    let kept = desk.shared.chooser.reads_intent();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(&chose, Err(RadioError::Told { reason }) if reason == "That service is not offered here."), "{chose:?}");
    assert!(kept.is_none(), "a choice no computer took was kept: {kept:?}");
}

#[test]
fn a_computer_not_reached_is_sent_the_choice_when_it_is_and_then_it_is_kept_for_later() {
    let dir = temp("later");
    let (studio, laptop) = (tower("studio", true), tower("laptop", true));
    studio.answers::<at::Reads>(&ReadsSet { reads: "Haiku".into() });
    laptop.answers::<at::Reads>(&ReadsSet { reads: "Haiku".into() });
    let desk = desk_with(&dir, vec![(link(1), studio), (link(2), laptop.clone())]);
    desk.set_links(vec![link(1), link(2)]);
    assert!(all_answered(&desk, 2));
    // the laptop is away when he chooses: off, so nothing reaches it until it is back
    laptop.away.store(true, std::sync::atomic::Ordering::SeqCst);
    let laptop_id = { let mut state = desk.shared.state.lock().unwrap(); let c = state.computers.iter_mut().find(|c| c.link == link(2)).unwrap(); c.online = false; c.id() };
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false).unwrap();
    let waiting = reads_lines(&desk);
    assert_eq!(laptop.asked_on::<at::Reads>(), 0);
    // it is reached: it is sent the choice, and with every computer having it nothing is left waiting
    laptop.away.store(false, std::sync::atomic::Ordering::SeqCst);
    let epoch = desk.shared.state.lock().unwrap().computer(&laptop_id).map(|c| c.epoch).unwrap();
    crate::choices::tell_reads(&desk.shared, &laptop_id, epoch, true);
    let kept = desk.shared.chooser.reads_intent();
    let after = reads_lines(&desk);
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(waiting, [("laptop".to_string(), DeskDelivery::Waiting)]);
    assert_eq!(laptop.asked_on::<at::Reads>(), 1);
    // every computer has it: nothing is left waiting, and it is kept for a computer paired later (P-B4)
    assert!(after.is_empty(), "{after:?}");
    assert!(kept.is_some_and(|k| k.per.values().all(|d| *d == super::super::intents::Delivery::Done)));
}

#[test]
fn a_computer_is_sent_the_key_only_when_it_has_none_of_its_own() {
    let wanted = crate::choices::ReadsWanted { id: "anthropic".into(), offer: "haiku".into(), set: BTreeMap::new(), his: true };
    let offers = |has_key: bool| ReadsOffered { offers: vec![ReadsOffer { provider: "anthropic".into(), offer: "haiku".into(), key: true, has_key, ..ReadsOffer::default() }], ..ReadsOffered::default() };
    // it keeps its own: none is sent from a phone that has none
    assert_eq!(wanted.body_for(None, &offers(true)).map(|b| b.key), Ok(String::new()));
    // it keeps its own and the phone has one too: the computer's own is used, the phone's goes nowhere (C-B1)
    assert_eq!(wanted.body_for(Some("sk-1"), &offers(true)).map(|b| b.key), Ok(String::new()));
    // it has none and the phone has one: the phone's is sent
    assert_eq!(wanted.body_for(Some("sk-1"), &offers(false)).map(|b| b.key), Ok("sk-1".to_string()));
    // neither has one: it is not sent, and why is said
    assert_eq!(wanted.body_for(None, &offers(false)).map(|b| b.key), Err(crate::choices::NO_KEY.to_string()));
    // one that does not offer it at all: the provider's own row says a key is needed
    assert_eq!(wanted.body_for(None, &ReadsOffered::default()).map(|b| b.key), Err(crate::choices::NO_KEY.to_string()));
}

#[test]
fn what_each_computer_was_told_is_still_known_after_the_app_opens_again() {
    let dir = temp("relaunch");
    let target = airband_core::radio::PushTarget { provider: PushService::default(), token: "abc".into() };
    {
        let mut kept = super::super::intents::KeptIntents::open(dir.to_str().unwrap());
        let (changed, lost) = kept.change(|i| {
            let changed = i.want_push(target.clone());
            i.push.as_mut().unwrap().record(&StationId::from("studio"), &Reach::<()>::Done(()));
            i.push.as_mut().unwrap().record(&StationId::from("laptop"), &Reach::<()>::Refused("No.".into()));
            changed
        });
        assert!(changed && lost.is_none());
    }
    let mut again = super::super::intents::KeptIntents::open(dir.to_str().unwrap());
    let push = again.now.push.clone().unwrap();
    // the same address again is nothing new: nobody is told twice
    let (changed, _) = again.change(|i| i.want_push(target.clone()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!changed);
    assert!(!push.owed(&StationId::from("studio")) && !push.owed(&StationId::from("laptop")) && push.owed(&StationId::from("mini")));
    assert_eq!(push.per["laptop"], Delivery::Refused { reason: "No.".into() });
}

#[test]
fn where_the_phone_can_be_woken_goes_to_each_computer_once_and_a_no_stays_with_that_computer() {
    let dir = temp("push");
    let (studio, laptop) = (tower("studio", true), tower("laptop", true));
    studio.answers::<at::Push>(&Done {});
    laptop.refuses::<at::Push>(409, &Refusal::told("Notifications are off here."));
    let desk = desk_with(&dir, vec![(link(1), studio.clone()), (link(2), laptop.clone())]);
    desk.set_links(vec![link(1), link(2)]);
    assert!(all_answered(&desk, 2));
    desk.set_push(PushService::default(), "abc".into());
    let settled = || desk.shared.state.lock().unwrap().intents.now.push.as_ref().is_some_and(|p| p.per.len() == 2);
    for _ in 0..300 {
        if settled() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let asked = (studio.asked_on::<at::Push>(), laptop.asked_on::<at::Push>());
    // the follow loop goes round again: neither is asked again, the one that said no included
    thread::sleep(Duration::from_millis(100));
    let after = (studio.asked_on::<at::Push>(), laptop.asked_on::<at::Push>());
    let lines: Vec<(String, DeskDelivery)> = looked(&desk, None, None).intents.into_iter().filter(|i| i.kind == IntentKind::Push).flat_map(|i| i.computers.into_iter().map(|c| (c.name, c.state))).collect();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(asked.0 >= 1 && asked.1 >= 1, "{asked:?}");
    assert_eq!(asked, after);
    assert_eq!(lines, [("laptop".to_string(), DeskDelivery::Refused { reason: "Notifications are off here.".into() })]);
}

/// As [`desk_with`], with this summary choice waiting to be sent (`reads`, as `choices.json` keeps it).
fn desk_wanting(dir: &std::path::Path, towers: Vec<(String, Arc<FakeTower>)>, reads: serde_json::Value) -> Desk {
    let kept = serde_json::json!({ "reads": reads, "offered": { "offers": [{ "provider": "anthropic", "offer": "haiku", "label": "Haiku", "key": true, "has_key": true, "preferred": true }] } });
    std::fs::write(dir.join("choices.json"), kept.to_string()).unwrap();
    let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
    Desk::through(dir.to_str().unwrap(), host, move |l| towers.iter().find(|(k, _)| k == l).map(|(_, t)| t.clone()).unwrap_or_default())
}
fn wanted_haiku(per: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "rev": 1, "wanted": { "id": "anthropic", "offer": "haiku", "his": true }, "per": per })
}
/// How that computer stands with the summary choice, as the desk keeps it now.
fn standing_of(desk: &Desk, computer: &StationId) -> Option<super::super::intents::Delivery> {
    desk.shared.chooser.reads_intent().and_then(|i| i.per.get(computer).cloned())
}
fn until(mut done: impl FnMut() -> bool) -> bool {
    (0..500).any(|_| {
        let now = done();
        if !now {
            thread::sleep(Duration::from_millis(10));
        }
        now
    })
}

#[test]
fn a_computer_that_could_not_say_what_it_reads_with_is_asked_again_and_never_told_it_lacks_a_key() {
    let dir = temp("unsaid");
    let studio = Arc::new(FakeTower::default());
    studio.answers::<at::Feed>(&Feed { station: "studio".into(), ..Feed::default() });
    studio.refuses::<at::ReadsOffered>(503, &Refusal::NotNow { reason: "Starting up.".into() });
    let desk = desk_wanting(&dir, vec![(link(1), studio.clone())], wanted_haiku(serde_json::json!({})));
    desk.set_links(vec![link(1)]);
    let id = station_id(&link(1));
    // asked on each round while it owes it, as push and tools are (audit 6, P-B3, P-B5)
    let again = until(|| studio.asked_on::<at::ReadsOffered>() >= 2);
    let meanwhile = standing_of(&desk, &id);
    // it comes back: the choice goes, and it has it
    let offer = ReadsOffer { provider: "anthropic".into(), offer: "haiku".into(), label: "Haiku".into(), key: true, has_key: true, preferred: true, ..ReadsOffer::default() };
    studio.answers::<at::ReadsOffered>(&ReadsOffered { offers: vec![offer], ..ReadsOffered::default() });
    studio.answers::<at::Reads>(&ReadsSet { reads: "Haiku".into() });
    let took = until(|| standing_of(&desk, &id) == Some(super::super::intents::Delivery::Done));
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(again, "a computer that could not say was not asked again");
    assert!(matches!(meanwhile, None | Some(super::super::intents::Delivery::Pending)), "trouble that may pass was kept as a no: {meanwhile:?}");
    assert!(took, "it never got the choice once it could say");
}

#[test]
fn a_summary_choice_every_computer_refused_says_why_and_does_not_wait_for_good() {
    let dir = temp("all-no");
    let per = serde_json::json!({ station_id(&link(1)).as_str(): { "state": "refused", "reason": "Enter your API key." } });
    let desk = desk_wanting(&dir, Vec::new(), wanted_haiku(per));
    let state = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.state);
    let _ = std::fs::remove_dir_all(&dir);
    // (audit 6, C-B2)
    assert_eq!(state, Some(ChoiceState::Failed { why: "Enter your API key.".into() }));
}

#[test]
fn a_computer_paired_after_the_others_took_the_summary_choice_is_sent_it_and_an_old_connection_sends_nothing() {
    let dir = temp("paired-later");
    let (studio, laptop) = (tower("studio", true), tower("laptop", true));
    laptop.answers::<at::Reads>(&ReadsSet { reads: "Haiku".into() });
    let per = serde_json::json!({ station_id(&link(1)).as_str(): { "state": "done" } });
    let desk = desk_wanting(&dir, vec![(link(1), studio.clone()), (link(2), laptop.clone())], wanted_haiku(per));
    desk.set_links(vec![link(1), link(2)]);
    // (audit 6, P-B4)
    let sent = until(|| laptop.asked_on::<at::Reads>() == 1);
    let laptop_id = station_id(&link(2));
    let took = until(|| standing_of(&desk, &laptop_id) == Some(super::super::intents::Delivery::Done));
    // a connection that is no longer the computer's sends nothing (audit 6, C-B3)
    let epoch = desk.shared.state.lock().unwrap().computer(&laptop_id).map(|c| c.epoch).unwrap();
    let before = laptop.asked_on::<at::ReadsOffered>();
    crate::choices::tell_reads(&desk.shared, &laptop_id, epoch + 100, true);
    let after = laptop.asked_on::<at::ReadsOffered>();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(sent && took, "a computer paired later never got the choice the others had");
    assert_eq!(studio.asked_on::<at::Reads>(), 0, "a computer that had it was sent it again");
    assert_eq!(after, before);
}

#[test]
fn the_main_computer_owed_the_choice_is_asked_once_what_it_reads_with() {
    let dir = temp("once");
    let studio = tower("studio", true);
    studio.refuses::<at::Reads>(503, &Refusal::NotNow { reason: "Busy.".into() });
    let desk = desk_with(&dir, vec![(link(1), studio.clone())]);
    desk.set_links(vec![link(1)]);
    assert!(all_answered(&desk, 1));
    let id = desk.choices(ChoiceJob::Reads).into_iter().find(|c| c.provider == "anthropic").map(|c| c.id).unwrap();
    let _ = desk.choose_for(ChoiceJob::Reads, id, None, HashMap::new(), false);
    let studio_id = desk.shared.state.lock().unwrap().computers[0].id();
    let epoch = desk.shared.state.lock().unwrap().computers[0].epoch;
    let (offered, sent) = (studio.asked_on::<at::ReadsOffered>(), studio.asked_on::<at::Reads>());
    // owed the choice, and the main one: one question about what it reads with serves both
    crate::choices::tell_reads(&desk.shared, &studio_id, epoch, true);
    let asked = (studio.asked_on::<at::ReadsOffered>() - offered, studio.asked_on::<at::Reads>() - sent);
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(asked, (1, 1));
}
