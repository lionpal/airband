//! Listening over a two-way content stream, with a model that only transcribes: the model, the language and
//! the names to expect go in the first message, audio goes up as base64 in JSON, and finished and unfinished
//! stretches come back as JSON. Where the service is, where its pass is asked for and how the key is presented
//! are its row's to say (`stream`, `pass`, `auth`).
#![expect(clippy::disallowed_macros, reason = "a listening service's own messages")]
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "bidi", can: &[], told: &[crate::models::field::STREAM, crate::models::field::PASS, crate::models::field::AUTH] }, wire: &WIRE, pass, once: false };

const WIRE: Wire = Wire { connect, audio, end: |_| Message::text(json!({ "realtimeInput": { "audioStreamEnd": true } }).to_string()), read, header: false, reopens: false };

/// The service's short-lived token: asked for with how long it is to last, good for twenty minutes and for
/// any number of connections in that time, so one fetched ahead is still good when he opens the microphone,
/// and again if a connection drops.
fn pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let until = airband_core::clock::rfc3339(now_ms() + 20 * 60 * 1000);
    let body = serde_json::json!({ "uses": 0, "expireTime": until, "newSessionExpireTime": until });
    pass_field(by, service, set, key, Some(body), "name")
}

/// The pass goes in the address; the model, the language and the names to expect go in the first message.
fn connect(grant: &Pass, _request: &str) -> Result<Socket, ()> {
    let (url, host) = grant.stream()?;
    let url = super::with_query(&url, &[("access_token", &grant.token)]);
    let mut socket = dial(&host, url.into_client_request().map_err(|_| ())?)?;
    let mut hearing = json!({ "mode": "VERBATIM" });
    if !grant.get(crate::models::field::LANGUAGE).is_empty() {
        hearing["languageCodes"] = json!([grant.get(crate::models::field::LANGUAGE)]);
    }
    if !grant.phrases.is_empty() {
        // it does best with a hundred or fewer
        hearing["customVocabulary"] = json!(grant.phrases.iter().take(100).collect::<Vec<_>>());
    }
    let setup = json!({ "setup": { "model": format!("models/{}", grant.get(crate::models::field::MODEL)), "generationConfig": { "responseModalities": ["TEXT"] }, "inputAudioTranscription": hearing } });
    socket.send(Message::text(setup.to_string())).map_err(|_| ())?;
    // it says when it is ready, and nothing may be sent before
    loop {
        let m = socket.read().map_err(|_| ())?;
        if m.is_close() {
            return Err(());
        }
        if serde_json::from_slice::<Value>(&m.into_data()).is_ok_and(|v| v.get("setupComplete").is_some()) {
            break;
        }
    }
    read_briefly(&socket);
    Ok(socket)
}

fn audio(_request: &str, bytes: &[u8]) -> Message {
    Message::text(json!({ "realtimeInput": { "audio": { "data": airband_core::encode::base64(bytes), "mimeType": format!("audio/pcm;rate={}", crate::ears::RATE) } } }).to_string())
}

/// Its messages come as JSON, in a text frame or a binary one.
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool) -> Option<String> {
    let v: Value = serde_json::from_slice(&m.clone().into_data()).ok()?;
    let content = &v["serverContent"];
    if content["turnComplete"] == true || content["generationComplete"] == true {
        *ended = true;
    }
    // a finished stretch is kept, and the unfinished one starts again from nothing
    if let Some(done) = content["inputTranscription"]["text"].as_str() {
        if !done.trim().is_empty() {
            kept.push(done.trim().to_string());
        }
        return Some(String::new());
    }
    content["interimInputTranscription"]["text"].as_str().map(|t| t.trim().to_string())
}
