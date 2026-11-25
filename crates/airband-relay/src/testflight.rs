//! TestFlight invites: an address given on the homepage is added as a beta tester to the app's external group
//! through the App Store Connect API, and Apple emails the invite. The relay holds a key for that API with the
//! App Manager role, made for this alone, read from AWS Secrets Manager the first time an invite is asked for.
//!
//!   AIRBAND_TESTFLIGHT_SECRET   the secret's name in Secrets Manager, in AWS_REGION. Its text is JSON:
//!                               {"keyId": "…", "issuerId": "…", "key": "<the .p8 file's text>"}
//!                               (scripts/testflight-invite-key.sh puts it there). Without this setting, or
//!                               while the secret is not there, addresses are kept and no invite is sent, and
//!                               the page is told so.
//!
//! What it asks of Apple, each answer read into its own type: the app by its bundle id, its group by name (both
//! once, then kept), then `POST /v1/betaTesters` with the address and that group. A 409 is not taken on its word:
//! Apple's own list then says whether the address is already a tester in the group (done before), a tester of
//! the team outside it (added to the group, which sends the invite), or neither (refused, with Apple's codes).
//! Nothing here logs or returns the address or the key.
use crate::dynamo::Dynamo;
use crate::store::Invited;
use airband_core::encode::base64_url;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The app whose testers these are, and its external TestFlight group.
pub(crate) const APP: &str = "dev.airband.app";
pub(crate) const GROUP: &str = "airband pilot";
const API: &str = "https://api.appstoreconnect.apple.com";
/// A token's life. Apple refuses one that lives more than twenty minutes.
const TOKEN_LIFE: u64 = 19 * 60;
const _: () = assert!(TOKEN_LIFE <= 20 * 60);
/// A token is made again this long before it runs out, so none is sent in its last moments.
const RENEW_BEFORE: u64 = 2 * 60;

/// Why no invite was asked for: the relay is not set up to send them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotSetUp {
    /// AIRBAND_TESTFLIGHT_SECRET is not set.
    NoSecretNamed,
    /// Secrets Manager has no secret by that name.
    SecretAbsent,
}

/// Why an invite was asked for and did not go out. The address stays on the list without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Secrets Manager did not give the secret (its own name for why).
    SecretUnreadable(String),
    /// The secret is there but is not the three fields, or its key is not a P-256 .p8 key.
    KeyUnusable(&'static str),
    CannotSign,
    TimedOut,
    Unreachable,
    /// Apple refused the key's token, again after a fresh secret and token.
    Unauthorized,
    /// Apple has no app with the bundle id [`APP`] that this key can see.
    AppNotFound,
    /// The app has no group named [`GROUP`].
    GroupNotFound,
    /// That group is an internal one: Apple does not invite addresses to it.
    GroupIsInternal,
    /// Apple said no: its status and the codes it gave.
    Refused { status: u16, codes: Vec<String> },
    /// Apple's answer was not the shape its API documents.
    Unreadable { status: u16 },
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::SecretUnreadable(kind) => write!(f, "the secret could not be read ({kind})"),
            Failure::KeyUnusable(why) => write!(f, "the secret's key cannot be used: {why}"),
            Failure::CannotSign => f.write_str("the key could not sign a token"),
            Failure::TimedOut => f.write_str("App Store Connect did not answer in time"),
            Failure::Unreachable => f.write_str("App Store Connect could not be reached"),
            Failure::Unauthorized => f.write_str("App Store Connect refused the key (401)"),
            Failure::AppNotFound => write!(f, "App Store Connect has no app {APP} for this key"),
            Failure::GroupNotFound => write!(f, "the app has no TestFlight group named \"{GROUP}\""),
            Failure::GroupIsInternal => write!(f, "the group \"{GROUP}\" is an internal one"),
            Failure::Refused { status, codes } => write!(f, "App Store Connect said no (status {status}: {})", codes.join(", ")),
            Failure::Unreadable { status } => write!(f, "App Store Connect's answer could not be read (status {status})"),
        }
    }
}

/// What came of asking for one invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Invited(Invited),
    NotSetUp(NotSetUp),
    Failed(Failure),
}

