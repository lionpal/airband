//! Amazon DynamoDB, spoken to directly: JSON over HTTPS, each request signed (AWS Signature Version 4) with
//! what the relay already has for Apple's push service (`ring`, `reqwest`). No AWS SDK: this is the whole of it.
//!
//! Who the relay is comes from the machine it runs on: on EC2, the instance's role, asked of the metadata
//! service (IMDSv2) and asked again every quarter of an hour. Nothing is read from a credentials file. For a
//! test on a developer's machine the three standard variables are taken if they are set
//! (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN).
use ring::{digest, hmac};
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) struct Dynamo {
    region: String,
    http: reqwest::Client,
    creds: tokio::sync::Mutex<Option<(Creds, Instant)>>,
}

#[derive(Clone)]
struct Creds {
    key: String,
    secret: String,
    token: Option<String>,
}

/// Why a request failed. `kind` is DynamoDB's own name for it, such as `ConditionalCheckFailedException`.
#[derive(Debug)]
pub(crate) struct Failed {
    pub(crate) kind: String,
    pub(crate) message: String,
    /// For a transaction that was cancelled: what became of each of its parts, in order ("None" for a part
    /// that would have gone through).
    pub(crate) reasons: Vec<String>,
}

impl Failed {
    /// A condition written into the request did not hold. For these requests that is an answer, not a fault.
    pub(crate) fn condition(&self) -> bool {
        self.kind == "ConditionalCheckFailedException"
    }
    /// The transaction's part at `index` is the one whose condition did not hold.
    pub(crate) fn part_refused(&self, index: usize) -> bool {
        self.kind == "TransactionCanceledException" && self.reasons.get(index).is_some_and(|r| r == "ConditionalCheckFailed")
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DynamoDB: {} {}", self.kind, self.message)
    }
}

use super::var;
use airband_core::encode::hex;
fn sha256(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}
fn mac(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data).as_ref().to_vec()
}

/// The key a day's requests to one service in one region are signed with.
fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let day = mac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    mac(&mac(&mac(&day, region.as_bytes()), service.as_bytes()), b"aws4_request")
}

