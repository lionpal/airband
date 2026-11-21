//! Listening by upload: a finished recording in, the words out. The services here take a whole clip, where a
//! live one (`ears_stream.rs`) is fed while he talks. Both the phone and the tower call this: the phone when he
//! has given it a key of his own, the tower when the key is on the computer and the phone sends it the clip.
//!
//! Each way of sending a recording is an interface: a function here and a line of [`UPLOADS`]. A service that
//! speaks OpenAI's transcription interface (`POST {url}/audio/transcriptions`) is a row of the catalogue and
//! no code. There is one such interface today.
use airband_core::providers::{By, Interface, Job, Offer, Use};
use serde_json::Value;
use std::time::Duration;

/// A recording as a WAV file: 16-bit mono PCM at `rate`.
pub fn wav(rate: u32, pcm: &[i16]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    pcm.iter().for_each(|s| out.extend_from_slice(&s.to_le_bytes()));
    out
}

/// One way of sending a finished recording to a service.
struct Upload {
    is: Interface,
    /// What was said in a WAV recording, with the names he is likely to say. Err says why not, in words for him.
    hear: fn(&Use, &By, &[u8], &[String]) -> Result<String, String>,
}

/// Every upload interface this build speaks.
const UPLOADS: &[Upload] = &[Upload { is: Interface { name: "transcribe", can: &[], told: &[] }, hear: openai_transcribe }];

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

/// The live dialect this setting listens with, by the name a pass calls it. Nothing for one that listens by
/// upload, or that this build has no code for.
pub fn live(with: &Use) -> Option<&'static str> {
    with.offer(Job::Ears).and_then(|(_, o)| crate::ears::live(&o)).map(|l| l.is.name)
}

/// What was said in a recording, by the service `with` names. `phrases` are names he is likely to say. Empty
/// when nothing was said. Blocks. Err says why not, in words for him.
pub fn transcribe(with: &Use, wav: &[u8], phrases: &[String]) -> Result<String, String> {
    let (_, offer) = with.offer(Job::Ears).ok_or("This version of airband does not know that listening service.")?;
    let by = upload(&offer).ok_or("That service does not listen this way in this version of airband.")?;
    (by.hear)(with, &offer.by, wav, phrases)
}

/// OpenAI's transcription interface: one multipart form with the file and the model.
fn openai_transcribe(with: &Use, by: &By, wav: &[u8], phrases: &[String]) -> Result<String, String> {
    let set = |field: &str| with.get(Job::Ears, field);
    let agent = crate::net::agent(Duration::from_secs(60));
    let edge = format!("airband{}", airband_core::relay::random_hex(12));
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
    if !with.key.is_empty() {
        // how this service takes a key is its row's to say
        let auth = by.auth().ok_or("This version of airband was not told how that service takes its key.")?;
        req = req.header(auth.header.as_str(), auth.value(&with.key));
    }
    let mut res = req.send(&body[..]).map_err(|_| "Could not reach the listening service.".to_string())?;
    match res.status().as_u16() {
        200 => {}
        401 | 403 => return Err("The listening service did not accept that key.".into()),
        code => return Err(format!("The listening service said no ({code}).")),
    }
    let v: Value = res.body_mut().read_json().map_err(|_| "The listening service answered with something unreadable.".to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
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
                assert!(by_upload(&Use::of(&p.id, &o.id)) != live(&Use::of(&p.id, &o.id)).is_some(), "{}", p.id);
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
