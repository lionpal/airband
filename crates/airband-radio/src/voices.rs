//! The voice services: a sentence in, 16-bit mono PCM out, a piece at a time. The tower uses these to make speech
//! for a phone, and a phone uses them itself when he has given it a key of his own (`mouth.rs`).
//!
//! A service that speaks OpenAI's speech interface is a row of `airband_core::providers::PROVIDERS` and no
//! code. One with an interface of its own is an `impl Synth` here, a `By` for it, and its row.
use airband_core::audio::base64_decode;
use airband_core::providers::Use;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

/// A voice service.
pub trait Synth: Send + Sync {
    /// Samples a second of the PCM it produces.
    fn rate(&self) -> u32;
    /// Speak `text`, handing each piece of 16-bit little-endian mono PCM to `sink` as it arrives. A service that
    /// says when each word is spoken tells `mark` so, before the sound of that word: where the word starts in
    /// `text` (in characters) and how many milliseconds into the clip.
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String>;
}

struct Gemini {
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
            "contents": [{ "parts": [{ "text": text, "speech_metadata": { "style": "Calm and clear, at a brisk pace, like a radio dispatcher. Say the words exactly as written and nothing else." } }] }],
            "generationConfig": { "responseModalities": ["AUDIO"], "speechConfig": { "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": self.voice } } } },
        });
        let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse", self.model);
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

struct Azure {
    /// The service's key.
    key: String,
    region: String,
    voice: String,
}

impl Synth for Azure {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        let ssml = format!("<speak version='1.0' xml:lang='en-US'><voice name='{}'>{escaped}</voice></speak>", self.voice);
        let res = crate::net::agent(Duration::from_secs(30))
            .post(format!("https://{}.tts.speech.microsoft.com/cognitiveservices/v1", self.region))
            .header("Ocp-Apim-Subscription-Key", &self.key)
            .header("Content-Type", "application/ssml+xml")
            .header("X-Microsoft-OutputFormat", "raw-24khz-16bit-mono-pcm")
            .header("User-Agent", "airband")
            .send(ssml.as_bytes())
            .map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("Microsoft answered {}", res.status()));
        }
        let mut reader = res.into_body().into_reader();
        let mut buf = [0u8; 4800]; // a tenth of a second
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => sink(&buf[..n]),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

struct Elevenlabs {
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
            .post(format!("https://api.elevenlabs.io/v1/text-to-speech/{}/stream/with-timestamps?output_format=pcm_24000", self.voice))
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

/// Any service that speaks OpenAI's speech interface: `POST {url}/audio/speech`, asked for raw 16-bit mono PCM.
/// The interface does not say the rate in a way every service agrees on, so the row does. It gives no word
/// timings.
struct OpenAiSpeech {
    url: String,
    key: String,
    model: String,
    voice: String,
    rate: u32,
}

impl Synth for OpenAiSpeech {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let body = json!({ "model": self.model, "voice": self.voice, "input": text, "response_format": "pcm" });
        let mut req = crate::net::agent(Duration::from_secs(30)).post(format!("{}/audio/speech", self.url.trim_end_matches('/')));
        if !self.key.is_empty() {
            req = req.header("authorization", format!("Bearer {}", self.key));
        }
        let res = req.send_json(&body).map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("the voice service answered {}", res.status()));
        }
        let mut body = res.into_body().into_reader();
        let mut buf = [0u8; 8192];
        loop {
            match body.read(&mut buf).map_err(|e| e.to_string())? {
                0 => return Ok(()),
                n => sink(&buf[..n]),
            }
        }
    }
}

/// The voice a setting names, through whichever interface its row says. Nothing when no voice is set, or the
/// settings name one this build does not have.
pub fn synth_for(voice: &Use) -> Option<Box<dyn Synth>> {
    use airband_core::providers::{By, Job};
    let (_, offer) = voice.offer(Job::Voice)?;
    let set = |field: &str| voice.get(Job::Voice, field);
    let key = voice.key.clone();
    Some(match offer.by {
        By::Speech => Box::new(OpenAiSpeech { url: set("url"), key, model: set("model"), voice: set("voice"), rate: set("rate").parse().ok().filter(|r| (8000..=48000).contains(r)).unwrap_or(24000) }),
        By::GeminiVoice => Box::new(Gemini { key, model: set("model"), voice: set("voice") }),
        By::AzureVoice => Box::new(Azure { key, region: set("region"), voice: set("voice") }),
        By::ElevenlabsVoice => Box::new(Elevenlabs { key, voice: set("voice"), model: set("model") }),
        _ => return None,
    })
}

