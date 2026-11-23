//! What the tower keeps on disk: one SQLite file, `~/.airband/airband.db`.
//!
//! - `messages`: the chat log, every message in either direction, for good (pictures by id; the copies are files).
//!   Memory holds only the newest thousand. Its `grp` column is the id of the team a line was written under.
//! - `events`: every `airband.agent/1` event as it arrived, understood or not (minus any delivery token), for good.
//!   This is the raw record: what the judge is tested against, and what an adapter's tests should replay.
//! - `receipts`: every utterance id the tower has taken, with the answer it gave. A retry of the same id gets the
//!   same answer and is never delivered twice, across restarts.
//! - `deliveries`: each attempt to hand his words to an agent, and how it went.
//! - `replays`: the id of every sealed request a radio sent the tower, with when, for as long as one counts as
//!   fresh: a copy played back is dropped, after a restart too.
//! - `tower_here`: the id this computer pulls from the relay under, kept with a digest of the machine it was made
//!   on: its own pull from before a restart is never taken for another computer's, and a copy of `~/.airband` on
//!   another machine (Migration Assistant, a backup) makes an id of its own there rather than pull as this one.
//! - `state`: who is on the radio, his teams, the agents the tower holds, and (apart) where each phone can be
//!   woken. Small, rewritten on change.
//!
//! Intent is written before anything is done about it: an utterance has a receipt before it is routed, and an
//! event is in `events` before it is acted on.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) struct Store {
    db: Mutex<Connection>,
}

pub(super) enum Receipt {
    /// Nobody has answered this id: it is now ours to answer.
    Begun,
    /// Already answered: this is what was said.
    Done(Reply),
    /// Whether it was answered could not be read just now, so it is not answered now: it could be a second time.
    Unknown(String),
}

/// A value kept under its key in the state table, in place of what was there.
const KEEP_UNDER: &str = "insert into state (key, value) values (?1, ?2) on conflict (key) do update set value = excluded.value";