/// Where the key comes from.
enum Source {
    Unnamed,
    Named { aws: Dynamo, name: String },
    /// A test's: the secret's text, or nothing for one that is not there.
    #[cfg(test)]
    Given(Option<String>),
}

/// The secret's text.
#[derive(Deserialize)]
struct Secret {
    #[serde(rename = "keyId")]
    key_id: String,
    #[serde(rename = "issuerId")]
    issuer_id: String,
    key: String,
}

/// Every invite the relay sends goes through here, one at a time.
pub(crate) struct Invites {
    source: Source,
    api: String,
    http: reqwest::Client,
    /// The key once read, its token and the group's id once found: kept until Apple refuses the key.
    asc: tokio::sync::Mutex<Option<Asc>>,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).connect_timeout(Duration::from_secs(4)).build().expect("an HTTP client can be made")
}

impl Invites {
    /// Set up as the environment says. Fails, with the reason, when a secret is named but no region is.
    pub(crate) fn from_env() -> Result<Invites, String> {
        let Some(name) = super::var("AIRBAND_TESTFLIGHT_SECRET") else { return Ok(Invites::unnamed()) };
        let region = super::var("AWS_REGION").or_else(|| super::var("AWS_DEFAULT_REGION")).ok_or("AIRBAND_TESTFLIGHT_SECRET is set, so set AWS_REGION to the region the secret is in")?;
        Ok(Invites { source: Source::Named { aws: Dynamo::new(region), name }, api: API.into(), http: client(), asc: tokio::sync::Mutex::new(None) })
    }
    /// A relay that sends no invites.
    pub(crate) fn unnamed() -> Invites {
        Invites { source: Source::Unnamed, api: API.into(), http: client(), asc: tokio::sync::Mutex::new(None) }
    }
    #[cfg(test)]
    pub(crate) fn given(secret: Option<String>, api: &str) -> Invites {
        Invites { source: Source::Given(secret), api: api.into(), http: client(), asc: tokio::sync::Mutex::new(None) }
    }
    pub(crate) fn describe(&self) -> String {
        match &self.source {
            Source::Unnamed => "sends no TestFlight invites (AIRBAND_TESTFLIGHT_SECRET is not set)".into(),
            Source::Named { name, .. } => format!("sends TestFlight invites with the key in the secret {name}"),
            #[cfg(test)]
            Source::Given(_) => "a test's".into(),
        }
    }

    /// The key, read from its secret. Nothing when there is none to read.
    async fn read(&self) -> Result<Asc, Outcome> {
        let text = match &self.source {
            Source::Unnamed => return Err(Outcome::NotSetUp(NotSetUp::NoSecretNamed)),
            Source::Named { aws, name } => aws.secret(name).await.map_err(|e| Outcome::Failed(Failure::SecretUnreadable(e.kind)))?,
            #[cfg(test)]
            Source::Given(text) => text.clone(),
        };
        let text = text.ok_or(Outcome::NotSetUp(NotSetUp::SecretAbsent))?;
        Asc::new(&text).map_err(|why| Outcome::Failed(Failure::KeyUnusable(why)))
    }

    /// Ask Apple to invite `email`. A key Apple refuses is read again from its secret, with a new token, once.
    pub(crate) async fn invite(&self, email: &str) -> Outcome {
        let mut have = self.asc.lock().await;
        for last in [false, true] {
            let asc = match have.take() {
                Some(asc) => asc,
                None => match self.read().await {
                    Ok(asc) => asc,
                    Err(outcome) => return outcome,
                },
            };
            match have.insert(asc).invite(&self.http, &self.api, email).await {
                Ok(how) => return Outcome::Invited(how),
                Err(Failure::Unauthorized) if !last => *have = None,
                Err(failure) => {
                    if failure == Failure::Unauthorized {
                        *have = None;
                    }
                    return Outcome::Failed(failure);
                }
            }
        }
        Outcome::Failed(Failure::Unauthorized)
    }
}

/// The App Store Connect key in use, its token, and the group's id once found.
struct Asc {
    key: EcdsaKeyPair,
    key_id: String,
    issuer_id: String,
    /// The token and when it runs out (seconds since 1970).
    token: Option<(String, u64)>,
    group: Option<String>,
}

