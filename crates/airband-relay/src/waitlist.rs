//! The site's waitlist: who may sign up, from where, and how often.
use super::*;

/// The pages allowed to post to the waiting list from a browser.
pub(super) const SITES: [&str; 2] = ["https://airband.dev", "https://www.airband.dev"];

/// How many sign-ups are taken: from one address on the internet in an hour, from everyone in a minute, and from
/// everyone in a day. The list is for people typing their own address into one page; anything faster is a
/// script, and the daily figure is what bounds how much a determined one can ever write to the database.
pub(super) const PER_ADDRESS_AN_HOUR: u32 = 5;

pub(super) const A_MINUTE: u32 = 20;

pub(super) const A_DAY: u32 = 2000;


pub(super) struct Signups {
    /// Everyone's, under one key.
    minute: Limiter<()>,
    day: Limiter<()>,
    by_address: Limiter<Caller>,
}

impl Default for Signups {
    fn default() -> Signups {
        Signups { minute: Limiter::new(A_MINUTE, 60), day: Limiter::new(A_DAY, 86_400), by_address: Limiter::new(PER_ADDRESS_AN_HOUR, 3600) }
    }
}

impl Signups {
    /// May this one be taken? Counts it if so, against everyone's minute and day and its own address's hour:
    /// all three or none.
    pub(super) fn allow(&mut self, address: &Caller, now: Instant) -> bool {
        limits::all_or_none(&mut [&mut (&mut self.minute, &()), &mut (&mut self.day, &()), &mut (&mut self.by_address, address)], now).is_ok()
    }
}

/// The headers that let the homepage, and only it, call this from a browser.
/// A page from anywhere else is named nothing, so its browser refuses it.
pub(super) fn from_site(headers: &HeaderMap) -> axum::response::AppendHeaders<Vec<(&'static str, String)>> {
    let mut named = vec![("access-control-allow-methods", "POST, OPTIONS".to_string()), ("access-control-allow-headers", "content-type".into()), ("vary", "origin".into())];
    if let Some(origin) = header(headers, "origin").filter(|o| SITES.contains(o)) {
        named.push(("access-control-allow-origin", origin.to_string()));
    }
    axum::response::AppendHeaders(named)
}

/// Is this plausibly an email address? Whether it is a real one only a message to it can tell. Only the plain
/// characters nearly every address is made of are let through: the list will be read in spreadsheets and mail
/// tools one day, and nothing that could mean something to one of those belongs in it.
pub(super) fn email_shaped(e: &str) -> bool {
    let Some((name, domain)) = e.split_once('@') else { return false };
    let plain = |part: &str, extra: &str| part.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    e.len() <= 254 && !name.is_empty() && plain(name, "._%+-") && plain(domain, ".-") && domain.contains('.') && !domain.starts_with(['.', '-']) && !domain.ends_with(['.', '-']) && !domain.contains("..")
}

pub(super) async fn waitlist_preflight(headers: HeaderMap) -> Response {
    (StatusCode::NO_CONTENT, from_site(&headers)).into_response()
}

/// What came of the TestFlight invite, as the page is told it. An address that was already a tester is told
/// `Sent` like a new one, so the answer never says who was on the list before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InviteAnswer {
    /// Apple has it: the invite comes from Apple by email.
    Sent,
    /// This relay does not send invites yet: the address is kept for when it does.
    NotSetUp,
    /// Asked for and not sent: the address is kept, and sent again with `airband-relay invite-waiting`.
    Failed,
}

/// What the sign-up form is answered.
#[derive(serde::Serialize)]
struct Answer {
    ok: bool,
    message: String,
    /// Only once the address is kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    invite: Option<InviteAnswer>,
}

/// What the page is told for each [`InviteAnswer`].
fn told(invite: InviteAnswer) -> &'static str {
    match invite {
        InviteAnswer::Sent => "Check your email for the TestFlight invite.",
        InviteAnswer::NotSetUp => "You're on the list. Invites aren't going out yet, and yours will come by email.",
        InviteAnswer::Failed => "You're on the list. The invite didn't go out just now. We'll send it soon.",
    }
}

