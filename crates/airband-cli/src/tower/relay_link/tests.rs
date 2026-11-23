use super::*;
use super::links::Links;
use super::pull::{Answering, refused_pull};
use std::sync::atomic::Ordering;

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_undone_is_never_opened_on_the_relay_again() {
    let app = crate::tower::http::tests::app("unpaired-link");
    let phone = crate::config::PairedRadio { name: "phone".into(), secret: "11".repeat(32) };
    app.settings.change(|c| c.pairings.push(phone.clone())).unwrap();
    // a relay nothing listens on: were it asked, the link would wait on it, not end
    let serve = Serve::new(&app, Some("http://127.0.0.1:9".into()), &phone, crate::tower::http::router(app.clone()), tokio::runtime::Handle::current()).unwrap();
    assert!(serve.still_paired());
    app.settings.change(|c| c.pairings.clear()).unwrap();
    assert!(!serve.still_paired());
    let ended = tokio::task::spawn_blocking(move || serve.run());
    let ended = tokio::time::timeout(Duration::from_secs(5), ended).await.expect("it ends without opening the channel").unwrap();
    assert_eq!(ended, Ended::Unpaired);
}

/// A relay that keeps the channel closed answers every open with 410: the link ends for good, and the
/// watcher never opens it again for the same pairing (the relay would count each open against the address).
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_the_relay_keeps_closed_is_opened_once_and_never_again() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let relay = format!("http://{}", listener.local_addr().unwrap());
    let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = opens.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _read = stream.read(&mut buf);
            counted.fetch_add(1, Ordering::SeqCst);
            stream.write_all(b"HTTP/1.1 410 Gone\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").ok();
        }
    });
    let app = crate::tower::http::tests::app("closed-link");
    let phone = crate::config::PairedRadio { name: "phone".into(), secret: "22".repeat(32) };
    app.settings.change(|c| {
        c.pairings.push(phone.clone());
        c.relay = Some(relay.clone());
    })
    .unwrap();
    let serve = Serve::new(&app, Some(relay), &phone, crate::tower::http::router(app.clone()), tokio::runtime::Handle::current()).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), tokio::task::spawn_blocking(move || serve.run())).await.unwrap().unwrap();
    assert_eq!(ended, Ended::Closed);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    // the watcher, with the same pairing wanted, opens it once of its own and never again
    let (router, handle) = (crate::tower::http::router(app.clone()), tokio::runtime::Handle::current());
    let mut links = Links::default();
    links.watch(&app, &router, &handle);
    while links.running.values().any(|(_, t)| !t.is_finished()) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    links.watch(&app, &router, &handle);
    links.watch(&app, &router, &handle);
    assert!(links.running.is_empty());
    assert_eq!(opens.load(Ordering::SeqCst), 2, "opened once by the watcher, and never again while the same pairing stands");
    // and its straight route is gone with it: a phone the relay keeps closed is not answered on the network
    assert!(app.served.is_empty());
}

/// A relay that answers each request with what `answer` says for it (its request line and headers, and how
/// many came before it), and keeps every request it was sent.
/// Each request is answered on a thread of its own, so one held open (a pull the relay holds) never holds up
/// the next.
fn fake_relay(answer: impl Fn(&str, usize) -> Vec<u8> + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let relay = format!("http://{}", listener.local_addr().unwrap());
    let asked: Arc<Mutex<Vec<String>>> = Default::default();
    let kept = asked.clone();
    let answer = Arc::new(answer);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (kept, answer) = (kept.clone(), answer.clone());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 65536];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let before = { let mut k = kept.lock().unwrap(); k.push(head.clone()); k.len() - 1 };
                stream.write_all(&answer(&head, before)).ok();
            });
        }
    });
    (relay, asked)
}

fn serving(app: &Arc<App>, relay: &str, secret: &str) -> Serve {
    let phone = crate::config::PairedRadio { name: "phone".into(), secret: secret.into() };
    app.settings.change(|c| {
        c.pairings.push(phone.clone());
        c.relay = Some(relay.to_string());
    })
    .unwrap();
    Serve::new(app, Some(relay.into()), &phone, crate::tower::http::router(app.clone()), tokio::runtime::Handle::current()).unwrap()
}

