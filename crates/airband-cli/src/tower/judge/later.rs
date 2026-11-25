//! Short versions made behind the requests that want them: a few at a time, each sent to every radio as it is done.
use super::*;
use super::super::edges::models::Model;

/// How many short versions one feed request waits for. The rest are made behind it and arrive as changes.
pub(in crate::tower) const JUDGE_NOW: usize = 12;

/// How many are made at once behind a request.
const JUDGE_AT_ONCE: usize = 3;

/// One line being judged, marked in `judging` while it is, and unmarked however its judging ends: done, failed
/// or panicked. A line marked is never judged a second time meanwhile.
struct Marked {
    app: Arc<App>,
    id: String,
}
impl Drop for Marked {
    fn drop(&mut self) {
        // filling it unmarks it; this is for a judging that ended another way
        if self.app.read().judging.contains(&self.id) {
            self.app.change().judging.remove(&self.id);
        }
    }
}

/// The one way a line is judged: these lines, each once, at most `at_once` at a time. One already being judged
/// (by the feed, the call's queue or behind a request) is left to that judging. Each is given its short version
/// and considered for a notification as it is done. Done when all of them are.
pub(in crate::tower) async fn judge_marked(app: &Arc<App>, ids: Vec<String>, at_once: usize) {
    let marked: Vec<(Marked, Item)> = {
        let mut inner = app.change();
        // only a line still in the log is marked: one it has let go of would stay marked for good
        let items: Vec<(String, Item)> = ids.into_iter().filter_map(|id| inner.log.iter().find(|e| e.id == id).map(|e| (id, Item { say: None, ..inner.item(e) }))).collect();
        items.into_iter().filter(|(id, _)| inner.judging.insert(id.clone())).map(|(id, item)| (Marked { app: app.clone(), id }, item)).collect()
    };
    let limit = Arc::new(tokio::sync::Semaphore::new(at_once.clamp(1, tokio::sync::Semaphore::MAX_PERMITS)));
    let mut tasks = tokio::task::JoinSet::new();
    for (mark, item) in marked {
        let limit = limit.clone();
        tasks.spawn(async move {
            let _slot = limit.acquire().await;
            let settings = &mark.app.settings;
            judge_one(&mark.app, settings, &settings.current(), &mark.id, &item).await;
        });
    }
    // each on its own. In the tower a panic stops it whole (`serve::stop_whole_on_failure`), and it is started
    // again; where no such hook is set, one that panics leaves the others to finish, its mark is taken off as it
    // unwinds, and the state it held is taken as it was left (`App::hold`)
    while let Some(done) = tasks.join_next().await {
        if done.is_err() {
            log("a line's short version could not be made: its judging failed");
        }
    }
}

/// How long his notification waits for a line's summary. One not made by then (the model is slow, on a busy
/// computer) wakes him with the message as written; the summary is still made, up to its own limit, and takes
/// its place. His choice of how long a notification may wait, not a stand-in for anything the tower could know.
pub(in crate::tower) const NOTICE_DEADLINE: Duration = Duration::from_secs(8);

/// Give one line its short version, asking `model`, and consider it for a notification. A summary made within
/// [`NOTICE_DEADLINE`] is the line's, as it always was. One that is not yet made by then, where there is a phone
/// to wake, leaves the line as written for now (the chat, the reading and the notification alike) and is
/// considered for a notification at once; the summary, when it comes, becomes the line's in all three, its
/// notification replaced in place and quietly (`push::replace`). No summary at all leaves it as written.
async fn judge_one(app: &Arc<App>, model: &dyn Model, cfg: &Config, id: &str, item: &Item) {
    let summary = summarize_by(model, cfg, item);
    tokio::pin!(summary);
    let first = if app.push.any() { tokio::time::timeout(NOTICE_DEADLINE, summary.as_mut()).await.ok() } else { Some(summary.as_mut().await) };
    if let Some(j) = first {
        let mut inner = app.change();
        inner.fill(id, &j.unwrap_or_else(|| Judged::unread(item)));
        return push::consider(app, &mut inner, id);
    }
    log(format!("line {id}: no summary within {} seconds; he is woken with the message as written, and the summary is still being made", NOTICE_DEADLINE.as_secs()));
    {
        let mut inner = app.change();
        inner.fill(id, &Judged::unread(item));
        push::consider(app, &mut inner, id);
    }
    // none after all: it stays as written, as he has it
    let Some(j) = summary.await else { return };
    let mut inner = app.change();
    inner.fill(id, &j);
    push::replace(app, &inner, id);
    // one that was not worth waking him for as written may be now
    push::consider(app, &mut inner, id);
}

/// Judge these lines for a request that waits on them, on a task of its own: a request closed meanwhile (the
/// radio went) leaves the judging to finish and its lines filled, never the models' answers paid for and
/// dropped.
pub(in crate::tower) async fn judge_for_request(app: &Arc<App>, ids: Vec<String>) {
    let app = app.clone();
    tokio::spawn(async move { judge_marked(&app, ids, JUDGE_NOW).await }).await.ok();
}

