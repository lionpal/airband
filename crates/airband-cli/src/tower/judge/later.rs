//! Short versions made behind the requests that want them: a few at a time, each sent to every radio as it is done.
use super::*;

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
            let j = prepare(&mark.app.settings, &item).await;
            let mut inner = mark.app.change();
            inner.fill(&mark.id, &j);
            push::consider(&mark.app, &mut inner, &mark.id);
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

    #[tokio::test]
    async fn a_judging_that_ends_without_filling_its_line_unmarks_it() {
        let app = crate::tower::http::tests::app("judged-unmarked");
        app.change().judging.insert("l9".into());
        drop(Marked { app: app.clone(), id: "l9".into() });
        assert!(app.read().judging.is_empty());
    }
}
