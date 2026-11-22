//! Listening by audio chunks with manual commits: each piece of audio goes up as base64 in JSON, what he has
//! said so far comes back whole each time (`partial_transcript`), and a stretch is closed when it is asked to
//! be (`commit` on a final piece of silence, when he has finished) or by the service itself after about half a
//! minute of speech (`committed_transcript`). Where the service is, where its pass is asked for and how the
//! key is presented are its row's to say (`stream`, `pass`, `auth`).
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "chunks", can: &[], told: &["stream", "pass", "auth"] }, wire: &WIRE, pass, once: true };

const WIRE: Wire = Wire { connect, audio: |_, bytes| chunk(bytes, false), end: |_| chunk(&[0u8; 3200], true), read, header: false, reopens: false };

/// The service's token: asked for with nothing in the request, good for a quarter of an hour and for one
/// connection.
fn pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    pass_field(by, service, set, key, None, "token")
}

/// The pass goes in the address, with the model, the language and the names he is likely to say.
fn connect(grant: &Pass, _request: &str) -> Result<Socket, ()> {
    let (stream, host) = grant.stream()?;
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    let mut url = format!("{stream}?model_id={}&audio_format=pcm_16000&commit_strategy=manual&token={}", enc(grant.get("model")), enc(&grant.token));
    // its language codes are two letters ("en"), where a setting may say "en-US"
    if let Some(language) = grant.get("language").split('-').next().filter(|l| !l.is_empty()) {
        url.push_str(&format!("&language_code={}", enc(language)));
    }
    // the names he is likely to say, so "shop two" is not heard as something else
    for term in grant.phrases.iter().take(50) {
        url.push_str(&format!("&keyterms={}", enc(term)));
    }
    let mut socket = dial(&host, url.into_client_request().map_err(|_| ())?)?;
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
    Message::text(json!({ "message_type": "input_audio_chunk", "audio_base_64": airband_core::encode::base64(bytes), "commit": commit, "sample_rate": 16000 }).to_string())
}

/// A partial replaces the one before it; a committed stretch is final, and the next partial starts from nothing.
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool) -> Option<String> {
    let v: Value = serde_json::from_slice(&m.clone().into_data()).ok()?;
    match v["message_type"].as_str()? {
        // one without its text says nothing about what was heard, and is passed over
        "partial_transcript" => Some(v["text"].as_str()?.trim().to_string()),
        "committed_transcript" => {
            let done = v["text"].as_str()?.trim();
            if !done.is_empty() {
                kept.push(done.to_string());
            }
            *ended = true;
            Some(String::new())
        }
        _ => None,
    }
}
