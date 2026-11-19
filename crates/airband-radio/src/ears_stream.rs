//! Microsoft's speech-to-text over its streaming connection: the same audio as the plain upload, sent as he
//! speaks, with the words coming back while he is still talking. One WebSocket, opened as the microphone opens
//! so it is ready by his first word.
//!
//! The wire format is the one Microsoft's own Speech SDK uses. A text message is headers, a blank line and JSON;
//! an audio message is a two-byte header length, headers, then the audio. The service answers with
//! `speech.hypothesis` (what it has so far, revised as he goes on) and `speech.phrase` (a finished stretch).
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use airband_core::radio::EarsGrant;
use serde_json::json;
use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// `2026-11-19T10:00:00.000Z`, as the service wants on every message.
fn stamp() -> String {
    let ms = now_ms();
    let (days, rest) = ((ms / 86_400_000) as i64, ms % 86_400_000);
    // the date for a count of days since 1970
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000)
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
fn connect(grant: &EarsGrant, request: &str) -> Result<Socket, ()> {
    let host = format!("{}.stt.speech.microsoft.com", grant.region);
    let url = format!("wss://{host}/speech/recognition/conversation/cognitiveservices/v1?language={}&format=simple", grant.language);
    let mut req = url.into_client_request().map_err(|_| ())?;
    req.headers_mut().insert("Authorization", format!("Bearer {}", grant.token).parse().map_err(|_| ())?);
    req.headers_mut().insert("X-ConnectionId", random_hex(16).parse().map_err(|_| ())?);
    let address = (host.as_str(), 443).to_socket_addrs().map_err(|_| ())?.next().ok_or(())?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(6)).map_err(|_| ())?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(6)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(6)));
    // the same TLS as every other request this library makes, with the same roots
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|_| ())?.with_root_certificates(roots).with_no_client_auth();
    let (mut socket, _) = tungstenite::client_tls_with_config(req, tcp, None, Some(Connector::Rustls(Arc::new(tls)))).map_err(|_| ())?;
    let config = json!({ "context": { "system": { "name": "airband", "version": env!("CARGO_PKG_VERSION") }, "os": { "platform": std::env::consts::OS, "name": "airband", "version": "1" }, "audio": { "source": { "connectivity": "Unknown", "manufacturer": "Unknown", "model": "Unknown", "type": "Microphones" } } } });
    socket.send(text("speech.config", request, &config)).map_err(|_| ())?;
    if !grant.phrases.is_empty() {
        let names: Vec<Value> = grant.phrases.iter().map(|p| json!({ "Text": p })).collect();
        socket.send(text("speech.context", request, &json!({ "dgi": { "Groups": [{ "Type": "Generic", "Items": names }] } }))).map_err(|_| ())?;
    }
    // from here on a read waits only a moment, so audio keeps going out while answers are looked for
    if let MaybeTlsStream::Rustls(stream) = socket.get_ref() {
        let _ = stream.get_ref().set_read_timeout(Some(Duration::from_millis(12)));
    }
    Ok(socket)
}

