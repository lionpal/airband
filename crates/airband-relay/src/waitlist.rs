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
        if !(self.minute.room(&(), now) && self.day.room(&(), now) && self.by_address.room(address, now)) {
            return false;
        }
        self.minute.allow(&(), now) && self.day.allow(&(), now) && self.by_address.allow(address, now)
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

/// Someone on the homepage asks to be told when the app is ready.
///
/// What this guards against. Writing to the database from a script: the three limits above, counted before the
/// database is touched, and at most two writes at once, so the server it shares with other applications is never
/// flooded with connections. Other websites posting on a visitor's behalf: only JSON is taken, which a browser
/// will not send across sites without asking first, and only the homepage is told yes. Form-filling robots: a
/// field no person can see, which they fill in. Finding out who is on the list: the answer is the same whether
/// or not the address was already there. Injection: the address is only ever passed to the database as a value,
/// is never logged and is never shown on a page.
/// What it does not do: prove the address belongs to whoever typed it. Nothing is sent to it yet; when something
/// is, the first message must ask for confirmation.
/// What the sign-up form is answered.
#[derive(serde::Serialize)]
struct Answer {
    ok: bool,
    message: String,
}

pub(super) async fn waitlist(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    let cors = from_site(&headers);
    let answer = |code: StatusCode, why: &str| (code, cors.clone(), axum::Json(Answer { ok: code == StatusCode::OK, message: why.to_string() })).into_response();
    let origin = header(&headers, "origin").unwrap_or("");
    if !SITES.contains(&origin) {
        return answer(StatusCode::FORBIDDEN, "Sign up on airband.dev.");
    }
    if !header(&headers, "content-type").is_some_and(|t| t.starts_with("application/json")) {
        return answer(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.");
    }
    if body.len() > 1024 {
        return answer(StatusCode::PAYLOAD_TOO_LARGE, "That is too long to be an email address.");
    }
    let Ok(sent) = serde_json::from_slice::<serde_json::Value>(&body) else { return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.") };
    let email = sent["email"].as_str().map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if !email_shaped(&email) {
        return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.");
    }
    // the field nobody can see: a person leaves it empty. Say yes and keep nothing, so the robot learns nothing.
    if sent["company"].as_str().is_some_and(|c| !c.is_empty()) {
        return answer(StatusCode::OK, "You are on the list.");
    }
    if !relay.signups.lock().unwrap().allow(&caller(&headers), Instant::now()) {
        return answer(StatusCode::TOO_MANY_REQUESTS, "Too many sign-ups right now. Try again later.");
    }
    let Ok(_turn) = relay.writing.try_acquire() else { return answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.") };
    match tokio::time::timeout(Duration::from_secs(8), relay.store.join_waitlist(&email)).await {
        Ok(Ok(())) => answer(StatusCode::OK, "You are on the list."),
        failed => {
            eprintln!("airband-relay: could not save a waitlist address: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(a: &str) -> Caller {
        Caller::At(a.into())
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
