//! Inbound sound: what turns his speech into text. The phone does the listening; the tower only says how, and
//! when a service is used, gives the phone a short-lived pass for it so the service's key never leaves this
//! machine. A new listening service is an arm in [`grant`] here, its settings in `config::Ears`, and the code
//! that talks to it in `airband-radio/src/ears.rs`.
use super::*;
use airband_core::radio::EarsGrant;

/// A pass from Microsoft lasts ten minutes; one is handed out for eight.
const AZURE_PASS: Duration = Duration::from_secs(480);

/// What the phone should listen with right now.
pub(super) async fn grant(app: &Arc<App>) -> EarsGrant {
    match app.cfg.ears.clone() {
        // none set here: the relay's, when it lends one. Never the phone's own recognition
        config::Ears::Device => match app.lent() {
            Some((region, token)) => EarsGrant { kind: "azure".into(), region, token, language: "en-US".into(), phrases: phrases(app) },
            None => EarsGrant::default(),
        },
        config::Ears::Azure { key, region, language } => azure(app, key, region, language).await,
    }
}

async fn azure(app: &Arc<App>, key: String, region: String, language: String) -> EarsGrant {
    let grant = |token: String| EarsGrant { kind: "azure".into(), region: region.clone(), token, language: language.clone(), phrases: phrases(app) };
    if let Some((token, when)) = app.ears_pass.lock().unwrap().clone() {
        if when.elapsed() < AZURE_PASS {
            return grant(token);
        }
    }
    let url = format!("https://{region}.api.cognitive.microsoft.com/sts/v1.0/issueToken");
    let minted = tokio::task::spawn_blocking(move || {
        let mut res = airband_radio::net::agent(Duration::from_secs(8)).post(&url).header("Ocp-Apim-Subscription-Key", &key).header("Content-Length", "0").send_empty().ok()?;
        (res.status().as_u16() == 200).then(|| res.body_mut().read_to_string().ok()).flatten()
    })
    .await
    .ok()
    .flatten();
    match minted {
        Some(token) => {
            *app.ears_pass.lock().unwrap() = Some((token.clone(), Instant::now()));
            grant(token)
        }
        None => {
            log("ears: Microsoft gave no pass; the phone will use its own recognition");
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