impl Store {
    /// The database file, and the log and index the database keeps beside it (`-wal`, `-shm`), readable by him
    /// alone. Each that is not there yet is made so when it is; the database makes them on its first write.
    #[cfg(unix)]
    fn private(path: &std::path::Path) -> Result<(), String> {
        for file in [path.to_path_buf(), airband_io::files::beside(path, "-wal"), airband_io::files::beside(path, "-shm")] {
            match airband_io::files::set_mode(&file, 0o600) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                other => other.map_err(|e| format!("cannot make {} private: {e}", file.display()))?,
            }
        }
        Ok(())
    }
    /// Elsewhere a file's mode is not how it is kept private.
    #[cfg(not(unix))]
    fn private(_: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    pub(super) fn open(path: &std::path::Path) -> Result<Store, String> {
        let db = Connection::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        // the database and the two files the database writes beside it, which hold the same lines
        Store::private(path)?;
        db.execute_batch(
            "pragma journal_mode = wal;
             pragma cache_size = -256;
             create table if not exists messages (seq integer primary key, id text not null unique, at integer not null, sender text not null,
                 grp text not null, recipients text not null, kind text, short text, full text not null, detail text, options text,
                 recommended integer, pictures text, agent text, to_agents text, rev integer, read text, waiting text, own_session integer,
                 seen integer, aloud text, notice integer, request text, note text, his integer, more integer, background integer);
             create table if not exists events (n integer primary key autoincrement, at integer not null, agent text not null, harness text not null, event text not null, agent_doc text);
             create table if not exists receipts (id text primary key, at integer not null, reply text);
             create table if not exists deliveries (n integer primary key autoincrement, at integer not null, message text not null, callsign text not null, ok integer not null, error text, utterance text);
             create table if not exists state (key text primary key, value text not null);
             create table if not exists replays (id text primary key, at integer not null);
             create table if not exists tower_here (one integer primary key check (one = 1), id text not null, machine text not null);",
        )
        .map_err(|e| format!("cannot prepare {}: {e}", path.display()))?;
        // its log and index are there now that it has been written to
        Store::private(path)?;
        Ok(Store { db: Mutex::new(db) })
    }

    fn warn<T>(what: &str, r: rusqlite::Result<T>) -> Option<T> {
        r.map_err(|e| log(format!("database: {what}: {e}"))).ok()
    }

    /// A value kept under `key` in the state table, read as `T`; the default when none is kept. One this build
    /// cannot read is set aside under `<key>.unreadable`, not written over, and the default is used: what was
    /// there may matter to whoever looks into why. A database that cannot be read at all is an error, and nothing
    /// is started on top of it.
    fn kept<T: serde::de::DeserializeOwned + Default>(db: &rusqlite::Connection, key: &str) -> Result<T, String> {
        let text = db.query_row("select value from state where key = ?1", params![key], |r| r.get::<_, String>(0)).optional().map_err(|e| format!("the database could not be read ({e})"))?;
        match text.as_deref().map(serde_json::from_str::<T>) {
            None => Ok(T::default()),
            Some(Ok(value)) => Ok(value),
            Some(Err(e)) => {
                log(format!("database: what was kept as '{key}' could not be read ({e}); kept as '{key}.unreadable', starting without it"));
                Self::warn("setting state aside", db.execute(KEEP_UNDER, params![format!("{key}.unreadable"), text]));
                Ok(T::default())
            }
        }
    }

    /// Everything needed to carry on after a restart: the kept tables and the newest messages, oldest first.
    /// Each table is read on its own, so one this build cannot read is set aside alone. Agents check in again
    /// by themselves, so a table of them that cannot be read starts with nobody on the radio.
    pub(super) fn load(&self, keep: usize) -> Result<(Saved, Vec<Entry>), String> {
        let db = self.db.lock().unwrap();
        let saved = Saved {
            sessions: Self::kept(&db, "sessions")?,
            teams: Self::kept(&db, "teams")?,
            held: Self::kept(&db, "held")?,
            menus: Self::kept(&db, "menus")?,
            tools: Self::kept(&db, "tools")?,
            placed: Self::kept(&db, "placed")?,
            links: Self::kept(&db, "links")?,
        };
        Ok((saved, Self::messages(&db, keep)))
    }

    /// The newest `keep` messages, oldest first. A row this build cannot read, or a column of one that does not
    /// read as what it should be, is said in the log with the message's number, never passed over in silence.
    fn messages(db: &Connection, keep: usize) -> Vec<Entry> {
        let Some(mut q) = Self::warn("reading messages", db.prepare("select seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen, own_session, aloud, notice, request, note, his, more, background from messages order by seq desc limit ?1")) else { return Vec::new() };
        let rows = q.query_map(params![keep as i64], |r| {
            let seq = r.get::<_, i64>(0)? as u64;
            let column = |n: usize| r.get::<_, Option<String>>(n);
            Ok(Entry {
                seq,
                id: r.get(1)?,
                at: r.get::<_, i64>(2)? as u64,
                from: r.get(3)?,
                team: Some(r.get::<_, String>(4)?).filter(|t| !t.is_empty()).map(TeamId::new),
                to: read_json(seq, "recipients", column(5)?).unwrap_or_default(),
                kind: read_word(seq, "kind", column(6)?),
                short: r.get(7)?,
                full: r.get(8)?,
                detail: r.get(9)?,
                options: read_json(seq, "options", column(10)?).unwrap_or_default(),
                recommended: r.get::<_, Option<i64>>(11)?.map(|n| n as u32),
                pictures: read_json(seq, "pictures", column(12)?).unwrap_or_default(),
                agent: r.get(13)?,
                his: r.get::<_, Option<bool>>(24)?.unwrap_or(false),
                more: r.get::<_, Option<bool>>(25)?.unwrap_or(false),
                to_agents: read_json(seq, "to_agents", column(14)?).unwrap_or_default(),
                rev: r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                read: r.get(16)?,
                own_session: r.get::<_, Option<i64>>(19)?.unwrap_or(0) != 0,
                background: r.get::<_, Option<bool>>(26)?.unwrap_or(false),
                held: read_word(seq, "waiting", column(17)?),
                seen: r.get::<_, Option<i64>>(18)?.map(|t| t as u64),
                permit: None,
                leave: read_json(seq, "request", column(22)?),
                aloud: read_json(seq, "aloud", column(20)?).unwrap_or_default(),
                notice: r.get::<_, Option<i64>>(21)? == Some(1),
                note: read_word(seq, "note", column(23)?),
            })
        });
        let Some(rows) = Self::warn("reading messages", rows) else { return Vec::new() };
        let mut entries = Vec::new();
        for row in rows {
            match row {
                Ok(e) => entries.push(e),
                Err(e) => log(format!("database: a message could not be read and is left out ({e})")),
            }
        }
        entries.reverse();
        // in the order they were said: something he typed at a keyboard is added after the fact, with the
        // time he typed it
        entries.sort_by_key(|e: &Entry| (e.at, e.seq));
        entries
    }

    /// Write what changed in the state: each kept table under its own key, and the messages, in one go. All of it
    /// is written or none of it is, and an error says so: the caller keeps the batch and tries again.
    pub(super) fn write(&self, batch: &Batch) -> rusqlite::Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        for (key, text) in &batch.tables {
            tx.execute(KEEP_UNDER, params![key, text])?;
        }
        for e in &batch.messages {
            // one under another line's id is left out and said, never let stop every later write
            if Self::put_one(&tx, e)? == 0 {
                log(format!("database: message {} has the id of another and is not written", e.seq));
            }
        }
        for (id, reply) in &batch.receipts {
            Self::set_reply(&tx, id, reply)?;
        }
        tx.commit()
    }

    /// Where each paired radio can be woken with a notification, by the radio's name. Kept apart from the rest
    /// of the state: it changes rarely, and it is not something a status page should ever show.
    pub(super) fn push_targets(&self) -> Result<BTreeMap<String, Vec<airband_core::radio::PushTarget>>, String> {
        Self::kept(&self.db.lock().unwrap(), "push")
    }
    pub(super) fn save_push_targets(&self, targets: &BTreeMap<String, Vec<airband_core::radio::PushTarget>>) {
        let db = self.db.lock().unwrap();
        Self::warn("saving where phones can be woken", db.execute(KEEP_UNDER, params!["push", serde_json::to_string(targets).unwrap()]));
    }

    /// Write new messages, and messages whose short or fuller version has just been made.
    #[cfg(test)]
    pub(super) fn put_messages(&self, entries: &[Entry]) {
        self.write(&Batch { messages: entries.to_vec(), ..Batch::default() }).unwrap();
    }

    fn put_one(db: &Connection, e: &Entry) -> rusqlite::Result<usize> {
        let kind = e.kind.as_ref().and_then(word);
        db.execute(
            "insert into messages (seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen, own_session, aloud, notice, request, note, his, more, background) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)
             on conflict (seq) do update set background = excluded.background, grp = excluded.grp, kind = excluded.kind, short = excluded.short, full = excluded.full, detail = excluded.detail, options = excluded.options, recommended = excluded.recommended, rev = excluded.rev, read = excluded.read, waiting = excluded.waiting, seen = excluded.seen, request = excluded.request, own_session = excluded.own_session, aloud = excluded.aloud, more = excluded.more
             on conflict do nothing",
            params![e.seq as i64, e.id, e.at as i64, e.from, e.team.as_ref().map_or("", |t| t.as_str()), serde_json::to_string(&e.to).unwrap(), kind, e.short, e.full, e.detail, serde_json::to_string(&e.options).unwrap(), e.recommended.map(|n| n as i64), serde_json::to_string(&e.pictures).unwrap(), e.agent, serde_json::to_string(&e.to_agents).unwrap(), e.rev as i64, e.read, e.held.as_ref().and_then(word), e.seen.map(|t| t as i64), e.own_session as i64, serde_json::to_string(&e.aloud).unwrap(), e.notice as i64, e.leave.as_ref().map(|r| serde_json::to_string(r).unwrap()), e.note.as_ref().and_then(word), e.his, e.more, e.background],
        )
    }

    /// The highest message number and change count ever used, so both carry on after a restart even past what
    /// memory holds. A database that cannot say is an error: counting again from nothing would write new lines
    /// over old ones.
    pub(super) fn last_counts(&self) -> Result<(u64, u64), String> {
        self.db.lock().unwrap().query_row("select coalesce(max(seq), 0), coalesce(max(rev), 0) from messages", [], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))).map_err(|e| format!("the database could not say how many messages it holds ({e})"))
    }

    /// Drop what is too old to be wanted, so the file does not grow for ever: the record of raw events and of
    /// deliveries after a month, the ids of what he said after a month (the relay holds a message for a week at
    /// most, so none can come again later than that), and all but the newest 5,000 messages.
    pub(super) fn prune(&self) {
        let before = now_ms().saturating_sub(30 * 24 * 3600 * 1000) as i64;
        let db = self.db.lock().unwrap();
        for table in ["events", "deliveries", "receipts"] {
            Self::warn("dropping old records", db.execute(&format!("delete from {table} where at < ?1"), params![before]));
        }
        Self::warn("dropping old messages", db.execute("delete from messages where seq <= (select max(seq) from messages) - 5000", []));
    }

    /// The record of what an agent's harness reported, as it arrived: the event and what the agent said of
    /// itself, whether or not this build understands them. The one thing left out is a token for reaching the
    /// agent, which is not this file's to keep.
    pub(super) fn event(&self, raw: &Value) {
        let mut about = raw["agent"].clone();
        if let Some(deliver) = about.get_mut("deliver").and_then(Value::as_object_mut) {
            deliver.remove("token");
        }
        let db = self.db.lock().unwrap();
        Self::warn(
            "recording an event",
            db.execute("insert into events (at, agent, harness, event, agent_doc) values (?1, ?2, ?3, ?4, ?5)", params![now_ms() as i64, raw["agent"]["id"].as_str().unwrap_or(""), raw["agent"]["harness"].as_str().unwrap_or(""), airband_core::redact::redact(&raw["event"].to_string()), about.to_string()]),
        );
    }

    /// Take an utterance id before doing anything about it. The caller knows which ids this tower is answering
    /// right now and does not ask about those: a receipt found here with no answer was left by a tower that
    /// stopped part way.
    pub(super) fn begin_receipt(&self, id: &str) -> Receipt {
        let db = self.db.lock().unwrap();
        let found = match db.query_row("select reply from receipts where id = ?1", params![id], |r| r.get::<_, Option<String>>(0)).optional() {
            Ok(found) => found,
            Err(e) => return Receipt::Unknown(e.to_string()),
        };
        match found {
            Some(Some(reply)) => match serde_json::from_str(&reply) {
                Ok(reply) => Receipt::Done(reply),
                // answered, but the answer cannot be read: it is never answered a second time
                Err(e) => Receipt::Done(Reply { say: format!("This was already answered; the answer could not be read ({e})."), listen: false, done: true, ..Reply::default() }),
            },
            // A receipt with no answer: the tower stopped part way. If the words had already reached their
            // agents, say so and deliver nothing again; a message arriving twice can be acted on twice. If who
            // they reached cannot be read, nothing is done now: doing it again could deliver them twice.
            Some(None) => match Self::delivered(&db, id) {
                Err(e) => Receipt::Unknown(e),
                Ok(reached) if !reached.is_empty() => {
                    let reply = Reply { say: format!("Sent to {}.", list(&reached)), listen: false, done: true, ..Reply::default() };
                    Self::warn("closing a receipt", Self::set_reply(&db, id, &reply));
                    Receipt::Done(reply)
                }
                Ok(_) => Self::take(&db, id),
            },
            None => Self::take(&db, id),
        }
    }

    /// An utterance id taken: from now on it is this tower's to answer.
    /// What came of an utterance, written to its receipt: the one place a receipt is closed.
    fn set_reply(db: &Connection, id: &str, reply: &Reply) -> rusqlite::Result<usize> {
        let reply = serde_json::to_string(reply).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        db.execute("update receipts set reply = ?2 where id = ?1", params![id, reply])
    }

    fn take(db: &Connection, id: &str) -> Receipt {
        match db.execute("insert into receipts (id, at) values (?1, ?2) on conflict (id) do update set at = excluded.at", params![id, now_ms() as i64]) {
            Ok(_) => Receipt::Begun,
            Err(e) => Receipt::Unknown(e.to_string()),
        }
    }

    /// Is this the first time a sealed request of this id has come? Ids are kept for `keep_ms` after they came,
    /// across restarts, so a recorded request played back after the tower started again is still dropped. An
    /// error says it could not be checked: the request is not run, since it could be a second time.
    pub(super) fn first_time(&self, id: &str, now: u64, keep_ms: u64) -> Result<bool, String> {
        let db = self.db.lock().unwrap();
        db.execute("delete from replays where at < ?1", params![now.saturating_sub(keep_ms) as i64]).map_err(|e| e.to_string())?;
        db.execute("insert into replays (id, at) values (?1, ?2) on conflict (id) do nothing", params![id, now as i64]).map(|n| n == 1).map_err(|e| e.to_string())
    }

    /// The id this computer pulls from the relay under: the one kept, when it was made on this machine
    /// (`machine`, a digest of the machine's own id, [`crate::vault::machine_digest`]); else a new one, kept from
    /// then on. Two computers with one copy of the database never pull under one id.
    pub(super) fn tower_id(&self, machine: &str) -> Result<String, String> {
        let db = self.db.lock().unwrap();
        let kept = db.query_row("select id, machine from tower_here where one = 1", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(|e| e.to_string())?;
        if let Some((id, _)) = kept.filter(|(_, made_on)| made_on == machine) {
            return Ok(id);
        }
        let id = airband_core::encode::random_hex(8);
        db.execute("insert into tower_here (one, id, machine) values (1, ?1, ?2) on conflict (one) do update set id = excluded.id, machine = excluded.machine", params![id, machine]).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// `utterance` is the id the radio gave what he said, when it gave one.
    pub(super) fn delivery(&self, message: &str, utterance: Option<&str>, callsign: &str, outcome: &Result<(), String>) {
        let db = self.db.lock().unwrap();
        Self::warn("recording a delivery", db.execute("insert into deliveries (at, message, callsign, ok, error, utterance) values (?1, ?2, ?3, ?4, ?5, ?6)", params![now_ms() as i64, message, callsign, outcome.is_ok() as i64, outcome.as_ref().err(), utterance]));
    }

    /// Who an utterance has already reached, or why that cannot be read.
    fn delivered(db: &Connection, utterance: &str) -> Result<Vec<String>, String> {
        let mut q = db.prepare("select callsign from deliveries where utterance = ?1 and ok = 1 order by n").map_err(|e| e.to_string())?;
        let rows = q.query_map(params![utterance], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }

    /// The database held, as another writer would hold it: for tests of what does not wait on it.
    #[cfg(test)]
    pub(super) fn hold(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap()
    }

    /// Counts, for `airband status`.
    pub(super) fn counts(&self) -> Option<(u64, u64)> {
        let db = self.db.lock().unwrap();
        let n = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).ok().map(|n| n as u64);
        Some((n("select count(*) from messages")?, n("select count(*) from events")?))
    }
}

/// A column kept as JSON, read as what it should be; one that does not read is said in the log.
fn read_json<T: serde::de::DeserializeOwned>(seq: u64, column: &str, text: Option<String>) -> Option<T> {
    let text = text?;
    serde_json::from_str(&text).map_err(|e| log(format!("database: message {seq}: its {column} could not be read ({e})"))).ok()
}
/// A column kept as one word of a typed set, read as that type; one this build does not know is said in the log.
fn read_word<T: serde::de::DeserializeOwned>(seq: u64, column: &str, text: Option<String>) -> Option<T> {
    let text = text?;
    serde_json::from_value(Value::String(text)).map_err(|e| log(format!("database: message {seq}: its {column} could not be read ({e})"))).ok()
}
pub(super) use airband_core::encode::word;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn the_database_and_the_files_beside_it_are_his_alone() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("airband-private-{}.db", crate::config::random_hex(4)));
        let s = Store::open(&path).unwrap();
        s.put_messages(&[Entry { seq: 1, id: "a".into(), ..Entry::default() }]);
        Store::private(&path).unwrap();
        for tail in ["", "-wal", "-shm"] {
            let file = airband_io::files::beside(&path, tail);
            if let Ok(meta) = std::fs::metadata(&file) {
                assert_eq!(meta.permissions().mode() & 0o777, 0o600, "{}", file.display());
            }
        }
        assert!(airband_io::files::beside(&path, "-wal").exists());
    }

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("airband-store-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn a_line_kept_while_its_agent_waits_on_its_own_work_says_so_after_a_restart() {
        let (s, path) = store("background");
        s.put_messages(&[Entry { seq: 1, id: "a".into(), short: Some("Waiting on the build.".into()), background: true, ..Entry::default() }]);
        assert!(s.load(10).unwrap().1[0].background);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_line_under_another_lines_id_never_stops_the_writes_after_it() {
        let (s, path) = store("same-id");
        s.put_messages(&[Entry { seq: 1, id: "a".into(), ..Entry::default() }]);
        // the same id under another number, with a line after it in the same batch
        s.write(&Batch { messages: vec![Entry { seq: 2, id: "a".into(), ..Entry::default() }, Entry { seq: 3, id: "b".into(), ..Entry::default() }], ..Batch::default() }).expect("written, the copy left out");
        assert_eq!(s.load(10).unwrap().1.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 3]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn whether_a_line_is_his_is_kept_as_itself_and_never_read_from_the_sender() {
        let (s, path) = store("his");
        // an agent that calls itself what his lines are labelled, and a line of his
        s.put_messages(&[Entry { seq: 1, id: "a".into(), from: HIM.into(), team: Some(TeamId::new("/w/shop")), ..Entry::default() }, Entry { seq: 2, id: "b".into(), from: HIM.into(), his: true, ..Entry::default() }]);
        let (_, kept) = s.load(10).unwrap();
        assert_eq!(kept.iter().map(|e| (e.id.as_str(), e.his)).collect::<Vec<_>>(), [("a", false), ("b", true)]);
        // a line saved again under its team's id is kept under it
        s.put_messages(&[Entry { seq: 1, id: "a".into(), team: Some(TeamId::new("/w/garden")), ..Entry::default() }]);
        assert_eq!(s.load(10).unwrap().1[0].team, Some(TeamId::new("/w/garden")));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_replayed_request_is_dropped_after_a_restart_too_and_let_go_once_it_is_stale() {
        let (s, path) = store("replays");
        assert_eq!(s.first_time("r1", 1_000, 100), Ok(true));
        assert_eq!(s.first_time("r1", 1_050, 100), Ok(false));
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.first_time("r1", 1_060, 100), Ok(false), "the tower started again and still knows it");
        assert_eq!(s.first_time("r1", 1_200, 100), Ok(true), "too old to count as fresh: forgotten");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_tower_pulls_under_one_id_kept_across_restarts() {
        let (s, path) = store("tower-id");
        let id = s.tower_id("this-mac").unwrap();
        assert_eq!(s.tower_id("this-mac").unwrap(), id);
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.tower_id("this-mac").unwrap(), id);
        // the same database on another machine: an id of its own, kept there from then on
        let there = s.tower_id("another-mac").unwrap();
        assert_ne!(there, id);
        assert_eq!(s.tower_id("another-mac").unwrap(), there);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_utterance_is_taken_once() {
        let (s, path) = store("receipts");
        assert!(matches!(s.begin_receipt("u1"), Receipt::Begun));
        // the answer is written with the line that came of it, in one go
        let line = Entry { seq: 9, id: "said".into(), ..Entry::default() };
        s.write(&Batch { messages: vec![line], receipts: vec![("u1".into(), Reply { say: "Sent to shop one.".into(), listen: false, ..Reply::default() })], ..Batch::default() }).unwrap();
        assert!(matches!(s.begin_receipt("u1"), Receipt::Done(r) if r.say == "Sent to shop one."));
        assert_eq!(s.load(10).unwrap().1.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["said"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_tower_that_stopped_after_delivering_does_not_deliver_again() {
        let (s, path) = store("redeliver");
        assert!(matches!(s.begin_receipt("u2"), Receipt::Begun));
        s.delivery("m1", Some("u2"), "backoffice one", &Ok(()));
        // the tower stops here, before the receipt is closed; the radio tries the same utterance again
        assert!(matches!(s.begin_receipt("u2"), Receipt::Done(r) if r.say == "Sent to backoffice one."));
        // one that stopped before reaching anyone is taken up again
        assert!(matches!(s.begin_receipt("u3"), Receipt::Begun));
        s.delivery("m2", Some("u3"), "shop one", &Err("gone".into()));
        assert!(matches!(s.begin_receipt("u3"), Receipt::Begun));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_event_is_kept_as_it_came_without_the_way_to_reach_the_agent() {
        let (s, path) = store("events");
        s.event(&json!({ "schema": "airband.agent/1", "agent": { "id": "a", "harness": "new-thing", "deliver": { "kind": "socket", "path": "/s", "token": "secret" } }, "event": { "type": "molting", "depth": 3 } }));
        let (event, about): (String, String) = s.db.lock().unwrap().query_row("select event, agent_doc from events", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&event).unwrap(), json!({ "type": "molting", "depth": 3 }));
        assert!(about.contains("/s") && !about.contains("secret"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn where_phones_are_woken_that_cannot_be_read_is_set_aside_and_not_written_over() {
        let (s, path) = store("push-unreadable");
        s.db.lock().unwrap().execute("insert into state (key, value) values ('push', 'not json')", []).unwrap();
        assert_eq!(s.push_targets().unwrap().len(), 0);
        let aside: String = s.db.lock().unwrap().query_row("select value from state where key = 'push.unreadable'", [], |r| r.get(0)).unwrap();
        assert_eq!(aside, "not json");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_kept_table_that_cannot_be_read_is_set_aside_alone() {
        let (s, path) = store("one-table");
        s.write(&Batch { tables: vec![("sessions", "[]".into()), ("teams", "not json".into()), ("tools", r#"["some-tool"]"#.into())], ..Batch::default() }).unwrap();
        let (saved, _) = s.load(10).unwrap();
        assert!(saved.teams.is_empty(), "the unreadable table starts empty");
        assert_eq!(saved.tools, ["some-tool"], "the others are read");
        let aside: String = s.db.lock().unwrap().query_row("select value from state where key = 'teams.unreadable'", [], |r| r.get(0)).unwrap();
        assert_eq!(aside, "not json");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_receipt_whose_deliveries_cannot_be_read_is_not_taken_again() {
        let (s, path) = store("receipt-unknown");
        assert!(matches!(s.begin_receipt("u4"), Receipt::Begun));
        // the tower stopped part way, and who the words reached cannot be read now
        s.db.lock().unwrap().execute("drop table deliveries", []).unwrap();
        assert!(matches!(s.begin_receipt("u4"), Receipt::Unknown(_)), "never taken as not yet delivered");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_message_row_that_cannot_be_read_is_left_out_and_the_rest_are_read() {
        let (s, path) = store("bad-row");
        s.put_messages(&[Entry { seq: 1, id: "a".into(), ..Entry::default() }, Entry { seq: 2, id: "b".into(), ..Entry::default() }]);
        s.db.lock().unwrap().execute("update messages set at = 'not a number' where seq = 1", []).unwrap();
        s.db.lock().unwrap().execute("update messages set options = 'not json' where seq = 2", []).unwrap();
        let (_, kept) = s.load(10).unwrap();
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_database_that_cannot_count_its_messages_stops_the_start() {
        let (s, path) = store("counts");
        s.db.lock().unwrap().execute("drop table messages", []).unwrap();
        assert!(s.last_counts().is_err());
        let _ = std::fs::remove_file(path);
    }
}
