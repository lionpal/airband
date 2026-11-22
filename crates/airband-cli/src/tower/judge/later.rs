//! Short versions made behind the requests that want them: a few at a time, each sent to every radio as it is done.
use super::*;

/// How many short versions one feed request waits for. The rest are made behind it and arrive as changes.
pub(in crate::tower) const JUDGE_NOW: usize = 12;

/// How many are made at once behind a request.
const JUDGE_AT_ONCE: usize = 3;

/// Make the short version of these logged messages, a few at a time, and mark each changed as it is done so
/// every radio following the feed is sent it. Nothing is left unjudged because a radio was away when it came in.
pub(in crate::tower) fn judge_later(app: &Arc<App>, entries: Vec<Entry>) {
    let entries: Vec<Entry> = {
        let mut inner = app.change();
        entries.into_iter().filter(|e| inner.judging.insert(e.id.clone())).collect()
    };
    if entries.is_empty() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let limit = Arc::new(tokio::sync::Semaphore::new(JUDGE_AT_ONCE));
        let mut tasks = tokio::task::JoinSet::new();
        for e in entries {
            let (app, limit) = (app.clone(), limit.clone());
            tasks.spawn(async move {
                let _slot = limit.acquire().await;
                let j = judge_entry(&app, e.clone()).await;
                let mut inner = app.change();
                inner.fill(&e.id, &j);
                push::consider(&app, &mut inner, &e.id);
            });
        }
        tasks.join_all().await;
    });
}

pub(in crate::tower) async fn judge_entry(app: &Arc<App>, e: Entry) -> Judged {
    let item = Item { say: None, ..app.read().item(&e) };
    prepare(&app.settings, &item).await
}
