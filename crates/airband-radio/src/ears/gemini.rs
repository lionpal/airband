//! Google's Live API, with its model that only transcribes.
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "gemini", can: &[], told: &[] }, wire: &WIRE, upload: None, pass, once: false };

const WIRE: Wire = Wire { connect, audio, end: |_| Message::text(json!({ "realtimeInput": { "audioStreamEnd": true } }).to_string()), read, header: false, reopens: false };

/// Google's short-lived token for its Live API: good for twenty minutes and for any number of connections in
/// that time, so one fetched ahead is still good when he opens the microphone, and again if a connection drops.
fn pass(_set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let until = crate::ears_stream::stamp_at(now_ms() + 20 * 60 * 1000);
    let body = serde_json::json!({ "uses": 0, "expireTime": until, "newSessionExpireTime": until });
    let mut res = crate::net::agent(Duration::from_secs(8)).post("https://generativelanguage.googleapis.com/v1beta/auth_tokens").header("x-goog-api-key", key.trim()).send_json(&body).map_err(|_| "Could not reach Google's speech service.".to_string())?;
    match res.status().as_u16() {
        200 => res.body_mut().read_json::<Value>().ok().and_then(|v| v["name"].as_str().map(String::from)).ok_or_else(|| "Google's speech service gave no pass.".to_string()),
        400 | 401 | 403 => Err("Google's speech service did not accept that key.".into()),
        code => Err(format!("Google's speech service said no ({code}).")),
    }
}

/// Google's Live API, with its model that only transcribes. The pass is a short-lived token made for the Live
/// API; the model, the language and the names to expect go in the first message.
fn connect(grant: &EarsGrant, _request: &str) -> Result<Socket, ()> {
    const HOST: &str = "generativelanguage.googleapis.com";
    let url = format!("wss://{HOST}/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained?access_token={}", grant.token);
    let mut socket = dial(HOST, url.into_client_request().map_err(|_| ())?)?;
    let mut hearing = json!({ "mode": "VERBATIM" });
    if !grant.get("language").is_empty() {
        hearing["languageCodes"] = json!([grant.get("language")]);
    }
    if !grant.phrases.is_empty() {
        // it does best with a hundred or fewer
        hearing["customVocabulary"] = json!(grant.phrases.iter().take(100).collect::<Vec<_>>());
    }
    let setup = json!({ "setup": { "model": format!("models/{}", grant.get("model")), "generationConfig": { "responseModalities": ["TEXT"] }, "inputAudioTranscription": hearing } });
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
    Message::text(json!({ "realtimeInput": { "audio": { "data": airband_core::audio::base64_encode(bytes), "mimeType": "audio/pcm;rate=16000" } } }).to_string())
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
