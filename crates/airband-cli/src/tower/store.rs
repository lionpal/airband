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
    /// Nobody has taken this id: it is now ours to answer.
    Begun,
    /// Already answered: this is what was said.
    Done(Reply),
    /// Someone is answering it right now.
    InProgress,
}

/// A receipt with no answer after this long belongs to a tower that stopped mid-answer; it may be taken again.
const ABANDONED_MS: u64 = 120_000;

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
                 grp text not null, recipients text not null, kind text, short text, full text not null, detail text);
             create table if not exists events (n integer primary key autoincrement, at integer not null, agent text not null, harness text not null, event text not null);
             create table if not exists receipts (id text primary key, at integer not null, reply text);
             create table if not exists deliveries (n integer primary key autoincrement, at integer not null, message text not null, callsign text not null, ok integer not null, error text);
             create table if not exists state (key text primary key, value text not null);",
        )
        .map_err(|e| format!("cannot prepare {}: {e}", path.display()))?;
        // added after the first version: fails harmlessly when the column is already there
        let _ = db.execute("alter table messages add column options text", []);
        let _ = db.execute("alter table messages add column recommended integer", []);
        let _ = db.execute("alter table messages add column pictures text", []);
        let _ = db.execute("alter table messages add column agent text", []);
        let _ = db.execute("alter table messages add column to_agents text", []);
        let _ = db.execute("alter table messages add column rev integer", []);
        let _ = db.execute("alter table messages add column read text", []);
        let _ = db.execute("alter table messages add column waiting text", []);
        let _ = db.execute("alter table messages add column seen integer", []);
        let _ = db.execute("alter table events add column agent_doc text", []);
        let _ = db.execute("alter table deliveries add column utterance text", []);
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
        if let Some(mut q) = Self::warn("reading messages", db.prepare("select seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen from messages order by seq desc limit ?1")) {
            let rows = q.query_map(params![keep as i64], |r| {
                Ok(Entry {
                    seq: r.get::<_, i64>(0)? as u64,
                    id: r.get(1)?,
                    at: r.get::<_, i64>(2)? as u64,
                    from: r.get(3)?,
                    group: r.get(4)?,
                    to: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    kind: r.get::<_, Option<String>>(6)?.and_then(|k| serde_json::from_value(Value::String(k)).ok()),
                    short: r.get(7)?,
                    full: r.get(8)?,
                    detail: r.get(9)?,
                    options: r.get::<_, Option<String>>(10)?.and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default(),
                    recommended: r.get::<_, Option<i64>>(11)?.map(|n| n as u32),
                    pictures: r.get::<_, Option<String>>(12)?.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
                    agent: r.get(13)?,
                    to_agents: r.get::<_, Option<String>>(14)?.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
                    rev: r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                    read: r.get(16)?,
                    background: false,
                    seen: r.get::<_, Option<i64>>(18)?.map(|t| t as u64),
                    queued: r.get::<_, Option<String>>(17)?.as_deref() == Some("queued"),
                    cancelled: r.get::<_, Option<String>>(17)?.as_deref() == Some("cancelled"),
                    permit: None,
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
            let kind = e.kind.and_then(|k| serde_json::to_value(k).ok()).and_then(|v| v.as_str().map(String::from));
            Self::warn(
                "saving a message",
                db.execute(
                    "insert into messages (seq, id, at, sender, grp, recipients, kind, short, full, detail, options, recommended, pictures, agent, to_agents, rev, read, waiting, seen) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
                     on conflict (seq) do update set kind = excluded.kind, short = excluded.short, full = excluded.full, detail = excluded.detail, options = excluded.options, recommended = excluded.recommended, rev = excluded.rev, read = excluded.read, waiting = excluded.waiting, seen = excluded.seen",
                    params![e.seq as i64, e.id, e.at as i64, e.from, e.group, serde_json::to_string(&e.to).unwrap(), kind, e.short, e.full, e.detail, serde_json::to_string(&e.options).unwrap(), e.recommended.map(|n| n as i64), serde_json::to_string(&e.pictures).unwrap(), e.agent, serde_json::to_string(&e.to_agents).unwrap(), e.rev as i64, e.read, if e.cancelled { Some("cancelled") } else if e.queued { Some("queued") } else { None }, e.seen.map(|t| t as i64)],
                ),
            );
        }
    }

    /// The highest message number and change count ever used, so both carry on after a restart even past what
    /// memory holds.
    pub(super) fn last_counts(&self) -> (u64, u64) {
        self.db.lock().unwrap().query_row("select coalesce(max(seq), 0), coalesce(max(rev), 0) from messages", [], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))).unwrap_or((0, 0))
    }

    /// The record of what an agent's harness reported, as it arrived: the event and what the agent said of
    /// itself, whether or not this build understands them. The one thing left out is a token for reaching the
    /// agent, which is not this file's to keep.
    /// Mask credentials in the record of raw events kept before they were masked on the way in.
    pub(super) fn scrub_events(&self) {
        let db = self.db.lock().unwrap();
        let rows: Vec<(i64, String)> = match db.prepare("select n, event from events where event like '%ghp_%' or event like '%github_pat_%' or event like '%sk-%' or event like '%sk_live_%' or event like '%AKIA%' or event like '%PRIVATE KEY-----%' or event like '%secret=%' or event like '%AIza%' or event like '%eyJ%' or event like '%xox%'") {
            Ok(mut q) => q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default(),
            Err(_) => return,
        };
        for (n, event) in rows {
            let clean = airband_core::redact::redact(&event);
            if clean != event {
                Self::warn("masking an old event", db.execute("update events set event = ?1 where n = ?2", params![clean, n]));
            }
        }
    }

    /// Drop what is too old to be wanted, so the file does not grow for ever: the record of raw events and of
    /// deliveries after a month, the ids of what he said after a month (the relay holds a message for a week at
    /// most, so none can come again later than that), and all but the newest 5,000 messages.
    pub(super) fn prune(&self) {
        let before = now().saturating_sub(30 * 24 * 3600 * 1000) as i64;
        let db = self.db.lock().unwrap();
        for table in ["events", "deliveries", "receipts"] {
            Self::warn("dropping old records", db.execute(&format!("delete from {table} where at < ?1"), params![before]));
        }
        Self::warn("dropping old messages", db.execute("delete from messages where seq <= (select max(seq) from messages) - 5000", []));
    }

    pub(super) fn event(&self, raw: &Value) {
        let mut about = raw["agent"].clone();
        if let Some(deliver) = about.get_mut("deliver").and_then(Value::as_object_mut) {
            deliver.remove("token");
        }
        let db = self.db.lock().unwrap();
        Self::warn(
            "recording an event",
            db.execute("insert into events (at, agent, harness, event, agent_doc) values (?1, ?2, ?3, ?4, ?5)", params![now() as i64, raw["agent"]["id"].as_str().unwrap_or(""), raw["agent"]["harness"].as_str().unwrap_or(""), airband_core::redact::redact(&raw["event"].to_string()), about.to_string()]),
        );
    }

    /// Take an utterance id before doing anything about it.
    pub(super) fn begin_receipt(&self, id: &str) -> Receipt {
        let db = self.db.lock().unwrap();
        let found = db.query_row("select at, reply from receipts where id = ?1", params![id], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Option<String>>(1)?))).optional().ok().flatten();
        match found {
            Some((_, Some(reply))) => Receipt::Done(serde_json::from_str(&reply).unwrap_or_default()),
            Some((at, None)) if now().saturating_sub(at) < ABANDONED_MS => Receipt::InProgress,
            // A receipt with no answer: the tower stopped part way. If the words had already reached their
            // agents, say so and deliver nothing again; a message arriving twice can be acted on twice.
            Some(_) if !Self::delivered(&db, id).is_empty() => {
                let reply = Reply { say: format!("Sent to {}.", list(&Self::delivered(&db, id))), listen: false, ..Reply::default() };
                Self::warn("closing a receipt", db.execute("update receipts set reply = ?2 where id = ?1", params![id, serde_json::to_string(&reply).unwrap()]));
                Receipt::Done(reply)
            }
            _ => {
                Self::warn("taking a receipt", db.execute("insert into receipts (id, at) values (?1, ?2) on conflict (id) do update set at = excluded.at", params![id, now() as i64]));
                Receipt::Begun
            }
        }
    }

    pub(super) fn finish_receipt(&self, id: &str, reply: &Reply) {
        let db = self.db.lock().unwrap();
        Self::warn("closing a receipt", db.execute("update receipts set reply = ?2 where id = ?1", params![id, serde_json::to_string(reply).unwrap()]));
    }

    pub(super) fn receipt_reply(&self, id: &str) -> Option<Reply> {
        let db = self.db.lock().unwrap();
        db.query_row("select reply from receipts where id = ?1", params![id], |r| r.get::<_, Option<String>>(0)).optional().ok().flatten().flatten().and_then(|t| serde_json::from_str(&t).ok())
    }

    /// `utterance` is the id the radio gave what he said, when it gave one.
    pub(super) fn delivery(&self, message: &str, utterance: Option<&str>, callsign: &str, outcome: &Result<(), String>) {
        let db = self.db.lock().unwrap();
        Self::warn("recording a delivery", db.execute("insert into deliveries (at, message, callsign, ok, error, utterance) values (?1, ?2, ?3, ?4, ?5, ?6)", params![now() as i64, message, callsign, outcome.is_ok() as i64, outcome.as_ref().err(), utterance]));
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
    fn an_utterance_is_taken_once() {
        let (s, path) = store("receipts");
        assert!(matches!(s.begin_receipt("u1"), Receipt::Begun));
        assert!(matches!(s.begin_receipt("u1"), Receipt::InProgress));
        s.finish_receipt("u1", &Reply { say: "Sent to shop one.".into(), listen: false, ..Reply::default() });
        assert!(matches!(s.begin_receipt("u1"), Receipt::Done(r) if r.say == "Sent to shop one."));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_tower_that_stopped_after_delivering_does_not_deliver_again() {
        let (s, path) = store("redeliver");
        assert!(matches!(s.begin_receipt("u2"), Receipt::Begun));
        s.delivery("m1", Some("u2"), "backoffice one", &Ok(()));
        // the tower stops here, before the receipt is closed; much later the radio tries the same utterance again
        s.db.lock().unwrap().execute("update receipts set at = 0 where id = 'u2'", []).unwrap();
        assert!(matches!(s.begin_receipt("u2"), Receipt::Done(r) if r.say == "Sent to backoffice one."));
        // one that stopped before reaching anyone is taken up again
        assert!(matches!(s.begin_receipt("u3"), Receipt::Begun));
        s.delivery("m2", Some("u3"), "shop one", &Err("gone".into()));
        s.db.lock().unwrap().execute("update receipts set at = 0 where id = 'u3'", []).unwrap();
        assert!(matches!(s.begin_receipt("u3"), Receipt::Begun));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_event_is_kept_as_it_came_without_the_way_to_reach_the_agent() {
        let (s, path) = store("events");
        s.event(&json!({ "schema": "airband.agent/1", "agent": { "id": "a", "harness": "new-thing", "deliver": { "kind": "claude-code-inbox", "socket": "/s", "token": "secret" } }, "event": { "type": "molting", "depth": 3 } }));
        let (event, about): (String, String) = s.db.lock().unwrap().query_row("select event, agent_doc from events", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&event).unwrap(), json!({ "type": "molting", "depth": 3 }));
        assert!(about.contains("/s") && !about.contains("secret"));
        let _ = std::fs::remove_file(path);
    }
}