/// Whatever the service has said since last asked, without waiting for more. `kept` gathers finished stretches;
/// returns the unfinished one, if it changed. Err when the connection has gone.
fn drain(socket: &mut Socket, kept: &mut Vec<String>, ended: &mut bool) -> Result<Option<String>, ()> {
    let mut hypothesis = None;
    loop {
        match socket.read() {
            Ok(m) if m.is_text() => match m.to_text().ok().and_then(parse) {
                Some((path, v)) if path == "speech.hypothesis" => hypothesis = v["Text"].as_str().map(String::from),
                Some((path, v)) if path == "speech.phrase" => {
                    if let Some(t) = v["DisplayText"].as_str().map(str::trim).filter(|t| !t.is_empty() && v["RecognitionStatus"] == "Success") {
                        kept.push(t.to_string());
                    }
                    hypothesis = Some(String::new());
                }
                Some((path, _)) if path == "turn.end" => *ended = true,
                _ => {}
            },
            Ok(m) if m.is_close() => return Err(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(hypothesis),
            Err(_) => return Err(()),
        }
    }
}

/// Listen for one reply over the streaming connection, telling `hearing` what he has said so far as it changes.
/// Ok(None) is silence. Err is a failure of the service or the line: nothing was lost that the caller cannot
/// get by listening again another way.
pub(crate) fn azure(grant: &EarsGrant, host: &dyn RadioHost, rate: u32, header: Vec<u8>) -> Result<Option<String>, ()> {
    let request = random_hex(16);
    // opened now, on its own thread, so it is ready by the time he has said a word
    let opening: Receiver<Result<Socket, ()>> = {
        let (tx, rx) = channel();
        let (grant, request) = (grant.clone(), request.clone());
        thread::spawn(move || {
            let _ = tx.send(connect(&grant, &request));
        });
        rx
    };
    let mut socket: Option<Socket> = None;
    let mut end = Endpointer::new(rate);
    // audio from just before he was judged to be speaking, and whatever is said before the connection is up
    let mut waiting: VecDeque<Vec<u8>> = VecDeque::from([header]);
    let (mut kept, mut ended, mut shown) = (Vec::<String>::new(), false, String::new());
    let tell = |kept: &[String], now: &str, shown: &mut String| {
        let all = [kept.join(" "), now.to_string()].join(" ").trim().to_string();
        if all != *shown {
            *shown = all.clone();
            host.hearing(all);
        }
    };
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
        waiting.push_back(bytes);
        if !end.speaking {
            // not speaking yet: keep only the header and the last half second
            while waiting.len() > 6 {
                waiting.remove(1);
            }
        }
        if socket.is_none() {
            match opening.try_recv() {
                Ok(Ok(s)) => socket = Some(s),
                Ok(Err(())) => return Err(()),
                Err(_) => {}
            }
        }
        if let (Some(s), true) = (socket.as_mut(), end.speaking) {
            for chunk in waiting.drain(..) {
                s.send(audio(&request, &chunk)).map_err(|_| ())?;
            }
            if let Some(now) = drain(s, &mut kept, &mut ended)? {
                tell(&kept, &now, &mut shown);
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return Ok(None),
            _ => {}
        }
    }
    if !end.speaking {
        return Ok(None);
    }
    host.status(RadioStatus::Sending);
    // he has stopped: whatever was not sent yet goes now, then the end of the audio, then the last words come back
    let mut s = match socket {
        Some(s) => s,
        None => opening.recv_timeout(Duration::from_secs(6)).map_err(|_| ())??,
    };
    for chunk in waiting.drain(..) {
        s.send(audio(&request, &chunk)).map_err(|_| ())?;
    }
    s.send(audio(&request, &[])).map_err(|_| ())?;
    let until = Instant::now() + Duration::from_secs(5);
    while !ended && Instant::now() < until {
        if let Some(now) = drain(&mut s, &mut kept, &mut ended)? {
            tell(&kept, &now, &mut shown);
        }
    }
    let _ = s.close(None);
    let said = kept.join(" ");
    Ok((!said.is_empty()).then_some(said))
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

    /// Against the real service, with a recording in place of the microphone:
    ///   AZURE_TOKEN=… AZURE_REGION=… SPEECH_WAV=said.wav cargo test -p airband-radio -- --ignored --nocapture streams
    #[test]
    #[ignore]
    fn streams_to_the_real_service() {
        struct Tape(Mutex<VecDeque<Vec<u8>>>, Mutex<Vec<(u128, String)>>, Instant);
        impl RadioHost for Tape {
            fn speak(&self, _: String) {}
            fn listen(&self) -> Option<String> { None }
            fn status(&self, _: RadioStatus) {}
            fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
            fn record_start(&self, _: u32) -> bool { true }
            fn record_read(&self) -> Vec<u8> {
                thread::sleep(Duration::from_millis(100));
                self.0.lock().unwrap().pop_front().unwrap_or_else(|| vec![0u8; 3200])
            }
            fn record_stop(&self) {}
            fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
            fn hearing(&self, text: String) {
                println!("{:>5} ms  {text}", self.2.elapsed().as_millis());
                self.1.lock().unwrap().push((self.2.elapsed().as_millis(), text));
            }
        }
        let wav = std::fs::read(std::env::var("SPEECH_WAV").unwrap()).unwrap();
        let pcm = &wav[44..];
        let tape = Tape(Mutex::new(pcm.chunks(3200).map(|c| c.to_vec()).collect()), Mutex::new(Vec::new()), Instant::now());
        let grant = EarsGrant { kind: "azure".into(), token: std::env::var("AZURE_TOKEN").unwrap(), region: std::env::var("AZURE_REGION").unwrap(), language: "en-US".into(), phrases: vec![] };
        let said = azure(&grant, &tape, 16000, crate::ears::wav_header()).unwrap();
        println!("{:>5} ms  FINAL: {said:?} (the recording is {} ms long)", tape.2.elapsed().as_millis(), pcm.len() / 32);
        assert!(said.is_some_and(|s| !s.is_empty()));
        assert!(tape.1.lock().unwrap().len() >= 2, "words should arrive while he is still speaking");
    }
}
