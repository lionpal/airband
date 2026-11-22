//! Where the relay remembers which channels exist: the channel's name, the hash of its tower's pull token, the
//! hash of its radio's token. With the sealed messages kept for
//! a tower that is away, that is all it ever stores about anyone's traffic. It also keeps the homepage's waiting
//! list: email addresses people gave to be told when the app is ready, and nothing else about them. A folder
//! when run on a machine with a disk (folder.rs); a Postgres database (postgres.rs) or Amazon DynamoDB
//! (dynamo_store.rs) when run somewhere without one. This file chooses one and hands each job to it.
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
//!   DB_HOST, DB_PORT (5432 when not set), DB_USER, DB_PASS, DB_NAME (airband)
//!   DB_CA    a PEM file of certificate authorities to trust for the connection (for Amazon RDS, its global bundle)
//!   DB_SSL   `off` for a plain local database
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use crate::dynamo::{self, Dynamo, n, s};
use serde_json::{Value, json};
use super::var;

mod dynamo_store;
mod folder;
mod postgres;
pub(crate) use dynamo_store::Dyn;
pub(crate) use postgres::Pg;

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

/// How long a kept message waits for its tower, in seconds, in every store.
const HELD_FOR_SECS: u64 = airband_core::relay::HELD_FOR_MS / 1000;

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
        Ok(Store::Postgres(Pg::from_env(host)?))
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
            Store::Folder(dir) => folder::load(dir),
            Store::Postgres(pg) => pg.load().await,
            Store::Dynamo(d) => d.load().await,
        }
    }

    /// Put an address on the waiting list. Returns only once it is kept; an address already there is left as it is.
    pub(crate) async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => folder::join_waitlist(dir, email),
            Store::Postgres(pg) => pg.join_waitlist(email).await,
            Store::Dynamo(d) => d.join_waitlist(email).await,
        }
    }

    /// Forget a channel its tower has closed.
    pub(crate) async fn remove(&self, channel: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => folder::remove(dir, channel),
            Store::Postgres(pg) => pg.remove(channel).await,
            Store::Dynamo(d) => d.remove(channel).await,
        }
    }

    /// Remember a new channel, with its radio's token hash. Returns only once it is durable. A channel
    /// already there is left as it is.
    pub(crate) async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => folder::add(dir, channel, pull_hash, radio_hash),
            Store::Postgres(pg) => pg.add(channel, pull_hash, radio_hash).await,
            Store::Dynamo(d) => d.add(channel, pull_hash, radio_hash).await,
        }
    }
}

/// Sealed requests kept for a tower that is away, to hand over in the order they came once it is back. The relay
/// cannot read them. Each has an id of the radio's choosing, so one sent twice is kept once.
impl Store {
    /// Keep one. False when the channel already has as many as it may; true when it is kept, now or before.
    pub(crate) async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String> {
        match self {
            Store::Folder(dir) => folder::hold(dir, channel, id, sealed, most),
            Store::Postgres(pg) => pg.hold(channel, id, sealed, most).await,
            Store::Dynamo(d) => d.hold(channel, id, sealed, most).await,
        }
    }

    /// The oldest one kept for a channel, if any.
    pub(crate) async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        match self {
            Store::Folder(dir) => folder::held_next(dir, channel),
            Store::Postgres(pg) => pg.held_next(channel).await,
            Store::Dynamo(d) => d.held_next(channel).await,
        }
    }

    /// The tower has taken one: it is kept no longer.
    pub(crate) async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => folder::held_done(dir, channel, id),
            Store::Postgres(pg) => pg.held_done(channel, id).await,
            Store::Dynamo(d) => d.held_done(channel, id).await,
        }
    }

    /// Nothing is kept for ever: what a tower never came back for is dropped once it is past
    /// [`HELD_FOR_SECS`]. Until then every store passes over it (`held_next`, and the count in `hold`), so the stores
    /// agree on what is kept whenever this last ran.
    pub(crate) async fn held_purge(&self) -> Result<(), String> {
        match self {
            Store::Folder(dir) => folder::held_purge(dir),
            Store::Postgres(pg) => pg.held_purge().await,
            // DynamoDB drops what is past its time by itself
            Store::Dynamo(_) => Ok(()),
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
        std::fs::write(folder::held_dir(&dir, "c").join(".tmp-half"), b"x").unwrap();
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
        let held = folder::held_dir(&dir, "c");
        let first = folder::held_list(&held).unwrap().items[0].seq;
        std::fs::remove_file(held.join(first.to_string())).unwrap();
        assert_eq!(store.held_next("c").await, Ok(Some(("b".into(), b"two".to_vec()))));
        assert_eq!(folder::held_list(&held).unwrap().items.len(), 1);
        // one past its time is passed over and does not count toward the most a channel may keep
        let mut list = folder::held_list(&held).unwrap();
        list.items[0].at = 1;
        folder::held_keep(&held, &list).unwrap();
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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_message_found_is_handed_over_even_when_its_list_cannot_be_tidied() {
        // N19: the list cannot be written just now; the message behind a lost one is still handed over
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-relay-tidy-{}", airband_core::encode::random_hex(6)));
        let store = Store::Folder(dir.clone());
        assert_eq!(store.hold("c", "aaaaaaaa01", b"one", 10).await, Ok(true));
        assert_eq!(store.hold("c", "aaaaaaaa02", b"two", 10).await, Ok(true));
        let held = folder::held_dir(&dir, "c");
        let first = folder::held_list(&held).unwrap().items[0].seq;
        std::fs::remove_file(held.join(first.to_string())).unwrap();
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o500)).unwrap();
        let got = store.held_next("c").await;
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(got, Ok(Some(("aaaaaaaa02".to_string(), b"two".to_vec()))));
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
