//! ElevenLabs' own route, which says when each word is spoken. The row says where (`url`).
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Use};
use airband_core::audio::base64_decode;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "elevenlabs", can: &[], told: &["url"] }, make };

fn make(voice: &Use, by: &By) -> Made {
    Ok(Box::new(Elevenlabs { url: by.text("url").into(), key: voice.key.clone(), voice: voice.get(Job::Voice, "voice"), model: voice.get(Job::Voice, "model") }))
}

struct Elevenlabs {
    url: String,
    key: String,
    voice: String,
    model: String,
}

impl Synth for Elevenlabs {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        // the streaming endpoint with timings: a line of JSON at a time, each with a piece of the sound and
        // when each character of it is spoken, counted from the start of the clip
        let res = crate::net::agent(Duration::from_secs(60))
            .post(format!("{}/text-to-speech/{}/stream/with-timestamps?output_format=pcm_24000", self.url.trim_end_matches('/'), self.voice))
            .header("xi-api-key", &self.key)
            .send_json(json!({ "text": text, "model_id": self.model }))
            .map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("ElevenLabs answered {}", res.status()));
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
            if let Some(pcm) = v["audio_base64"].as_str().and_then(base64_decode) {
                sink(&pcm);
            }
        }
        Ok(())
    }
}
