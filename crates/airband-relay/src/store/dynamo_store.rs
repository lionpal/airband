//! The relay's records kept in DynamoDB: the same jobs the folder and Postgres stores do, in its tables.
use super::*;
use crate::dynamo::b;


pub(crate) struct Dyn {
    pub(super) db: Dynamo,
    pub(super) channels: String,
    pub(super) held: String,
    pub(super) waitlist: String,
}


impl Dyn {
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
    /// A channel's line: the place of every message kept for it, oldest first, as (key, id, expires).
    pub(super) async fn line(&self, channel: &str) -> Result<Vec<(String, String, u64)>, String> {
        let (mut all, mut from) = (Vec::new(), None::<Value>);
        loop {
            let (items, next) = self.db.page("Query", json!({ "TableName": self.held, "ConsistentRead": true, "KeyConditionExpression": "#c = :c AND begins_with(#k, :m)", "ExpressionAttributeNames": { "#c": "channel", "#k": "k" }, "ExpressionAttributeValues": { ":c": s(channel), ":m": s("m#") } }), from.take()).await?;
            for item in &items {
                if let (Some(k), Some(id)) = (dynamo::text(item, "k"), dynamo::text(item, "id")) {
                    all.push((k, id, dynamo::number(item, "expires").unwrap_or(0)));
                }
            }
            match next {
                Some(next) => from = Some(next),
                None => return Ok(all),
            }
        }
    }
    /// Take a message out of the table without touching the count: for one that has outlived its time.
    pub(super) async fn drop_stale(&self, channel: &str, place: &str, id: &str) {
        let _ = self.db.call("DeleteItem", json!({ "TableName": self.held, "Key": Self::key(channel, place) })).await;
        let _ = self.db.call("DeleteItem", json!({ "TableName": self.held, "Key": Self::key(channel, &format!("i#{id}")), "ConditionExpression": "#at = :at", "ExpressionAttributeNames": { "#at": "at" }, "ExpressionAttributeValues": { ":at": s(place) } })).await;
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
    pub(super) async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String> {
        for attempt in 0..2 {
            let (now, place) = (airband_core::clock::now_secs(), format!("m#{:013}#{id}", next_ms()));
            let expires = n(now + airband_core::relay::HELD_FOR_MS / 1000);
            let ask = json!({ "TransactItems": [
                // kept once: only if nothing is there under this id, or what is there has outlived its time
                { "Put": { "TableName": self.held, "Item": { "channel": s(channel), "k": s(&format!("i#{id}")), "at": s(&place), "sealed": b(sealed), "expires": expires },
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
    pub(super) async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let mut from = None::<Value>;
        loop {
            let (items, next) = self.db.page("Query", json!({ "TableName": self.held, "ConsistentRead": true, "Limit": 20, "KeyConditionExpression": "#c = :c AND begins_with(#k, :m)", "ExpressionAttributeNames": { "#c": "channel", "#k": "k" }, "ExpressionAttributeValues": { ":c": s(channel), ":m": s("m#") } }), from.take()).await?;
            let now = airband_core::clock::now_secs();
            for item in &items {
                let (Some(place), Some(id)) = (dynamo::text(item, "k"), dynamo::text(item, "id")) else { continue };
                // past its time and not yet removed: as good as gone
                if dynamo::number(item, "expires").unwrap_or(0) <= now {
                    continue;
                }
                let kept = self.db.call("GetItem", json!({ "TableName": self.held, "Key": Self::key(channel, &format!("i#{id}")), "ConsistentRead": true })).await.map_err(|e| e.to_string())?;
                match dynamo::bytes(&kept["Item"], "sealed").filter(|_| dynamo::text(&kept["Item"], "at").as_deref() == Some(place.as_str())) {
                    Some(sealed) => return Ok(Some((id, sealed))),
                    // a place in line with no message behind it: taken already, or half removed
                    None => self.drop_stale(channel, &place, &id).await,
                }
            }
            match next {
                Some(next) => from = Some(next),
                None => return Ok(None),
            }
        }
    }
    pub(super) async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        let kept = self.db.call("GetItem", json!({ "TableName": self.held, "Key": Self::key(channel, &format!("i#{id}")), "ConsistentRead": true, "ProjectionExpression": "#at", "ExpressionAttributeNames": { "#at": "at" } })).await.map_err(|e| e.to_string())?;
        let Some(place) = dynamo::text(&kept["Item"], "at") else { return Ok(()) };
        let ask = json!({ "TransactItems": [
            { "Delete": { "TableName": self.held, "Key": Self::key(channel, &format!("i#{id}")), "ConditionExpression": "#at = :at", "ExpressionAttributeNames": { "#at": "at" }, "ExpressionAttributeValues": { ":at": s(&place) } } },
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