// --- Apple's JSON, as its API documents it: only the parts read or sent here ---

#[derive(Deserialize)]
struct List<A> {
    data: Vec<Resource<A>>,
}
#[derive(Deserialize)]
struct Resource<A> {
    id: String,
    attributes: A,
}
#[derive(Deserialize)]
struct AppFields {
    #[serde(rename = "bundleId")]
    bundle_id: String,
}
#[derive(Deserialize)]
struct GroupFields {
    name: String,
    #[serde(rename = "isInternalGroup")]
    internal: bool,
}
#[derive(Deserialize)]
struct TesterFields {
    email: Option<String>,
}
#[derive(Deserialize)]
struct Errors {
    errors: Vec<ErrorItem>,
}
#[derive(Deserialize)]
struct ErrorItem {
    code: String,
}

#[derive(Serialize)]
struct Ref<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
}
#[derive(Serialize)]
struct Linkage<'a> {
    data: Vec<Ref<'a>>,
}
#[derive(Serialize)]
struct NewTester<'a> {
    data: NewTesterData<'a>,
}
#[derive(Serialize)]
struct NewTesterData<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    attributes: NewTesterFields<'a>,
    relationships: NewTesterLinks<'a>,
}
#[derive(Serialize)]
struct NewTesterFields<'a> {
    email: &'a str,
}
#[derive(Serialize)]
struct NewTesterLinks<'a> {
    #[serde(rename = "betaGroups")]
    groups: Linkage<'a>,
}

#[derive(Serialize)]
struct TokenHead<'a> {
    alg: &'static str,
    kid: &'a str,
    typ: &'static str,
}
#[derive(Serialize)]
struct TokenClaims<'a> {
    iss: &'a str,
    iat: u64,
    exp: u64,
    aud: &'static str,
}

/// One answer from Apple: its status and body.
struct Answer {
    status: u16,
    body: Vec<u8>,
}
impl Answer {
    fn read<T: serde::de::DeserializeOwned>(&self) -> Result<T, Failure> {
        serde_json::from_slice(&self.body).map_err(|_| Failure::Unreadable { status: self.status })
    }
    /// Apple's codes for a refusal, in the order it gave them.
    fn codes(&self) -> Vec<String> {
        serde_json::from_slice::<Errors>(&self.body).map(|e| e.errors.into_iter().map(|e| e.code).collect()).unwrap_or_default()
    }
    fn refused(&self) -> Failure {
        if self.status == 401 { Failure::Unauthorized } else { Failure::Refused { status: self.status, codes: self.codes() } }
    }
}

