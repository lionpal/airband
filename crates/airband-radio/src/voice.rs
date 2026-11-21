//! Outbound: saying something aloud. The words come from his computer; the sound is made on this device, by the
//! voice he chose on it ([`crate::mouth`]): a model here, or a service reached with a key of his kept here. His
//! computer makes no sound. A host that speaks its own way (a terminal prints) does that. With no voice chosen
//! nothing is said, and the reason goes back so what was not heard is not counted as heard.
use super::*;

/// Why nothing was said, when no voice is chosen. Also what he is told where he turns reading aloud on.
pub(crate) const NOTHING_CHOSEN: &str = "No voice is chosen on this device. Choose one in Settings, under Voice.";

/// How saying something went.
pub(crate) enum Said {
    /// He heard it, or stopped it himself.
    Aloud,
    /// No sound came out, and why. It has not been heard.
    Nothing(String),
}

/// Is there anything to speak with: a voice he chose here, or a host that speaks its own way?
pub(crate) fn can_speak(host: &dyn RadioHost, mouth: &crate::mouth::Mouth) -> bool {
    mouth.ready() || host.own_voice()
}

/// Say `text` with the voice he chose on this device.
pub(crate) fn say(host: &dyn RadioHost, mouth: &crate::mouth::Mouth, text: &str) {
    say_until(host, mouth, text, &|| false);
}

/// The same, leaving off as soon as `stop` says so, and saying how it went: only what made a sound counts as
/// heard.
pub(crate) fn say_until(host: &dyn RadioHost, mouth: &crate::mouth::Mouth, text: &str, stop: &dyn Fn() -> bool) -> Said {
    if text.trim().is_empty() || stop() {
        return Said::Aloud;
    }
    // When the voice he chose cannot speak, it stays the one he chose and he is told: nothing else speaks in
    // its place.
    if mouth.ready() {
        return match mouth.say(host, text, stop) {
            Ok(()) => Said::Aloud,
            Err(why) => {
                host.told(why.clone());
                Said::Nothing(why)
            }
        };
    }
    if host.own_voice() {
        host.speak(text.to_string());
        return Said::Aloud;
    }
    Said::Nothing(NOTHING_CHOSEN.into())
}
