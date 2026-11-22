//! Speech as a stream that says when each word is spoken: a line of JSON at a time, each with a piece of the
//! sound and when each character of it is said, counted from the start of the clip. The row says where it is
//! reached (`url`, which names his voice in braces), how the key is presented (`auth`) and the rate of what
//! comes back (`rate`).
use super::{Made, Reach, Synth, Voicing};
use airband_core::encode::unbase64;
use airband_core::providers::{By, Interface, Job, Provider, Use};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "timed-stream", can: &[], told: &["url", "auth", "rate"] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    Ok(Box::new(Timed { reach: Reach::of(voice, by, provider)?, model: voice.get(Job::Voice, "model") }))
}

struct Timed {
    reach: Reach,
    model: String,
}

impl Synth for Timed {
    fn rate(&self) -> u32 {
        self.reach.rate
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let res = self.reach.post(Duration::from_secs(60)).send_json(json!({ "text": text, "model_id": self.model })).map_err(|_| self.reach.unreachable())?;
        if res.status().as_u16() != 200 {
            return Err(self.reach.answered(res.status().as_u16()));
        }
        // where in the text the service has got to, and whether the character before was a gap between words
        let (mut at, mut gap) = (0u32, true);
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let (characters, starts) = (v["alignment"]["characters"].as_array(), v["alignment"]["character_start_times_seconds"].as_array());
            for (c, start) in characters.into_iter().flatten().zip(starts.into_iter().flatten()) {
                let space = c.as_str().is_none_or(|c| c.trim().is_empty());
                if gap && !space {
                    mark(at, (start.as_f64().unwrap_or(0.0) * 1000.0) as u32);
                }
                gap = space;
                at += c.as_str().map(|c| c.chars().count() as u32).unwrap_or(1);
            }
            if let Some(pcm) = v["audio_base64"].as_str().and_then(unbase64) {
                sink(&pcm);
            }
        }
        Ok(())
    }
}
