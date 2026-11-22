//! Speech from SSML: the words go up as an SSML document and raw PCM comes back. Everything about the service
//! is its row's to say: where it is reached (`url`, which may name his settings in braces), in which language
//! the voice is addressed (`language`), how the key is presented (`auth`), what else it must be sent
//! (`headers`, such as the sound format it is asked for) and the rate of what comes back (`rate`).
use super::{Made, Reach, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Provider, Use};
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "ssml", can: &[], told: &["url", "language", "auth", "rate"] }, make };

fn make(voice: &Use, by: &By, provider: &Provider) -> Made {
    let reach = Reach::of(voice, by, provider)?;
    Ok(Box::new(Ssml { language: by.text("language").into(), voice: voice.get(Job::Voice, "voice"), reach }))
}

struct Ssml {
    reach: Reach,
    language: String,
    voice: String,
}

impl Synth for Ssml {
    fn rate(&self) -> u32 {
        self.reach.rate
    }
    fn service(&self) -> &super::Answering {
        &self.reach.service
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        let ssml = format!("<speak version='1.0' xml:lang='{}'><voice name='{}'>{escaped}</voice></speak>", self.language, self.voice);
        let res = self.reach.sent(self.reach.post(Duration::from_secs(30)).header("Content-Type", "application/ssml+xml").header("User-Agent", "airband").send(ssml.as_bytes()))?;
        super::pump(res.into_body().into_reader(), sink)
    }
}
