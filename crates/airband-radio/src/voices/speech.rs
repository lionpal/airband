//! OpenAI's speech endpoint, which several services and most local voice servers speak. A service that does
//! is a row and no code.
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Use};
use serde_json::json;
use std::io::Read;
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "speech", can: &[], told: &[] }, make };

fn make(voice: &Use, by: &By, _name: &str) -> Made {
    let set = |field: &str| voice.get(Job::Voice, field);
    // the interface does not say the rate in a way every service agrees on, so the row or his setting does
    let rate = set("rate").parse().ok().filter(|r| (8000..=48000).contains(r)).ok_or_else(|| format!("\"{}\" is not a rate a voice can be played at. It is a number like 24000.", set("rate")))?;
    // how this service takes a key is its row's to say; one sent no key needs nothing said
    let auth = match voice.key.is_empty() {
        true => None,
        false => Some(by.auth().map(|a| (a.header.clone(), a.value(&voice.key))).ok_or("This version of airband was not told how that voice service takes its key.")?),
    };
    Ok(Box::new(OpenAiSpeech { url: set("url"), auth, model: set("model"), voice: set("voice"), rate }))
}

/// Any service that speaks OpenAI's speech interface: `POST {url}/audio/speech`, asked for raw 16-bit mono PCM.
/// The interface does not say the rate in a way every service agrees on, so the row does. It gives no word
/// timings.
struct OpenAiSpeech {
    url: String,
    /// The header a key goes in, and its value there. Nothing for a service that takes none.
    auth: Option<(String, String)>,
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
        if let Some((header, value)) = &self.auth {
            req = req.header(header.as_str(), value.as_str());
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
