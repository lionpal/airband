//! The common speech endpoint (`POST {url}/audio/speech`), which several services and most local voice servers
//! speak. A service that does is a row and no code.
#![expect(clippy::disallowed_macros, reason = "a voice service's own request body")]
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Provider, Use};
use serde_json::json;
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "speech", can: &[], told: &[] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    let set = |field: &str| voice.get(Job::Voice, field);
    // the interface does not say the rate in a way every service agrees on, so the row or his setting does
    // nothing given is said as that, never as an empty rate (audit 7, P-N11)
    let given = set(crate::models::field::RATE);
    let rate = match given.trim() {
        "" => super::rate_of(None),
        r => r.parse::<u64>().map_err(|_| format!("\"{r}\" is not a rate a voice can be played at. It is a number like 24000.")).and_then(|r| super::rate_of(Some(r))),
    }?;
    let reach = super::Reach { url: format!("{}/audio/speech", set(crate::models::field::URL).trim_end_matches('/')), auth: super::key_header(by, &voice.key)?, headers: Vec::new(), rate, service: super::Answering::of(provider) };
    Ok(Box::new(SpeechServer { reach, model: set(crate::models::field::MODEL), voice: set(crate::models::field::VOICE) }))
}

/// Any service that speaks the common speech interface: `POST {url}/audio/speech`, asked for raw 16-bit mono PCM.
/// The interface does not say the rate in a way every service agrees on, so the row does. It gives no word
/// timings.
struct SpeechServer {
    /// Where it is, its key, its rate and its name: reached as any voice service is.
    reach: super::Reach,
    model: String,
    voice: String,
}

impl Synth for SpeechServer {
    fn rate(&self) -> u32 {
        self.reach.rate
    }
    fn service(&self) -> &super::Answering {
        &self.reach.service
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let body = json!({ "model": self.model, "voice": self.voice, "input": text, "response_format": "pcm" });
        let res = self.reach.sent(Duration::from_secs(30), self.reach.post().json(&body))?;
        super::pump(res.into_body().into_reader(), sink)
    }
}
