//! Speech as a stream that says when each word is spoken: a line of JSON at a time, each with a piece of the
//! sound and when each character of it is said, counted from the start of the clip. The row says where it is
//! reached (`url`, which names his voice in braces), how the key is presented (`auth`) and the rate of what
//! comes back (`rate`).
#![expect(clippy::disallowed_macros, reason = "a voice service's own request body")]
use super::{Made, Reach, Synth, Voicing};
use airband_core::encode::unbase64;
use airband_core::providers::{By, Interface, Job, Provider, Use};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "timed-stream", can: &[], told: &[crate::models::field::URL, crate::models::field::AUTH, crate::models::field::RATE] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    Ok(Box::new(Timed { reach: Reach::of(voice, by, provider)?, model: voice.get(Job::Voice, crate::models::field::MODEL) }))
}

struct Timed {
    reach: Reach,
    model: String,
}

impl Synth for Timed {
    fn rate(&self) -> u32 {
        self.reach.rate
    }
    fn service(&self) -> &super::Answering {
        &self.reach.service
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let res = self.reach.sent(Duration::from_secs(60), self.reach.post().json(&json!({ "text": text, "model_id": self.model })))?;
        // where in the text the service has got to, and whether the character before was a gap between words
        let (mut at, mut gap) = (0u32, true);
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            // a line that is not a part is the service saying something else (an error, most likely): it is
            // told, never taken for silence
            let part = serde_json::from_str::<Part>(&line).map_err(|_| unreadable(&self.reach.service))?;
            // the timings, read as the service sends them; a part whose timings do not read marks nothing
            if let Some(Alignment { characters, character_start_times_seconds: starts }) = part.alignment {
                for (c, start) in characters.iter().zip(starts) {
                    let space = c.trim().is_empty();
                    if gap && !space {
                        mark(at, (start * 1000.0) as u32);
                    }
                    gap = space;
                    at += c.chars().count() as u32;
                }
            }
            if let Some(pcm) = part.audio_base64.as_deref().and_then(unbase64) {
                sink(&pcm);
            }
        }
        Ok(())
    }
}

/// What he is told when the service's stream holds something this build cannot read.
fn unreadable(service: &super::Answering) -> String {
    format!("{} sent something this version of airband cannot read.", service.name)
}

/// One line of what the service streams back: some of the sound, and where in the text each character of it
/// starts.
#[derive(serde::Deserialize)]
struct Part {
    #[serde(default)]
    audio_base64: Option<String>,
    #[serde(default)]
    alignment: Option<Alignment>,
}
#[derive(serde::Deserialize)]
struct Alignment {
    characters: Vec<String>,
    character_start_times_seconds: Vec<f64>,
}
