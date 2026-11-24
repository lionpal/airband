//! Listening over a realtime transcription session: a client secret is asked for with his key, saying what the
//! session is (only transcribing, the model, the sound's format); audio goes up as base64 in JSON events
//! (`input_audio_buffer.append`) at 24 kHz, the only rate the session takes; what it has heard so far comes
//! back as pieces of text added to the end (`…input_audio_transcription.delta`), and the whole of a stretch once
//! it is asked to finish it (`input_audio_buffer.commit`, answered by `…completed`). Where the service is, where
//! its secret is asked for and how the key is presented are its row's to say (`stream`, `pass`, `auth`).
#![expect(clippy::disallowed_macros, reason = "a listening service's own messages")]
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "realtime", can: &[], told: &[crate::models::field::STREAM, crate::models::field::PASS, crate::models::field::AUTH] }, wire: &WIRE, pass, once: true };

const WIRE: Wire = Wire { connect, audio, end: |_| Message::text(json!({ "type": "input_audio_buffer.commit" }).to_string()), read, header: false, reopens: false };

/// The one rate the session takes sound at.
pub(super) const SESSION_RATE: u32 = 24000;

/// A client secret for one session: what it is to be (transcription only, by the model he chose, of sound at
/// the session's rate, ended by the phone and never by the service's own idea of a pause), good for ten minutes.
fn pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str) -> Result<Ticket, String> {
    let body = json!({
        "expires_after": { "anchor": "created_at", "seconds": 600 },
        "session": { "type": "transcription", "audio": { "input": {
            "format": { "type": "audio/pcm", "rate": SESSION_RATE },
            "transcription": { "model": set.get(crate::models::field::MODEL).cloned().unwrap_or_default() },
            "turn_detection": null,
        } } },
    });
    let mut res = ask_pass(by, service, set, key, Some(body))?;
    let v: Value = res.body_mut().read_json().map_err(|_| no_pass(service))?;
    let token = v["value"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| no_pass(service))?.to_string();
    Ok(Ticket { token, until: v["expires_at"].as_u64().and_then(|s| s.checked_mul(1000)) })
}

/// The secret goes in the request's own header; the session was described when it was made.
fn connect(grant: &Pass, _request: &str) -> Result<Socket, ()> {
    let (url, host) = grant.stream()?;
    let mut req = url.into_client_request().map_err(|_| ())?;
    req.headers_mut().insert("Authorization", airband_core::token::as_bearer(&grant.token).parse().map_err(|_| ())?);
    let socket = dial(&host, req)?;
    read_briefly(&socket);
    Ok(socket)
}

fn audio(_request: &str, bytes: &[u8]) -> Message {
    let at_24k = resample(&airband_core::audio::samples(bytes), crate::ears::RATE, SESSION_RATE);
    let pcm: Vec<u8> = at_24k.iter().flat_map(|s| s.to_le_bytes()).collect();
    Message::text(json!({ "type": "input_audio_buffer.append", "audio": airband_core::encode::base64(&pcm) }).to_string())
}

/// The microphone's sound at another rate, each new sample drawn on a straight line between its two neighbours.
pub(super) fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if input.is_empty() || from == to {
        return input.to_vec();
    }
    let n = (input.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    (0..n)
        .map(|i| {
            let at = i as f64 * f64::from(from) / f64::from(to);
            let (j, part) = (at.floor() as usize, at.fract());
            let a = f64::from(input[j.min(input.len() - 1)]);
            let b = f64::from(input[(j + 1).min(input.len() - 1)]);
            (a + (b - a) * part).round() as i16
        })
        .collect()
}

/// A piece of text adds to the stretch so far; the whole of it, once finished, is kept and ends its turn.
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool, so_far: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(&m.clone().into_data()).ok()?;
    match v["type"].as_str()? {
        "conversation.item.input_audio_transcription.delta" => Some(format!("{so_far}{}", v["delta"].as_str()?)),
        "conversation.item.input_audio_transcription.completed" => {
            let done = v["transcript"].as_str()?.trim();
            if !done.is_empty() {
                kept.push(done.to_string());
            }
            *ended = true;
            Some(String::new())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_pieces_add_up_and_the_finished_stretch_is_kept_and_ends_its_turn() {
        let (mut kept, mut ended) = (Vec::new(), false);
        let delta = |d: &str| Message::text(json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i", "content_index": 0, "delta": d }).to_string());
        let now = read(&delta("ship"), &mut kept, &mut ended, "").unwrap();
        let now = read(&delta(" it"), &mut kept, &mut ended, &now).unwrap();
        assert_eq!(now, "ship it");
        assert!(kept.is_empty() && !ended);
        let done = Message::text(json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i", "content_index": 0, "transcript": "Ship it." }).to_string());
        assert_eq!(read(&done, &mut kept, &mut ended, &now), Some(String::new()));
        assert_eq!((kept, ended), (vec!["Ship it.".to_string()], true));
    }

    #[test]
    fn sound_goes_at_the_rate_the_session_takes() {
        // a tenth of a second at 16 kHz is a tenth of a second at 24 kHz, the same line drawn through it
        let ramp: Vec<i16> = (0..1600).map(|i| i as i16 * 10).collect();
        let out = resample(&ramp, 16000, 24000);
        assert_eq!(out.len(), 2400);
        assert_eq!((out[0], out[3], out[300]), (0, 20, 2000));
    }
}
