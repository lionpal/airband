//! Listening over the Universal Speech Protocol, the wire format of a speech SDK's streaming connection: a
//! text message is headers, a blank line and JSON; an audio message is a two-byte header length, headers, then
//! the audio. The service answers with `speech.hypothesis` (what it has so far, revised as he goes on) and
//! `speech.phrase` (a finished stretch). Where the service is, where its pass is asked for and how the key is
//! presented are its row's to say (`stream`, `pass`, `auth`).
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "usp", can: &[], told: &["stream", "pass", "auth"] }, wire: &WIRE, pass, once: false };

const WIRE: Wire = Wire { connect, audio, end: |request| audio(request, &[]), read, header: true, reopens: true };

/// `2026-11-19T10:00:00.000Z`, as the service wants on every message.
fn stamp() -> String {
    airband_core::clock::rfc3339(now_ms())
}

fn text(path: &str, request: &str, body: &Value) -> Message {
    Message::text(format!("Path: {path}\r\nX-RequestId: {request}\r\nX-Timestamp: {}\r\nContent-Type: application/json\r\n\r\n{body}", stamp()))
}

fn audio(request: &str, bytes: &[u8]) -> Message {
    let head = format!("Path: audio\r\nX-RequestId: {request}\r\nX-Timestamp: {}\r\nContent-Type: audio/x-wav\r\n", stamp());
    let mut out = Vec::with_capacity(2 + head.len() + bytes.len());
    out.extend_from_slice(&(head.len() as u16).to_be_bytes());
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(bytes);
    Message::binary(out)
}

/// What a text message from the service is about, and its JSON.
fn parse(message: &str) -> Option<(String, Value)> {
    let (head, body) = message.split_once("\r\n\r\n")?;
    let path = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("path")).map(|(_, v)| v.trim().to_lowercase()))?;
    Some((path, serde_json::from_str(body).unwrap_or(Value::Null)))
}

/// Open the connection and say what is coming: the microphone's format, and the names worth listening for.
fn connect(grant: &Pass, request: &str) -> Result<Socket, ()> {
    let (url, host) = grant.stream()?;
    let url = format!("{url}?language={}&format=simple&profanity=raw", grant.get("language"));
    let mut req = url.into_client_request().map_err(|_| ())?;
    req.headers_mut().insert("Authorization", format!("Bearer {}", grant.token).parse().map_err(|_| ())?);
    req.headers_mut().insert("X-ConnectionId", random_hex(16).parse().map_err(|_| ())?);
    let mut socket = dial(&host, req)?;
    let config = json!({ "context": { "system": { "name": "airband", "version": env!("CARGO_PKG_VERSION") }, "os": { "platform": std::env::consts::OS, "name": "airband", "version": "1" }, "audio": { "source": { "connectivity": "Unknown", "manufacturer": "Unknown", "model": "Unknown", "type": "Microphones" } } } });
    socket.send(text("speech.config", request, &config)).map_err(|_| ())?;
    if !grant.phrases.is_empty() {
        let names: Vec<Value> = grant.phrases.iter().map(|p| json!({ "Text": p })).collect();
        socket.send(text("speech.context", request, &json!({ "dgi": { "Groups": [{ "Type": "Generic", "Items": names }] } }))).map_err(|_| ())?;
    }
    read_briefly(&socket);
    Ok(socket)
}

/// What one message from the service says: an unfinished stretch (replacing the last), a finished one
/// (into `kept`), or that its turn is over.
fn read(m: &Message, kept: &mut Vec<String>, ended: &mut bool) -> Option<String> {
    match m.to_text().ok().and_then(parse) {
        Some((path, v)) if path == "speech.hypothesis" => v["Text"].as_str().map(String::from),
        Some((path, v)) if path == "speech.phrase" => {
            if let Some(t) = v["DisplayText"].as_str().map(str::trim).filter(|t| !t.is_empty() && v["RecognitionStatus"] == "Success") {
                kept.push(t.to_string());
            }
            Some(String::new())
        }
        Some((path, _)) if path == "turn.end" => {
            *ended = true;
            None
        }
        _ => None,
    }
}

/// The service's token for a key: asked for with nothing in the request, and the answer is the token itself.
fn pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    pass_whole(by, service, set, key)
}
