//! What the tower keeps on disk: one SQLite file, `~/.airband/airband.db`.
//!
//! - `messages`: the chat log, every message in either direction, for good (pictures by id; the copies are files). Memory holds only the newest thousand.
//! - `events`: every `airband.agent/1` event as it arrived, understood or not (minus any delivery token), for good.
//!   This is the raw record: what the judge is tested against, and what an adapter's tests should replay.
//! - `receipts`: every utterance id the tower has taken, with the answer it gave. A retry of the same id gets the
//!   same answer and is never delivered twice, across restarts.
//! - `deliveries`: each attempt to hand his words to an agent, and how it went.
//! - `state`: who is on the radio, what is unheard, which projects are known. Small, rewritten on change.
//!
//! Intent is written before anything is done about it: an utterance has a receipt before it is routed, and an
//! event is in `events` before it is acted on.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};

pub struct Store {
    db: Mutex<Connection>,
}

pub(super) enum Receipt {
    /// Nobody has answered this id: it is now ours to answer.
    Begun,
    /// Already answered: this is what was said.
    Done(Reply),
}

impl Store {
    pub(super) fn open(path: &std::path::Path) -> Result<Store, String> {
        let db = Connection::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        db.execute_batch(
            "pragma journal_mode = wal;
             pragma cache_size = -256;
             create table if not exists messages (seq integer primary key, id text not null unique, at integer not null, sender text not null,
                 grp text not null, recipients text not null, kind text, short text, full text not null, detail text, options text,
                 recommended integer, pictures text, agent text, to_agents text, rev integer, read text, waiting text, own_session integer,
                 seen integer, leave integer, aloud text, notice integer);
             create table if not exists events (n integer primary key autoincrement, at integer not null, agent text not null, harness text not null, event text not null, agent_doc text);
             create table if not exists receipts (id text primary key, at integer not null, reply text);
             create table if not exists deliveries (n integer primary key autoincrement, at integer not null, message text not null, callsign text not null, ok integer not null, error text, utterance text);
             create table if not exists state (key text primary key, value text not null);",
        )
        .map_err(|e| format!("cannot prepare {}: {e}", path.display()))?;
        // what a request for leave asked and where it stands; a database made before this has no such column
        // columns added since the table was first made: what a request for leave asked and where it stands,
        // and what airband says around the tool's words
        for (column, kind) in [("request", "text"), ("note", "text"), ("his", "integer")] {
            if db.prepare(&format!("select {column} from messages limit 0")).is_err() {
                db.execute_batch(&format!("alter table messages add column {column} {kind};")).map_err(|e| format!("cannot prepare {}: {e}", path.display()))?;
                // Kept from before whether a line is his had its own column: his lines were the ones written
                // under his label. Read so once, here, and kept in the column from then on.
                if column == "his" {
                    db.execute("update messages set his = (sender = ?1)", params![HIM]).map_err(|e| format!("cannot prepare {}: {e}", path.display()))?;
                }
            }
        }
        Ok(Store { db: Mutex::new(db) })
    }

    fn warn<T>(what: &str, r: rusqlite::Result<T>) -> Option<T> {
        r.map_err(|e| log(format!("database: {what}: {e}"))).ok()
    }

