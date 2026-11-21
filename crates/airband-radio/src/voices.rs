//! The voice services: a sentence in, 16-bit mono PCM out, a piece at a time. The tower uses these to make speech
//! for a phone, and a phone uses them itself when he has given it a key of his own (`mouth.rs`).
//!
//! Each way of reaching a voice service is an interface: a file under `voices/` with an `impl Synth` and a
//! [`Voicing`] that names it, and a line of [`VOICES`]. A provider's row names the interface it speaks
//! (`by.interface`) and tells it what it needs (`by.with`). A service that speaks OpenAI's speech interface
//! is a row and no code; one with a route of its own is one file here and its row.
use airband_core::providers::{By, Interface, Job, Offer, Use};

mod azure;
mod elevenlabs;
mod gemini;
mod speech;

/// A voice service.
pub trait Synth: Send + Sync {
    /// Samples a second of the PCM it produces.
    fn rate(&self) -> u32;
    /// Speak `text`, handing each piece of 16-bit little-endian mono PCM to `sink` as it arrives. A service that
    /// says when each word is spoken tells `mark` so, before the sound of that word: where the word starts in
    /// `text` (in characters) and how many milliseconds into the clip.
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String>;
}

/// A voice made ready to speak, or why it cannot be, in words for him.
type Made = Result<Box<dyn Synth>, String>;

/// One way of reaching a voice service.
struct Voicing {
    is: Interface,
    /// The voice a setting names, with what the row tells this interface.
    make: fn(&Use, &By) -> Made,
}

/// Every voice interface this build speaks.
const VOICES: &[Voicing] = &[speech::VOICING, gemini::VOICING, azure::VOICING, elevenlabs::VOICING];

/// Does this build have the code to speak through what this offer names?
pub fn speaks(offer: &Offer) -> bool {
    offer.job == Job::Voice && VOICES.iter().any(|v| v.is.fits(offer))
}

/// The voice a setting names, through whichever interface its row says. Err says why not, in words for him.
pub fn synth(voice: &Use) -> Made {
    if voice.is_none() {
        return Err("No voice is set.".into());
    }
    let (_, offer) = voice.offer(Job::Voice).ok_or("This version of airband has no such voice service.")?;
    let voicing = VOICES.iter().find(|v| v.is.fits(offer)).ok_or("This version of airband cannot speak through that voice service yet. Update airband.")?;
    (voicing.make)(voice, &offer.by)
}

/// The same, for a caller with nothing to say about why not.
pub fn synth_for(voice: &Use) -> Option<Box<dyn Synth>> {
    synth(voice).ok()
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
                assert!(synth(&Use::of(&p.id)).is_ok(), "{}", p.id);
            }
        }
    }

    #[test]
    fn a_voice_this_build_cannot_speak_is_not_offered_and_says_why() {
        let offer: Offer = serde_json::from_value(serde_json::json!({ "job": "voice", "by": { "interface": "something-new" }, "label": "New" })).unwrap();
        assert!(!speaks(&offer));
        // an interface it has, asked for something it cannot do yet, or not told what it needs
        let needs: Offer = serde_json::from_value(serde_json::json!({ "job": "voice", "by": { "interface": "speech" }, "label": "New", "needs": ["word-timings"] })).unwrap();
        let untold: Offer = serde_json::from_value(serde_json::json!({ "job": "voice", "by": { "interface": "gemini" }, "label": "New" })).unwrap();
        assert!(!speaks(&needs) && !speaks(&untold));
        // a rate that is not one is said, not replaced with a guess
        let odd = Use { kind: "local".into(), key: String::new(), set: [("rate".to_string(), serde_json::json!("fast"))].into_iter().collect() };
        assert!(synth(&odd).is_err_and(|why| why.contains("rate")));
    }
}
