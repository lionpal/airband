//! OpenCode's own record of a held agent's session: what its turns wrote, and when each was over.
//!
//! OpenCode keeps every session in one SQLite database under the folder his system names for data (its row:
//! `data_env`, `data_home`, `record`), which the server `opencode acp` runs for itself writes as the session
//! goes. Each thing in a session is a row of `session_message` with its kind (`type`) and its place in the
//! session (`seq`): his words (`user`), each answer (`assistant`, its parts under `content`), and, once a turn
//! is over, an `idle` row (with its `outcome`). That last is what says the turn is over, a command handed on
//! included (OpenCode 2.3.2, checked against a real one). Opened read-only: OpenCode is writing it. Its
//! server keeps the database open in write-ahead mode while it runs, so the log beside it is there to watch.
use super::NAME;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::PathBuf;

pub(super) struct Record {
    db: PathBuf,
    session: String,
}

impl Record {
    /// The record of this session, where OpenCode keeps it on this computer.
    pub(super) fn of(session: &str) -> Record {
        let data = crate::adapters::env(&crate::adapters::row_says(NAME, "data_env")).map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(crate::adapters::row_says(NAME, "data_home")));
        Record { db: data.join(crate::adapters::row_says(NAME, "record")), session: session.to_string() }
    }

    fn open(&self) -> Option<Connection> {
        Connection::open_with_flags(&self.db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()
    }
}

impl crate::adapters::AcpRecord for Record {
    fn files(&self) -> Vec<PathBuf> {
        // SQLite writes what is new to the database's write-ahead log first (`-wal` beside it), and into the
        // database itself when it folds the log in
        let mut log = self.db.clone().into_os_string();
        log.push("-wal");
        vec![self.db.clone(), PathBuf::from(log)]
    }

    fn mark(&self) -> Option<u64> {
        // a session with nothing in it yet stands before its first row
        self.open()?.query_row("SELECT COALESCE(MAX(seq), 0) FROM session_message WHERE session_id = ?1", [&self.session], |r| r.get(0)).ok()
    }

    fn over(&self, mark: u64) -> Option<String> {
        let db = self.open()?;
        // the turn begun after the mark is over at the first row after it that says a turn is over
        let idle: u64 = db.query_row("SELECT seq FROM session_message WHERE session_id = ?1 AND seq > ?2 AND type = 'idle' ORDER BY seq LIMIT 1", rusqlite::params![self.session, mark], |r| r.get(0)).optional().ok()??;
        let mut answers = db.prepare("SELECT data FROM session_message WHERE session_id = ?1 AND seq > ?2 AND seq < ?3 AND type = 'assistant' ORDER BY seq").ok()?;
        let answers: Vec<Value> = answers.query_map(rusqlite::params![self.session, mark, idle], |r| r.get::<_, String>(0)).ok()?.filter_map(Result::ok).filter_map(|data| serde_json::from_str(&data).ok()).collect();
        Some(answers.iter().flat_map(super::words).collect::<Vec<_>>().join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AcpRecord;

    /// A database laid out as OpenCode 2.3.2's is, with one session's rows as its server wrote them for a
    /// command it was handed and then for a question.
    fn opencode_db(at: &std::path::Path) {
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        let db = Connection::open(at).unwrap();
        db.execute_batch("CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);").unwrap();
    }
    fn row(at: &std::path::Path, session: &str, seq: u64, kind: &str, data: &str) {
        Connection::open(at).unwrap().execute("INSERT INTO session_message VALUES (?1, ?2, ?3, ?4, 0, 0, ?5)", rusqlite::params![format!("msg_{session}_{seq}"), session, kind, seq, data]).unwrap();
    }

    #[test]
    fn a_turn_is_over_when_the_record_says_so_and_what_it_wrote_is_its_answers_words() {
        let home = crate::adapters::accounts::tests::folder();
        crate::adapters::test_env::set("XDG_DATA_HOME", home.join("data").to_str().unwrap());
        let record = Record::of("ses_1");
        assert_eq!(record.db, home.join("data/opencode/opencode.db"), "where its row says, under the folder his system names for data");
        assert_eq!(record.files(), [home.join("data/opencode/opencode.db"), home.join("data/opencode/opencode.db-wal")]);
        assert_eq!(record.mark(), None, "no record yet: nothing to read");
        opencode_db(&record.db);
        let mark = record.mark().unwrap();
        assert_eq!(mark, 0);
        // the command handed on: his words as the command made them, and nothing yet that says it is over
        row(&record.db, "ses_1", 4, "user", r#"{"text":"Reply with exactly this line: HELLO Ada"}"#);
        assert_eq!(record.over(mark), None);
        row(&record.db, "ses_1", 5, "assistant", r#"{"content":[{"type":"reasoning","text":"He wants one line."},{"type":"text","text":" HELLO Ada "}],"finish":"stop"}"#);
        row(&record.db, "ses_2", 6, "idle", r#"{"outcome":"succeeded"}"#);
        assert_eq!(record.over(mark), None, "another session's turn being over is not this one's");
        row(&record.db, "ses_1", 13, "idle", r#"{"outcome":"succeeded"}"#);
        assert_eq!(record.over(mark).as_deref(), Some("HELLO Ada"));
        // the next turn is read from where the record stood as it began: two answers, and one that wrote nothing
        let mark = record.mark().unwrap();
        assert_eq!(mark, 13);
        row(&record.db, "ses_1", 18, "user", r#"{"text":"two things"}"#);
        row(&record.db, "ses_1", 19, "assistant", r#"{"content":[{"type":"text","text":"First."}]}"#);
        row(&record.db, "ses_1", 20, "assistant", r#"{"content":[{"type":"tool","text":"ls"}]}"#);
        row(&record.db, "ses_1", 21, "assistant", r#"{"content":[{"type":"text","text":"Second."}]}"#);
        assert_eq!(record.over(mark), None);
        row(&record.db, "ses_1", 27, "idle", r#"{"outcome":"succeeded"}"#);
        assert_eq!(record.over(mark).as_deref(), Some("First.\n\nSecond."));
        // a turn that wrote nothing (a compaction) is over with nothing said
        let mark = record.mark().unwrap();
        row(&record.db, "ses_1", 31, "compaction", r#"{"status":"completed"}"#);
        row(&record.db, "ses_1", 34, "idle", r#"{"outcome":"succeeded"}"#);
        assert_eq!(record.over(mark).as_deref(), Some(""));
        crate::adapters::test_env::clear();
        let _ = std::fs::remove_dir_all(home);
    }
}
