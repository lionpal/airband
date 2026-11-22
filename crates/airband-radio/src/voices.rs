//! The voice services: a sentence in, 16-bit mono PCM out, a piece at a time. The phone uses these itself,
//! with the key of his that is kept on it (`mouth.rs`). His computer makes no speech.
//!
//! Each way of reaching a voice service is an interface: a file under `voices/` with an `impl Synth` and a
//! [`Voicing`] that names it, and a line of [`VOICES`]. A provider's row names the interface it speaks
//! (`by.interface`) and tells it what it needs (`by.with`). A service that speaks the common speech interface
//! is a row and no code; one with a route of its own is one file here and its row.
#![expect(clippy::disallowed_methods, reason = "asks a voice service")]
use airband_core::providers::{By, Interface, Job, Offer, Provider, Use};

mod generate_content;
mod ssml;
mod timed_stream;
mod speech;
use airband_core::providers::Answering;

/// A voice service.
pub(crate) trait Synth: Send + Sync {
    /// Samples a second of the PCM it produces.
    fn rate(&self) -> u32;
    /// Speak `text`, handing each piece of 16-bit little-endian mono PCM to `sink` as it arrives. A service that
    /// says when each word is spoken tells `mark` so, before the sound of that word: where the word starts in
    /// `text` (in characters) and how many milliseconds into the clip.
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String>;
    /// The service, for telling him what went wrong.
    fn service(&self) -> &Answering;
}

/// A voice made ready to speak, or why it cannot be, in words for him.
type Made = Result<Box<dyn Synth>, String>;

/// One way of reaching a voice service.
struct Voicing {
    is: Interface,
    /// The voice a setting names, with what the row tells this interface and what the service is called.
    make: fn(&Use, &By, &Provider) -> Made,
}

/// Where a voice service with a route of its own is reached and how, all of it from its row: the address
/// (with his settings filled in where it names them), the header its key goes in, whatever else it must be
/// sent, the rate of the sound it gives back, and its name for when it has to be named to him.
pub(crate) struct Reach {
    url: String,
    /// The header its key goes in and the value there. Nothing for a service sent no key.
    auth: Option<(String, String)>,
    headers: Vec<(String, String)>,
    pub rate: u32,
    service: Answering,
}

/// The rates a voice's sound can be played at.
const RATES: std::ops::RangeInclusive<u32> = 8000..=48000;

/// How a voice service takes this key, by its row. One sent no key needs nothing said.
fn key_header(by: &By, key: &str) -> Result<Option<(String, String)>, String> {
    by.key_header(key).map_err(|_| "This version of airband was not told how that voice service takes its key.".to_string())
}

/// Hand a service's sound on as it arrives, a tenth of a second at a time.
fn pump(mut sound: impl std::io::Read, sink: &mut dyn FnMut(&[u8])) -> Result<(), String> {
    let mut buf = [0u8; 4800];
    loop {
        match sound.read(&mut buf).map_err(|e| e.to_string())? {
            0 => return Ok(()),
            n => sink(&buf[..n]),
        }
    }
}

impl Reach {
    fn of(voice: &Use, by: &By, provider: &Provider) -> Result<Reach, String> {
        let url = airband_core::providers::fill(by.text("url"), &|field| voice.get(Job::Voice, field))?;
        let auth = key_header(by, &voice.key)?;
        let rate = by.with.get(crate::models::field::RATE).and_then(serde_json::Value::as_u64).map(|r| r as u32).filter(|r| RATES.contains(r)).ok_or("That voice service's row does not say the rate of its sound.")?;
        let headers = by.with.get(crate::models::field::HEADERS).and_then(serde_json::Value::as_object).map(|h| h.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
        Ok(Reach { url, auth, headers, rate, service: Answering::of(provider) })
    }

    /// A request to it, with the key and what else the row says to send.
    fn post(&self, timeout: std::time::Duration) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.auth.iter().chain(self.headers.iter()).fold(crate::net::agent(timeout).post(&self.url), |r, (k, v)| r.header(k.as_str(), v.as_str()))
    }

