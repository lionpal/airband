//! Where the relay remembers which channels exist: the channel's name, the hash of its tower's pull token, the
//! hash of its radio's token. With the sealed messages kept for
//! a tower that is away, that is all it ever stores about anyone's traffic. It also keeps the homepage's waiting
//! list: email addresses people gave to be told when the app is ready, and nothing else about them. A folder
//! (one JSON file) when run on a machine with a disk; a Postgres database or Amazon DynamoDB when run somewhere
//! without one.
//!
//! DynamoDB is chosen by setting AIRBAND_RELAY_DYNAMO to a prefix. Three tables, which must already exist
//! (infra/cdk.mjs makes them; the relay's role needs no right to make or change a table):
//!
//!   <prefix>-channels   key `channel`. One item per channel: pull_hash, radio_hash.
//!   <prefix>-held       key `channel` + `k`; time to live on `expires`. Per channel:
//!                         k = "n"              how many are kept (`held`), changed only inside a transaction
//!                         k = "m#<ms>#<id>"    one per kept message, small, sorted as they arrived
//!                         k = "i#<id>"         the sealed message itself, and `at`, the key of its place in line
//!   <prefix>-waitlist   key `email`.
//!
//!   AWS_REGION   the region the tables are in. Who the relay is comes from the machine's role (dynamo.rs).
//!
//! Keeping a message is one transaction: the item named by its id is written only if it is not there (so one
//! left twice is kept once), its place in line is written, and the count goes up only if it is under the most
//! a channel may have. All three happen or none does. DynamoDB removes what is past `expires` by itself, some
//! time after; until it has, reads pass over those items. It does not lower the count when it does, so the
//! count can only run high, never low, and is counted again from the line when a channel looks full.
//!
//! Postgres is chosen by setting DB_HOST. The relay keeps its own database on that server (DB_NAME, default
//! `airband`) and creates it and its one table on first start, so it can share a server with other applications
//! without touching anything of theirs.
//!
//!   DB_HOST, DB_PORT (5432), DB_USER, DB_PASS, DB_NAME (airband)
//!   DB_CA    a PEM file of certificate authorities to trust for the connection (for Amazon RDS, its global bundle)
//!   DB_SSL   `off` for a plain local database
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_postgres::{Client, Config, NoTls};
use crate::dynamo::{self, Dynamo, n, s};
use serde_json::{Value, json};

pub(crate) enum Store {
    Folder(PathBuf),
    Postgres(Pg),
    Dynamo(Dyn),
}

/// What is remembered about one channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Saved {
    /// The hash of the token its tower pulls with.
    #[serde(rename = "pull")]
    pub(crate) pull_hash: String,
    /// The hash of the token its radio proves itself with.
    #[serde(rename = "radio")]
    pub(crate) radio_hash: String,
}

/// channels.json: each channel's [`Saved`]. A file that is there but cannot be read is an error, never "no
/// channels": taken for empty, the next change would write that back and lose every pairing.
/// One change to a folder's channel list or waiting list at a time: two towers opening at once must not each read
/// the list and write it back without the other's channel.
static LISTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_channels(dir: &std::path::Path) -> Result<HashMap<String, Saved>, String> {
    airband_core::kept::read(&dir.join("channels.json")).map(Option::unwrap_or_default)
}
fn write_channels(dir: &std::path::Path, all: &HashMap<String, Saved>) -> Result<(), String> {
    let file = dir.join("channels.json");
    airband_core::kept::save(&file, all).map_err(|e| format!("cannot save {}: {e}", file.display()))
}

pub(crate) struct Pg {
    host: String,
    port: u16,
    user: String,
    pass: String,
    name: String,
    tls: Option<Arc<rustls::ClientConfig>>,
}

use super::var;
mod dynamo_store;
pub(crate) use dynamo_store::Dyn;