const CLOSE: &str = "content-length: 0\r\nconnection: close\r\n\r\n";

/// What a pull the fake relay was sent said in `header`.
fn said(asked: &str, header: &str) -> Option<String> {
    asked.lines().find_map(|l| l.strip_prefix(&format!("{header}: ")).map(|v| v.trim().to_string()))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_to_a_kept_message_that_does_not_reach_the_relay_is_handed_over_again_by_the_relay() {
    use airband_core::relay::{ANSWERING_HEADER, GOT_HEADER, KEPT_HEADER, PULL_HEADER, PULL_VERSION, TOWER_HEADER};
    // open; a kept one handed over; every reply refused (the relay's store is down); then pulls with nothing
    let (relay, asked) = fake_relay(|head, n| match n {
        _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
        _ if head.starts_with("post") => format!("HTTP/1.1 503 Busy\r\n{CLOSE}").into_bytes(),
        1 => b"HTTP/1.1 200 OK\r\nx-airband-request: k1\r\nx-airband-kept: 1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
        _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
    });
    let app = crate::tower::http::tests::app("kept-answer-lost");
    let serve = serving(&app, &relay, &"33".repeat(32));
    let stop = serve.stop.clone();
    let running = tokio::task::spawn_blocking(move || serve.run());
    // until the answer was tried, and a pull after it no longer named it (one sent while the answer was on
    // its way still does)
    let tried = |asked: &[String]| asked.iter().position(|r| r.starts_with("post") && r.contains("/reply/k1"));
    let let_go = |asked: &[String]| tried(asked).and_then(|at| asked[at + 1..].iter().position(|r| r.starts_with("get") && said(r, ANSWERING_HEADER).is_none()).map(|n| at + 1 + n));
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while let_go(&asked.lock().unwrap()).is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the pulls went on naming a kept message whose answer was refused");
    stop.store(true, Ordering::SeqCst);
    running.await.unwrap();
    let asked = asked.lock().unwrap();
    let pulls: Vec<&String> = asked.iter().filter(|r| r.starts_with("get")).collect();
    // every pull says its version and the one tower it is from
    assert!(pulls.iter().all(|p| said(p, PULL_HEADER).as_deref() == Some(PULL_VERSION)));
    assert_eq!(pulls.iter().filter_map(|p| said(p, TOWER_HEADER)).collect::<std::collections::HashSet<_>>().len(), 1);
    // the pull after it says it got it; the answer went back marked kept, and was refused
    assert_eq!(said(pulls[1], GOT_HEADER).as_deref(), Some("k1"));
    let at = tried(&asked).unwrap();
    assert!(asked[at].contains(KEPT_HEADER));
    // while it was being answered the pulls named it, so the relay handed nothing kept meanwhile
    assert_eq!(said(pulls[1], ANSWERING_HEADER).as_deref(), Some("k1"));
    // the work on it ended without the relay having its answer: the pulls stop naming it, and the relay, which
    // still keeps it, hands it over again. The tower sends nothing again of its own
    assert!(let_go(&asked).is_some());
    assert_eq!(asked.iter().filter(|r| r.contains("/reply/k1")).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_work_on_a_kept_message_that_panics_still_ends_its_answering() {
    let pulling = Arc::new(Mutex::new(Pulling::of_tower("t")));
    let handing = pulling.lock().unwrap().handed("k1", true);
    assert_eq!(answering(&pulling).as_deref(), Some("k1"));
    let (turns, _inbox) = mpsc::channel();
    let guard = Answering { pulling: pulling.clone(), handing, delivered: false, turns };
    let died = tokio::spawn(async move {
        let _answering = guard;
        panic!("the work on it died");
    });
    assert!(died.await.is_err());
    assert_eq!(answering(&pulling), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_pull_says_why_and_told_the_relay_nothing_so_what_was_got_is_said_again() {
    use airband_core::relay::GOT_HEADER;
    let (relay, asked) = fake_relay(|head, n| match n {
        _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
        _ if head.starts_with("post") => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
        1 => b"HTTP/1.1 200 OK\r\nx-airband-request: r1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
        2 => b"HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\ncontent-length: 27\r\nconnection: close\r\n\r\n{\"refused\":\"another_tower\"}".to_vec(),
        _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
    });
    let app = crate::tower::http::tests::app("pull-refused");
    let serve = serving(&app, &relay, &"55".repeat(32));
    let stop = serve.stop.clone();
    let running = tokio::task::spawn_blocking(move || serve.run());
    while asked.lock().unwrap().iter().filter(|r| r.starts_with("get")).count() < 4 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.store(true, Ordering::SeqCst);
    running.await.unwrap();
    let asked = asked.lock().unwrap();
    let pulls: Vec<&String> = asked.iter().filter(|r| r.starts_with("get")).collect();
    assert_eq!(said(pulls[1], GOT_HEADER).as_deref(), Some("r1"));
    assert_eq!(said(pulls[2], GOT_HEADER).as_deref(), Some("r1"), "the refused pull was not taken: it is said again");
    assert_eq!(said(pulls[3], GOT_HEADER), None, "and once taken, not again");
}

#[test]
fn he_is_told_which_side_to_update_or_that_another_computer_pulls() {
    use airband_core::relay::{PullRefused, Side};
    let read = |body: &str| serde_json::from_str::<PullRefused>(body).unwrap();
    assert_eq!(read(r#"{"refused":"other_version","older":"tower"}"#), PullRefused::OtherVersion { older: Side::Tower });
    assert!(refused_pull("r", Ok(read(r#"{"refused":"other_version","older":"tower"}"#))).contains("update airband here"));
    assert!(refused_pull("r", Ok(read(r#"{"refused":"other_version","older":"relay"}"#))).contains("update the relay"));
    assert!(refused_pull("r", Ok(read(r#"{"refused":"another_tower"}"#))).contains("another computer"));
    assert!(serde_json::from_str::<PullRefused>(r#"{"refused":"tomorrow"}"#).is_err());
    // an answer whose reason cannot be read is said as that, never as a reason guessed for it
    let unread = refused_pull("r", Err("expected value"));
    assert!(unread.contains("could not be read") && !unread.contains("update"), "{unread}");
}

/// What the pulls name as being answered for.
fn answering(p: &Arc<Mutex<Pulling>>) -> Option<String> {
    p.lock().unwrap().headers().into_iter().find(|(k, _)| *k == airband_core::relay::ANSWERING_HEADER).map(|(_, v)| v)
}

#[test]
fn the_work_on_an_earlier_handing_of_a_kept_message_never_lets_go_of_the_one_after_it() {
    let pulling = Arc::new(Mutex::new(Pulling::of_tower("t")));
    let (turns, inbox) = mpsc::channel();
    let first = pulling.lock().unwrap().handed("k1", true);
    // the channel opened again while k1 was being answered: the relay hands it over a second time
    pulling.lock().unwrap().reopened();
    let second = pulling.lock().unwrap().handed("k1", true);
    drop(Answering { pulling: pulling.clone(), handing: first, delivered: false, turns: turns.clone() });
    assert_eq!(answering(&pulling).as_deref(), Some("k1"), "the first handing's work ending does not let go of the second");
    assert!(inbox.try_recv().is_err(), "and does not cut the pull in flight short");
    drop(Answering { pulling: pulling.clone(), handing: second, delivered: true, turns });
    assert_eq!(answering(&pulling), None);
    assert!(inbox.try_recv().is_err(), "its answer reached the relay: the pull in flight is left alone");
}

/// A kept message whose answer did not reach the relay is let go at once: the pull in flight, which still names it
/// and which the relay holds, is replaced by one that does not, without waiting out the hold.
#[tokio::test(flavor = "multi_thread")]
async fn a_kept_message_whose_answer_failed_is_let_go_without_waiting_out_the_pull_in_flight() {
    use airband_core::relay::ANSWERING_HEADER;
    let (relay, asked) = fake_relay(|head, n| match n {
        _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
        _ if head.starts_with("post") => format!("HTTP/1.1 503 Busy\r\n{CLOSE}").into_bytes(),
        1 => b"HTTP/1.1 200 OK\r\nx-airband-request: k1\r\nx-airband-kept: 1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
        // a pull that still names k1 is held as the relay holds it, far longer than this test waits
        _ if said(head, ANSWERING_HEADER).is_some() => {
            std::thread::sleep(Duration::from_secs(30));
            format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes()
        }
        _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
    });
    let app = crate::tower::http::tests::app("kept-let-go");
    let serve = serving(&app, &relay, &"66".repeat(32));
    let stop = serve.stop.clone();
    let running = tokio::task::spawn_blocking(move || serve.run());
    let freed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let seen = asked.lock().unwrap().clone();
            let tried = seen.iter().position(|r| r.starts_with("post") && r.contains("/reply/k1"));
            if tried.is_some_and(|at| seen[at + 1..].iter().any(|r| r.starts_with("get") && said(r, ANSWERING_HEADER).is_none())) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(freed.is_ok(), "the kept line was held until the pull in flight ended");
    stop.store(true, Ordering::SeqCst);
    drop(running);
}

/// The tower pulls under one id it keeps, so a link started again (after a restart, or under another spelling of
/// the relay's address) is never taken by the relay for another computer.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_started_again_pulls_under_the_same_tower_id() {
    use airband_core::relay::TOWER_HEADER;
    let app = crate::tower::http::tests::app("tower-id");
    let phone = crate::config::PairedRadio { name: "phone".into(), secret: "77".repeat(32) };
    let tower = |relay: &str| {
        let serve = Serve::new(&app, Some(relay.into()), &phone, crate::tower::http::router(app.clone()), tokio::runtime::Handle::current()).unwrap();
        let said = serve.pulling.lock().unwrap().headers();
        said.into_iter().find(|(k, _)| *k == TOWER_HEADER).map(|(_, v)| v)
    };
    assert_eq!(tower("https://relay.example"), tower("https://RELAY.example/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_cut_off_on_the_way_is_never_said_to_be_got_and_one_read_whole_is() {
    // open; a pull cut off mid-body; a pull read whole; then pulls with nothing
    let (relay, asked) = fake_relay(|head, n| match n {
        _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
        1 => b"HTTP/1.1 200 OK\r\nx-airband-request: cut\r\ncontent-length: 400\r\nconnection: close\r\n\r\nshort".to_vec(),
        2 => b"HTTP/1.1 200 OK\r\nx-airband-request: whole\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
        _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
    });
    let app = crate::tower::http::tests::app("got-after-read");
    let serve = serving(&app, &relay, &"44".repeat(32));
    let stop = serve.stop.clone();
    let running = tokio::task::spawn_blocking(move || serve.run());
    while asked.lock().unwrap().len() < 5 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.store(true, Ordering::SeqCst);
    running.await.unwrap();
    let asked = asked.lock().unwrap();
    let got = |n: usize| asked[n].lines().find_map(|l| l.strip_prefix(&format!("{}: ", airband_core::relay::GOT_HEADER)).map(str::to_string));
    assert_eq!(got(2), None, "the cut-off one is handed over again, not said to be got");
    assert_eq!(got(3).as_deref(), Some("whole"));
    assert_eq!(got(4), None, "said once, to the pull after it");
}

#[test]
fn the_relays_statuses_are_read_in_one_place() {
    assert_eq!(RelaySaid::of(201), RelaySaid::Done);
    assert_eq!(RelaySaid::of(204), RelaySaid::Nothing);
    assert_eq!(RelaySaid::of(409), RelaySaid::Refused);
    assert_eq!(RelaySaid::of(410), RelaySaid::Closed);
    assert_eq!(RelaySaid::of(426), RelaySaid::OtherVersion);
    assert_eq!(RelaySaid::of(418), RelaySaid::Other(418));
}
