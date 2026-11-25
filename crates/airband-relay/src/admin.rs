//! The admin page, for Charles alone: relay.airband.dev/admin lists who asked for an invite on the homepage (the
//! address, when, and whether its invite was sent) and sends the TestFlight invites he chooses (testflight.rs).
//! Nothing else ever sends one.
//!
//! Who may use it is decided the way the admin pages of LionPal and Store Watchdog decide it: one email address
//! on an allowlist, proved by signing in, and then a session cookie holding a random token of which the server
//! keeps only the SHA-256. Store Watchdog's staff sign in with their email and a password; so does this, and the
//! allowlist is one address. Both are kept in AWS Secrets Manager, never in the repository:
//!
//!   AIRBAND_ADMIN_SECRET   the secret's name, in AWS_REGION. Its text is JSON:
//!                          {"email": "…", "password": {"iterations": N, "salt": "<hex>", "hash": "<hex>"}}
//!                          with the password as PBKDF2-HMAC-SHA256 (scripts/admin-password.sh puts it there).
//!                          Without this setting, or while the secret is not there, nobody can sign in.
//!
//! The secret is read at each sign-in, so a new password works at once. A sign-in lasts [`SESSION_DAYS`], and
//! signing out ends it. Every route but the page and its two files answers 401 without a live sign-in: the
//! data is what is locked, not only the page that shows it. Other sites: the cookie is SameSite=Strict, and
//! every change is a JSON POST, which no other page can send without a CORS preflight this relay never answers.
//!
//!   GET  /admin               the page (admin/page.html, admin.js, admin.css beside it)
//!   POST /admin/sign-in       {"email", "password"}: sets the cookie
//!   POST /admin/sign-out      ends the sign-in and clears the cookie
//!   GET  /admin/requests      {"requests": [Request]}, newest first
//!   POST /admin/invites       {"emails": [...]}: sends each one's invite, one at a time; {"results": [Sending]}
//!
//! The log says what happened, never an address, a password or a token.
use super::*;
use crate::dynamo::Dynamo;
use crate::store::{Invite, Request};
use crate::testflight::{Failure, Outcome};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// The cookie. `__Host-` makes the browser keep it only from HTTPS, for this host alone, for every path.
pub(super) const COOKIE: &str = "__Host-airband-admin";
/// How long a sign-in lasts.
pub(super) const SESSION_DAYS: u64 = 30;
/// Sign-in tries taken from one internet address in a quarter of an hour, and from everyone in an hour. Right or
/// wrong, each counts.
const TRIES_PER_ADDRESS: u32 = 10;
const TRIES_IN_ALL: u32 = 30;
/// The fewest PBKDF2 rounds a password on record may have been hashed with. The script uses 600,000.
const LEAST_ROUNDS: u32 = 100_000;
/// The most invites asked for at once.
const MOST_AT_ONCE: usize = 100;
/// How long Apple is given to take one invite, start to end.
const INVITE_WITHIN: Duration = Duration::from_secs(15);
/// How long the store is given for one read or write.
const STORE_WITHIN: Duration = Duration::from_secs(8);

const PAGE: &str = include_str!("admin/page.html");
const SCRIPT: &str = include_str!("admin/admin.js");
const STYLE: &str = include_str!("admin/admin.css");

/// Where the allowlisted address and its password come from.
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
    email: String,
    password: Password,
}

/// A password as it is kept: PBKDF2-HMAC-SHA256 over this many rounds with this salt.
#[derive(Serialize, Deserialize)]
pub(crate) struct Password {
    pub(crate) iterations: u32,
    /// Hex.
    pub(crate) salt: String,
    /// Hex.
    pub(crate) hash: String,
}

/// What a sign-in came to.
#[derive(Debug, PartialEq, Eq)]
enum Checked {
    Yes,
    No,
    /// No secret is named, or none is there.
    NotSetUp,
    /// The secret could not be read or used: why, without anything from it.
    Failed(String),
}

/// The admin's sign-in: where its secret is, and how many tries have been made lately.
pub(crate) struct Admin {
    source: Source,
    tries: Mutex<(Limiter<Caller>, Limiter<()>)>,
}

