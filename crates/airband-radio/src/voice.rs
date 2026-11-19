//! Outbound: getting the tower's speech to the host's speaker. The tower makes the audio and hands it over in
//! parts while it is still being made; each part is decoded here and given to the host as plain PCM, so playback
//! starts on the first part. A host whose own voice is worth using (a terminal) reads the text itself when there
//! is no audio; a phone says nothing then, and he reads it.
use super::*;
use airband_core::audio::{BLOCK_BYTES, BLOCK_SAMPLES, base64_decode, decode_block};
use airband_core::radio::SpeechPart;
use std::time::Instant;

/// How long to wait for the first sound before giving the clip up.
const FIRST_SOUND: Duration = Duration::from_millis(8000);

/// Say `text`: with the tower's voice when it has one for this (`speech`), otherwise with the host's if that is
/// worth hearing.
pub(crate) fn say(link: &Link, host: &dyn RadioHost, text: &str, speech: Option<&str>) {
    if text.trim().is_empty() {
        return;
    }
    if speech.is_some_and(|id| play(link, host, id, &|| false)) {
        return;
    }
    if host.own_voice() {
        host.speak(text.to_string());
    }
}

/// Fetch and play one clip, until it is over or `stop` says to leave off. False if not a sound was played, so
/// the caller can fall back.
pub(crate) fn play(link: &Link, host: &dyn RadioHost, id: &str, stop: &dyn Fn() -> bool) -> bool {
    let began = Instant::now();
    let (mut from, mut started, mut rate) = (0u32, false, 24000u32);
    loop {
        if stop() {
            return true;
        }
        let part: Option<SpeechPart> = match link.request("GET", &format!("/speech/{id}?from={from}"), None) {
            Ok((200, v)) => serde_json::from_value(v).ok(),
            _ => None,
        };
        let Some(part) = part else {
            // the line dropped or the clip is gone: finish what is already queued, or give up before starting
            if started {
                host.play(rate, Vec::new(), true);
            }
            return started;
        };
        rate = part.rate;
        if part.data.is_empty() && !part.done {
            if !started && began.elapsed() > FIRST_SOUND {
                return false;
            }
            continue;
        }
        let bytes = base64_decode(&part.data).unwrap_or_default();
        let mut left = part.samples as usize;
        let mut pcm = Vec::with_capacity(left * 2);
        for block in bytes.chunks(BLOCK_BYTES) {
            let n = left.min(BLOCK_SAMPLES);
            for s in decode_block(block, n) {
                pcm.extend_from_slice(&s.to_le_bytes());
            }
            left -= n;
        }
        if !pcm.is_empty() || part.done {
            host.play(rate, pcm, part.done);
            started = true;
        }
        if part.done {
            return true;
        }
        from = part.next;
    }
}
