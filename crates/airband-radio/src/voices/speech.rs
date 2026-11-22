//! OpenAI's speech endpoint, which several services and most local voice servers speak. A service that does
//! is a row and no code.
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Provider, Use};
use serde_json::json;
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "speech", can: &[], told: &[] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    let set = |field: &str| voice.get(Job::Voice, field);
    // the interface does not say the rate in a way every service agrees on, so the row or his setting does
    let rate = set("rate").parse().ok().filter(|r| super::RATES.contains(r)).ok_or_else(|| format!("\"{}\" is not a rate a voice can be played at. It is a number like 24000.", set("rate")))?;
    Ok(Box::new(OpenAiSpeech { url: set("url"), auth: super::key_header(by, &voice.key)?, model: set("model"), voice: set("voice"), rate, service: super::Service::of(provider) }))
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
    /// What the service is called, and what its statuses mean, for when something has to be said to him.
    service: super::Service,
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
        let res = req.send_json(&body).map_err(|_| self.service.unreachable())?;
        if res.status().as_u16() != 200 {
            return Err(self.service.answered(res.status().as_u16()));
        }
        super::pump(res.into_body().into_reader(), sink)
    }
}
