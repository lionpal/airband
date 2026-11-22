//! Listening by upload: a finished recording in, the words out. The services here take a whole clip, where a
//! live one (`ears_stream.rs`) is fed while he talks. The phone calls this, with the key of his that is kept on it.
//!
//! Each way of sending a recording is an interface: a function here and a line of [`UPLOADS`]. A service that
//! speaks OpenAI's transcription interface (`POST {url}/audio/transcriptions`) is a row of the catalogue and
//! no code. There is one such interface today.
use airband_core::providers::{By, Interface, Job, Offer, Trouble, Use};
use serde_json::Value;
use std::time::Duration;

/// The 44 bytes a WAV file of 16-bit mono PCM at `rate` starts with. `data` is how many bytes of sound
/// follow; a recording still being made does not know, and says nought for both lengths.
pub(crate) fn wav_header(rate: u32, data: Option<u32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data.map_or(0, |d| 36 + d).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.unwrap_or(0).to_le_bytes());
    out
}

/// A recording as a WAV file: 16-bit mono PCM at `rate`.
pub fn wav(rate: u32, pcm: &[i16]) -> Vec<u8> {
    let mut out = wav_header(rate, Some((pcm.len() * 2) as u32));
    out.reserve(pcm.len() * 2);
    pcm.iter().for_each(|s| out.extend_from_slice(&s.to_le_bytes()));
    out
}

/// One way of hearing a finished recording: what is used, its row, the recording and the names to expect.
type Hear = fn(&Use, &By, &[u8], &[String]) -> Result<String, String>;

/// One way of sending a finished recording to a service.
struct Upload {
    is: Interface,
    /// What was said in a WAV recording, with the names he is likely to say. Err says why not, in words for him.
    hear: Hear,
}

/// Every upload interface this build speaks.
const UPLOADS: &[Upload] = &[Upload { is: Interface { name: "transcribe", can: &[], told: &[] }, hear: transcriptions }];

fn upload(offer: &Offer) -> Option<&'static Upload> {
    UPLOADS.iter().find(|u| offer.job == Job::Ears && u.is.fits(offer))
}

/// Does this build have the code to listen with what this offer names, live or by upload?
pub fn listens(offer: &Offer) -> bool {
    crate::ears::live(offer).is_some() || upload(offer).is_some()
}

/// Does this setting listen by upload? False for a live dialect, and for anything that does not listen.
pub fn by_upload(with: &Use) -> bool {
    with.offer(Job::Ears).is_some_and(|(_, o)| upload(&o).is_some())
}

/// What was said in a recording, by the service `with` names. `phrases` are names he is likely to say. Empty
/// when nothing was said. Blocks. Err says why not, in words for him.
pub fn transcribe(with: &Use, wav: &[u8], phrases: &[String]) -> Result<String, String> {
    let (_, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    let by = upload(&offer).ok_or("That service does not listen this way in this version of airband.")?;
    (by.hear)(with, &offer.by, wav, phrases)
}

/// OpenAI's transcription interface: one multipart form with the file and the model.
fn transcriptions(with: &Use, by: &By, wav: &[u8], phrases: &[String]) -> Result<String, String> {
    let set = |field: &str| with.get(Job::Ears, field);
    let agent = crate::net::agent(Duration::from_secs(60));
    let edge = format!("airband{}", airband_core::encode::random_hex(12));
    let mut body = Vec::with_capacity(wav.len() + 600);
    let mut field = |name: &str, value: &str| body.extend_from_slice(format!("--{edge}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    field("model", &set("model"));
    field("response_format", "json");
    if !phrases.is_empty() {
        // the interface's own place for words to expect
        field("prompt", &phrases.join(", "));
    }
    body.extend_from_slice(format!("--{edge}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"said.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{edge}--\r\n").as_bytes());
    let mut req = agent.post(format!("{}/audio/transcriptions", set("url").trim_end_matches('/'))).header("content-type", format!("multipart/form-data; boundary={edge}"));
    // how this service takes a key is its row's to say; one sent no key needs nothing said
    if let Some((header, value)) = by.key_header(&with.key).map_err(|_| "This version of airband was not told how that service takes its key.")? {
        req = req.header(header.as_str(), value.as_str());
    }
    // what went wrong is worded once, for every service, by the service's own row and name
    let service = airband_core::providers::provider(&with.kind);
    let (name, check) = (service.as_ref().map(|p| p.name.as_str()).unwrap_or_default(), service.as_ref().and_then(|p| p.check.as_ref()));
    let mut res = req.send(&body[..]).map_err(|_| Trouble::Unreachable.said(name))?;
    match res.status().as_u16() {
        200 => {}
        code => return Err(Trouble::of_status(code, check).said(name)),
    }
    let v: Value = res.body_mut().read_json().map_err(|_| Trouble::NoAnswer.said(name))?;
    // an answer with no text in it is no answer, not silence
    v["text"].as_str().map(|t| t.trim().to_string()).ok_or_else(|| Trouble::NoAnswer.said(name))
}

/// Does the service take this key and these settings? One real call, with half a second of silence.
pub fn check(with: &Use) -> Result<(), String> {
    transcribe(with, &wav(16000, &[0i16; 8000]), &[]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listener_in_the_catalogue_is_one_this_build_has() {
        for p in &airband_core::catalog::built().providers {
            for o in p.offers.iter().filter(|o| o.job == Job::Ears) {
                assert!(listens(o), "{}: nothing here listens by \"{}\"", p.id, o.by.interface);
                // one way or the other, never both
                assert!(by_upload(&Use::of(&p.id, &o.id)) != crate::ears::live(o).is_some(), "{}", p.id);
                // one that takes a key by upload says how the key is presented
                assert!(!p.key || upload(o).is_none() || o.by.auth().is_some(), "{}: its row does not say how its key is presented", p.id);
            }
        }
        let odd: Offer = serde_json::from_value(serde_json::json!({ "id": "new", "job": "ears", "by": { "interface": "martian" }, "label": "New" })).unwrap();
        assert!(!listens(&odd));
    }

    #[test]
    fn a_wav_says_how_long_it_is() {
        let w = wav(16000, &[1, -1, 2]);
        assert_eq!((w.len(), &w[..4], u32::from_le_bytes(w[40..44].try_into().unwrap()), u32::from_le_bytes(w[4..8].try_into().unwrap())), (50, &b"RIFF"[..], 6, 42));
        assert_eq!(&w[44..46], &1i16.to_le_bytes());
    }
}