/// How long Apple is given to take one invite, start to end.
const INVITE_WITHIN: Duration = Duration::from_secs(15);

/// Ask for an address's invite and record it once Apple has it. Logs what came of it, never the address.
pub(super) async fn invite_and_record(relay: &Relay, email: &str) -> InviteAnswer {
    let began = Instant::now();
    let outcome = tokio::time::timeout(INVITE_WITHIN, relay.invites.invite(email)).await.unwrap_or(testflight::Outcome::Failed(testflight::Failure::TimedOut));
    let ms = began.elapsed().as_millis();
    match outcome {
        testflight::Outcome::Invited(how) => {
            let recorded = tokio::time::timeout(Duration::from_secs(8), async {
                let _turn = relay.writing.acquire().await.map_err(|e| e.to_string())?;
                relay.store.invited(email, how, airband_core::clock::now_secs()).await
            })
            .await
            .unwrap_or_else(|_| Err("the database did not answer in time".into()));
            match recorded {
                Ok(()) => println!("airband-relay: waitlist: invite sent ({}, {ms} ms)", how.name()),
                // it went out all the same; sent again, Apple answers that it is already a tester, and that is recorded
                Err(e) => eprintln!("airband-relay: waitlist: invite sent ({}, {ms} ms) but not recorded: {e}", how.name()),
            }
            InviteAnswer::Sent
        }
        testflight::Outcome::NotSetUp(why) => {
            println!("airband-relay: waitlist: no invite sent, not set up ({why:?})");
            InviteAnswer::NotSetUp
        }
        testflight::Outcome::Failed(why) => {
            eprintln!("airband-relay: waitlist: invite not sent ({ms} ms): {why}");
            InviteAnswer::Failed
        }
    }
}

