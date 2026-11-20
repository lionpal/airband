//! Inbound sound: what turns his speech into text. The phone does the listening; the tower only says how, and
//! gives the phone a short-lived pass for the service so the service's key never leaves this machine. Which
//! service is a row of `providers::PROVIDERS`; how to get its pass and how to talk to it is the phone core's
//! (`airband-radio/src/ears.rs`), asked for here by the dialect the row names.
use super::*;
use crate::providers::{By, Job, LENT_LISTENER};
use airband_core::radio::EarsGrant;

/// A pass lasts ten minutes at the shortest of the services; one is handed out for eight.
const PASS: Duration = Duration::from_secs(480);

/// What the phone should listen with right now.
pub(super) async fn grant(app: &Arc<App>) -> EarsGrant {
    let ears = &app.cfg.ears;
    let Some((_, offer)) = ears.offer(Job::Ears) else {
        // none set here: the relay's, when it lends one. Never the phone's own recognition
        return match app.lent() {
            Some((region, token)) => EarsGrant { kind: LENT_LISTENER.into(), region, token, language: "en-US".into(), phrases: phrases(app) },
            None => EarsGrant::default(),
        };
    };
    let By::Listener(kind) = offer.by else { return EarsGrant::default() };
    let (region, language, key) = (ears.get(Job::Ears, "region"), ears.get(Job::Ears, "language"), ears.key.clone());
    let grant = |token: String| EarsGrant { kind: kind.into(), region: region.clone(), token, language: language.clone(), phrases: phrases(app) };
    if let Some((token, when)) = app.ears_pass.lock().unwrap().clone() {
        if when.elapsed() < PASS {
            return grant(token);
        }
    }
    let asked = region.clone();
    match tokio::task::spawn_blocking(move || airband_radio::listening_pass(kind, &asked, &key)).await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(token) => {
            *app.ears_pass.lock().unwrap() = Some((token.clone(), Instant::now()));
            grant(token)
        }
        Err(why) => {
            log(format!("ears: no pass from the listening service ({why}); he types until there is one"));
            EarsGrant::default()
        }
    }
}

/// Words of the trade that a recogniser otherwise hears as everyday ones ("sync" as "sink", "cache" as "cash").
const SOFTWARE: [&str; 40] = [
    "sync", "cache", "repo", "commit", "merge", "rebase", "deploy", "staging", "prod", "branch", "pull request", "diff", "lint", "regex", "API", "SDK", "CLI", "JSON", "YAML", "SQL",
    "Postgres", "Redis", "Docker", "Kubernetes", "AWS", "S3", "Lambda", "NAS", "SSH", "cron", "webhook", "endpoint", "refactor", "async", "queue", "token", "TestFlight", "Xcode", "Claude", "airband",
];

/// The names he is likely to say, for a recogniser that can be told what to expect.
fn phrases(app: &App) -> Vec<String> {
    let inner = app.inner.lock().unwrap();
    let mut out: Vec<String> = inner.sessions.iter().flat_map(|s| [s.callsign.text(), s.callsign.project.clone()]).collect();
    // he is talking to coding agents: where a sound could be two words, the one from software is the likelier
    out.extend(SOFTWARE.iter().map(|w| w.to_string()));
    out.sort();
    out.dedup();
    out
}