    /// Everything needed to carry on after a restart: the saved state and the newest messages, oldest first.
    pub(super) fn load(&self, keep: usize) -> (Saved, Vec<Entry>) {
        let db = self.db.lock().unwrap();
        let text = db.query_row("select value from state where key = 'tower'", [], |r| r.get::<_, String>(0)).optional().ok().flatten();
        let saved = match text.as_deref().map(serde_json::from_str::<Saved>) {
            Some(Ok(saved)) => saved,
            None => Saved::default(),
            // state this build cannot read is set aside, not written over: agents check in again by themselves,
            // but what was there may matter to whoever looks into why
            Some(Err(e)) => {
                log(format!("database: the saved state could not be read ({e}); kept as 'tower.unreadable', starting with nobody on the radio"));
                Self::warn("setting state aside", db.execute("insert into state (key, value) values ('tower.unreadable', ?1) on conflict (key) do update set value = excluded.value", params![text]));
                Saved::default()
            }
        };
        let mut entries = Vec::new();
        if let Some(mut q) = Self::warn("reading messages", db.prepare("select seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen, leave, own_session, aloud, notice, request, note, his from messages order by seq desc limit ?1")) {
            let rows = q.query_map(params![keep as i64], |r| {
                Ok(Entry {
                    seq: r.get::<_, i64>(0)? as u64,
                    id: r.get(1)?,
                    at: r.get::<_, i64>(2)? as u64,
                    from: r.get(3)?,
                    team: r.get(4)?,
                    to: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    kind: r.get::<_, Option<String>>(6)?.and_then(|k| serde_json::from_value(Value::String(k)).ok()),
                    short: r.get(7)?,
                    full: r.get(8)?,
                    detail: r.get(9)?,
                    options: r.get::<_, Option<String>>(10)?.and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default(),
                    recommended: r.get::<_, Option<i64>>(11)?.map(|n| n as u32),
                    pictures: r.get::<_, Option<String>>(12)?.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
                    agent: r.get(13)?,
                    his: r.get::<_, Option<bool>>(25)?.unwrap_or(false),
                    to_agents: r.get::<_, Option<String>>(14)?.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
                    rev: r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                    read: r.get(16)?,
                    own_session: r.get::<_, Option<i64>>(20)?.unwrap_or(0) != 0,
                    background: false,
                    dismissed: held(r)? == Some(Held::Dismissed),
                    seen: r.get::<_, Option<i64>>(18)?.map(|t| t as u64),
                    queued: held(r)? == Some(Held::Queued),
                    cancelled: held(r)? == Some(Held::Cancelled),
                    permit: None,
                    // a request kept before what it asked was written down is still known as one, so a late
                    // answer to it is never passed on as words
                    leave: match r.get::<_, Option<String>>(23)?.and_then(|t| serde_json::from_str(&t).ok()) {
                        Some(request) => Some(request),
                        None => (r.get::<_, Option<i64>>(19)? == Some(1)).then_some(super::permit::Request { what: None, at_keyboard: true }),
                    },
                    aloud: r.get::<_, Option<String>>(21)?.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default(),
                    notice: r.get::<_, Option<i64>>(22)? == Some(1),
                    note: r.get::<_, Option<String>>(24)?.and_then(|n| serde_json::from_value(Value::String(n)).ok()),
                })
            });
            if let Some(rows) = Self::warn("reading messages", rows) {
                entries = rows.flatten().collect();
                entries.reverse();
                // in the order they were said: something he typed at a keyboard is added after the fact, with the
                // time he typed it
                entries.sort_by_key(|e: &Entry| (e.at, e.seq));
            }
        }
        (saved, entries)
    }

    pub(super) fn save_state(&self, saved: &Saved) {
        let db = self.db.lock().unwrap();
        Self::warn("saving state", db.execute("insert into state (key, value) values ('tower', ?1) on conflict (key) do update set value = excluded.value", params![serde_json::to_string(saved).unwrap()]));
    }

