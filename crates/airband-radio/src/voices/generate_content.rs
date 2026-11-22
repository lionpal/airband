//! Speech as generated content: the words go up as a content request that asks for audio, and the sound comes
//! back a piece at a time as server-sent events. The row says where it is reached (`url`, which names his
//! model in braces), how the key is presented (`auth`), how the words are to be said (`style`) and the rate of
//! what comes back (`rate`).
use super::{Made, Reach, Synth, Voicing};
use airband_core::audio::base64_decode;
use airband_core::providers::{By, Interface, Job, Provider, Use};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "generate-content", can: &[], told: &["url", "style", "auth", "rate"] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    Ok(Box::new(Generated { reach: Reach::of(voice, by, provider)?, style: by.text("style").into(), voice: voice.get(Job::Voice, "voice") }))
}

struct Generated {
    reach: Reach,
    /// How to say the words, which goes beside them and is never spoken.
    style: String,
    voice: String,
}

impl Synth for Generated {
    fn rate(&self) -> u32 {
        self.reach.rate
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        // The words go in bare, and how to say them goes beside them in `speech_metadata`. Put the direction in the
        // text and the model reads it aloud: the first version here opened every message with "Read this aloud
        // exactly as written", found by transcribing its own output.
        let body = json!({
            "contents": [{ "parts": [{ "text": text, "speech_metadata": { "style": self.style } }] }],
            "generationConfig": { "responseModalities": ["AUDIO"], "speechConfig": { "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": self.voice } } } },
        });
        let res = self.reach.post(Duration::from_secs(30)).send_json(&body).map_err(|_| self.reach.unreachable())?;
        if res.status().as_u16() != 200 {
            return Err(self.reach.answered(res.status().as_u16()));
        }
        // server-sent events: one JSON document per "data:" line, each with a piece of audio in base64
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Some(data) = line.strip_prefix("data: ") else { continue };
            let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
            for part in v["candidates"][0]["content"]["parts"].as_array().into_iter().flatten() {
                if let Some(pcm) = part["inlineData"]["data"].as_str().and_then(base64_decode) {
                    sink(&pcm);
                }
            }
        }
        Ok(())
    }
}
