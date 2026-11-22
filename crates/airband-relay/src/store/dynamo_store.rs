//! The relay's records kept in DynamoDB: the same jobs the folder and Postgres stores do, in its tables.
use super::*;
use crate::dynamo::{Pages, b};

/// A time in milliseconds that only goes up, so two messages kept in the same millisecond still have an order.
fn next_ms() -> u64 {
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let now = airband_core::clock::now_ms();
    let mut last = LAST.lock().unwrap();
    *last = now.max(*last + 1);
    *last
}

pub(crate) struct Dyn {
    pub(super) db: Dynamo,
    pub(super) channels: String,
    pub(super) held: String,
    pub(super) waitlist: String,
}


impl Dyn {
    /// The three tables named by `prefix`, in `region`.
    pub(crate) fn new(region: String, prefix: &str) -> Dyn {
        Dyn { db: Dynamo::new(region), channels: format!("{prefix}-channels"), held: format!("{prefix}-held"), waitlist: format!("{prefix}-waitlist") }
    }
    /// Write an item unless one with its key is already there, which is left as it is.
    pub(super) async fn put_new(&self, table: &str, key: &str, item: Value) -> Result<(), String> {
        match self.db.call("PutItem", json!({ "TableName": table, "Item": item, "ConditionExpression": "attribute_not_exists(#k)", "ExpressionAttributeNames": { "#k": key } })).await {
            Ok(_) => Ok(()),
            Err(e) if e.condition() => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub(super) fn key(channel: &str, k: &str) -> Value {
        json!({ "channel": s(channel), "k": s(k) })
    }
    /// The key of the item that holds a kept message itself, by the id the radio gave it.
    fn body(id: &str) -> String {
        format!("i#{id}")
    }
    /// Asking for a channel's line: its places, oldest first.
    fn line_ask(&self, channel: &str) -> Value {
        json!({ "TableName": self.held, "ConsistentRead": true, "KeyConditionExpression": "#c = :c AND begins_with(#k, :m)", "ExpressionAttributeNames": { "#c": "channel", "#k": "k" }, "ExpressionAttributeValues": { ":c": s(channel), ":m": s("m#") } })
    }
    /// A channel's line: the place of every message kept for it, oldest first, as (key, id, expires).
    pub(super) async fn line(&self, channel: &str) -> Result<Vec<(String, String, u64)>, String> {
        let items = self.db.all("Query", self.line_ask(channel)).await?;
        // every place is written with its time; one without is not the relay's and is passed over, never read as
        // long past its time and removed
        Ok(items.iter().filter_map(|item| Some((dynamo::text(item, "k")?, dynamo::text(item, "id")?, dynamo::number(item, "expires")?))).collect())
    }
    /// Take a message out of the table without touching the count: for one that has outlived its time. One that
    /// could not be taken out is said; it is passed over all the same, and DynamoDB drops it in its time.
    pub(super) async fn drop_stale(&self, channel: &str, place: &str, id: &str) {
        let line = self.db.call("DeleteItem", json!({ "TableName": self.held, "Key": Self::key(channel, place) })).await;
        let body = self.db.call("DeleteItem", json!({ "TableName": self.held, "Key": Self::key(channel, &Self::body(id)), "ConditionExpression": "#at = :at", "ExpressionAttributeNames": { "#at": "at" }, "ExpressionAttributeValues": { ":at": s(place) } })).await;
        for failed in [line.err(), body.err().filter(|e| !e.condition())].into_iter().flatten() {
            eprintln!("airband-relay: a kept message past its time could not be removed: {failed}");
        }
    }
    /// The count ran high because DynamoDB removed old messages by itself: count the line and put that in its
    /// place, unless the count changed while we looked, in which case someone else's change stands.
    pub(super) async fn recount(&self, channel: &str) -> Result<(), String> {
        let counter = self.db.call("GetItem", json!({ "TableName": self.held, "Key": Self::key(channel, "n"), "ConsistentRead": true })).await.map_err(|e| e.to_string())?;
        let Some(was) = dynamo::number(&counter["Item"], "held") else { return Ok(()) };
        let (line, now) = (self.line(channel).await?, airband_core::clock::now_secs());
        let mut live = 0u64;
        for (place, id, expires) in &line {
            if *expires > now {
                live += 1;
            } else {
                self.drop_stale(channel, place, id).await;
            }
        }
        if live != was {
            let ask = json!({ "TableName": self.held, "Key": Self::key(channel, "n"), "UpdateExpression": "SET #h = :live", "ConditionExpression": "#h = :was", "ExpressionAttributeNames": { "#h": "held" }, "ExpressionAttributeValues": { ":live": n(live), ":was": n(was) } });
            match self.db.call("UpdateItem", ask).await {
                Ok(_) => {}
                Err(e) if e.condition() => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelStore for Dyn {
    fn describe(&self) -> String {
        format!("the DynamoDB table {} in {}", self.channels, self.db.region())
    }
    async fn load(&self) -> Result<HashMap<String, Saved>, String> {
        let items = self.db.all("Scan", json!({ "TableName": self.channels, "ConsistentRead": true })).await?;
        Ok(items
            .iter()
            .filter_map(|item| match (dynamo::text(item, "channel"), dynamo::text(item, "pull_hash"), dynamo::text(item, "radio_hash")) {
                (Some(channel), Some(pull_hash), Some(radio_hash)) => Some((channel, Saved { pull_hash, radio_hash })),
                _ => None,
            })
            .collect())
    }
    async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        self.put_new(&self.waitlist, "email", json!({ "email": s(email), "joined_at": n(airband_core::clock::now_secs()) })).await
    }
    async fn remove(&self, channel: &str) -> Result<(), String> {
        self.db.call("DeleteItem", json!({ "TableName": self.channels, "Key": { "channel": s(channel) } })).await.map_err(|e| e.to_string())?;
        // what was kept for it goes too: its count, its line and the messages themselves
        let ask = json!({ "TableName": self.held, "ConsistentRead": true, "KeyConditionExpression": "#c = :c", "ProjectionExpression": "#c, #k", "ExpressionAttributeNames": { "#c": "channel", "#k": "k" }, "ExpressionAttributeValues": { ":c": s(channel) } });
        for item in self.db.all("Query", ask).await? {
            let Some(k) = dynamo::text(&item, "k") else { continue };
            self.db.call("DeleteItem", json!({ "TableName": self.held, "Key": Self::key(channel, &k) })).await.map_err(|e| format!("the channel is forgotten, but what was kept for it could not all be removed: {e}"))?;
        }
        Ok(())
    }
    async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<(), String> {
        self.put_new(&self.channels, "channel", json!({ "channel": s(channel), "pull_hash": s(pull_hash), "radio_hash": s(radio_hash), "created_at": n(airband_core::clock::now_secs()) })).await
    }
    async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String> {
        for attempt in 0..2 {
            let (now, place) = (airband_core::clock::now_secs(), format!("m#{:013}#{id}", next_ms()));
            let expires = n(now + HELD_FOR_SECS);
            let ask = json!({ "TransactItems": [
                // kept once: only if nothing is there under this id, or what is there has outlived its time
                { "Put": { "TableName": self.held, "Item": { "channel": s(channel), "k": s(&Self::body(id)), "at": s(&place), "sealed": b(sealed), "expires": expires },
                    "ConditionExpression": "attribute_not_exists(#k) OR #e <= :now", "ExpressionAttributeNames": { "#k": "k", "#e": "expires" }, "ExpressionAttributeValues": { ":now": n(now) } } },
                { "Put": { "TableName": self.held, "Item": { "channel": s(channel), "k": s(&place), "id": s(id), "expires": expires } } },
                // and only while the channel has fewer than it may
                { "Update": { "TableName": self.held, "Key": Self::key(channel, "n"), "UpdateExpression": "ADD #h :one",
                    "ConditionExpression": "attribute_not_exists(#h) OR #h < :most", "ExpressionAttributeNames": { "#h": "held" }, "ExpressionAttributeValues": { ":one": n(1), ":most": n(most as u64) } } },
            ] });
            match self.db.call("TransactWriteItems", ask).await {
                Ok(_) => return Ok(true),
                Err(e) if e.part_refused(0) => return Ok(true),
                Err(e) if e.part_refused(2) => {
                    if attempt == 0 {
                        self.recount(channel).await?;
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(false)
    }
    async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let mut ask = self.line_ask(channel);
        ask["Limit"] = 20.into();
        let mut pages = Pages::new("Query", ask);
        while let Some(items) = pages.next(&self.db).await? {
            let now = airband_core::clock::now_secs();
            for item in &items {
                let (Some(place), Some(id), Some(expires)) = (dynamo::text(item, "k"), dynamo::text(item, "id"), dynamo::number(item, "expires")) else { continue };
                // past its time and not yet removed: as good as gone
                if expires <= now {
                    continue;
                }
                let kept = self.db.call("GetItem", json!({ "TableName": self.held, "Key": Self::key(channel, &Self::body(&id)), "ConsistentRead": true })).await.map_err(|e| e.to_string())?;
                match dynamo::bytes(&kept["Item"], "sealed").filter(|_| dynamo::text(&kept["Item"], "at").as_deref() == Some(place.as_str())) {
                    Some(sealed) => return Ok(Some((id, sealed))),
                    // a place in line with no message behind it: taken already, or half removed
                    None => self.drop_stale(channel, &place, &id).await,
                }
            }
        }
        Ok(None)
    }
    async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        let kept = self.db.call("GetItem", json!({ "TableName": self.held, "Key": Self::key(channel, &Self::body(id)), "ConsistentRead": true, "ProjectionExpression": "#at", "ExpressionAttributeNames": { "#at": "at" } })).await.map_err(|e| e.to_string())?;
        let Some(place) = dynamo::text(&kept["Item"], "at") else { return Ok(()) };
        let ask = json!({ "TransactItems": [
            { "Delete": { "TableName": self.held, "Key": Self::key(channel, &Self::body(id)), "ConditionExpression": "#at = :at", "ExpressionAttributeNames": { "#at": "at" }, "ExpressionAttributeValues": { ":at": s(&place) } } },
            { "Delete": { "TableName": self.held, "Key": Self::key(channel, &place) } },
            { "Update": { "TableName": self.held, "Key": Self::key(channel, "n"), "UpdateExpression": "ADD #h :less", "ConditionExpression": "#h > :none", "ExpressionAttributeNames": { "#h": "held" }, "ExpressionAttributeValues": { ":less": { "N": "-1" }, ":none": n(0) } } },
        ] });
        match self.db.call("TransactWriteItems", ask).await {
            Ok(_) => Ok(()),
            // someone took it first
            Err(e) if e.part_refused(0) => Ok(()),
            // the count was already nothing: take the message away all the same
            Err(e) if e.part_refused(2) => {
                self.drop_stale(channel, &place, id).await;
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }
}
