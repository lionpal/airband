//! Inbound sound: what turns his speech into text. The phone does the listening; the tower only says how, and
//! gives the phone a short-lived pass for the service so the service's key never leaves this machine. Which
//! service is a row of `providers::PROVIDERS`; how to get its pass and how to talk to it is the phone core's
//! (`airband-radio/src/ears.rs`), asked for here by the dialect the row names.
use super::*;
use crate::providers::{BY_TOWER, By, Job};
use airband_core::radio::EarsGrant;

/// A pass lasts ten minutes at the shortest of the services; one is handed out for eight.
const PASS: Duration = Duration::from_secs(480);

/// What the phone should listen with right now.
pub(super) async fn grant(app: &Arc<App>) -> EarsGrant {
    let ears = &app.cfg.ears;
    let none = EarsGrant { phrases: phrases(app), ..EarsGrant::default() };
    let Some((_, offer)) = ears.offer(Job::Ears) else {
        // none set here: the phone listens with a model or a key of its own, or he types
        return none;
    };
    let set = ears.settings(Job::Ears);
    let kind = match offer.by {
        By::Listener(kind) => kind,
        // a service that takes a finished recording: the phone sends it here, where the key is
        By::Transcribe => return EarsGrant { kind: BY_TOWER.into(), token: String::new(), set: Default::default(), phrases: Vec::new() },
        _ => return none,
    };
    let grant = |token: String| EarsGrant { kind: kind.into(), token, set: set.clone(), phrases: phrases(app) };
    if let Some((token, when)) = app.ears_pass.lock().unwrap().clone() {
        if when.elapsed() < PASS && !airband_radio::listening_pass_once(kind) {
            return grant(token);
        }
    }
    let (asked, key) = (set.clone(), ears.key.clone());
    match tokio::task::spawn_blocking(move || airband_radio::listening_pass(kind, &asked, &key)).await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(token) => {
            *app.ears_pass.lock().unwrap() = Some((token.clone(), Instant::now()));
            grant(token)
        }
        Err(why) => {
            log(format!("ears: no pass from the listening service ({why}); he types until there is one"));
            none
        }
    }
}

/// How long a recording that stopped arriving part way is kept.
const CLIP_KEPT: Duration = Duration::from_secs(120);

/// Part of a recording from a phone, for a listening service that takes a finished clip. The parts are kept
/// until the last, then the service is asked what was said, with the key that stays on this computer.
pub(super) async fn hear(app: &Arc<App>, part: airband_core::radio::HearPart) -> Result<String, String> {
    let blocks = airband_core::audio::base64_decode(&part.data).ok_or("that was not a recording")?;
    let mut left = part.samples as usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(left);
    for block in blocks.chunks(airband_core::audio::BLOCK_BYTES) {
        let n = left.min(airband_core::audio::BLOCK_SAMPLES);
        pcm.extend(airband_core::audio::decode_block(block, n));
        left -= n;
    }
    let whole = {
        let mut clips = app.clips.lock().unwrap();
        clips.retain(|_, (_, when)| when.elapsed() < CLIP_KEPT);
        let (kept, when) = clips.entry(part.clip.clone()).or_insert_with(|| (Vec::new(), Instant::now()));
        // five minutes of speech is more than anyone says in one go
        if kept.len() + pcm.len() > 16000 * 300 {
            clips.remove(&part.clip);
            return Err("that recording is too long".into());
        }
        kept.extend(pcm);
        *when = Instant::now();
        if !part.last {
            return Ok(String::new());
        }
        clips.remove(&part.clip).map(|(pcm, _)| pcm).unwrap_or_default()
    };
    let (with, expect) = (app.cfg.ears.clone(), phrases(app));
    tokio::task::spawn_blocking(move || airband_radio::hears::transcribe(&with, &airband_radio::hears::wav(16000, &whole), &expect)).await.map_err(|e| e.to_string())?
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
