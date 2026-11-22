//! The one writer of the tower's state to its database. A change to the state is put in writing while the
//! state is held (cheap: a copy of what changed) and handed over here in that same order; the database is
//! written on this thread, never while the state is held, so a slow disk never holds up a phone or an agent.
use super::*;
use std::sync::mpsc;

enum Job {
    Write(Batch),
    /// Everything handed over before this is written; then the sender is told.
    Flush(mpsc::SyncSender<()>),
}

pub(crate) struct Saver {
    jobs: mpsc::Sender<Job>,
}

impl Saver {
    /// Start writing to `store`, on a thread of its own.
    pub(super) fn start(store: Arc<Store>) -> Saver {
        let (jobs, taken) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("airband-saver".into())
            .spawn(move || {
                for job in taken {
                    match job {
                        Job::Write(batch) => store.write(&batch),
                        Job::Flush(done) => {
                            let _ = done.send(());
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

    /// Wait until everything handed over so far is written: before the tower stops, and in tests.
    pub(super) fn flush(&self) {
        let (done, wait) = mpsc::sync_channel(1);
        if self.jobs.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> Agent {
        Agent { id: id.into(), harness: "test".into(), cwd: Some("/x/shop".into()), name: None, title: None, pid: None, deliver: None, meta: Default::default() }
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