impl Asc {
    fn new(secret: &str) -> Result<Asc, &'static str> {
        let secret: Secret = serde_json::from_str(secret).map_err(|_| "the secret is not {keyId, issuerId, key}")?;
        let text = secret.key.replace("\\n", "\n");
        let der = rustls_pemfile::pkcs8_private_keys(&mut text.as_bytes()).next().and_then(Result::ok).ok_or("its key is not a .p8 key file's text")?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, der.secret_pkcs8_der(), &SystemRandom::new()).map_err(|_| "its key is not a P-256 signing key")?;
        Ok(Asc { key, key_id: secret.key_id, issuer_id: secret.issuer_id, token: None, group: None })
    }

    /// The token App Store Connect asks for (ES256), the one made before until it is near its end.
    fn token(&mut self, now: u64) -> Result<String, Failure> {
        if let Some((token, ends)) = &self.token
            && now + RENEW_BEFORE < *ends
        {
            return Ok(token.clone());
        }
        let ends = now + TOKEN_LIFE;
        let signed = format!("{}.{}", part(&TokenHead { alg: "ES256", kid: &self.key_id, typ: "JWT" }), part(&TokenClaims { iss: &self.issuer_id, iat: now, exp: ends, aud: "appstoreconnect-v1" }));
        let signature = self.key.sign(&SystemRandom::new(), signed.as_bytes()).map_err(|_| Failure::CannotSign)?;
        let token = format!("{signed}.{}", base64_url(signature.as_ref()));
        self.token = Some((token.clone(), ends));
        Ok(token)
    }

    async fn send(&mut self, req: reqwest::RequestBuilder) -> Result<Answer, Failure> {
        let token = self.token(airband_core::clock::now_secs())?;
        let res = req.bearer_auth(token).send().await.map_err(|e| if e.is_timeout() { Failure::TimedOut } else { Failure::Unreachable })?;
        let status = res.status().as_u16();
        let body = res.bytes().await.map_err(|e| if e.is_timeout() { Failure::TimedOut } else { Failure::Unreachable })?.to_vec();
        Ok(Answer { status, body })
    }

    /// The group's id: found once, by the app's bundle id and the group's name, then kept.
    async fn group(&mut self, http: &reqwest::Client, api: &str) -> Result<String, Failure> {
        if let Some(group) = &self.group {
            return Ok(group.clone());
        }
        let apps = self.send(http.get(format!("{api}/v1/apps")).query(&[("filter[bundleId]", APP), ("fields[apps]", "bundleId"), ("limit", "10")])).await?;
        if apps.status != 200 {
            return Err(apps.refused());
        }
        let app = apps.read::<List<AppFields>>()?.data.into_iter().find(|a| a.attributes.bundle_id == APP).ok_or(Failure::AppNotFound)?;
        let groups = self.send(http.get(format!("{api}/v1/betaGroups")).query(&[("filter[app]", app.id.as_str()), ("filter[name]", GROUP), ("fields[betaGroups]", "name,isInternalGroup"), ("limit", "50")])).await?;
        if groups.status != 200 {
            return Err(groups.refused());
        }
        let group = groups.read::<List<GroupFields>>()?.data.into_iter().find(|g| g.attributes.name == GROUP).ok_or(Failure::GroupNotFound)?;
        if group.attributes.internal {
            return Err(Failure::GroupIsInternal);
        }
        self.group = Some(group.id.clone());
        Ok(group.id)
    }

    /// The team's testers with this address, in the group when one is named.
    async fn testers(&mut self, http: &reqwest::Client, api: &str, email: &str, in_group: Option<&str>) -> Result<Vec<String>, Failure> {
        let mut query = vec![("filter[email]", email), ("fields[betaTesters]", "email"), ("limit", "10")];
        if let Some(group) = in_group {
            query.push(("filter[betaGroups]", group));
        }
        let found = self.send(http.get(format!("{api}/v1/betaTesters")).query(&query)).await?;
        if found.status != 200 {
            return Err(found.refused());
        }
        // the same address, however Apple wrote its letters
        Ok(found.read::<List<TesterFields>>()?.data.into_iter().filter(|t| t.attributes.email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(email))).map(|t| t.id).collect())
    }

    async fn invite(&mut self, http: &reqwest::Client, api: &str, email: &str) -> Result<Invited, Failure> {
        let group = self.group(http, api).await?;
        let body = NewTester { data: NewTesterData { kind: "betaTesters", attributes: NewTesterFields { email }, relationships: NewTesterLinks { groups: Linkage { data: vec![Ref { kind: "betaGroups", id: &group }] } } } };
        let made = self.send(http.post(format!("{api}/v1/betaTesters")).json(&body)).await?;
        match made.status {
            201 => return Ok(Invited::New),
            409 => {}
            _ => return Err(made.refused()),
        }
        // a conflict: what Apple's list says decides it
        if !self.testers(http, api, email, Some(&group)).await?.is_empty() {
            return Ok(Invited::AlreadyTester);
        }
        let Some(tester) = self.testers(http, api, email, None).await?.into_iter().next() else { return Err(made.refused()) };
        let added = self.send(http.post(format!("{api}/v1/betaGroups/{group}/relationships/betaTesters")).json(&Linkage { data: vec![Ref { kind: "betaTesters", id: &tester }] })).await?;
        match added.status {
            204 => Ok(Invited::New),
            _ => Err(added.refused()),
        }
    }
}

