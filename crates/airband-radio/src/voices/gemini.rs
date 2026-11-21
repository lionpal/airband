//! Google's own speech route: the words in, sound back a piece at a time as server-sent events. The row says
//! where (`url`) and how the words are to be said (`style`).
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Use};
use airband_core::audio::base64_decode;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "gemini", can: &[], told: &["url", "style"] }, make };

fn make(voice: &Use, by: &By) -> Made {
    Ok(Box::new(Gemini { url: by.text("url").into(), style: by.text("style").into(), key: voice.key.clone(), model: voice.get(Job::Voice, "model"), voice: voice.get(Job::Voice, "voice") }))
}

struct Gemini {
    url: String,
    /// How to say the words, which goes beside them and is never spoken.
    style: String,
    key: String,
    model: String,
    voice: String,
}

impl Synth for Gemini {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        // The words go in bare, and how to say them goes beside them in `speech_metadata`. Put the direction in the
        // text and this generation of model reads it aloud: the first version here opened every message with
        // "Read this aloud exactly as written", found by transcribing its own output. (Backoffice hit the same thing.)
        let body = json!({
            "contents": [{ "parts": [{ "text": text, "speech_metadata": { "style": self.style } }] }],
            "generationConfig": { "responseModalities": ["AUDIO"], "speechConfig": { "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": self.voice } } } },
        });
        let url = format!("{}/models/{}:streamGenerateContent?alt=sse", self.url.trim_end_matches('/'), self.model);
        let res = crate::net::agent(Duration::from_secs(30)).post(&url).header("x-goog-api-key", &self.key).send_json(&body).map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("Gemini answered {}", res.status()));
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