impl Store {
    /// From the environment. Fails at once, with the reason, on a half-configured database.
    pub(crate) fn from_env() -> Result<Store, String> {
        if let Some(prefix) = var("AIRBAND_RELAY_DYNAMO") {
            let region = var("AWS_REGION").or_else(|| var("AWS_DEFAULT_REGION")).ok_or("AIRBAND_RELAY_DYNAMO is set, so set AWS_REGION to the region its tables are in")?;
            return Ok(Store::Dynamo(Dyn { db: Dynamo::new(region), channels: format!("{prefix}-channels"), held: format!("{prefix}-held"), waitlist: format!("{prefix}-waitlist") }));
        }
        let Some(host) = var("DB_HOST") else {
            let dir = PathBuf::from(var("AIRBAND_RELAY_DATA").unwrap_or_else(|| "relay-data".into()));
            std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            return Ok(Store::Folder(dir));
        };
        let tls = if var("DB_SSL").as_deref() == Some("off") {
            None
        } else {
            let ca = var("DB_CA").ok_or("DB_HOST is set, so set DB_CA to a PEM file of authorities to trust, or DB_SSL=off for a local database")?;
            let pem = std::fs::read(&ca).map_err(|e| format!("cannot read DB_CA {ca}: {e}"))?;
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut pem.as_slice()) {
                roots.add(cert.map_err(|e| format!("DB_CA {ca} is not PEM: {e}"))?).map_err(|e| format!("DB_CA {ca}: {e}"))?;
            }
            if roots.is_empty() {
                return Err(format!("DB_CA {ca} holds no certificates"));
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(|e| e.to_string())?.with_root_certificates(roots).with_no_client_auth();
            Some(Arc::new(config))
        };
        Ok(Store::Postgres(Pg {
            host,
            port: var("DB_PORT").and_then(|p| p.parse().ok()).unwrap_or(5432),
            user: var("DB_USER").ok_or("DB_HOST is set but DB_USER is not")?,
            pass: var("DB_PASS").ok_or("DB_HOST is set but DB_PASS is not")?,
            name: var("DB_NAME").unwrap_or_else(|| "airband".into()),
            tls,
        }))
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Store::Folder(dir) => format!("the folder {}", dir.display()),
            Store::Postgres(pg) => format!("the database {} on {}", pg.name, pg.host),
            Store::Dynamo(d) => format!("the DynamoDB table {} in {}", d.channels, d.db.region()),
        }
    }

    /// Every channel and what is remembered about it. For Postgres, also makes the database and table if they are missing.
    pub(crate) async fn load(&self) -> Result<HashMap<String, Saved>, String> {
        match self {
            Store::Folder(dir) => read_channels(dir),
            Store::Dynamo(d) => {
                let items = d.db.all("Scan", json!({ "TableName": d.channels, "ConsistentRead": true })).await?;
                Ok(items
                    .iter()
                    .filter_map(|item| match (dynamo::text(item, "channel"), dynamo::text(item, "pull_hash"), dynamo::text(item, "radio_hash")) {
                        (Some(channel), Some(pull_hash), Some(radio_hash)) => Some((channel, Saved { pull_hash, radio_hash })),
                        _ => None,
                    })
                    .collect())
            }
            Store::Postgres(pg) => {
                // a database cannot be created from inside itself: ask the server's own `postgres` database
                let admin = pg.connect("postgres").await?;
                let exists = admin.query_opt("select 1 from pg_database where datname = $1", &[&pg.name]).await.map_err(|e| e.to_string())?.is_some();
                if !exists {
                    if !pg.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                        return Err(format!("DB_NAME \"{}\" must be lowercase letters, digits and underscores", pg.name));
                    }
                    admin.batch_execute(&format!("create database {}", pg.name)).await.map_err(|e| format!("cannot create database {}: {e}", pg.name))?;
                }
                let db = pg.connect(&pg.name).await?;
                db.batch_execute(
                    "create table if not exists relay_channels (channel text primary key, pull_hash text not null, radio_hash text not null, created_at timestamptz not null default now());
                     create table if not exists waitlist (email text primary key, joined_at timestamptz not null default now());
                     create table if not exists relay_held (seq bigserial primary key, channel text not null, id text not null, sealed bytea not null, held_at timestamptz not null default now(), unique (channel, id))",
                )
                .await
                .map_err(|e| e.to_string())?;
                let rows = db.query("select channel, pull_hash, radio_hash from relay_channels", &[]).await.map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| (r.get(0), Saved { pull_hash: r.get(1), radio_hash: r.get(2) })).collect())
            }
        }
    }

    /// Put an address on the waiting list. Returns only once it is kept; an address already there is left as it is.
    pub(crate) async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let _one = LISTS.lock().unwrap();
                let file = dir.join("waitlist.txt");
                // not there yet is an empty list; there and unreadable is an error, never written over as empty
                let have = match std::fs::read_to_string(&file) {
                    Ok(have) => have,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
                };
                if have.lines().any(|l| l == email) {
                    return Ok(());
                }
                airband_core::kept::write_private(&file, format!("{have}{email}\n")).map_err(|e| format!("cannot save {}: {e}", file.display()))
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("insert into waitlist (email) values ($1) on conflict (email) do nothing", &[&email]).await.map(|_| ()).map_err(|e| e.to_string())
            }
            Store::Dynamo(d) => d.put_new(&d.waitlist, "email", json!({ "email": s(email), "joined_at": n(airband_core::clock::now_secs()) })).await,
        }
    }

    /// Forget a channel its tower has closed.
    pub(crate) async fn remove(&self, channel: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let _one = LISTS.lock().unwrap();
                let mut all = read_channels(dir)?;
                all.remove(channel);
                write_channels(dir, &all)
            }
            Store::Dynamo(d) => d.db.call("DeleteItem", json!({ "TableName": d.channels, "Key": { "channel": s(channel) } })).await.map(|_| ()).map_err(|e| e.to_string()),
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_channels where channel = $1", &[&channel]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Remember a new channel, with its radio's token hash. Returns only once it is durable. A channel
    /// already there is left as it is.
    pub(crate) async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let _one = LISTS.lock().unwrap();
                let mut all = read_channels(dir)?;
                all.entry(channel.into()).or_insert(Saved { pull_hash: pull_hash.into(), radio_hash: radio_hash.into() });
                write_channels(dir, &all)
            }
            Store::Dynamo(d) => {
                d.put_new(&d.channels, "channel", json!({ "channel": s(channel), "pull_hash": s(pull_hash), "radio_hash": s(radio_hash), "created_at": n(airband_core::clock::now_secs()) })).await
            }
            Store::Postgres(pg) => {
                // one short connection per new channel: channels are opened rarely, and nothing is held open to go stale
                let db = pg.connect(&pg.name).await?;
                db.execute("insert into relay_channels (channel, pull_hash, radio_hash) values ($1, $2, $3) on conflict (channel) do nothing", &[&channel, &pull_hash, &radio_hash]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

}

/// How long a kept message waits for its tower, in seconds, in every store.
const HELD_FOR_SECS: u64 = airband_core::relay::HELD_FOR_MS / 1000;

/// A time in milliseconds that only goes up, so two messages kept in the same millisecond still have an order.
fn next_ms() -> u64 {
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let now = airband_core::clock::now_ms();
    let mut last = LAST.lock().unwrap();
    *last = now.max(*last + 1);
    *last
}
/// What a channel's folder keeps, in the order it came: each by the number it is kept under and the id the
/// radio gave it. Written whole (`kept`), so what is kept is read from here and never worked out from the names
/// of the files, and a half-written file in the folder counts for nothing.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct HeldList {
    items: Vec<HeldItem>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct HeldItem {
    seq: u64,
    id: String,
    /// When it was kept, in seconds since 1970: what a tower never came back for is dropped after a while.
    at: u64,
}

/// One change to a folder's list at a time: two requests for one channel must not each read it and write it.
static HELD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Sealed requests kept for a tower that is away, to hand over in the order they came once it is back. The relay
/// cannot read them. Each has an id of the radio's choosing, so one sent twice is kept once.
impl Store {
    fn held_dir(dir: &std::path::Path, channel: &str) -> PathBuf {
        dir.join("held").join(channel)
    }
    fn held_list(held: &std::path::Path) -> Result<HeldList, String> {
        airband_core::kept::read(&held.join("list.json")).map(Option::unwrap_or_default)
    }
    fn held_keep(held: &std::path::Path, list: &HeldList) -> Result<(), String> {
        airband_core::kept::save(&held.join("list.json"), list).map_err(|e| e.to_string())
    }

    /// Keep one. False when the channel already has as many as it may; true when it is kept, now or before.
    pub(crate) async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String> {
        match self {
            Store::Folder(dir) => {
                let _one = HELD.lock().unwrap();
                let held = Self::held_dir(dir, channel);
                std::fs::create_dir_all(&held).map_err(|e| e.to_string())?;
                let mut list = Self::held_list(&held)?;
                if list.items.iter().any(|h| h.id == id) {
                    return Ok(true);
                }
                let now = airband_core::clock::now_secs();
                if list.items.iter().filter(|h| now.saturating_sub(h.at) <= HELD_FOR_SECS).count() >= most {
                    return Ok(false);
                }
                // in the order they came: a number that only goes up
                let seq = list.items.iter().map(|h| h.seq).max().map_or(1, |n| n + 1);
                airband_core::kept::write_atomic(&held.join(seq.to_string()), sealed).map_err(|e| e.to_string())?;
                list.items.push(HeldItem { seq, id: id.to_string(), at: airband_core::clock::now_secs() });
                Self::held_keep(&held, &list).map(|_| true)
            }
            Store::Dynamo(d) => d.hold(channel, id, sealed, most).await,
            Store::Postgres(pg) => {
                // counted and kept in one transaction, under a lock on the channel held until it ends, so two at
                // once cannot both see room for one more and both be kept
                let mut db = pg.connect(&pg.name).await?;
                let tx = db.transaction().await.map_err(|e| e.to_string())?;
                tx.execute("select pg_advisory_xact_lock(hashtext($1))", &[&channel]).await.map_err(|e| e.to_string())?;
                let already = tx.query_opt("select 1 from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map_err(|e| e.to_string())?.is_some();
                if already {
                    return Ok(true);
                }
                let have: i64 = tx.query_one("select count(*) from relay_held where channel = $1 and held_at > now() - make_interval(secs => $2)", &[&channel, &(HELD_FOR_SECS as f64)]).await.map_err(|e| e.to_string())?.get(0);
                if have as usize >= most {
                    return Ok(false);
                }
                tx.execute("insert into relay_held (channel, id, sealed) values ($1, $2, $3) on conflict (channel, id) do nothing", &[&channel, &id, &sealed]).await.map_err(|e| e.to_string())?;
                tx.commit().await.map(|_| true).map_err(|e| e.to_string())
            }
        }
    }

    /// The oldest one kept for a channel, if any.
    pub(crate) async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        match self {
            Store::Folder(dir) => {
                let _one = HELD.lock().unwrap();
                let held = Self::held_dir(dir, channel);
                let mut list = Self::held_list(&held)?;
                list.items.sort_by_key(|h| h.seq);
                let now = airband_core::clock::now_secs();
                // what is past its time is passed over, as every store does; an entry whose message is not there
                // (its file was lost) is dropped from the list and said, so the channel does not stop at it for ever
                let mut lost = Vec::new();
                let mut found = None;
                for item in list.items.iter().filter(|h| now.saturating_sub(h.at) <= HELD_FOR_SECS) {
                    match std::fs::read(held.join(item.seq.to_string())) {
                        Ok(sealed) => {
                            found = Some((item.id.clone(), sealed));
                            break;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => lost.push(item.seq),
                        Err(e) => return Err(e.to_string()),
                    }
                }
                if !lost.is_empty() {
                    eprintln!("airband-relay: {} kept message(s) on a channel had no body and were dropped from its list", lost.len());
                    list.items.retain(|h| !lost.contains(&h.seq));
                    Self::held_keep(&held, &list)?;
                }
                Ok(found)
            }
            Store::Dynamo(d) => d.held_next(channel).await,
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                let row = db.query_opt("select id, sealed from relay_held where channel = $1 and held_at > now() - make_interval(secs => $2) order by seq limit 1", &[&channel, &(HELD_FOR_SECS as f64)]).await.map_err(|e| e.to_string())?;
                Ok(row.map(|r| (r.get(0), r.get(1))))
            }
        }
    }

    /// The tower has taken one: it is kept no longer.
    pub(crate) async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let _one = HELD.lock().unwrap();
                let held = Self::held_dir(dir, channel);
                let mut list = Self::held_list(&held)?;
                let gone: Vec<u64> = list.items.iter().filter(|h| h.id == id).map(|h| h.seq).collect();
                list.items.retain(|h| h.id != id);
                Self::held_keep(&held, &list)?;
                gone.iter().for_each(|seq| { let _ = std::fs::remove_file(held.join(seq.to_string())); });
                Ok(())
            }
            Store::Dynamo(d) => d.held_done(channel, id).await,
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Nothing is kept for ever: what a tower never came back for is dropped once it is past
    /// [`HELD_FOR_SECS`]. Until then every store passes over it (`held_next`, and the count in `hold`), so the stores
    /// agree on what is kept whenever this last ran.
    pub(crate) async fn held_purge(&self) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let _one = HELD.lock().unwrap();
                let Ok(channels) = std::fs::read_dir(dir.join("held")) else { return Ok(()) };
                let now = airband_core::clock::now_secs();
                for held in channels.flatten().map(|c| c.path()) {
                    let Ok(mut list) = Self::held_list(&held) else { continue };
                    let old: Vec<u64> = list.items.iter().filter(|h| now.saturating_sub(h.at) > HELD_FOR_SECS).map(|h| h.seq).collect();
                    if old.is_empty() {
                        continue;
                    }
                    list.items.retain(|h| !old.contains(&h.seq));
                    Self::held_keep(&held, &list)?;
                    old.iter().for_each(|seq| { let _ = std::fs::remove_file(held.join(seq.to_string())); });
                }
                Ok(())
            }
            // DynamoDB drops what is past its time by itself
            Store::Dynamo(_) => Ok(()),
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_held where held_at < now() - make_interval(secs => $1)", &[&(HELD_FOR_SECS as f64)]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }
}

