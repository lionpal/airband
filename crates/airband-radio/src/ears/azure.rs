//! Microsoft's speech-to-text. Live, over its streaming connection, in the wire format Microsoft's own Speech
//! SDK uses: a text message is headers, a blank line and JSON; an audio message is a two-byte header length,
//! headers, then the audio. The service answers with `speech.hypothesis` (what it has so far, revised as he
//! goes on) and `speech.phrase` (a finished stretch). And the same audio in one upload, for when the live
//! connection cannot be made.
use super::*;
use crate::ears_stream::{Socket, Wire, dial, read_briefly};
use serde_json::json;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

pub(super) const LISTENER: Listener = Listener { is: Interface { name: "azure", can: &[], told: &[] }, wire: &WIRE, upload: Some(upload), pass, once: false };

const WIRE: Wire = Wire { connect, audio, end: |request| audio(request, &[]), read, header: true, reopens: true };

/// `2026-11-19T10:00:00.000Z`, as the service wants on every message.
fn stamp() -> String {
    crate::ears_stream::stamp_at(now_ms())
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
    let host = format!("{}.stt.speech.microsoft.com", grant.get("region"));
    let url = format!("wss://{host}/speech/recognition/conversation/cognitiveservices/v1?language={}&format=simple&profanity=raw", grant.get("language"));
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

/// What one message from Microsoft's service says: an unfinished stretch (replacing the last), a finished one
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

fn pass(set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let region = set.get("region").map(|r| r.trim().to_lowercase()).unwrap_or_default();
    if region.is_empty() || !region.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("That is not a region. It looks like eastus or japaneast.".into());
    }
    let agent = crate::net::agent(Duration::from_secs(8));
    let mut res = agent.post(format!("https://{region}.api.cognitive.microsoft.com/sts/v1.0/issueToken")).header("Ocp-Apim-Subscription-Key", key.trim()).header("Content-Length", "0").send_empty().map_err(|_| "Could not reach the speech service. Check the region.".to_string())?;
    match res.status().as_u16() {
        200 => res.body_mut().read_to_string().map_err(|_| "The speech service gave no pass.".to_string()),
        401 | 403 => Err("The speech service did not accept that key for that region.".into()),
        code => Err(format!("The speech service said no ({code}).")),
    }
}

/// Microsoft's speech-to-text, fed while he is still talking so the answer is ready almost as he stops.
/// Ok(None) is silence; Err is a failure of the service or the line.
fn upload(grant: &Pass, host: &dyn RadioHost, manual: bool) -> Result<Option<String>, ()> {
    let mut end = Endpointer::new(RATE);
    // a little audio from before he was judged to be speaking, so the first word is not clipped
    let mut lead: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
    let mut sending: Option<(std::sync::mpsc::SyncSender<Vec<u8>>, thread::JoinHandle<Result<Option<String>, ()>>)> = None;
    loop {
        let bytes = host.record_read();
        if bytes.is_empty() {
            break; // the host stopped the microphone
        }
        let mut verdict = Heard::Waiting;
        for frame in samples(&bytes).chunks(320) {
            verdict = end.feed(frame);
            if matches!(verdict, Heard::Finished | Heard::Silence) {
                break;
            }
        }
        // he opened the microphone himself: it is all sent, and only he ends it
        if manual {
            verdict = Heard::Waiting;
        }
        if (end.speaking || manual) && sending.is_none() {
            let (tx, rx) = sync_channel::<Vec<u8>>(256);
            let url = format!("https://{}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language={}&format=simple&profanity=raw", grant.get("region"), grant.get("language"));
            let auth = format!("Bearer {}", grant.token);
            let worker = thread::spawn(move || -> Result<Option<String>, ()> {
                let mut body = Feed(rx, wav_header());
                let mut res = net::agent(Duration::from_secs(60))
                    .post(&url)
                    .header("Authorization", &auth)
                    .header("Content-Type", "audio/wav; codecs=audio/pcm; samplerate=16000")
                    .header("Accept", "application/json")
                    .send(ureq::SendBody::from_reader(&mut body))
                    .map_err(|_| ())?;
                if res.status().as_u16() != 200 {
                    return Err(());
                }
                let v: Value = res.body_mut().read_json().map_err(|_| ())?;
                Ok(v["DisplayText"].as_str().map(str::trim).filter(|t| !t.is_empty() && v["RecognitionStatus"] == "Success").map(String::from))
            });
            for early in lead.drain(..) {
                let _ = tx.send(early);
            }
            sending = Some((tx, worker));
        }
        match &sending {
            Some((tx, _)) => {
                let _ = tx.send(bytes);
            }
            None => {
                lead.push_back(bytes);
                while lead.len() > 5 {
                    lead.pop_front();
                }
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return Ok(None),
            _ => {}
        }
    }
    let Some((tx, worker)) = sending else { return Ok(None) };
    drop(tx); // ends the upload; the service answers with what it heard
    host.status(RadioStatus::Sending);
    worker.join().map_err(|_| ())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_from_the_service_is_read_whatever_its_spacing() {
        let (path, v) = parse("X-RequestId:abc\r\nPath:speech.hypothesis\r\nContent-Type:application/json; charset=utf-8\r\n\r\n{\"Text\":\"make the add\",\"Offset\":100}").unwrap();
        assert_eq!((path.as_str(), v["Text"].as_str()), ("speech.hypothesis", Some("make the add")));
        assert_eq!(parse("path: Turn.End\r\n\r\n{}").map(|(p, _)| p).as_deref(), Some("turn.end"));
        assert!(parse("no blank line").is_none());
        // 2026-11-19T10:00:00.000Z is 1795082400000
        assert_eq!(stamp().len(), 24);
    }
}