    /// The request, sent: its answer when it gives the sound, else what went wrong, in words for him.
    fn sent(&self, res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, String> {
        airband_io::http::sent(&self.service, res)
    }
}

/// Every voice interface this build speaks.
const VOICES: &[Voicing] = &[speech::VOICING, generate_content::VOICING, ssml::VOICING, timed_stream::VOICING];

/// Does this build have the code to speak through what this offer names?
pub fn speaks(offer: &Offer) -> bool {
    offer.job == Job::Voice && VOICES.iter().any(|v| v.is.fits(offer))
}

/// The voice a setting names, through whichever interface its row says. Err says why not, in words for him.
pub(crate) fn synth(voice: &Use) -> Made {
    if voice.is_none() {
        return Err(crate::voice::NOTHING_CHOSEN.into());
    }
    let (provider, offer) = voice.offer(Job::Voice).ok_or("This version of airband has no such voice service.")?;
    let voicing = VOICES.iter().find(|v| v.is.fits(&offer)).ok_or("This version of airband cannot speak through that voice service yet. Update airband.")?;
    (voicing.make)(voice, &offer.by, &provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_voice_in_the_catalogue_is_one_this_build_speaks() {
        let mut named: Vec<&str> = VOICES.iter().map(|v| v.is.name).collect();
        named.dedup();
        assert_eq!(named.len(), VOICES.len(), "an interface is in the table twice");
        for p in &airband_core::catalog::built().providers {
            for o in p.offers.iter().filter(|o| o.job == Job::Voice) {
                assert!(speaks(o), "{}: nothing here speaks \"{}\", or its row does not tell it what it needs", p.id, o.by.interface);
                // and it can be made ready with nothing but the row's usual settings
                assert!(synth(&Use::of(&p.id, &o.id)).is_ok(), "{}", p.id);
            }
        }
    }

    /// Against a real service. VOICE_WITH is a provider's id in the catalogue, VOICE_KEY its key, and VOICE_SET
    /// any of its settings as `field=value,field=value`:
    ///   VOICE_WITH=… VOICE_KEY=… VOICE_SET=region=… cargo test -p airband-radio -- --ignored --nocapture speaks_with
    #[test]
    #[ignore]
    fn speaks_with_a_real_service() {
        let provider = airband_core::providers::provider(&std::env::var("VOICE_WITH").unwrap()).expect("a provider in the catalogue");
        let offer = provider.offers.iter().find(|o| o.job == Job::Voice).expect("an offer of its that speaks");
        let mut with = Use::of(&provider.id, &offer.id);
        with.key = std::env::var("VOICE_KEY").unwrap();
        for (field, value) in std::env::var("VOICE_SET").unwrap_or_default().split(',').filter_map(|s| s.split_once('=')) {
            with.set.insert(field.into(), serde_json::json!(value));
        }
        let synth = synth(&with).unwrap();
        let (mut bytes, mut marks) = (0usize, 0usize);
        synth.speak("Shop two is waiting.", &mut |pcm| bytes += pcm.len(), &mut |_, _| marks += 1).unwrap();
        println!("{}: {:.1} s of sound at {} Hz, {marks} word timings", provider.name, bytes as f32 / 2.0 / synth.rate() as f32, synth.rate());
        assert!(bytes > synth.rate() as usize);
        // and a key it does not know is said by the service's own name, from its row
        with.key = "airband-not-a-key".into();
        let why = super::synth(&with).unwrap().speak("Shop two.", &mut |_| {}, &mut |_, _| {}).unwrap_err();
        println!("  with a broken key: {why}");
        assert!(why.starts_with(&provider.name));
    }

    #[test]
    fn a_voice_this_build_cannot_speak_is_not_offered_and_says_why() {
        let offer: Offer = serde_json::from_value(serde_json::json!({ "id": "new", "job": "voice", "by": { "interface": "something-new" }, "label": "New" })).unwrap();
        assert!(!speaks(&offer));
        // an interface it has, asked for something it cannot do yet, or not told what it needs
        let needs: Offer = serde_json::from_value(serde_json::json!({ "id": "new", "job": "voice", "by": { "interface": "speech" }, "label": "New", "needs": ["word-timings"] })).unwrap();
        let untold: Offer = serde_json::from_value(serde_json::json!({ "id": "new", "job": "voice", "by": { "interface": "generate-content" }, "label": "New" })).unwrap();
        assert!(!speaks(&needs) && !speaks(&untold));
        // a rate that is not one is said, not replaced with a guess
        let odd = Use { kind: "local".into(), offer: "voice-server".into(), key: String::new(), set: [("rate".to_string(), serde_json::json!("fast"))].into_iter().collect() };
        assert!(synth(&odd).is_err_and(|why| why.contains("rate")));
    }
}
