//! What Anthropic's account service is asked: whose a token is, how much of an account is used, and a new
//! access token for a saved login. Every answer is read by its status and its own fields; one that cannot be
//! read is said to be so, never taken for an empty one.
#![expect(clippy::disallowed_methods, reason = "asks the account's own service over the network")]
#![expect(clippy::disallowed_macros, reason = "the account service's own request bodies")]
use super::{maker, Here, Res, ADD_AGAIN};
use airband_core::radio::Limit;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

fn http() -> ureq::Agent {
    airband_io::http::agent(Duration::from_secs(20))
}

/// One question to the account service with a login's token: its status and its answer's text.
fn ask(here: &Here, path: &str, token: &str) -> Result<(u16, String), String> {
    let mut res = http()
        .get(format!("{}{path}", here.api))
        .header("authorization", format!("Bearer {token}"))
        .header(&super::row("beta_header"), super::row("oauth_beta"))
        .call()
        .map_err(|_| format!("Could not reach {} to check it.", maker()))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().map_err(|_| format!("{}'s answer could not be read.", maker()))?;
    Ok((status, text))
}

/// The profile answer, as far as an account record is made from it.
#[derive(Deserialize)]
struct Profile {
    account: ProfileAccount,
    organization: ProfileOrg,
}
#[derive(Deserialize)]
struct ProfileAccount {
    uuid: String,
    email: String,
    display_name: Option<String>,
    created_at: Option<Value>,
}
#[derive(Deserialize)]
struct ProfileOrg {
    uuid: String,
    has_extra_usage_enabled: Option<bool>,
    cc_onboarding_flags: Option<Value>,
    claude_code_trial_ends_at: Option<Value>,
    claude_code_trial_duration_days: Option<Value>,
    seat_tier: Option<Value>,
    billing_type: Option<Value>,
    subscription_created_at: Option<Value>,
}

/// Whose token this is: its email, and the account record Claude Code keeps for that account (`oauthAccount`).
/// Nothing when the server cannot say (expired, offline), or says it in a shape that lacks the account's own
/// fields.
pub(super) fn profile(here: &Here, token: &str) -> Option<(String, Value)> {
    match ask(here, "/api/oauth/profile", token).ok()? {
        (200, text) => account_record(&serde_json::from_str(&text).ok()?),
        _ => None,
    }
}

/// The `oauthAccount` record for a profile answer, field for field as Claude Code writes it. A field the
/// profile does not have is left out, never filled in.
pub(super) fn account_record(v: &Value) -> Option<(String, Value)> {
    let p = Profile::deserialize(v).ok()?;
    let (a, o) = (p.account, p.organization);
    let mut record = json!({
        "accountUuid": a.uuid,
        "emailAddress": a.email,
        "organizationUuid": o.uuid,
    });
    let given = [
        ("accountCreatedAt", a.created_at),
        ("claudeCodeTrialEndsAt", o.claude_code_trial_ends_at),
        ("claudeCodeTrialDurationDays", o.claude_code_trial_duration_days),
        ("seatTier", o.seat_tier),
        ("hasExtraUsageEnabled", o.has_extra_usage_enabled.map(Value::Bool)),
        ("ccOnboardingFlags", o.cc_onboarding_flags),
        ("billingType", o.billing_type),
        ("subscriptionCreatedAt", o.subscription_created_at),
        ("displayName", a.display_name.filter(|n| !n.is_empty()).map(Value::String)),
    ];
    for (field, value) in given {
        if let Some(value) = value {
            record[field] = value;
        }
    }
    Some((a.email, record))
}

/// A new login for a saved one, from its refresh token: the login as it was, with the new tokens and times.
pub(super) fn refresh(here: &Here, o: &Value) -> Res<Value> {
    let refresh = o.get("refreshToken").and_then(|x| x.as_str()).filter(|x| !x.is_empty()).ok_or(ADD_AGAIN)?;
    let scopes = o.get("scopes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    // the login says which client it was made by; one that does not was made by Claude Code's own, as its row has it
    let client = o.get("clientId").and_then(|x| x.as_str()).map(String::from).unwrap_or_else(|| super::row("oauth_client"));
    let mut res = http()
        .post(&here.token_url)
        .header("content-type", "application/json")
        .send(json!({ "grant_type": "refresh_token", "refresh_token": refresh, "client_id": client, "scope": scopes }).to_string())
        .map_err(|_| format!("Could not reach {} to check it.", maker()))?;
    let status = res.status().as_u16();
    let body: Option<Value> = res.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok());
    // OAuth names a refusal of the refresh token itself `invalid_grant` (RFC 6749, section 5.2)
    let refused = body.as_ref().and_then(|b| b.get("error")).and_then(|e| e.as_str()) == Some("invalid_grant");
    let body = match status {
        200 => body.ok_or_else(|| format!("{}'s new login for this account could not be read. Try again later.", maker()))?,
        401 => return Err(ADD_AGAIN.into()),
        400 if refused => return Err(ADD_AGAIN.into()),
        429 => return Err(format!("{} is busy just now. Try again in a minute.", maker())),
        500..=599 => return Err(format!("{} had trouble just now ({status}). Try again later.", maker())),
        n => return Err(format!("{} would not refresh this login ({n}).", maker())),
    };
    let new_token = body.get("access_token").and_then(|x| x.as_str()).ok_or_else(|| format!("{}'s new login for this account came without a token. Try again later.", maker()))?;
    let mut n = o.clone();
    n["accessToken"] = json!(new_token);
    if let Some(r) = body.get("refresh_token").and_then(|x| x.as_str()) {
        n["refreshToken"] = json!(r);
    }
    let now = airband_core::clock::now_ms() as i64;
    if let Some(e) = body.get("expires_in").and_then(|x| x.as_i64()) {
        n["expiresAt"] = json!(now + e * 1000);
    }
    if let Some(e) = body.get("refresh_token_expires_in").and_then(|x| x.as_i64()) {
        n["refreshTokenExpiresAt"] = json!(now + e * 1000);
    }
    Ok(n)
}

/// Why an account's allowance could not be read.
pub(super) enum Unchecked {
    /// The service refused the login it was asked with.
    Expired,
    /// Anything else, in words for him.
    Said(String),
}
impl From<String> for Unchecked {
    fn from(said: String) -> Self {
        Unchecked::Said(said)
    }
}

/// What Anthropic's usage answer says, as limits: the week, then the 5 hours when that window is open.
pub(super) fn limits_in(usage: &Value) -> Vec<Limit> {
    [("week", "seven_day"), ("5 hours", "five_hour")]
        .into_iter()
        .filter_map(|(name, key)| {
            let w = usage.get(key)?;
            Some(Limit { name: name.into(), used: (w.get("utilization")?.as_f64()? / 100.0).clamp(0.0, 1.0), resets_at: w.get("resets_at").and_then(|r| r.as_str()).and_then(airband_core::clock::parse_rfc3339) })
        })
        .collect()
}

/// How much of an account is used. Costs nothing: it is the same question Claude Code's own usage page asks.
pub(super) fn usage(here: &Here, token: &str) -> Result<Vec<Limit>, Unchecked> {
    match ask(here, "/api/oauth/usage", token)? {
        (200, text) => serde_json::from_str::<Value>(&text).map(|u| limits_in(&u)).map_err(|_| Unchecked::Said(format!("{}'s answer about how much is used could not be read.", maker()))),
        (401 | 403, _) => Err(Unchecked::Expired),
        (n, _) => Err(Unchecked::Said(format!("{} would not say how much is used ({n}).", maker()))),
    }
}