impl Admin {
    fn with(source: Source) -> Admin {
        Admin { source, tries: Mutex::new((Limiter::new(TRIES_PER_ADDRESS, 15 * 60), Limiter::new(TRIES_IN_ALL, 3600))) }
    }
    /// Set up as the environment says. Fails, with the reason, when a secret is named but no region is.
    pub(crate) fn from_env() -> Result<Admin, String> {
        let Some(name) = super::var("AIRBAND_ADMIN_SECRET") else { return Ok(Admin::unnamed()) };
        let region = super::var("AWS_REGION").or_else(|| super::var("AWS_DEFAULT_REGION")).ok_or("AIRBAND_ADMIN_SECRET is set, so set AWS_REGION to the region the secret is in")?;
        Ok(Admin::with(Source::Named { aws: Dynamo::new(region), name }))
    }
    /// Nobody can sign in.
    pub(crate) fn unnamed() -> Admin {
        Admin::with(Source::Unnamed)
    }
    #[cfg(test)]
    pub(crate) fn given(secret: Option<String>) -> Admin {
        Admin::with(Source::Given(secret))
    }
    pub(crate) fn describe(&self) -> String {
        match &self.source {
            Source::Unnamed => "has no admin sign-in (AIRBAND_ADMIN_SECRET is not set)".into(),
            Source::Named { name, .. } => format!("signs the admin in with the secret {name}"),
            #[cfg(test)]
            Source::Given(_) => "a test's".into(),
        }
    }

    /// Is this the allowlisted address and its password? The password is checked whatever the address, so how
    /// long the answer takes says nothing about which was wrong.
    async fn check(&self, email: &str, password: &str) -> Checked {
        let text = match &self.source {
            Source::Unnamed => return Checked::NotSetUp,
            Source::Named { aws, name } => match aws.secret(name).await {
                Ok(text) => text,
                Err(e) => return Checked::Failed(format!("the secret could not be read ({})", e.kind)),
            },
            #[cfg(test)]
            Source::Given(text) => text.clone(),
        };
        let Some(text) = text else { return Checked::NotSetUp };
        let Ok(secret) = serde_json::from_str::<Secret>(&text) else { return Checked::Failed("the secret is not an email and a password".into()) };
        let p = &secret.password;
        let (Some(salt), Some(hash), Some(rounds)) = (airband_core::encode::unhex(&p.salt), airband_core::encode::unhex(&p.hash), NonZeroU32::new(p.iterations)) else {
            return Checked::Failed("the secret's password is not rounds, a salt and a hash".into());
        };
        if p.iterations < LEAST_ROUNDS || salt.len() < 16 || hash.len() != 32 {
            return Checked::Failed("the secret's password was hashed too weakly".into());
        }
        let given = password.to_string();
        let right_password = match tokio::task::spawn_blocking(move || ring::pbkdf2::verify(ring::pbkdf2::PBKDF2_HMAC_SHA256, rounds, &salt, given.as_bytes(), &hash).is_ok()).await {
            Ok(right) => right,
            Err(_) => return Checked::Failed("the password could not be checked".into()),
        };
        let right_email = same(&secret.email.trim().to_lowercase(), email);
        if right_email && right_password { Checked::Yes } else { Checked::No }
    }

    /// May another sign-in be tried from this address now? Counts it if so.
    fn try_allowed(&self, from: &Caller) -> bool {
        let mut tries = self.tries.lock().unwrap();
        let (by_address, all) = &mut *tries;
        limits::all_or_none(&mut [&mut (by_address, from), &mut (all, &())], Instant::now()).is_ok()
    }
}

/// A plain answer for the page: what to show.
#[derive(Serialize)]
struct Said {
    message: String,
}
fn said(code: StatusCode, message: &str) -> Response {
    (code, no_store(), axum::Json(Said { message: message.to_string() })).into_response()
}
fn no_store() -> [(&'static str, &'static str); 1] {
    [("cache-control", "no-store")]
}
fn signed_out() -> Response {
    said(StatusCode::UNAUTHORIZED, "Sign in to continue.")
}
fn is_json(headers: &HeaderMap) -> bool {
    header(headers, "content-type").is_some_and(|t| t.starts_with("application/json"))
}

/// The token in this request's cookie, when it has one of the right shape.
fn token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value)
        .filter(|t| airband_core::encode::is_hex(t, 64..=64))
}

/// A request is let through only with a live sign-in: the answer it gets otherwise, or nothing when it may go on.
async fn refused(relay: &Relay, headers: &HeaderMap) -> Option<Response> {
    let Some(token) = token(headers) else { return Some(signed_out()) };
    match tokio::time::timeout(STORE_WITHIN, relay.store.session_live(&token_hash(token), airband_core::clock::now_secs())).await {
        Ok(Ok(true)) => None,
        Ok(Ok(false)) => Some(signed_out()),
        failed => {
            eprintln!("airband-relay: admin: could not check a sign-in: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            Some(said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't check your sign-in just now. Try again in a minute."))
        }
    }
}

