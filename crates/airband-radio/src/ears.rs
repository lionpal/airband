//! Inbound: turning what he says into text. Which service does it is the tower's setting, separate from the
//! voice. `device` is the host's own recognition. Any other is a provider here: the host only hands over raw
//! microphone audio, and deciding when he has finished (airband_core::audio::Endpointer) and talking to the
//! service are done once, for every platform.
//!
//! A provider is a function from (pass, microphone) to text; adding one is one more arm in [`hear`].
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use airband_core::radio::EarsGrant;
use std::io::Read;
use std::sync::mpsc::{Receiver as Rx, sync_channel};
use std::time::Instant;

const RATE: u32 = 16000;
/// A pass is asked for again after this long (Microsoft's last ten minutes).
const PASS_LIFE: Duration = Duration::from_secs(300);

#[derive(Default)]
pub(crate) struct Ears {
    grant: Mutex<Option<(EarsGrant, Instant)>>,
    /// After a service fails, the host's own recognition is used until this time.
    device_until: Mutex<Option<Instant>>,
}

impl Ears {
    fn grant(&self, link: &Link) -> EarsGrant {
        if let Some((g, when)) = self.grant.lock().unwrap().clone() {
            if when.elapsed() < PASS_LIFE {
                return g;
            }
        }
        let g: EarsGrant = match link.request("GET", "/ears", None) {
            Ok((200, v)) => serde_json::from_value(v).unwrap_or_default(),
            _ => EarsGrant::default(),
        };
        *self.grant.lock().unwrap() = Some((g.clone(), Instant::now()));
        g
    }

    /// Listen for one reply. None if he said nothing.
    pub(crate) fn hear(&self, link: &Link, host: &dyn RadioHost) -> Option<String> {
        let resting = self.device_until.lock().unwrap().is_some_and(|t| Instant::now() < t);
        let grant = if resting { EarsGrant::default() } else { self.grant(link) };
        match grant.kind.as_str() {
            "azure" if host.record_start(RATE) => {
                // as he speaks, with the words coming back while he talks. If that connection cannot be made, the
                // same microphone is listened to again the plain way (one upload, the words at the end).
                let heard = crate::ears_stream::azure(&grant, host, RATE, wav_header());
                host.record_stop();
                let heard = match heard {
                    Err(()) if host.record_start(RATE) => {
                        let plain = azure(&grant, host);
                        host.record_stop();
                        plain
                    }
                    other => other,
                };
                match heard {
                    Ok(text) => text,
                    Err(_) => {
                        // the service or the line failed: use the host's own recognition for the next minute
                        *self.device_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(60));
                        *self.grant.lock().unwrap() = None;
                        host.listen()
                    }
                }
            }
            _ => host.listen(),
        }
    }
}

/// Lets an HTTP upload read microphone audio as it is captured.
struct Feed(Rx<Vec<u8>>, Vec<u8>);
impl Read for Feed {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.1.is_empty() {
            match self.0.recv() {
                Ok(more) => self.1 = more,
                Err(_) => return Ok(0), // the microphone side hung up: end of the recording
            }
        }
        let n = out.len().min(self.1.len());
        out[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

/// Microsoft's speech-to-text, fed while he is still talking so the answer is ready almost as he stops.
/// Ok(None) is silence; Err is a failure of the service or the line.
fn azure(grant: &EarsGrant, host: &dyn RadioHost) -> Result<Option<String>, ()> {
    let mut end = Endpointer::new(RATE);
    // a little audio from before he was judged to be speaking, so the first word is not clipped
    let mut lead: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
    let mut upload: Option<(std::sync::mpsc::SyncSender<Vec<u8>>, thread::JoinHandle<Result<Option<String>, ()>>)> = None;
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
        if end.speaking && upload.is_none() {
            let (tx, rx) = sync_channel::<Vec<u8>>(256);
            let url = format!("https://{}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language={}&format=simple", grant.region, grant.language);
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
            upload = Some((tx, worker));
        }
        match &upload {
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
    let Some((tx, worker)) = upload else { return Ok(None) };
    drop(tx); // ends the upload; the service answers with what it heard
    host.status(RadioStatus::Sending);
    worker.join().map_err(|_| ())?
}

/// A WAV header for a recording whose length is not known yet.
pub(crate) fn wav_header() -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&(RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&0u32.to_le_bytes());
    h
}