/// One of a token's two JSON parts, as it is sent.
fn part<T: Serialize>(v: &T) -> String {
    base64_url(serde_json::to_string(v).expect("JSON can be written").as_bytes())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::http::Uri;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A pretend App Store Connect: one app, its groups, and the team's testers. It checks every token as
    /// Apple does and answers as its API documents.
    #[derive(Default)]
    pub(crate) struct Apple {
        pub(crate) public: Vec<u8>,
        /// The group's name and whether it is internal.
        pub(crate) group: Option<(&'static str, bool)>,
        /// tester id → (address, in the group)
        pub(crate) testers: HashMap<String, (String, bool)>,
        /// Answer every new tester with this status and code.
        pub(crate) refuse: Option<(u16, &'static str)>,
        /// Refuse this many tokens with 401 first.
        pub(crate) unauthorized: u32,
        pub(crate) tokens: Vec<String>,
        pub(crate) lookups: u32,
    }
    type Shared = Arc<Mutex<Apple>>;

    /// A request's query, its names and values decoded.
    fn query(uri: &Uri) -> HashMap<String, String> {
        let decode = |s: &str| {
            let (mut out, bytes) = (Vec::new(), s.replace('+', " ").into_bytes());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap()
        };
        uri.query().unwrap_or("").split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect()
    }
    fn reply(status: u16, body: String) -> Response {
        (StatusCode::from_u16(status).unwrap(), [("content-type", "application/json")], body).into_response()
    }
    fn error(status: u16, code: &str) -> Response {
        reply(status, format!(r#"{{"errors":[{{"status":"{status}","code":"{code}","title":"t","detail":"d"}}]}}"#))
    }
    /// Is this a token Apple would take: ES256, signed by the key, for App Store Connect, alive at most 20 minutes?
    fn checked(apple: &mut Apple, headers: &HeaderMap) -> Result<(), Box<Response>> {
        let token = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").to_string();
        let parts: Vec<&str> = token.split('.').collect();
        let read = |p: &str| airband_core::encode::unbase64_url(p).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).unwrap_or_default();
        let good = parts.len() == 3 && {
            let (head, claims) = (read(parts[0]), read(parts[1]));
            let life = claims["exp"].as_u64().unwrap_or(0).saturating_sub(claims["iat"].as_u64().unwrap_or(u64::MAX));
            head["alg"] == "ES256" && head["kid"] == "KEY123" && head["typ"] == "JWT" && claims["iss"] == "ISSUER" && claims["aud"] == "appstoreconnect-v1" && life > 0 && life <= 1200
                && airband_core::encode::unbase64_url(parts[2]).is_some_and(|s| UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &apple.public).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &s).is_ok())
        };
        if !good || apple.unauthorized > 0 {
            apple.unauthorized = apple.unauthorized.saturating_sub(1);
            return Err(Box::new(error(401, "NOT_AUTHORIZED")));
        }
        if !apple.tokens.contains(&token) {
            apple.tokens.push(token);
        }
        Ok(())
    }

    async fn apps(State(apple): State<Shared>, headers: HeaderMap, uri: Uri) -> Response {
        let (apple, q) = (&mut apple.lock().unwrap(), query(&uri));
        if let Err(r) = checked(apple, &headers) {
            return *r;
        }
        apple.lookups += 1;
        let found = if q.get("filter[bundleId]").map(String::as_str) == Some(APP) { r#"[{"type":"apps","id":"app1","attributes":{"bundleId":"dev.airband.app"}}]"# } else { "[]" };
        reply(200, format!(r#"{{"data":{found}}}"#))
    }
    async fn groups(State(apple): State<Shared>, headers: HeaderMap, uri: Uri) -> Response {
        let (apple, q) = (&mut apple.lock().unwrap(), query(&uri));
        if let Err(r) = checked(apple, &headers) {
            return *r;
        }
        let found = match apple.group {
            Some((name, internal)) if q.get("filter[app]").map(String::as_str) == Some("app1") && q.get("filter[name]").map(String::as_str) == Some(name) => format!(r#"[{{"type":"betaGroups","id":"group1","attributes":{{"name":"{name}","isInternalGroup":{internal}}}}}]"#),
            _ => "[]".into(),
        };
        reply(200, format!(r#"{{"data":{found}}}"#))
    }
    async fn new_tester(State(apple): State<Shared>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
        let apple = &mut apple.lock().unwrap();
        if let Err(r) = checked(apple, &headers) {
            return *r;
        }
        let asked: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(asked["data"]["type"], "betaTesters");
        assert_eq!(asked["data"]["relationships"]["betaGroups"]["data"][0], serde_json::json!({ "type": "betaGroups", "id": "group1" }));
        if let Some((status, code)) = apple.refuse {
            return error(status, code);
        }
        let email = asked["data"]["attributes"]["email"].as_str().unwrap().to_string();
        if apple.testers.values().any(|(e, _)| e.eq_ignore_ascii_case(&email)) {
            return error(409, "ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE");
        }
        let id = format!("t{}", apple.testers.len() + 1);
        apple.testers.insert(id.clone(), (email.clone(), true));
        reply(201, format!(r#"{{"data":{{"type":"betaTesters","id":"{id}","attributes":{{"email":"{email}"}}}}}}"#))
    }
    async fn find_testers(State(apple): State<Shared>, headers: HeaderMap, uri: Uri) -> Response {
        let (apple, q) = (&mut apple.lock().unwrap(), query(&uri));
        if let Err(r) = checked(apple, &headers) {
            return *r;
        }
        let email = q.get("filter[email]").cloned().unwrap_or_default();
        let in_group = q.contains_key("filter[betaGroups]");
        let found: Vec<String> = apple.testers.iter().filter(|(_, (e, g))| e.eq_ignore_ascii_case(&email) && (*g || !in_group)).map(|(id, (e, _))| format!(r#"{{"type":"betaTesters","id":"{id}","attributes":{{"email":"{}"}}}}"#, e.to_uppercase())).collect();
        reply(200, format!(r#"{{"data":[{}]}}"#, found.join(",")))
    }
    async fn add_to_group(State(apple): State<Shared>, headers: HeaderMap, Path(group): Path<String>, body: axum::body::Bytes) -> Response {
        let apple = &mut apple.lock().unwrap();
        if let Err(r) = checked(apple, &headers) {
            return *r;
        }
        assert_eq!(group, "group1");
        let asked: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = asked["data"][0]["id"].as_str().unwrap();
        apple.testers.get_mut(id).unwrap().1 = true;
        StatusCode::NO_CONTENT.into_response()
    }

    /// Start the pretend Apple; its address.
    pub(crate) async fn pretend_apple(apple: Shared) -> String {
        let app = axum::Router::new()
            .route("/v1/apps", get(apps))
            .route("/v1/betaGroups", get(groups))
            .route("/v1/betaTesters", post(new_tester).get(find_testers))
            .route("/v1/betaGroups/{group}/relationships/betaTesters", post(add_to_group))
            .with_state(apple);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        at
    }

    /// A key made for a test, as the secret's text, with a pretend Apple that knows its public half.
    pub(crate) fn key_and_apple() -> (String, Shared) {
        let (p8, public) = crate::push::tests::test_key();
        let secret = serde_json::to_string(&serde_json::json!({ "keyId": "KEY123", "issuerId": "ISSUER", "key": p8 })).unwrap();
        (secret, Arc::new(Mutex::new(Apple { public, group: Some((GROUP, false)), ..Apple::default() })))
    }

    #[tokio::test]
    async fn a_new_address_is_invited_and_one_already_there_is_not_asked_twice() {
        let (secret, apple) = key_and_apple();
        let invites = Invites::given(Some(secret), &pretend_apple(apple.clone()).await);
        assert_eq!(invites.invite("new@example.com").await, Outcome::Invited(Invited::New));
        assert_eq!(apple.lock().unwrap().testers.values().filter(|(e, g)| e == "new@example.com" && *g).count(), 1);
        // the same address again: a conflict, and Apple's list shows it in the group
        assert_eq!(invites.invite("new@example.com").await, Outcome::Invited(Invited::AlreadyTester));
        // a tester of the team outside the group is added to it, which sends the invite
        apple.lock().unwrap().testers.insert("t9".into(), (String::from("team@example.com"), false));
        assert_eq!(invites.invite("team@example.com").await, Outcome::Invited(Invited::New));
        assert!(apple.lock().unwrap().testers["t9"].1);
        // one token for all of it, and the app and group looked up once
        let apple = apple.lock().unwrap();
        assert_eq!((apple.tokens.len(), apple.lookups), (1, 1));
    }

    #[tokio::test]
    async fn a_conflict_apple_cannot_explain_is_a_refusal_with_its_codes() {
        let (secret, apple) = key_and_apple();
        apple.lock().unwrap().refuse = Some((409, "STATE_ERROR"));
        let invites = Invites::given(Some(secret), &pretend_apple(apple.clone()).await);
        assert_eq!(invites.invite("a@example.com").await, Outcome::Failed(Failure::Refused { status: 409, codes: vec!["STATE_ERROR".into()] }));
        apple.lock().unwrap().refuse = Some((500, "UNEXPECTED_ERROR"));
        assert_eq!(invites.invite("a@example.com").await, Outcome::Failed(Failure::Refused { status: 500, codes: vec!["UNEXPECTED_ERROR".into()] }));
    }

    #[tokio::test]
    async fn a_refused_key_is_read_again_once_then_the_refusal_stands() {
        let (secret, apple) = key_and_apple();
        let invites = Invites::given(Some(secret), &pretend_apple(apple.clone()).await);
        apple.lock().unwrap().unauthorized = 1;
        assert_eq!(invites.invite("a@example.com").await, Outcome::Invited(Invited::New));
        apple.lock().unwrap().unauthorized = 5;
        assert_eq!(invites.invite("b@example.com").await, Outcome::Failed(Failure::Unauthorized));
    }

    #[tokio::test]
    async fn no_secret_named_none_there_or_a_bad_one_each_say_so() {
        assert_eq!(Invites::unnamed().invite("a@example.com").await, Outcome::NotSetUp(NotSetUp::NoSecretNamed));
        assert_eq!(Invites::given(None, "http://127.0.0.1:1").invite("a@example.com").await, Outcome::NotSetUp(NotSetUp::SecretAbsent));
        assert!(matches!(Invites::given(Some("{}".into()), "http://127.0.0.1:1").invite("a@example.com").await, Outcome::Failed(Failure::KeyUnusable(_))));
        let not_a_key = serde_json::to_string(&serde_json::json!({ "keyId": "K", "issuerId": "I", "key": "not a key" })).unwrap();
        assert!(matches!(Invites::given(Some(not_a_key), "http://127.0.0.1:1").invite("a@example.com").await, Outcome::Failed(Failure::KeyUnusable(_))));
        let (secret, _) = key_and_apple();
        assert_eq!(Invites::given(Some(secret), "http://127.0.0.1:1").invite("a@example.com").await, Outcome::Failed(Failure::Unreachable));
    }

    #[tokio::test]
    async fn the_group_must_be_there_and_external() {
        let (secret, apple) = key_and_apple();
        let at = pretend_apple(apple.clone()).await;
        apple.lock().unwrap().group = None;
        assert_eq!(Invites::given(Some(secret.clone()), &at).invite("a@example.com").await, Outcome::Failed(Failure::GroupNotFound));
        apple.lock().unwrap().group = Some((GROUP, true));
        assert_eq!(Invites::given(Some(secret), &at).invite("a@example.com").await, Outcome::Failed(Failure::GroupIsInternal));
    }

    #[test]
    fn a_token_lives_under_twenty_minutes_and_is_made_again_near_its_end() {
        let (secret, _) = key_and_apple();
        let mut asc = Asc::new(&secret).unwrap();
        let first = asc.token(1_000_000).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&airband_core::encode::unbase64_url(first.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!((claims["iat"].as_u64(), claims["exp"].as_u64(), claims["aud"].as_str(), claims["iss"].as_str()), (Some(1_000_000), Some(1_000_000 + TOKEN_LIFE), Some("appstoreconnect-v1"), Some("ISSUER")));
        assert_eq!(asc.token(1_000_000 + TOKEN_LIFE - RENEW_BEFORE - 1).unwrap(), first);
        assert_ne!(asc.token(1_000_000 + TOKEN_LIFE - RENEW_BEFORE).unwrap(), first);
    }
}
