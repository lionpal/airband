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

/// How many internet addresses are remembered at once for the hourly count.
pub(super) const ADDRESSES_KEPT: usize = 20_000;

#[derive(Default)]
pub(super) struct Signups {
    pub(super) minute: Option<(Instant, u32)>,
    pub(super) day: Option<(Instant, u32)>,
    pub(super) by_address: HashMap<String, (Instant, u32)>,
}

impl Signups {
    /// May this one be taken? Counts it if so.
    pub(super) fn allow(&mut self, address: &str, now: Instant) -> bool {
        let fresh = |slot: &Option<(Instant, u32)>, window: u64| slot.filter(|(began, _)| now.duration_since(*began) < Duration::from_secs(window)).unwrap_or((now, 0));
        let (minute, day) = (fresh(&self.minute, 60), fresh(&self.day, 86_400));
        self.by_address.retain(|_, (began, _)| now.duration_since(*began) < Duration::from_secs(3600));
        let mine = self.by_address.get(address).copied().unwrap_or((now, 0));
        // an address not seen before, with the table full: refuse rather than forget who has already had their five
        let room = self.by_address.contains_key(address) || self.by_address.len() < ADDRESSES_KEPT;
        if minute.1 >= A_MINUTE || day.1 >= A_DAY || mine.1 >= PER_ADDRESS_AN_HOUR || !room {
            return false;
        }
        self.minute = Some((minute.0, minute.1 + 1));
        self.day = Some((day.0, day.1 + 1));
        self.by_address.insert(address.to_string(), (mine.0, mine.1 + 1));
        true
    }
}

/// The headers that let the homepage, and only it, call this from a browser.
pub(super) fn from_site(headers: &HeaderMap) -> [(&'static str, String); 4] {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).filter(|o| SITES.contains(o)).unwrap_or(SITES[0]);
    [("access-control-allow-origin", origin.to_string()), ("access-control-allow-methods", "POST, OPTIONS".into()), ("access-control-allow-headers", "content-type".into()), ("vary", "origin".into())]
}

/// Is this plausibly an email address? Whether it is a real one only a message to it can tell. Only the plain
/// characters nearly every address is made of are let through: the list will be read in spreadsheets and mail
/// tools one day, and nothing that could mean something to one of those belongs in it.
pub(super) fn email_shaped(e: &str) -> bool {
    let Some((name, domain)) = e.split_once('@') else { return false };
    let plain = |part: &str, extra: &str| part.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    e.len() <= 254 && !name.is_empty() && plain(name, "._%+-") && plain(domain, ".-") && domain.contains('.') && !domain.starts_with(['.', '-']) && !domain.ends_with(['.', '-']) && !domain.contains("..")
}

/// Who is asking, as the load balancer saw them. It adds the caller's address to the end of this header, so the
/// last entry is the one a caller cannot make up; anything before it is whatever they chose to send.
pub(super) fn caller(headers: &HeaderMap) -> String {
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| "direct".into())
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
pub(super) async fn waitlist(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    let cors = from_site(&headers);
    let answer = |code: StatusCode, why: &str| (code, cors.clone(), axum::Json(serde_json::json!({ "ok": code == StatusCode::OK, "message": why }))).into_response();
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !SITES.contains(&origin) {
        return answer(StatusCode::FORBIDDEN, "Sign up on airband.dev.");
    }
    if !headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("application/json")) {
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
