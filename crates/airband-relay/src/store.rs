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
#[cfg(test)]
use std::sync::Arc;
use crate::dynamo::{self, Dynamo, n, s};
use serde_json::{Value, json};
use super::var;

mod dynamo_store;
mod folder;
mod postgres;
pub(crate) use dynamo_store::Dyn;
pub(crate) use folder::Folder;
pub(crate) use postgres::Pg;

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

/// Every job the relay asks of the place it keeps its records. Each store does all of them its own way, in its
/// own file; nothing outside a store knows which one is in use.
#[async_trait::async_trait]
pub(crate) trait ChannelStore: Send + Sync {
    /// Where the records are, for the log.
    fn describe(&self) -> String;
    /// Every channel and what is remembered about it. A store that needs its tables made makes them here.
    async fn load(&self) -> Result<HashMap<String, Saved>, String>;
    /// Put an address on the waiting list. Returns only once it is kept; an address already there is left as it is.
    async fn join_waitlist(&self, email: &str) -> Result<(), String>;
    /// Forget a channel its tower has closed, and everything kept for it.
    async fn remove(&self, channel: &str) -> Result<(), String>;
    /// Remember a new channel, with its radio's token hash. Returns only once it is durable. A channel already
    /// there is left as it is.
    async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<(), String>;

    // Sealed requests kept for a tower that is away, to hand over in the order they came once it is back. The
    // relay cannot read them. Each has an id of the radio's choosing, so one sent twice is kept once.

