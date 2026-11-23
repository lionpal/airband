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
        let fresh: Vec<String> = ids.into_iter().filter(|id| inner.judging.insert(id.clone())).collect();
        fresh.into_iter().filter_map(|id| inner.log.iter().find(|e| e.id == id).map(|e| Item { say: None, ..inner.item(e) }).map(|item| (Marked { app: app.clone(), id }, item))).collect()
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
    // each on its own: one that panics leaves the others to finish, and its mark is taken off as it unwinds
    while let Some(done) = tasks.join_next().await {
        if done.is_err() {
            log("a line's short version could not be made: its judging failed");
        }
    }
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
    async fn a_judging_that_ends_without_filling_its_line_unmarks_it() {
        let app = crate::tower::http::tests::app("judged-unmarked");
        app.change().judging.insert("l9".into());
        drop(Marked { app: app.clone(), id: "l9".into() });
        assert!(app.read().judging.is_empty());
    }
}