/// Make the short version of these logged messages behind the request that wants them, a few at a time, and
/// mark each changed as it is done so every radio following the feed is sent it. Nothing is left unjudged
/// because a radio was away when it came in.
pub(in crate::tower) fn judge_later(app: &Arc<App>, entries: Vec<Entry>) {
    if entries.is_empty() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move { judge_marked(&app, entries.into_iter().map(|e| e.id).collect(), JUDGE_AT_ONCE).await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tower::edges::models::FakeModel;

    #[tokio::test]
    async fn a_line_being_judged_is_not_judged_again_and_one_judged_is_unmarked() {
        let app = crate::tower::http::tests::app("judged-once");
        app.change().log.push(Entry { id: "l1".into(), agent: Some("a".into()), full: "The deploy is done.".into(), ..Entry::default() });
        // being judged elsewhere (another radio's feed, the call's queue): left to that judging
        app.change().judging.insert("l1".into());
        judge_marked(&app, vec!["l1".into()], JUDGE_NOW).await;
        assert!(app.read().log[0].short.is_none(), "judged a second time");
        app.change().judging.clear();
        judge_marked(&app, vec!["l1".into()], JUDGE_NOW).await;
        let inner = app.read();
        assert!(inner.log[0].short.is_some(), "judged");
        assert!(inner.judging.is_empty(), "unmarked once judged");
    }

    #[tokio::test]
    async fn a_line_the_log_has_let_go_of_is_never_marked_as_being_judged() {
        let app = crate::tower::http::tests::app("judged-gone");
        judge_marked(&app, vec!["gone".into()], JUDGE_NOW).await;
        assert!(app.read().judging.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_closed_while_its_lines_are_judged_leaves_them_to_be_filled() {
        let app = crate::tower::http::tests::app("judged-closed");
        app.change().log.push(Entry { id: "l2".into(), agent: Some("a".into()), full: "The tests pass.".into(), ..Entry::default() });
        let asking = app.clone();
        let request = tokio::spawn(async move { judge_for_request(&asking, vec!["l2".into()]).await });
        // closed once its judging has begun
        while !app.read().judging.contains("l2") && app.read().log[0].short.is_none() {
            tokio::task::yield_now().await;
        }
        request.abort();
        let filled = tokio::time::timeout(Duration::from_secs(10), async {
            while app.read().log[0].short.is_none() || !app.read().judging.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(filled.is_ok(), "the judging went with the request");
    }

    #[test]
    fn a_holder_that_panicked_leaves_the_state_usable() {
        let app = crate::tower::http::tests::app("poisoned");
        let holder = app.clone();
        let died = std::thread::spawn(move || {
            let _held = holder.change();
            panic!("a fill went wrong");
        });
        assert!(died.join().is_err());
        app.change().judging.insert("x".into());
        assert!(app.read().judging.contains("x"));
    }

    /// A tower with one phone to wake (through a relay that answers nothing) and one agent's line waiting to be
    /// heard, as it is recorded; the phone's keys, to open what it was sent.
    fn waking(name: &str) -> (Arc<App>, airband_core::relay::Keys) {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let keys = airband_core::relay::Keys::derive(&phone.secret).unwrap();
        let app = crate::tower::http::tests::app_with(name, serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone] }));
        push::register(&app, &keys.channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        {
            let mut inner = app.change();
            let _ = inner.check_in(&airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") });
            let s = inner.sessions[0].clone();
            inner.record(Entry { aloud: Aloud::Waiting, ..Entry::said("l1".into(), &s, None, WRITTEN.into()) });
        }
        (app, keys)
    }
    const WRITTEN: &str = "I rebuilt the importer, ran all of its tests, and they pass. The old one is gone.";
    const SUMMARY: &str = r#"{"say":"The importer is rebuilt and its tests pass.","kind":"report","options":[],"recommended":null,"read":null,"background":false,"own_session":false,"as_written":false}"#;

    /// Each notification the phone was sent for the line, opened, with whether it was quiet; once every send
    /// begun has been made.
    async fn sent(app: &Arc<App>, keys: &airband_core::relay::Keys, at_least: usize) -> Vec<(airband_core::radio::Notice, bool, String)> {
        use airband_core::radio::Pushed;
        let _ = tokio::time::timeout(Duration::from_secs(60), async {
            while app.push.asked.lock().unwrap().len() < at_least {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // anything more that was going to be sent has been by now
        tokio::time::sleep(Duration::from_secs(1)).await;
        app.push.asked.lock().unwrap().iter().map(|(_, r)| {
            let open = keys.open(airband_core::relay::Direction::Push, &airband_core::encode::unbase64(&r.sealed).unwrap()).unwrap();
            let Ok(Pushed::Show { notice }) = serde_json::from_slice(&open) else { panic!("not a notification to show") };
            (notice, r.quiet, r.replaces.clone().unwrap_or_default())
        }).collect()
    }
    fn item(app: &Arc<App>) -> Item {
        let inner = app.read();
        Item { say: None, ..inner.item(&inner.log[0]) }
    }
    /// The line's words in the chat and as read out, and they are the same.
    fn words(app: &Arc<App>) -> String {
        let inner = app.read();
        let e = &inner.log[0];
        let said = inner.message(e).text;
        assert_eq!(super::super::super::pump::aloud_line(&inner, e), format!("{}. {said}", airband_core::prose::capital(&inner.sender_name(e))));
        said
    }

    #[tokio::test(start_paused = true)]
    async fn a_summary_made_in_time_is_the_one_notification_as_before() {
        let (app, keys) = waking("deadline-fast");
        let model = FakeModel::answers(Ok(SUMMARY)).taking(NOTICE_DEADLINE - Duration::from_secs(1));
        judge_one(&app, &model, &app.cfg(), "l1", &item(&app)).await;
        let sent = sent(&app, &keys, 1).await;
        assert_eq!(sent.len(), 1, "one notification");
        let (note, quiet, _) = &sent[0];
        assert_eq!((note.text.as_str(), *quiet, note.replacing), ("The importer is rebuilt and its tests pass.", false, false));
        assert_eq!(words(&app), note.text);
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_summary_wakes_him_with_the_message_then_quietly_takes_its_place() {
        let (app, keys) = waking("deadline-slow");
        let model = FakeModel::answers(Ok(SUMMARY)).taking(Duration::from_secs(60));
        let (judging, ask) = (app.clone(), item(&app));
        let task = tokio::spawn(async move { judge_one(&judging, &model, &judging.cfg(), "l1", &ask).await });
        // woken at the deadline, with the message as written, in the words the chat and the reading have then
        let first = sent(&app, &keys, 1).await;
        assert_eq!(first.len(), 1);
        let (note, quiet, tag) = first[0].clone();
        assert_eq!((note.text.as_str(), quiet, note.replacing), (WRITTEN, false, false));
        assert_eq!(words(&app), WRITTEN);
        assert!(!task.is_finished(), "the summary is still being made");
        task.await.unwrap();
        // the summary, in place of it under the same tag, quietly; the chat and the reading say it too
        let both = sent(&app, &keys, 2).await;
        assert_eq!(both.len(), 2);
        let (again, quiet, same) = both[1].clone();
        assert_eq!((again.text.as_str(), quiet, again.replacing, same), ("The importer is rebuilt and its tests pass.", true, true, tag));
        assert_eq!(words(&app), again.text);
        assert_eq!(app.read().log[0].kind, Some(Kind::Report));
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_summary_for_a_line_heard_meanwhile_wakes_nobody_again() {
        let (app, keys) = waking("deadline-heard");
        let model = FakeModel::answers(Ok(SUMMARY)).taking(Duration::from_secs(60));
        let (judging, ask) = (app.clone(), item(&app));
        let task = tokio::spawn(async move { judge_one(&judging, &model, &judging.cfg(), "l1", &ask).await });
        assert_eq!(sent(&app, &keys, 1).await.len(), 1);
        app.change().set_aloud("l1", Aloud::Heard { radio: "car".into(), at: 1 });
        task.await.unwrap();
        assert_eq!(sent(&app, &keys, 2).await.len(), 1, "nothing more sent");
        // the chat has the summary all the same
        assert_eq!(words(&app), "The importer is rebuilt and its tests pass.");
    }

    #[tokio::test(start_paused = true)]
    async fn no_summary_at_all_leaves_the_message_as_written_and_sends_nothing_more() {
        // slow, then none
        let (app, keys) = waking("deadline-failed");
        let model = FakeModel::answers(Err("claude took over 90 seconds")).taking(Duration::from_secs(90));
        judge_one(&app, &model, &app.cfg(), "l1", &item(&app)).await;
        let sent_slow = sent(&app, &keys, 2).await;
        assert_eq!(sent_slow.len(), 1);
        assert_eq!((sent_slow[0].0.text.as_str(), sent_slow[0].1), (WRITTEN, false));
        assert_eq!(words(&app), WRITTEN);
        assert_eq!(app.read().log[0].kind, Some(Kind::Unjudged));
        // none, at once: as it always was
        let (app, keys) = waking("deadline-failed-fast");
        judge_one(&app, &FakeModel::answers(Err("no answer")), &app.cfg(), "l1", &item(&app)).await;
        let sent_fast = sent(&app, &keys, 2).await;
        assert_eq!(sent_fast.len(), 1);
        assert_eq!((sent_fast[0].0.text.as_str(), sent_fast[0].1), (WRITTEN, false));
    }

    #[tokio::test]
    async fn a_judging_that_ends_without_filling_its_line_unmarks_it() {
        let app = crate::tower::http::tests::app("judged-unmarked");
        app.change().judging.insert("l9".into());
        drop(Marked { app: app.clone(), id: "l9".into() });
        assert!(app.read().judging.is_empty());
    }
}
