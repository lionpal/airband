//! Microsoft's own speech route: SSML in, raw PCM back. The row says where (`url`, with `{region}` where his
//! region goes) and in which language the voice is addressed (`language`).
use super::{Made, Synth, Voicing};
use airband_core::providers::{By, Interface, Job, Use};
use std::io::Read;
use std::time::Duration;

pub(super) const VOICING: Voicing = Voicing { is: Interface { name: "azure", can: &[], told: &["url", "language"] }, make };

fn make(voice: &Use, by: &By) -> Made {
    // the region is part of a host name: letters and digits, nothing that could lead elsewhere
    let region = voice.get(Job::Voice, "region").trim().to_lowercase();
    if region.is_empty() || !region.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("That is not a region. It looks like eastus or japaneast.".into());
    }
    Ok(Box::new(Azure { url: by.text("url").replace("{region}", &region), language: by.text("language").into(), key: voice.key.clone(), voice: voice.get(Job::Voice, "voice") }))
}

struct Azure {
    /// Where it is reached, with the region already in it.
    url: String,
    language: String,
    /// The service's key.
    key: String,
    voice: String,
}

impl Synth for Azure {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        let ssml = format!("<speak version='1.0' xml:lang='{}'><voice name='{}'>{escaped}</voice></speak>", self.language, self.voice);
        let res = crate::net::agent(Duration::from_secs(30))
            .post(&self.url)
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
