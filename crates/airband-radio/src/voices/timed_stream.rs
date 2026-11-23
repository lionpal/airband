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
        let (mut at, mut gap, mut sounded) = (0u32, true, false);
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            // A line that says it is an error, or that does not read, is told, never taken for silence (audit 7,
            // P-N10). One that reads and carries neither sound, timings nor an error (a closing marker) says
            // nothing about the clip and is passed over (audit 9, B-33).
            let part = serde_json::from_str::<Part>(&line).ok().filter(|p| p.error().is_none()).ok_or_else(|| super::unreadable(&self.reach.service))?;
            if !part.is_part() {
                continue;
            }
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
                sounded |= !pcm.is_empty();
                sink(&pcm);
            }
        }
        // a clip that ends with no sound in it was not spoken
        if sounded { Ok(()) } else { Err(super::unreadable(&self.reach.service)) }
    }
}

/// One line of what the service streams back: some of the sound, and where in the text each character of it
/// starts.
#[derive(serde::Deserialize)]
struct Part {
    #[serde(default)]
    audio_base64: Option<String>,
    #[serde(default)]
    alignment: Option<Alignment>,
    /// What the service says went wrong, in its own field for it.
    #[serde(default)]
    detail: Option<serde_json::Value>,
    #[serde(default, rename = "error")]
    error_field: Option<serde_json::Value>,
}
impl Part {
    /// What the service said went wrong, when the line says it is an error.
    fn error(&self) -> Option<&serde_json::Value> {
        self.detail.as_ref().or(self.error_field.as_ref())
    }
    /// It carries sound or timings: a line with neither is not one.
    fn is_part(&self) -> bool {
        self.audio_base64.is_some() || self.alignment.is_some()
    }
}
#[derive(serde::Deserialize)]
struct Alignment {
    characters: Vec<String>,
    character_start_times_seconds: Vec<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_with_no_sound_and_no_timings_is_not_taken_for_silence() {
        let read = |line: &str| serde_json::from_str::<Part>(line).ok().filter(|p| p.error().is_none()).map(|p| p.is_part());
        assert_eq!(read(r#"{"detail":{"status":"quota_exceeded","message":"out of credits"}}"#), None);
        assert_eq!(read(r#"{"error":"busy"}"#), None);
        assert_eq!(read("not json"), None);
        // a closing marker with nothing in it is passed over, not taken for an error after the sound (B-33)
        assert_eq!(read("{}"), Some(false));
        assert_eq!(read(r#"{"isFinal":true}"#), Some(false));
        assert_eq!(read(r#"{"audio_base64":"AAAA"}"#), Some(true));
        assert_eq!(read(r#"{"audio_base64":"AAAA","alignment":{"characters":["a"],"character_start_times_seconds":[0.0]}}"#), Some(true));
    }
}
