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
fn a_computer_not_reached_is_sent_the_choice_when_it_is_and_then_it_is_let_go() {
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
    let radio = desk.shared.state.lock().unwrap().computer(&laptop_id).map(|c| c.radio.clone()).unwrap();
    crate::choices::computer_reached(&desk.shared, &radio);
    let kept = desk.shared.chooser.reads_intent();
    desk.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(waiting, [("laptop".to_string(), DeskDelivery::Waiting)]);
    assert_eq!(laptop.asked_on::<at::Reads>(), 1);
    assert!(kept.is_none(), "{kept:?}");
}

#[test]
fn a_computer_is_sent_the_key_only_when_it_has_none_of_its_own() {
    let wanted = crate::choices::ReadsWanted { id: "anthropic".into(), offer: "haiku".into(), set: BTreeMap::new(), his: true };
    let offers = |has_key: bool| ReadsOffered { offers: vec![ReadsOffer { provider: "anthropic".into(), offer: "haiku".into(), key: true, has_key, ..ReadsOffer::default() }], ..ReadsOffered::default() };
    // it keeps its own: none is sent from a phone that has none
    assert_eq!(wanted.body_for(None, Some(&offers(true))).map(|b| b.key), Ok(String::new()));
    // it has none and the phone has one: the phone's is sent
    assert_eq!(wanted.body_for(Some("sk-1"), Some(&offers(false))).map(|b| b.key), Ok("sk-1".to_string()));
    // neither has one: it is not sent, and why is said
    assert_eq!(wanted.body_for(None, Some(&offers(false))).map(|b| b.key), Err(crate::choices::NO_KEY.to_string()));
    // one that could not say what it has: the provider's own row says a key is needed
    assert_eq!(wanted.body_for(None, None).map(|b| b.key), Err(crate::choices::NO_KEY.to_string()));
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
