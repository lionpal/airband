//! ElevenLabs' realtime transcription.
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "elevenlabs", can: &[], told: &[] }, wire: &WIRE, upload: None, pass, once: true };

const WIRE: Wire = Wire { connect, audio: |_, bytes| chunk(bytes, false), end: |_| chunk(&[0u8; 3200], true), read, header: false, reopens: false };

/// ElevenLabs' token for its realtime transcription: good for a quarter of an hour, and for one connection.
fn pass(_set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let mut res = crate::net::agent(Duration::from_secs(8)).post("https://api.elevenlabs.io/v1/single-use-token/realtime_scribe").header("xi-api-key", key.trim()).header("Content-Length", "0").send_empty().map_err(|_| "Could not reach ElevenLabs.".to_string())?;
    match res.status().as_u16() {
        200 => res.body_mut().read_json::<Value>().ok().and_then(|v| v["token"].as_str().map(String::from)).ok_or_else(|| "ElevenLabs gave no pass.".to_string()),
        401 | 403 => Err("ElevenLabs did not accept that key.".into()),
        code => Err(format!("ElevenLabs said no ({code}).")),
    }
}

/// ElevenLabs' realtime transcription. The pass goes in the address. A stretch is closed when it is asked to
/// be (`commit` on a final piece of silence, when he has finished), or by the service itself after about half
/// a minute of speech; what he has said so far comes back whole each time until then.
fn connect(grant: &Pass, _request: &str) -> Result<Socket, ()> {
    const HOST: &str = "api.elevenlabs.io";
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    let mut url = format!("wss://{HOST}/v1/speech-to-text/realtime?model_id={}&audio_format=pcm_16000&commit_strategy=manual&token={}", enc(&grant.get("model")), enc(&grant.token));
    // its language codes are two letters ("en"), where a setting may say "en-US"
    if let Some(language) = grant.get("language").split('-').next().filter(|l| !l.is_empty()) {
        url.push_str(&format!("&language_code={}", enc(language)));
    }
    // the names he is likely to say, so "shop two" is not heard as something else
    for term in grant.phrases.iter().take(50) {
        url.push_str(&format!("&keyterms={}", enc(term)));
    }
    let mut socket = dial(HOST, url.into_client_request().map_err(|_| ())?)?;
    // it says when the session has started, or why it will not
    loop {
        let m = socket.read().map_err(|_| ())?;
        if m.is_close() {
            return Err(());
        }
        match serde_json::from_slice::<Value>(&m.into_data()).ok().and_then(|v| v["message_type"].as_str().map(String::from)).as_deref() {
            Some("session_started") => break,
            Some(_) => return Err(()),
            None => {}
        }
    }
    read_briefly(&socket);
    Ok(socket)
}

fn chunk(bytes: &[u8], commit: bool) -> Message {
    Message::text(json!({ "message_type": "input_audio_chunk", "audio_base_64": airband_core::audio::base64_encode(bytes), "commit": commit, "sample_rate": 16000 }).to_string())
}

/// A partial replaces the one before it; a committed stretch is final, and the next partial starts from nothing.
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool) -> Option<String> {
    let v: Value = serde_json::from_slice(&m.clone().into_data()).ok()?;
    match v["message_type"].as_str()? {
        "partial_transcript" => Some(v["text"].as_str().unwrap_or_default().trim().to_string()),
        "committed_transcript" => {
            let done = v["text"].as_str().unwrap_or_default().trim();
            if !done.is_empty() {
                kept.push(done.to_string());
            }
            *ended = true;
            Some(String::new())
        }
        _ => None,
    }
}