impl Pg {
    async fn connect(&self, database: &str) -> Result<Client, String> {
        let mut cfg = Config::new();
        cfg.host(&self.host).port(self.port).user(&self.user).password(&self.pass).dbname(database).connect_timeout(std::time::Duration::from_secs(15));
        let why = |e: tokio_postgres::Error| format!("cannot connect to database {database} on {}: {e}", self.host);
        match &self.tls {
            Some(tls) => {
                let (client, conn) = cfg.connect(tokio_postgres_rustls::MakeRustlsConnect::new((**tls).clone())).await.map_err(why)?;
                tokio::spawn(conn);
                Ok(client)
            }
            None => {
                let (client, conn) = cfg.connect(NoTls).await.map_err(why)?;
                tokio::spawn(conn);
                Ok(client)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_folder_keeps_each_id_once_in_order_by_its_list_and_no_more_than_a_channel_may_have() {
        let dir = std::env::temp_dir().join(format!("airband-relay-held-{}", airband_core::encode::random_hex(6)));
        let store = Store::Folder(dir.clone());
        assert_eq!(store.hold("c", "a-1", b"one", 2).await, Ok(true));
        // the same id again is kept once; an id with a dash in it is still found by its record
        assert_eq!(store.hold("c", "a-1", b"one", 2).await, Ok(true));
        assert_eq!(store.hold("c", "b", b"two", 2).await, Ok(true));
        assert_eq!(store.hold("c", "c", b"three", 2).await, Ok(false));
        // a stray file in the folder (a write that never finished) is not something kept
        std::fs::write(Store::held_dir(&dir, "c").join(".tmp-half"), b"x").unwrap();
        assert_eq!(store.held_next("c").await, Ok(Some(("a-1".into(), b"one".to_vec()))));
        store.held_done("c", "a-1").await.unwrap();
        assert_eq!(store.held_next("c").await, Ok(Some(("b".into(), b"two".to_vec()))));
        assert_eq!(store.hold("c", "c", b"three", 2).await, Ok(true));
        store.held_done("c", "b").await.unwrap();
        store.held_done("c", "c").await.unwrap();
        assert_eq!(store.held_next("c").await, Ok(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_lost_body_or_an_old_message_never_stops_a_channel_and_an_unreadable_waitlist_is_kept() {
        let dir = std::env::temp_dir().join(format!("airband-relay-lost-{}", airband_core::encode::random_hex(6)));
        let store = Store::Folder(dir.clone());
        assert_eq!(store.hold("c", "a", b"one", 5).await, Ok(true));
        assert_eq!(store.hold("c", "b", b"two", 5).await, Ok(true));
        // the first one's body is gone: the next pull goes on to the second, and the list forgets the first
        let held = Store::held_dir(&dir, "c");
        let first = Store::held_list(&held).unwrap().items[0].seq;
        std::fs::remove_file(held.join(first.to_string())).unwrap();
        assert_eq!(store.held_next("c").await, Ok(Some(("b".into(), b"two".to_vec()))));
        assert_eq!(Store::held_list(&held).unwrap().items.len(), 1);
        // one past its time is passed over and does not count toward the most a channel may keep
        let mut list = Store::held_list(&held).unwrap();
        list.items[0].at = 1;
        Store::held_keep(&held, &list).unwrap();
        assert_eq!(store.held_next("c").await, Ok(None));
        assert_eq!(store.hold("c", "d", b"four", 1).await, Ok(true));
        // the waiting list: one that cannot be read is an error and stays as it was
        store.join_waitlist("a@example.com").await.unwrap();
        let waitlist = dir.join("waitlist.txt");
        std::fs::write(&waitlist, [0xff, 0xfe]).unwrap();
        assert!(store.join_waitlist("b@example.com").await.is_err());
        assert_eq!(std::fs::read(&waitlist).unwrap(), vec![0xff, 0xfe]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn two_channels_opened_at_once_are_both_kept() {
        let dir = std::env::temp_dir().join(format!("airband-relay-race-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(Store::Folder(dir.clone()));
        let opens: Vec<_> = (0..16).map(|i| {
            let store = store.clone();
            std::thread::spawn(move || futures_lite_block(store.add(&format!("c{i}"), "p", "r")))
        }).collect();
        opens.into_iter().for_each(|t| t.join().unwrap().unwrap());
        assert_eq!(store.load().await.map(|c| c.len()), Ok(16));
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Run one future to its end on this thread: each test thread opens its channel truly at the same time.
    fn futures_lite_block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
    }

    #[tokio::test]
    async fn an_unreadable_channels_file_is_refused_and_never_written_over() {
        let dir = std::env::temp_dir().join(format!("airband-relay-store-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::Folder(dir.clone());
        // not there yet: no channels, and one can be added
        assert_eq!(store.load().await.map(|c| c.len()), Ok(0));
        store.add("c1", "p", "r").await.unwrap();
        assert_eq!(store.load().await.map(|c| c.len()), Ok(1));
        // there but broken: loading says so, and neither adding nor removing writes over it
        std::fs::write(dir.join("channels.json"), b"{not json").unwrap();
        assert!(store.load().await.is_err());
        assert!(store.add("c2", "p", "r").await.is_err());
        assert!(store.remove("c1").await.is_err());
        assert_eq!(std::fs::read(dir.join("channels.json")).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against real tables, which is the only place DynamoDB's own rules can be seen to hold:
    ///   AIRBAND_TEST_DYNAMO=<prefix> AWS_REGION=<region> cargo test -p airband-relay -- --ignored dynamo
    /// with AWS_ACCESS_KEY_ID and friends in the environment. It works on a channel of its own and leaves nothing.
    #[tokio::test]
    #[ignore]
    async fn dynamo_keeps_each_id_once_in_order_and_no_more_than_a_channel_may_have() {
        let prefix = std::env::var("AIRBAND_TEST_DYNAMO").expect("set AIRBAND_TEST_DYNAMO to the tables' prefix");
        let store = Store::Dynamo(Dyn { db: Dynamo::new(std::env::var("AWS_REGION").expect("set AWS_REGION")), channels: format!("{prefix}-channels"), held: format!("{prefix}-held"), waitlist: format!("{prefix}-waitlist") });
        let Store::Dynamo(d) = &store else { unreachable!() };
        let channel = airband_core::encode::random_hex(16);
        let channel = channel.as_str();
        let count = || async { dynamo::number(&d.db.call("GetItem", json!({ "TableName": d.held, "Key": Dyn::key(channel, "n"), "ConsistentRead": true })).await.unwrap()["Item"], "held").unwrap_or(0) };
        let set = move |k: String, name: &'static str, to: u64| async move {
            d.db.call("UpdateItem", json!({ "TableName": d.held, "Key": Dyn::key(channel, &k), "UpdateExpression": "SET #a = :n", "ExpressionAttributeNames": { "#a": name }, "ExpressionAttributeValues": { ":n": n(to) } })).await.unwrap();
        };

        // the same id five times at once is kept once
        let once = || store.hold(channel, "aaaaaaaa", b"first", 10);
        let all = tokio::join!(once(), once(), once(), once(), once());
        assert_eq!(all, (Ok(true), Ok(true), Ok(true), Ok(true), Ok(true)));
        assert_eq!((d.line(channel).await.unwrap().len(), count().await), (1, 1));
        // no more than the channel may have, and a refusal changes nothing
        assert_eq!(store.hold(channel, "bbbbbbbb", b"second", 2).await, Ok(true));
        assert_eq!(store.hold(channel, "cccccccc", b"third", 2).await, Ok(false));
        assert_eq!(store.hold(channel, "aaaaaaaa", b"first", 2).await, Ok(true)); // one already kept is still kept
        assert_eq!((d.line(channel).await.unwrap().len(), count().await), (2, 2));
        // handed over oldest first; taking one makes room
        assert_eq!(store.held_next(channel).await, Ok(Some(("aaaaaaaa".into(), b"first".to_vec()))));
        store.held_done(channel, "aaaaaaaa").await.unwrap();
        store.held_done(channel, "aaaaaaaa").await.unwrap(); // twice is once
        assert_eq!(count().await, 1);
        assert_eq!(store.hold(channel, "cccccccc", b"third", 2).await, Ok(true));
        // a count that ran high (DynamoDB removed old ones by itself) is counted again when the channel looks full
        set("n".into(), "held", 9).await;
        assert_eq!(store.hold(channel, "dddddddd", b"fourth", 3).await, Ok(true));
        assert_eq!(store.hold(channel, "eeeeeeee", b"fifth", 3).await, Ok(false));
        assert_eq!(count().await, 3);
        // one past its time is passed over, though DynamoDB has not removed it yet
        set(d.line(channel).await.unwrap()[0].0.clone(), "expires", 1).await;
        assert_eq!(store.held_next(channel).await, Ok(Some(("cccccccc".into(), b"third".to_vec()))));
        store.held_done(channel, "cccccccc").await.unwrap();
        assert_eq!(store.held_next(channel).await, Ok(Some(("dddddddd".into(), b"fourth".to_vec()))));
        store.held_done(channel, "dddddddd").await.unwrap();
        assert_eq!(store.held_next(channel).await, Ok(None));
        // a channel's record: made once, read back, removed
        store.add(channel, "pull", "radio").await.unwrap();
        store.add(channel, "someone else's", "x").await.unwrap();
        assert_eq!(store.load().await.unwrap()[channel], Saved { pull_hash: "pull".into(), radio_hash: "radio".into() });
        store.remove(channel).await.unwrap();
        assert!(!store.load().await.unwrap().contains_key(channel));
        // leave nothing: the one past its time goes when the line is counted
        d.recount(channel).await.unwrap();
        assert!(d.line(channel).await.unwrap().is_empty());
        d.db.call("DeleteItem", json!({ "TableName": d.held, "Key": Dyn::key(channel, "n") })).await.unwrap();
    }
}