/// The headers that sign one request: `authorization` and those it covers besides `host`.
fn signed(creds: &Creds, region: &str, host: &str, target: &str, body: &[u8], secs: u64) -> Vec<(&'static str, String)> {
    let when = airband_core::clock::compact(secs);
    let date = &when[..8];
    let mut headers = vec![("content-type", "application/x-amz-json-1.0".to_string()), ("host", host.to_string()), ("x-amz-date", when.clone())];
    if let Some(token) = &creds.token {
        headers.push(("x-amz-security-token", token.clone()));
    }
    headers.push(("x-amz-target", target.to_string()));
    let names = headers.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(";");
    let listed: String = headers.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let request = format!("POST\n/\n\n{listed}\n{names}\n{}", sha256(body));
    let scope = format!("{date}/{region}/dynamodb/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{when}\n{scope}\n{}", sha256(request.as_bytes()));
    let signature = hex(&mac(&signing_key(&creds.secret, date, region, "dynamodb"), to_sign.as_bytes()));
    headers.push(("authorization", format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={names}, Signature={signature}", creds.key)));
    headers.retain(|(n, _)| *n != "host");
    headers
}

impl Dynamo {
    pub(crate) fn new(region: String) -> Dynamo {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).connect_timeout(Duration::from_secs(4)).pool_idle_timeout(Duration::from_secs(50)).build().expect("an HTTP client can be made");
        Dynamo { region, http, creds: tokio::sync::Mutex::new(None) }
    }
    pub(crate) fn region(&self) -> &str {
        &self.region
    }

    async fn creds(&self) -> Result<Creds, Failed> {
        let mut have = self.creds.lock().await;
        if let Some((creds, _)) = have.as_ref().filter(|(_, when)| when.elapsed() < Duration::from_secs(900)) {
            return Ok(creds.clone());
        }
        let fresh = match (var("AWS_ACCESS_KEY_ID"), var("AWS_SECRET_ACCESS_KEY")) {
            (Some(key), Some(secret)) => Creds { key, secret, token: var("AWS_SESSION_TOKEN") },
            _ => self.instance_creds().await.map_err(|why| Failed { kind: "NoCredentials".into(), message: format!("this machine's role could not be read from the metadata service: {why}"), reasons: Vec::new() })?,
        };
        *have = Some((fresh.clone(), Instant::now()));
        Ok(fresh)
    }

    /// The instance's role, from the metadata service (IMDSv2: a session token first, then the role's keys).
    async fn instance_creds(&self) -> Result<Creds, String> {
        const AT: &str = "http://169.254.169.254/latest";
        let quick = Duration::from_secs(2);
        let text = |res: Result<reqwest::Response, reqwest::Error>| async move {
            let res = res.map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("status {}", res.status().as_u16()));
            }
            res.text().await.map_err(|e| e.to_string())
        };
        let session = text(self.http.put(format!("{AT}/api/token")).header("x-aws-ec2-metadata-token-ttl-seconds", "300").timeout(quick).send().await).await?;
        let role = text(self.http.get(format!("{AT}/meta-data/iam/security-credentials/")).header("x-aws-ec2-metadata-token", &session).timeout(quick).send().await).await?;
        let role = role.lines().next().unwrap_or("").trim().to_string();
        let keys = text(self.http.get(format!("{AT}/meta-data/iam/security-credentials/{role}")).header("x-aws-ec2-metadata-token", &session).timeout(quick).send().await).await?;
        let keys: Value = serde_json::from_str(&keys).map_err(|e| e.to_string())?;
        let field = |name: &str| keys[name].as_str().map(str::to_string).ok_or(format!("no {name}"));
        Ok(Creds { key: field("AccessKeyId")?, secret: field("SecretAccessKey")?, token: Some(field("Token")?) })
    }

    /// Every item a `Scan` or a `Query` finds, page after page. For a reader that stops at the first it can use,
    /// [`Pages`].
    pub(crate) async fn all(&self, operation: &str, ask: Value) -> Result<Vec<Value>, String> {
        let (mut all, mut pages) = (Vec::new(), Pages::new(operation, ask));
        while let Some(items) = pages.next(self).await? {
            all.extend(items);
        }
        Ok(all)
    }

    /// One DynamoDB operation, such as `PutItem`. Tried again a few times when the service is busy, cannot be
    /// reached, or two transactions touched the same item at once; a refusal that trying again cannot change
    /// comes back at once.
    pub(crate) async fn call(&self, operation: &str, body: Value) -> Result<Value, Failed> {
        let host = format!("dynamodb.{}.amazonaws.com", self.region);
        let bytes = serde_json::to_vec(&body).expect("JSON can be written");
        let mut last = Failed { kind: "Unreachable".into(), message: String::new(), reasons: Vec::new() };
        for attempt in 0..6u32 {
            if attempt > 0 {
                // a little longer each time, and never the same wait as another request that collided with this one
                let apart = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 % 97).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis((40 << attempt) + apart * attempt as u64)).await;
            }
            let creds = self.creds().await?;
            let secs = airband_core::clock::now_secs();
            let mut req = self.http.post(format!("https://{host}/")).body(bytes.clone());
            for (name, value) in signed(&creds, &self.region, &host, &format!("DynamoDB_20120810.{operation}"), &bytes, secs) {
                req = req.header(name, value);
            }
            let res = match req.send().await {
                Ok(res) => res,
                Err(e) => {
                    last.message = e.without_url().to_string();
                    continue;
                }
            };
            let status = res.status().as_u16();
            let answer: Value = res.bytes().await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
            if status == 200 {
                return Ok(answer);
            }
            let kind = answer["__type"].as_str().and_then(|t| t.rsplit('#').next()).unwrap_or("Unknown").to_string();
            let reasons: Vec<String> = answer["CancellationReasons"].as_array().map(|all| all.iter().map(|r| r["Code"].as_str().unwrap_or("None").to_string()).collect()).unwrap_or_default();
            let message = answer["message"].as_str().or(answer["Message"].as_str()).unwrap_or("").to_string();
            let busy = status >= 500 || matches!(kind.as_str(), "ProvisionedThroughputExceededException" | "ThrottlingException" | "RequestLimitExceeded" | "TransactionConflictException" | "TransactionInProgressException") || reasons.iter().any(|r| r == "TransactionConflict" || r == "ThrottlingError");
            // the keys were replaced under us: ask for them again
            if matches!(kind.as_str(), "ExpiredTokenException" | "UnrecognizedClientException") {
                *self.creds.lock().await = None;
            } else if !busy {
                return Err(Failed { kind, message, reasons });
            }
            last = Failed { kind, message, reasons };
        }
        Err(last)
    }
}