    /// Keep one. False when the channel already has as many as it may; true when it is kept, now or before.
    async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String>;
    /// The oldest one kept for a channel, if any.
    async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String>;
    /// The tower has taken one: it is kept no longer.
    async fn held_done(&self, channel: &str, id: &str) -> Result<(), String>;
    /// Nothing is kept for ever: what a tower never came back for is dropped once it is past [`HELD_FOR_SECS`].
    /// Until then every store passes over it (`held_next`, and the count in `hold`), so the stores agree on what
    /// is kept whenever this last ran. A store whose database drops old items by itself has nothing to do.
    async fn held_purge(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The store the relay uses.
pub(crate) type Store = Box<dyn ChannelStore>;

/// The store chosen by the environment. Fails at once, with the reason, on a half-configured database.
pub(crate) fn from_env() -> Result<Store, String> {
    if let Some(prefix) = var("AIRBAND_RELAY_DYNAMO") {
        let region = var("AWS_REGION").or_else(|| var("AWS_DEFAULT_REGION")).ok_or("AIRBAND_RELAY_DYNAMO is set, so set AWS_REGION to the region its tables are in")?;
        return Ok(Box::new(Dyn::new(region, &prefix)));
    }
    let Some(host) = var("DB_HOST") else {
        let dir = PathBuf::from(var("AIRBAND_RELAY_DATA").unwrap_or_else(|| "relay-data".into()));
        airband_io::files::make_folder(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        return Ok(folder(dir));
    };
    Ok(Box::new(Pg::from_env(host)?))
}

/// A store in a folder on this machine's disk.
pub(crate) fn folder(dir: PathBuf) -> Store {
    Box::new(Folder { dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What every store must do, the same way. Run against the folder always, and against Postgres and DynamoDB
    /// when a test database is named (see the tests below). It works on a channel of its own and leaves nothing.
    async fn conformance(store: &dyn ChannelStore) {
        let channel = airband_core::encode::random_hex(16);
        let c = channel.as_str();
        // each id kept once, no more than the channel may have, handed over oldest first
        assert_eq!(store.hold(c, "aaaaaaaa", b"one", 2).await, Ok(true));
        assert_eq!(store.hold(c, "aaaaaaaa", b"one", 2).await, Ok(true));
        assert_eq!(store.hold(c, "bbbbbbbb", b"two", 2).await, Ok(true));
        assert_eq!(store.hold(c, "cccccccc", b"three", 2).await, Ok(false));
        assert_eq!(store.held_next(c).await, Ok(Some(("aaaaaaaa".into(), b"one".to_vec()))));
        store.held_done(c, "aaaaaaaa").await.unwrap();
        store.held_done(c, "aaaaaaaa").await.unwrap(); // twice is once
        assert_eq!(store.held_next(c).await, Ok(Some(("bbbbbbbb".into(), b"two".to_vec()))));
        // a channel's record: made once, read back
        store.add(c, "pull", "radio").await.unwrap();
        store.add(c, "someone else's", "x").await.unwrap();
        assert_eq!(store.load().await.unwrap()[c], Saved { pull_hash: "pull".into(), radio_hash: "radio".into() });
        // closed: the record and everything kept for it are gone, and a channel of that name starts empty
        store.remove(c).await.unwrap();
        assert!(!store.load().await.unwrap().contains_key(c));
        assert_eq!(store.held_next(c).await, Ok(None));
        assert_eq!(store.hold(c, "dddddddd", b"four", 1).await, Ok(true));
        store.remove(c).await.unwrap();
        assert_eq!(store.held_next(c).await, Ok(None));
    }

    #[tokio::test]
    async fn the_folder_store_does_every_job_as_every_store_must() {
        let dir = std::env::temp_dir().join(format!("airband-relay-conform-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        conformance(&*folder(dir.clone())).await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against a test Postgres, only when one is named: AIRBAND_TEST_PG=1 with DB_HOST and the rest set to it.
    #[tokio::test]
    async fn the_postgres_store_does_every_job_as_every_store_must() {
        if std::env::var("AIRBAND_TEST_PG").is_err() {
            return;
        }
        let pg = Pg::from_env(std::env::var("DB_HOST").expect("set DB_HOST to the test database")).unwrap();
        pg.load().await.unwrap();
        conformance(&pg).await;
    }

    /// Against test tables, only when they are named: AIRBAND_TEST_DYNAMO=<prefix> AWS_REGION=<region>, with
    /// AWS_ACCESS_KEY_ID and friends in the environment.
    #[tokio::test]
    async fn the_dynamo_store_does_every_job_as_every_store_must() {
        let Ok(prefix) = std::env::var("AIRBAND_TEST_DYNAMO") else { return };
        conformance(&Dyn::new(std::env::var("AWS_REGION").expect("set AWS_REGION"), &prefix)).await;
    }

    #[tokio::test]
    async fn a_list_of_kept_messages_that_cannot_be_read_is_set_aside_and_the_channel_goes_on() {
        let dir = std::env::temp_dir().join(format!("airband-relay-badlist-{}", airband_core::encode::random_hex(6)));
        let store = folder(dir.clone());
        assert_eq!(store.hold("c", "aaaaaaaa", b"one", 5).await, Ok(true));
        let held = folder::held_dir(&dir, "c");
        std::fs::write(held.join("list.json"), b"{not json").unwrap();
        // not refused for ever: what was kept cannot be had, so the channel starts again with nothing kept
        assert_eq!(store.held_next("c").await, Ok(None));
        assert_eq!(store.hold("c", "bbbbbbbb", b"two", 5).await, Ok(true));
        assert_eq!(store.held_next("c").await, Ok(Some(("bbbbbbbb".into(), b"two".to_vec()))));
        // and what could not be read is still there, beside it
        assert!(std::fs::read_dir(&held).unwrap().flatten().any(|f| f.file_name().to_string_lossy().starts_with("list.json.unreadable")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_folder_keeps_each_id_once_in_order_by_its_list_and_no_more_than_a_channel_may_have() {
        let dir = std::env::temp_dir().join(format!("airband-relay-held-{}", airband_core::encode::random_hex(6)));
        let store = folder(dir.clone());
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
        let store = folder(dir.clone());
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
        let store = folder(dir.clone());
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
        let store: Arc<Store> = Arc::new(folder(dir.clone()));
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
        let store = folder(dir.clone());
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
        let d = &Dyn::new(std::env::var("AWS_REGION").expect("set AWS_REGION"), &prefix);
        let store = d;
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
