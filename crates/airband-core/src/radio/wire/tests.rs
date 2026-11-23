use super::*;

/// What a tower's query reader does with an address: axum's `Query` is `serde_urlencoded`.
fn read_query<Q: DeserializeOwned>(q: &str) -> Q {
    serde_urlencoded::from_str(q).unwrap()
}

fn round<T: Serialize + DeserializeOwned>(v: &T) -> T {
    serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap()
}

macro_rules! every {
    ($($e:ident),* $(,)?) => {{
        let mut n = 0;
        $(
            // an empty query reads back as the default, and so does the query the default writes
            let _: <at::$e as Endpoint>::Query = read_query(&query_of(&<at::$e as Endpoint>::Query::default()));
            let _: <at::$e as Endpoint>::Query = round(&<at::$e as Endpoint>::Query::default());
            assert!(at::ALL.iter().any(|(name, m, p)| *name == stringify!($e) && *m == <at::$e as Endpoint>::METHOD && *p == <at::$e as Endpoint>::PATH));
            n += 1;
        )*
        n
    }};
}

#[test]
fn every_endpoints_query_reads_back_and_every_one_is_listed() {
    let n = every!(
        Next, Heard, Unheard, Presence, Ready, Utterance, Feed, Picture, ReadsOffered, Reads, Message, Attachment, Accounts,
        UseAccount, Team, RenameTeam, Folders, Recent, Project, TakeOver, Stop, Unsend, Dismiss, Taken, End, Push, Tools,
        Events, Permission, PermissionAnswer, PermissionWithdraw,
    );
    assert_eq!(n, at::ALL.len(), "an endpoint is missing from this test");
}

#[test]
fn no_two_routes_share_a_method_and_path_and_radio_routes_are_under_the_radio_path() {
    for (i, (name, m, p)) in at::ALL.iter().enumerate() {
        assert!(!at::ALL[..i].iter().any(|(_, m2, p2)| m2 == m && p2 == p), "{name} repeats a route");
        assert!(p.starts_with(crate::radio::PATH) || p.starts_with("/v1/agent/"), "{name} is outside the known paths");
    }
}

#[test]
fn a_feed_query_reads_back_as_it_was_written() {
    let q = FeedQuery { team: Some("t 1&x".into()), since: Some(4), after: 9, roster: Some(2), wait: 99, ..Default::default() };
    let back: FeedQuery = read_query(&query_of(&q));
    assert_eq!(back, q);
    assert_eq!(q.wait_for(), std::time::Duration::from_secs(FeedQuery::WAIT_MOST.into()));
    let next = NextQuery { joined: true, have: Some("a/b".into()) };
    assert_eq!(read_query::<NextQuery>(&query_of(&next)), next);
    assert_eq!(read_query::<NextQuery>(&query_of(&NextQuery::default())), NextQuery::default());
}

#[test]
fn an_address_puts_the_id_in_its_place_and_the_query_after() {
    assert_eq!(address::<at::Message>(Some("a b"), &MessageQuery::default()), "/v1/radio/message/a%20b");
    assert_eq!(address::<at::Feed>(None, &FeedQuery { after: 3, ..Default::default() }), "/v1/radio/feed?after=3&wait=0");
}

#[test]
fn a_refusal_says_what_it_is_and_one_from_a_newer_tower_still_reads() {
    for r in [Refusal::told("no"), Refusal::BadAsk { reason: "x".into() }, Refusal::OtherVersion { speaks: "airband.radio/9".into() }, Refusal::NotNow { reason: "later".into() }, Refusal::TooBig, Refusal::Gone, Refusal::NotAllowed, Refusal::NotPaired, Refusal::Unknown] {
        assert_eq!(round(&r), r);
        // a no is a 4xx; "not now" is the one refusal that is not a no
        assert!((400..500).contains(&r.status()) || matches!(r, Refusal::NotNow { .. }) && r.status() == 503);
    }
    assert_eq!(serde_json::to_value(Refusal::told("no")).unwrap(), serde_json::json!({"refusal": "told", "reason": "no"}));
    assert_eq!(serde_json::from_str::<Refusal>(r#"{"refusal":"something_new","x":1}"#).unwrap(), Refusal::Unknown);
}

#[test]
fn answers_round_trip() {
    assert_eq!(round(&MessageText { text: "hi".into() }), MessageText { text: "hi".into() });
    assert_eq!(round(&AttachmentNext { next: 7 }), AttachmentNext { next: 7 });
    assert_eq!(round(&ReadsSet { reads: "r".into() }), ReadsSet { reads: "r".into() });
    assert_eq!(round(&ReadyAnswer { waiting: 2 }), ReadyAnswer { waiting: 2 });
    assert_eq!(round(&Done {}), Done {});
    let nothing: Option<crate::radio::Transmission> = serde_json::from_str("null").unwrap();
    assert!(nothing.is_none());
}

#[test]
fn the_protocol_page_lists_every_route_and_the_version() {
    let page = include_str!("../../../../../docs/protocols.md");
    assert!(page.contains(crate::radio::SCHEMA), "docs/protocols.md does not name {}", crate::radio::SCHEMA);
    for (name, m, p) in at::ALL {
        assert!(page.contains(&format!("{} {p}", m.as_str())), "docs/protocols.md does not list {name}: {} {p}", m.as_str());
    }
}