/// Someone on the homepage asks for the app.
///
/// What this guards against. Writing to the database from a script: the three limits above, counted before the
/// database is touched, and at most two writes at once, so the server it shares with other applications is never
/// flooded with connections. Other websites posting on a visitor's behalf: only JSON is taken, which a browser
/// will not send across sites without asking first, and only the homepage is told yes. Form-filling robots: a
/// field no person can see, which they fill in. Finding out who is on the list: the answer is the same whether
/// or not the address was already there, or already a tester. Injection: the address is only ever passed to the
/// database and to Apple as a value, is never logged and is never shown on a page.
///
/// Once the address is kept, Apple is asked to invite it to TestFlight (testflight.rs), and Apple's email is
/// what proves the address belongs to whoever typed it: nothing else is ever sent to it.
pub(super) async fn waitlist(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    let cors = from_site(&headers);
    let answer = |code: StatusCode, why: &str, invite: Option<InviteAnswer>| (code, cors.clone(), axum::Json(Answer { ok: code == StatusCode::OK, message: why.to_string(), invite })).into_response();
    let origin = header(&headers, "origin").unwrap_or("");
    if !SITES.contains(&origin) {
        return answer(StatusCode::FORBIDDEN, "Sign up on airband.dev.", None);
    }
    if !header(&headers, "content-type").is_some_and(|t| t.starts_with("application/json")) {
        return answer(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.", None);
    }
    if body.len() > 1024 {
        return answer(StatusCode::PAYLOAD_TOO_LARGE, "That is too long to be an email address.", None);
    }
    let Ok(sent) = serde_json::from_slice::<serde_json::Value>(&body) else { return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.", None) };
    let email = sent["email"].as_str().map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if !email_shaped(&email) {
        return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.", None);
    }
    // the field nobody can see: a person leaves it empty. Answer as if sent and keep nothing, so the robot learns nothing.
    if sent["company"].as_str().is_some_and(|c| !c.is_empty()) {
        return answer(StatusCode::OK, told(InviteAnswer::Sent), Some(InviteAnswer::Sent));
    }
    if !relay.signups.lock().unwrap().allow(&caller(&headers), Instant::now()) {
        return answer(StatusCode::TOO_MANY_REQUESTS, "Too many sign-ups right now. Try again later.", None);
    }
    {
        let Ok(_turn) = relay.writing.try_acquire() else { return answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.", None) };
        match tokio::time::timeout(Duration::from_secs(8), relay.store.join_waitlist(&email)).await {
            Ok(Ok(())) => {}
            failed => {
                eprintln!("airband-relay: could not save a waitlist address: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
                return answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.", None);
            }
        }
    }
    let invite = invite_and_record(&relay, &email).await;
    answer(StatusCode::OK, told(invite), Some(invite))
}

/// `airband-relay invite-waiting`: ask for the invite of every address on the list that has none on record, one
/// at a time, and record each Apple takes. Says how many of each, never an address. Stops at once if the relay
/// is not set up to invite, and leaves every one it could not invite for next time.
pub(super) async fn invite_waiting(relay: &Relay) -> Result<(), String> {
    let waiting = relay.store.not_yet_invited().await?;
    println!("{} on the list without an invite", waiting.len());
    let (mut sent, mut failed) = (0, 0);
    for (i, email) in waiting.iter().enumerate() {
        match invite_and_record(relay, email).await {
            InviteAnswer::Sent => sent += 1,
            InviteAnswer::NotSetUp => return Err(format!("not set up to send invites; {sent} sent before that")),
            InviteAnswer::Failed => failed += 1,
        }
        if (i + 1) % 25 == 0 {
            println!("{} of {}: {sent} sent, {failed} not", i + 1, waiting.len());
        }
    }
    println!("{sent} sent (or already testers), {failed} not sent");
    if failed == 0 { Ok(()) } else { Err(format!("{failed} not sent; run it again to try those")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(a: &str) -> Caller {
        Caller::At(a.into())
    }

    /// A relay on a folder of its own, inviting through `invites`; the sign-up as the homepage sends it.
    fn relay_with(invites: testflight::Invites) -> (Arc<Relay>, std::path::PathBuf) {
        let folder = std::env::temp_dir().join(format!("airband-relay-waitlist-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::folder(folder.clone()), None, invites)), folder)
    }
    async fn sign_up(relay: &Arc<Relay>, body: &str) -> (StatusCode, serde_json::Value) {
        let mut h = HeaderMap::new();
        h.insert("origin", "https://airband.dev".parse().unwrap());
        h.insert("content-type", "application/json".parse().unwrap());
        let res = waitlist(State(relay.clone()), h, Bytes::from(body.to_string())).await;
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn a_sign_up_is_kept_and_invited_and_the_page_is_told_which() {
        let (secret, apple) = testflight::tests::key_and_apple();
        let (relay, folder) = relay_with(testflight::Invites::given(Some(secret), &testflight::tests::pretend_apple(apple.clone()).await));
        let (status, out) = sign_up(&relay, r#"{"email":" New@Example.com ","company":""}"#).await;
        assert_eq!((status, out["ok"].as_bool(), out["invite"].as_str()), (StatusCode::OK, Some(true), Some("sent")));
        assert_eq!(out["message"], "Check your email for the TestFlight invite.");
        assert_eq!(relay.store.not_yet_invited().await, Ok(vec![]));
        // again: the same answer, though Apple had it already
        let (_, again) = sign_up(&relay, r#"{"email":"new@example.com"}"#).await;
        assert_eq!(again, out);
        // Apple says no: kept, the page is told, and the address waits for the next run
        apple.lock().unwrap().refuse = Some((409, "STATE_ERROR"));
        let (status, out) = sign_up(&relay, r#"{"email":"later@example.com"}"#).await;
        assert_eq!((status, out["invite"].as_str()), (StatusCode::OK, Some("failed")));
        assert_eq!(relay.store.not_yet_invited().await, Ok(vec!["later@example.com".to_string()]));
        // the run that sends what is waiting, once Apple takes it
        assert!(invite_waiting(&relay).await.is_err());
        apple.lock().unwrap().refuse = None;
        invite_waiting(&relay).await.unwrap();
        assert_eq!(relay.store.not_yet_invited().await, Ok(vec![]));
        assert_eq!(apple.lock().unwrap().testers.len(), 2);
        // the robot's field: told the same, and nothing kept or sent
        let (_, robot) = sign_up(&relay, r#"{"email":"bot@example.com","company":"x"}"#).await;
        assert_eq!(robot["invite"].as_str(), Some("sent"));
        assert_eq!(apple.lock().unwrap().testers.len(), 2);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn without_a_key_the_address_is_kept_and_the_page_is_told_so() {
        let (relay, folder) = relay_with(testflight::Invites::unnamed());
        let (status, out) = sign_up(&relay, r#"{"email":"a@example.com"}"#).await;
        assert_eq!((status, out["invite"].as_str()), (StatusCode::OK, Some("not_set_up")));
        assert_eq!(relay.store.not_yet_invited().await, Ok(vec!["a@example.com".to_string()]));
        assert!(invite_waiting(&relay).await.is_err());
        let (relay, _) = (relay_with(testflight::Invites::given(None, "http://127.0.0.1:1")).0, ());
        assert_eq!(sign_up(&relay, r#"{"email":"a@example.com"}"#).await.1["invite"].as_str(), Some("not_set_up"));
        // a refusal before the address is kept says nothing about invites
        let (status, out) = sign_up(&relay, r#"{"email":"nope"}"#).await;
        assert_eq!((status, out.get("invite")), (StatusCode::BAD_REQUEST, None));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn what_passes_for_an_email_address() {
        for good in ["c@example.games", "a.b+c@example.co.uk"] {
            assert!(email_shaped(good), "{good}");
        }
        for bad in ["", "nobody", "@example.com", "a@b", "a@.com", "a@b.com.", "a b@example.com", "a@b@c.com", "x');drop--@example.com", "=cmd|calc@example.com", "<b>@example.com", "a@exa..mple.com"] {
            assert!(!email_shaped(bad), "{bad}");
        }
    }

    #[test]
    fn sign_ups_are_counted_per_caller_per_minute_and_per_day() {
        let (mut s, t0) = (Signups::default(), Instant::now());
        // one caller: five in an hour, and again after the hour
        assert!((0..5).all(|_| s.allow(&at("1.1.1.1"), t0)) && !s.allow(&at("1.1.1.1"), t0));
        assert!(s.allow(&at("1.1.1.1"), t0 + Duration::from_secs(3601)));
        // everyone together: twenty in a minute, whoever they are
        let (mut s, mut taken) = (Signups::default(), 0);
        for i in 0..40 {
            taken += s.allow(&at(&format!("2.2.2.{i}")), t0) as u32;
        }
        assert_eq!(taken, A_MINUTE);
        assert!(s.allow(&at("3.3.3.3"), t0 + Duration::from_secs(61)));
        // and a day's worth, however slowly they come
        let mut s = Signups::default();
        let taken = (0..3000u64).filter(|i| s.allow(&at(&format!("a{i}")), t0 + Duration::from_secs(i * 20))).count() as u32;
        assert_eq!(taken, A_DAY);
    }

    #[test]
    fn the_caller_is_the_address_the_load_balancer_added() {
        let mut h = HeaderMap::new();
        assert_eq!(caller(&h), Caller::Unseen);
        h.insert("x-forwarded-for", "9.9.9.9, 203.0.113.7".parse().unwrap()); // the first is whatever the caller sent
        assert_eq!(caller(&h), Caller::At("203.0.113.7".into()));
    }

    #[test]
    fn only_the_homepage_is_named_as_an_allowed_origin() {
        let mut h = HeaderMap::new();
        h.insert("origin", "https://evil.example".parse().unwrap());
        let named = |h: &HeaderMap| from_site(h).0.into_iter().find(|(n, _)| *n == "access-control-allow-origin").map(|(_, v)| v);
        assert_eq!(named(&h), None, "another site is named nothing");
        h.insert("origin", "https://www.airband.dev".parse().unwrap());
        assert_eq!(named(&h).as_deref(), Some("https://www.airband.dev"));
    }
}