/// The headers every one of the page's own files is sent with: run nothing but its own script and style, talk
/// to nothing but this relay, never sit in another site's frame, and never be kept or indexed.
fn page_headers(content_type: &'static str) -> [(&'static str, &'static str); 7] {
    [
        ("content-type", content_type),
        ("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'self' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
        ("x-frame-options", "DENY"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        ("x-robots-tag", "noindex, nofollow"),
    ]
}

pub(super) async fn page() -> Response {
    (page_headers("text/html; charset=utf-8"), PAGE).into_response()
}
pub(super) async fn script() -> Response {
    (page_headers("text/javascript; charset=utf-8"), SCRIPT).into_response()
}
pub(super) async fn style() -> Response {
    (page_headers("text/css; charset=utf-8"), STYLE).into_response()
}

fn cookie(token: &str, max_age: u64) -> [(&'static str, String); 2] {
    [("set-cookie", format!("{COOKIE}={token}; Path=/; Max-Age={max_age}; Secure; HttpOnly; SameSite=Strict")), ("cache-control", "no-store".into())]
}

pub(super) async fn sign_in(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    #[derive(Deserialize)]
    struct Given {
        email: String,
        password: String,
    }
    if !is_json(&headers) {
        return said(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.");
    }
    let Some(given) = (body.len() <= 1024).then(|| serde_json::from_slice::<Given>(&body).ok()).flatten() else {
        return said(StatusCode::BAD_REQUEST, "Enter your email and password.");
    };
    if !relay.admin.try_allowed(&caller(&headers)) {
        println!("airband-relay: admin: sign-in refused, too many tries");
        return said(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Wait a few minutes and try again.");
    }
    match relay.admin.check(&given.email.trim().to_lowercase(), &given.password).await {
        Checked::Yes => {}
        Checked::No => {
            println!("airband-relay: admin: sign-in refused");
            return said(StatusCode::UNAUTHORIZED, "That email and password don't match.");
        }
        Checked::NotSetUp => {
            println!("airband-relay: admin: sign-in refused, not set up");
            return said(StatusCode::SERVICE_UNAVAILABLE, "Sign-in isn't set up on this relay yet.");
        }
        Checked::Failed(why) => {
            eprintln!("airband-relay: admin: sign-in failed: {why}");
            return said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't sign you in just now. Try again in a minute.");
        }
    }
    let token = airband_core::encode::random_hex(32);
    let expires = airband_core::clock::now_secs() + SESSION_DAYS * 86_400;
    match tokio::time::timeout(STORE_WITHIN, relay.store.session_start(&token_hash(&token), expires)).await {
        Ok(Ok(())) => {
            println!("airband-relay: admin: signed in");
            (StatusCode::NO_CONTENT, cookie(&token, SESSION_DAYS * 86_400)).into_response()
        }
        failed => {
            eprintln!("airband-relay: admin: could not keep a sign-in: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't sign you in just now. Try again in a minute.")
        }
    }
}

pub(super) async fn sign_out(State(relay): State<Arc<Relay>>, headers: HeaderMap) -> Response {
    if !is_json(&headers) {
        return said(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.");
    }
    if let Some(token) = token(&headers) {
        match tokio::time::timeout(STORE_WITHIN, relay.store.session_end(&token_hash(token))).await {
            Ok(Ok(())) => println!("airband-relay: admin: signed out"),
            failed => {
                eprintln!("airband-relay: admin: could not end a sign-in: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
                return said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't sign you out just now. Try again in a minute.");
            }
        }
    }
    (StatusCode::NO_CONTENT, cookie("", 0)).into_response()
}

/// Everyone who asked, newest first.
#[derive(Serialize)]
struct Requests {
    requests: Vec<Request>,
}

pub(super) async fn requests(State(relay): State<Arc<Relay>>, headers: HeaderMap) -> Response {
    if let Some(refused) = refused(&relay, &headers).await {
        return refused;
    }
    match tokio::time::timeout(STORE_WITHIN, relay.store.waitlist()).await {
        Ok(Ok(mut requests)) => {
            requests.sort_by(|a, b| b.joined_at.cmp(&a.joined_at).then_with(|| a.email.cmp(&b.email)));
            (no_store(), axum::Json(Requests { requests })).into_response()
        }
        failed => {
            eprintln!("airband-relay: admin: could not read the waiting list: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't read the list just now. Try again in a minute.")
        }
    }
}

/// What came of one invite, as the page is told it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(crate) enum Sending {
    /// Apple has it, and it is recorded.
    Sent { email: String, invite: Invite },
    /// Apple has it and it could not be recorded. Sending it again is safe: Apple then has it as a tester already.
    NotRecorded { email: String, why: String },
    /// The relay has no App Store Connect key to send with.
    NotSetUp { email: String },
    /// Asked for and not sent.
    Failed { email: String, why: String },
    /// Not on the waiting list: nothing was asked of Apple.
    NotOnList { email: String },
}

/// Ask for one address's invite and record it once Apple has it. Logs what came of it, never the address.
pub(crate) async fn send(relay: &Relay, email: &str) -> Sending {
    let email_ = || email.to_string();
    let began = Instant::now();
    let outcome = tokio::time::timeout(INVITE_WITHIN, relay.invites.invite(email)).await.unwrap_or(Outcome::Failed(Failure::TimedOut));
    let ms = began.elapsed().as_millis();
    match outcome {
        Outcome::Invited(how) => {
            let invite = Invite { how, at: airband_core::clock::now_secs() };
            let recorded = tokio::time::timeout(STORE_WITHIN, async {
                let _turn = relay.writing.acquire().await.map_err(|e| e.to_string())?;
                relay.store.invited(email, how, invite.at).await
            })
            .await
            .unwrap_or_else(|_| Err("the database did not answer in time".into()));
            match recorded {
                Ok(()) => {
                    println!("airband-relay: admin: invite sent ({}, {ms} ms)", how.name());
                    Sending::Sent { email: email_(), invite }
                }
                Err(why) => {
                    eprintln!("airband-relay: admin: invite sent ({}, {ms} ms) but not recorded: {why}", how.name());
                    Sending::NotRecorded { email: email_(), why }
                }
            }
        }
        Outcome::NotSetUp(why) => {
            println!("airband-relay: admin: no invite sent, not set up ({why:?})");
            Sending::NotSetUp { email: email_() }
        }
        Outcome::Failed(why) => {
            eprintln!("airband-relay: admin: invite not sent ({ms} ms): {why}");
            Sending::Failed { email: email_(), why: why.to_string() }
        }
    }
}

#[derive(Serialize)]
struct Results {
    results: Vec<Sending>,
}

pub(super) async fn invites(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    #[derive(Deserialize)]
    struct Chosen {
        emails: Vec<String>,
    }
    if let Some(refused) = refused(&relay, &headers).await {
        return refused;
    }
    if !is_json(&headers) {
        return said(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.");
    }
    let Some(chosen) = (body.len() <= 64 * 1024).then(|| serde_json::from_slice::<Chosen>(&body).ok()).flatten() else {
        return said(StatusCode::BAD_REQUEST, "Choose who to invite.");
    };
    if chosen.emails.is_empty() || chosen.emails.len() > MOST_AT_ONCE {
        return said(StatusCode::BAD_REQUEST, &format!("Choose between 1 and {MOST_AT_ONCE} people at a time."));
    }
    let on_list: std::collections::HashSet<String> = match tokio::time::timeout(STORE_WITHIN, relay.store.waitlist()).await {
        Ok(Ok(all)) => all.into_iter().map(|r| r.email).collect(),
        failed => {
            eprintln!("airband-relay: admin: could not read the waiting list: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            return said(StatusCode::SERVICE_UNAVAILABLE, "Couldn't read the list just now. Try again in a minute.");
        }
    };
    let mut results = Vec::with_capacity(chosen.emails.len());
    for email in chosen.emails {
        let email = email.trim().to_lowercase();
        results.push(if on_list.contains(&email) { send(&relay, &email).await } else { Sending::NotOnList { email } });
    }
    (no_store(), axum::Json(Results { results })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Invited;

    /// A password as scripts/admin-password.sh keeps it.
    fn kept(password: &str) -> Password {
        let salt = airband_core::encode::random_hex(16);
        let mut hash = [0u8; 32];
        ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(LEAST_ROUNDS).unwrap(), &airband_core::encode::unhex(&salt).unwrap(), password.as_bytes(), &mut hash);
        Password { iterations: LEAST_ROUNDS, salt, hash: airband_core::encode::hex(&hash) }
    }
    fn secret(email: &str, password: &str) -> String {
        #[derive(Serialize)]
        struct Secret {
            email: String,
            password: Password,
        }
        serde_json::to_string(&Secret { email: email.into(), password: kept(password) }).unwrap()
    }

    /// A relay on a folder of its own, with this admin secret and these invites.
    fn relay(admin: Admin, invites: testflight::Invites) -> (Arc<Relay>, std::path::PathBuf) {
        let folder = std::env::temp_dir().join(format!("airband-relay-admin-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::folder(folder.clone()), None, invites).with_admin(admin)), folder)
    }
    fn json_headers(cookie: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/json".parse().unwrap());
        if let Some(c) = cookie {
            h.insert("cookie", c.parse().unwrap());
        }
        h
    }
    async fn body(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }
    async fn sign_in_as(relay: &Arc<Relay>, email: &str, password: &str) -> Response {
        let body = serde_json::to_string(&serde_json::Map::from_iter([("email".to_string(), email.into()), ("password".to_string(), password.into())])).unwrap();
        sign_in(State(relay.clone()), json_headers(None), Bytes::from(body)).await
    }
    /// The cookie a signed-in answer sets, as the browser sends it back.
    fn cookie_from(res: &Response) -> String {
        let set = res.headers().get("set-cookie").unwrap().to_str().unwrap();
        assert!(set.contains("HttpOnly") && set.contains("Secure") && set.contains("SameSite=Strict") && set.contains("Path=/"), "{set}");
        set.split(';').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn only_the_allowlisted_address_with_its_password_gets_in_and_the_data_is_locked_without_it() {
        let (relay, folder) = relay(Admin::given(Some(secret("Charles@Example.com", "right horse battery"))), testflight::Invites::unnamed());
        relay.store.join_waitlist("a@example.com").await.unwrap();
        // without a sign-in the list is not given, whatever the cookie
        assert_eq!(requests(State(relay.clone()), HeaderMap::new()).await.status(), StatusCode::UNAUTHORIZED);
        let forged = format!("{COOKIE}={}", "a".repeat(64));
        assert_eq!(requests(State(relay.clone()), json_headers(Some(&forged))).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(invites(State(relay.clone()), json_headers(Some(&forged)), Bytes::from(r#"{"emails":["a@example.com"]}"#)).await.status(), StatusCode::UNAUTHORIZED);
        // a wrong password, or another address with the right one
        assert_eq!(sign_in_as(&relay, "charles@example.com", "wrong").await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(sign_in_as(&relay, "someone@example.com", "right horse battery").await.status(), StatusCode::UNAUTHORIZED);
        // the address in any case, and its password
        let ok = sign_in_as(&relay, " CHARLES@example.com", "right horse battery").await;
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        let cookie = cookie_from(&ok);
        // the relay keeps the token's hash, never the token
        let sessions = std::fs::read_to_string(folder.join("sessions.json")).unwrap();
        let token = cookie.split_once('=').unwrap().1;
        assert!(!sessions.contains(token) && sessions.contains(&token_hash(token)));
        let listed = body(requests(State(relay.clone()), json_headers(Some(&cookie))).await).await;
        assert_eq!(listed["requests"][0]["email"], "a@example.com");
        assert_eq!(listed["requests"][0]["invite"], serde_json::Value::Null);
        // signed out: the same cookie gets nothing
        let out = sign_out(State(relay.clone()), json_headers(Some(&cookie))).await;
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        assert!(out.headers().get("set-cookie").unwrap().to_str().unwrap().contains("Max-Age=0"));
        assert_eq!(requests(State(relay.clone()), json_headers(Some(&cookie))).await.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn without_a_secret_nobody_signs_in_and_a_weak_or_broken_one_is_refused() {
        for admin in [Admin::unnamed(), Admin::given(None)] {
            let (relay, folder) = relay(admin, testflight::Invites::unnamed());
            assert_eq!(sign_in_as(&relay, "charles@example.com", "anything").await.status(), StatusCode::SERVICE_UNAVAILABLE);
            let _ = std::fs::remove_dir_all(&folder);
        }
        assert!(matches!(Admin::given(Some("{not json".into())).check("c@example.com", "x").await, Checked::Failed(_)));
        let weak = serde_json::to_string(&serde_json::Map::from_iter([
            ("email".to_string(), serde_json::Value::from("c@example.com")),
            ("password".to_string(), serde_json::to_value(Password { iterations: 1000, ..kept("x") }).unwrap()),
        ]))
        .unwrap();
        assert!(matches!(Admin::given(Some(weak)).check("c@example.com", "x").await, Checked::Failed(_)));
    }

    #[tokio::test]
    async fn a_password_kept_by_the_script_is_checked_as_the_script_hashed_it() {
        // made by scripts/admin-password.sh's own hashing: Python's hashlib.pbkdf2_hmac("sha256", …)
        let made = r#"{"email":"c@example.com","password":{"iterations":100000,"salt":"000102030405060708090a0b0c0d0e0f","hash":"c8d122ecdb9477cd48a6b1750c5985c0b8b05158d040d20517ccdb49865de436"}}"#;
        let admin = Admin::given(Some(made.into()));
        assert_eq!(admin.check("c@example.com", "correct horse battery").await, Checked::Yes);
        assert_eq!(admin.check("c@example.com", "correct horse batter").await, Checked::No);
    }

    #[tokio::test]
    async fn sign_in_tries_are_limited_per_address_right_or_wrong() {
        let (relay, folder) = relay(Admin::given(Some(secret("c@example.com", "pw"))), testflight::Invites::unnamed());
        for _ in 0..TRIES_PER_ADDRESS {
            assert_eq!(sign_in_as(&relay, "c@example.com", "wrong").await.status(), StatusCode::UNAUTHORIZED);
        }
        assert_eq!(sign_in_as(&relay, "c@example.com", "pw").await.status(), StatusCode::TOO_MANY_REQUESTS);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn the_chosen_addresses_are_invited_and_recorded_and_no_others() {
        let (key, apple) = testflight::tests::key_and_apple();
        let (relay, folder) = relay(Admin::given(Some(secret("c@example.com", "pw"))), testflight::Invites::given(Some(key), &testflight::tests::pretend_apple(apple.clone()).await));
        for e in ["a@example.com", "b@example.com"] {
            relay.store.join_waitlist(e).await.unwrap();
        }
        let cookie = cookie_from(&sign_in_as(&relay, "c@example.com", "pw").await);
        let ask = |emails: &str| invites(State(relay.clone()), json_headers(Some(&cookie)), Bytes::from(format!(r#"{{"emails":{emails}}}"#)));
        let out = body(ask(r#"["A@example.com","stranger@example.com"]"#).await).await;
        assert_eq!(out["results"][0]["outcome"], "sent");
        assert_eq!(out["results"][0]["invite"]["how"], "new");
        assert_eq!(out["results"][1]["outcome"], "not_on_list");
        // Apple was asked for the one chosen, and nobody else; the list says so
        assert_eq!(apple.lock().unwrap().testers.len(), 1);
        let listed = relay.store.waitlist().await.unwrap();
        let of = |e: &str| listed.iter().find(|r| r.email == e).unwrap().invite.map(|i| i.how);
        assert_eq!((of("a@example.com"), of("b@example.com")), (Some(Invited::New), None));
        // Apple says no: told why, and nothing recorded
        apple.lock().unwrap().refuse = Some((409, "STATE_ERROR"));
        let out = body(ask(r#"["b@example.com"]"#).await).await;
        assert_eq!(out["results"][0]["outcome"], "failed");
        assert!(out["results"][0]["why"].as_str().unwrap().contains("STATE_ERROR"));
        assert_eq!(relay.store.waitlist().await.unwrap().iter().find(|r| r.email == "b@example.com").unwrap().invite, None);
        // none, or too many at once, is refused before anything is asked
        assert_eq!(ask("[]").await.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn without_a_key_the_page_is_told_invites_are_not_set_up() {
        let (relay, folder) = relay(Admin::given(Some(secret("c@example.com", "pw"))), testflight::Invites::unnamed());
        relay.store.join_waitlist("a@example.com").await.unwrap();
        let cookie = cookie_from(&sign_in_as(&relay, "c@example.com", "pw").await);
        let out = body(invites(State(relay.clone()), json_headers(Some(&cookie)), Bytes::from(r#"{"emails":["a@example.com"]}"#)).await).await;
        assert_eq!(out["results"][0]["outcome"], "not_set_up");
        assert_eq!(relay.store.waitlist().await.unwrap()[0].invite, None);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn the_page_is_served_locked_down() {
        let res = page().await;
        let csp = res.headers().get("content-security-policy").unwrap().to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'") && csp.contains("connect-src 'self'"));
        assert_eq!(res.headers().get("cache-control").unwrap(), "no-store");
        assert_eq!(script().await.status(), StatusCode::OK);
        assert_eq!(style().await.status(), StatusCode::OK);
    }
}