/// A `Scan` or a `Query` read a page at a time: the one way the relay pages through DynamoDB.
pub(crate) struct Pages {
    operation: String,
    ask: Value,
    /// Where the next page starts; nothing once the last has been read.
    from: Option<Option<Value>>,
}

impl Pages {
    pub(crate) fn new(operation: &str, ask: Value) -> Pages {
        Pages { operation: operation.into(), ask, from: Some(None) }
    }
    /// The next page's items; nothing once every page has been read.
    pub(crate) async fn next(&mut self, db: &Dynamo) -> Result<Option<Vec<Value>>, String> {
        let Some(from) = self.from.take() else { return Ok(None) };
        let mut ask = self.ask.clone();
        if let Some(from) = from {
            ask["ExclusiveStartKey"] = from;
        }
        let mut page = db.call(&self.operation, ask).await.map_err(|e| e.to_string())?;
        self.from = page.get("LastEvaluatedKey").filter(|k| k.is_object()).cloned().map(Some);
        Ok(Some(match page["Items"].take() {
            Value::Array(items) => items,
            _ => Vec::new(),
        }))
    }
}

/// A text value, a number and bytes, as DynamoDB writes them.
pub(crate) fn s(text: &str) -> Value {
    json!({ "S": text })
}
pub(crate) fn n(number: u64) -> Value {
    json!({ "N": number.to_string() })
}
pub(crate) fn b(bytes: &[u8]) -> Value {
    json!({ "B": airband_core::encode::base64(bytes) })
}
pub(crate) fn text(item: &Value, name: &str) -> Option<String> {
    item[name]["S"].as_str().map(str::to_string)
}
pub(crate) fn number(item: &Value, name: &str) -> Option<u64> {
    item[name]["N"].as_str().and_then(|n| n.parse().ok())
}
pub(crate) fn bytes(item: &Value, name: &str) -> Option<Vec<u8>> {
    item[name]["B"].as_str().and_then(airband_core::encode::unbase64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signing_key_is_the_one_amazon_publishes_for_its_example() {
        // from "Examples of how to derive a signing key for Signature Version 4" in the AWS documentation
        let key = signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215", "us-east-1", "iam");
        assert_eq!(hex(&key), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");
    }
    #[test]
    fn a_request_is_signed_over_its_headers_and_body_and_the_session_token_when_there_is_one() {
        let creds = Creds { key: "AKIDEXAMPLE".into(), secret: "secret".into(), token: None };
        let plain = signed(&creds, "us-east-2", "dynamodb.us-east-2.amazonaws.com", "DynamoDB_20120810.GetItem", b"{}", 1_440_938_160);
        let auth = &plain.iter().find(|(n, _)| *n == "authorization").unwrap().1;
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-2/dynamodb/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-target, Signature="));
        let other = signed(&creds, "us-east-2", "dynamodb.us-east-2.amazonaws.com", "DynamoDB_20120810.GetItem", b"{ }", 1_440_938_160);
        assert_ne!(auth, &other.iter().find(|(n, _)| *n == "authorization").unwrap().1);
        let with = signed(&Creds { token: Some("session".into()), ..creds }, "us-east-2", "dynamodb.us-east-2.amazonaws.com", "DynamoDB_20120810.GetItem", b"{}", 1_440_938_160);
        assert!(with.iter().any(|(n, v)| *n == "x-amz-security-token" && v == "session"));
        assert!(with.iter().find(|(n, _)| *n == "authorization").unwrap().1.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target,"));
    }
}
