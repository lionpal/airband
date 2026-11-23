//! The one writer of the tower's state to its database. A change to the state is put in writing while the
//! state is held (cheap: a copy of what changed) and handed over here in that same order; the database is
//! written on this thread, never while the state is held, so a slow disk never holds up a phone or an agent.
use super::*;
use std::sync::mpsc;

/// How long what could not be written waits before it is tried again when nothing else comes.
const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

enum Job {
    Write(Batch),
    /// Everything handed over before this is written; then the sender is told.
    Flush(mpsc::SyncSender<()>),
}

pub(crate) struct Saver {
    jobs: mpsc::Sender<Job>,
}

/// Asks the writer to finish, and waits at most so long: the writer may be what failed.
pub(crate) struct Flusher {
    jobs: Mutex<mpsc::Sender<Job>>,
}
impl Flusher {
    pub(super) fn flush_within(&self, most: std::time::Duration) {
        let (done, wait) = mpsc::sync_channel(1);
        let Ok(jobs) = self.jobs.try_lock() else { return };
        if jobs.send(Job::Flush(done)).is_ok() {
            wait.recv_timeout(most).ok();
        }
    }
}

impl Saver {
    /// Start writing to `store`, on a thread of its own.
    pub(super) fn start(store: Arc<Store>) -> Saver {
        let (jobs, taken) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("airband-saver".into())
            .spawn(move || {
                // what could not be written yet: kept, with anything handed over since on top, and tried again
                // with the next change, on a flush, and every few seconds while it waits
                let mut unwritten: Option<Batch> = None;
                let mut told = false;
                let mut attempt = |batch: Batch, unwritten: &mut Option<Batch>| match store.write(&batch) {
                    Ok(()) => {
                        if told {
                            log("database: what could not be written before is written now");
                        }
                        told = false;
                    }
                    Err(e) => {
                        if !told {
                            log(format!("database: a change to the state could not be written ({e}); it is kept and tried again"));
                        }
                        told = true;
                        *unwritten = Some(batch);
                    }
                };
                loop {
                    let job = if unwritten.is_some() {
                        match taken.recv_timeout(RETRY) {
                            Ok(job) => Some(job),
                            Err(mpsc::RecvTimeoutError::Timeout) => None,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    } else {
                        match taken.recv() {
                            Ok(job) => Some(job),
                            Err(_) => return,
                        }
                    };
                    match job {
                        Some(Job::Write(batch)) => {
                            let batch = match unwritten.take() {
                                Some(before) => before.then(batch),
                                None => batch,
                            };
                            attempt(batch, &mut unwritten);
                        }
                        Some(Job::Flush(done)) => {
                            if let Some(batch) = unwritten.take() {
                                attempt(batch, &mut unwritten);
                            }
                            done.send(()).ok();
                        }
                        None => {
                            if let Some(batch) = unwritten.take() {
                                attempt(batch, &mut unwritten);
                            }
                        }
                    }
                }
            })
            .expect("a thread to write the tower's state");
        Saver { jobs }
    }

    /// Hand over what is to be written. Called while the state is held, so batches are written in the order
    /// the changes were made.
    pub(super) fn send(&self, batch: Batch) {
        if batch.is_empty() {
            return;
        }
        if self.jobs.send(Job::Write(batch)).is_err() {
            log("database: the writer has stopped; a change to the state was not written");
        }
    }

    /// A way to ask for what is handed over to be written, for the tower's failure hook.
    pub(super) fn flusher(&self) -> Flusher {
        Flusher { jobs: Mutex::new(self.jobs.clone()) }
    }

    /// Wait until everything handed over so far is written: before the tower stops, and in tests.
    pub(super) fn flush(&self) {
        let (done, wait) = mpsc::sync_channel(1);
        if self.jobs.send(Job::Flush(done)).is_ok() {
            wait.recv().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> Agent {
        airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with(id, "test") }
    }

    #[test]
    fn a_table_is_handed_over_only_when_what_it_holds_changed() {
        let mut inner = Inner::default();
        let first = inner.changes();
        assert_eq!(first.tables.iter().map(|(k, _)| *k).collect::<Vec<_>>(), KEPT, "everything, the first time");
        assert!(inner.changes().is_empty(), "nothing changed since");
        inner.check_in(&agent("a")).unwrap();
        let after = inner.changes();
        assert!(after.tables.iter().any(|(k, _)| *k == "sessions"), "an agent taken in is written");
        assert!(!after.tables.iter().any(|(k, _)| *k == "held" || *k == "menus"), "tables that did not change are not");
        // what an agent is doing this minute is not kept, so it writes nothing
        inner.session("a").unwrap().doing = Some("reading".into());
        assert!(inner.changes().tables.is_empty());
    }

    #[tokio::test]
    async fn the_failure_hook_writes_what_was_handed_over_and_never_waits_long_on_a_writer_that_is_stuck() {
        let app = crate::tower::http::tests::app("saver-hook");
        app.change().check_in(&agent("a")).unwrap();
        app.saver.flusher().flush_within(Duration::from_secs(3));
        assert_eq!(app.store.load(10).unwrap().0.sessions.len(), 1);
        // the writer held up: the hook gives up in its time
        let busy = app.store.hold();
        app.change().check_in(&agent("b")).unwrap();
        let began = std::time::Instant::now();
        app.saver.flusher().flush_within(Duration::from_millis(200));
        assert!(began.elapsed() < Duration::from_secs(2));
        drop(busy);
    }

    #[tokio::test]
    async fn a_change_the_database_refused_is_kept_and_written_once_it_can_be() {
        let app = crate::tower::http::tests::app("saver-refused");
        drop(app.change());
        app.flush();
        app.store.hold().execute_batch("PRAGMA query_only = ON").unwrap();
        app.change().check_in(&agent("a")).unwrap();
        app.flush();
        app.store.hold().execute_batch("PRAGMA query_only = OFF").unwrap();
        assert_eq!(app.store.load(10).unwrap().0.sessions.len(), 0, "nothing could be written while it refused");
        // nothing else changes: the one change that failed is still written
        app.flush();
        assert_eq!(app.store.load(10).unwrap().0.sessions.len(), 1, "the change it refused is written once it can be");
    }

    #[tokio::test]
    async fn a_change_is_let_go_at_once_while_the_database_is_busy_and_written_after() {
        let app = crate::tower::http::tests::app("saver-busy");
        drop(app.change());
        app.flush();
        // the database is held by someone else: a change to the state must not wait for it
        let busy = app.store.hold();
        let (done, waited) = std::sync::mpsc::channel();
        let changer = app.clone();
        std::thread::spawn(move || {
            changer.change().check_in(&agent("a")).unwrap();
            done.send(()).unwrap();
        });
        waited.recv_timeout(Duration::from_secs(5)).expect("the state was let go without waiting for the database");
        drop(busy);
        app.flush();
        assert_eq!(app.store.load(10).unwrap().0.sessions.len(), 1, "and written once the database was free");
    }

    #[tokio::test]
    async fn every_kept_table_comes_back_after_a_restart() {
        let app = crate::tower::http::tests::app("saver-restart");
        {
            let mut inner = app.change();
            inner.check_in(&agent("a")).unwrap();
            inner.tools = vec!["claude-code".into()];
        }
        app.flush();
        let (saved, _) = app.store.load(10).unwrap();
        assert_eq!(saved.sessions.len(), 1);
        assert_eq!(saved.tools, ["claude-code"]);
        assert_eq!(saved.teams.len(), 1, "the agent's folder's team");
    }
}