    /// Where each paired radio can be woken with a notification, by the radio's name. Kept apart from the rest
    /// of the state: it changes rarely, and it is not something a status page should ever show.
    pub(super) fn push_targets(&self) -> BTreeMap<String, Vec<airband_core::radio::PushTarget>> {
        let db = self.db.lock().unwrap();
        let text = db.query_row("select value from state where key = 'push'", [], |r| r.get::<_, String>(0)).optional().ok().flatten();
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub(super) fn save_push_targets(&self, targets: &BTreeMap<String, Vec<airband_core::radio::PushTarget>>) {
        let db = self.db.lock().unwrap();
        Self::warn("saving where phones can be woken", db.execute("insert into state (key, value) values ('push', ?1) on conflict (key) do update set value = excluded.value", params![serde_json::to_string(targets).unwrap()]));
    }

    /// Write new messages, and messages whose short or fuller version has just been made.
    pub(super) fn put_messages(&self, entries: &[Entry]) {
        if entries.is_empty() {
            return;
        }
        let db = self.db.lock().unwrap();
        for e in entries {
            let kind = e.kind.as_ref().and_then(word);
            Self::warn(
                "saving a message",
                db.execute(
                    "insert into messages (seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen, leave, own_session, aloud, notice, request, note, his) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)
                     on conflict (seq) do update set grp = excluded.grp, kind = excluded.kind, short = excluded.short, full = excluded.full, detail = excluded.detail, options = excluded.options, recommended = excluded.recommended, rev = excluded.rev, read = excluded.read, waiting = excluded.waiting, seen = excluded.seen, leave = excluded.leave, request = excluded.request, own_session = excluded.own_session, aloud = excluded.aloud",
                    params![e.seq as i64, e.id, e.at as i64, e.from, e.team, serde_json::to_string(&e.to).unwrap(), kind, e.short, e.full, e.detail, serde_json::to_string(&e.options).unwrap(), e.recommended.map(|n| n as i64), serde_json::to_string(&e.pictures).unwrap(), e.agent, serde_json::to_string(&e.to_agents).unwrap(), e.rev as i64, e.read, Held::of(e).map(Held::word), e.seen.map(|t| t as i64), e.leave.is_some() as i64, e.own_session as i64, serde_json::to_string(&e.aloud).unwrap(), e.notice as i64, e.leave.as_ref().map(|r| serde_json::to_string(r).unwrap()), e.note.as_ref().and_then(word), e.his],
                ),
            );
        }
    }

    /// The highest message number and change count ever used, so both carry on after a restart even past what
    /// memory holds.
    pub(super) fn last_counts(&self) -> (u64, u64) {
        self.db.lock().unwrap().query_row("select coalesce(max(seq), 0), coalesce(max(rev), 0) from messages", [], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))).unwrap_or((0, 0))
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
        let found = db.query_row("select reply from receipts where id = ?1", params![id], |r| r.get::<_, Option<String>>(0)).optional().ok().flatten();
        match found {
            Some(Some(reply)) => Receipt::Done(serde_json::from_str(&reply).unwrap_or_default()),
            // A receipt with no answer: the tower stopped part way. If the words had already reached their
            // agents, say so and deliver nothing again; a message arriving twice can be acted on twice.
            Some(None) if !Self::delivered(&db, id).is_empty() => {
                let reply = Reply { say: format!("Sent to {}.", list(&Self::delivered(&db, id))), listen: false, done: true, ..Reply::default() };
                Self::warn("closing a receipt", db.execute("update receipts set reply = ?2 where id = ?1", params![id, serde_json::to_string(&reply).unwrap()]));
                Receipt::Done(reply)
            }
            _ => {
                Self::warn("taking a receipt", db.execute("insert into receipts (id, at) values (?1, ?2) on conflict (id) do update set at = excluded.at", params![id, now_ms() as i64]));
                Receipt::Begun
            }
        }
    }

    pub(super) fn finish_receipt(&self, id: &str, reply: &Reply) {
        let db = self.db.lock().unwrap();
        Self::warn("closing a receipt", db.execute("update receipts set reply = ?2 where id = ?1", params![id, serde_json::to_string(reply).unwrap()]));
    }

    /// `utterance` is the id the radio gave what he said, when it gave one.
    pub(super) fn delivery(&self, message: &str, utterance: Option<&str>, callsign: &str, outcome: &Result<(), String>) {
        let db = self.db.lock().unwrap();
        Self::warn("recording a delivery", db.execute("insert into deliveries (at, message, callsign, ok, error, utterance) values (?1, ?2, ?3, ?4, ?5, ?6)", params![now_ms() as i64, message, callsign, outcome.is_ok() as i64, outcome.as_ref().err(), utterance]));
    }

    /// Who an utterance has already reached.
    fn delivered(db: &Connection, utterance: &str) -> Vec<String> {
        let Ok(mut q) = db.prepare("select callsign from deliveries where utterance = ?1 and ok = 1 order by n") else { return Vec::new() };
        q.query_map(params![utterance], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Counts, for `airband status`.
    pub(super) fn counts(&self) -> (u64, u64) {
        let db = self.db.lock().unwrap();
        let n = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
        (n("select count(*) from messages"), n("select count(*) from events"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("airband-store-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn whether_a_line_is_his_is_kept_as_itself_and_never_read_from_the_sender() {
        let (s, path) = store("his");
        // an agent that calls itself what his lines are labelled, and a line of his
        s.put_messages(&[Entry { seq: 1, id: "a".into(), from: HIM.into(), team: "/w/shop".into(), ..Entry::default() }, Entry { seq: 2, id: "b".into(), from: HIM.into(), his: true, ..Entry::default() }]);
        let (_, kept) = s.load(10);
        assert_eq!(kept.iter().map(|e| (e.id.as_str(), e.his)).collect::<Vec<_>>(), [("a", false), ("b", true)]);
        // a line saved again under its team's id is kept under it
        s.put_messages(&[Entry { seq: 1, id: "a".into(), team: "/w/garden".into(), ..Entry::default() }]);
        assert_eq!(s.load(10).1[0].team, "/w/garden");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_utterance_is_taken_once() {
        let (s, path) = store("receipts");
        assert!(matches!(s.begin_receipt("u1"), Receipt::Begun));
        s.finish_receipt("u1", &Reply { say: "Sent to shop one.".into(), listen: false, ..Reply::default() });
        assert!(matches!(s.begin_receipt("u1"), Receipt::Done(r) if r.say == "Sent to shop one."));
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
}

/// Where a line stands, as the `waiting` column keeps it.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Held {
    /// He let this question go.
    Dismissed,
    /// His words, waiting for the agent to stop.
    Queued,
    /// His words, taken back before they went.
    Cancelled,
}
impl Held {
    fn of(e: &Entry) -> Option<Held> {
        if e.cancelled { Some(Held::Cancelled) } else if e.queued { Some(Held::Queued) } else if e.dismissed { Some(Held::Dismissed) } else { None }
    }
    fn word(self) -> String {
        word(&self).unwrap_or_default()
    }
}
pub(super) use airband_core::encode::word;
fn held(r: &rusqlite::Row) -> rusqlite::Result<Option<Held>> {
    Ok(r.get::<_, Option<String>>(17)?.and_then(|t| serde_json::from_value(Value::String(t)).ok()))
}
