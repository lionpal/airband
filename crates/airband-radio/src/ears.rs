//! Inbound: turning what he says into text. The host only hands over raw microphone audio; deciding when he has
//! finished (airband_core::audio::Endpointer) and talking to the service are done once, for every platform.
//!
//! Which service is a row of `airband_core::providers::PROVIDERS`, and there are two ways to listen with one:
//!
//! - live, while he talks, by a dialect here (a row of [`LISTENERS`]): the phone talks to the service itself,
//!   with a short-lived pass. The words come back as he speaks.
//! - by upload (`hears.rs`): the phone records until he stops and the recording is sent off whole. With a key
//!   of his own on the phone it goes straight to the service; with the key on his computer it goes to the
//!   tower, which makes the call.
//!
//! What to listen with comes as an [`EarsGrant`]: from a key of his own on this device first, else from the
//! tower, else from the relay when it lends its service. Nothing here, in the tower or in the app names a
//! service.
use super::*;
use airband_core::audio::{BLOCK_SAMPLES, Encoder, Endpointer, Heard, base64_encode, samples};
use airband_core::providers::{BY_TOWER, By, Job, Use};
use airband_core::radio::{EarsGrant, HearPart, HeardText};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc::{Receiver as Rx, sync_channel};
use std::time::Instant;

const RATE: u32 = 16000;
/// A pass is asked for again after this long (the shortest-lived of them lasts ten minutes).
const PASS_LIFE: Duration = Duration::from_secs(300);
/// What a pass says, on this device only, when the recording goes straight to a service with his own key.
const BY_OWN_KEY: &str = "own-key";
/// And when a model on the device does the listening.
const BY_MODEL: &str = "on-device";

/// A pass for the service, live as he speaks: the pass, the microphone, its rate, the header of a recording,
/// and whether he opened the microphone himself. Ok(None) is silence.
type Stream = fn(Arc<dyn Fn() -> EarsGrant + Send + Sync>, &dyn RadioHost, u32, Vec<u8>, bool) -> Result<Option<String>, ()>;

/// A live listening dialect.
pub(crate) struct Listener {
    /// What a pass calls it (`EarsGrant::kind`), and what a provider's row names (`By::Listener`).
    pub kind: &'static str,
    stream: Stream,
    /// The same microphone in one upload to the same service, for when the live connection cannot be made.
    upload: Option<fn(&EarsGrant, &dyn RadioHost, bool) -> Result<Option<String>, ()>>,
    /// A short-lived pass for a key: the service's settings, and the key. Err says why not, in words for him.
    pass: fn(&BTreeMap<String, String>, &str) -> Result<String, String>,
    /// Its pass is good for one connection only: a new one is asked for each time, and none is kept.
    once: bool,
}

pub(crate) const LISTENERS: &[Listener] = &[
    Listener { kind: "azure", stream: crate::ears_stream::azure, upload: Some(azure), pass: azure_pass, once: false },
    Listener { kind: "gemini", stream: crate::ears_stream::gemini, upload: None, pass: gemini_pass, once: false },
    Listener { kind: "elevenlabs", stream: crate::ears_stream::elevenlabs, upload: None, pass: elevenlabs_pass, once: true },
];

/// A pass for this dialect is used up by one connection, so it is never kept for the next.
pub(crate) fn once(kind: &str) -> bool {
    listener(kind).is_some_and(|l| l.once)
}

fn listener(kind: &str) -> Option<&'static Listener> {
    LISTENERS.iter().find(|l| l.kind == kind)
}

/// A listening service of his own, when he has given the app a key for one. With it the phone needs neither
/// his computer nor the relay to listen. The host keeps the key (in the phone's keychain) and gives it here
/// at launch; it is never sent anywhere but to the service it is for.
static OWN: Mutex<Option<Use>> = Mutex::new(None);
/// The names the tower last said he is likely to say: used with a pass of his own, which comes with none.
static PHRASES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A listening model on the device, when he has chosen one: where its files are. It comes before everything
/// else: nothing he says leaves the device.
static LOCAL: Mutex<Option<(String, String)>> = Mutex::new(None);

pub(crate) fn set_local(dir: &str, kind: &str) {
    let chosen = crate::models::model(kind).filter(|m| m.job == Job::Ears && crate::models::fetched(dir, m)).map(|m| (kind.to_string(), crate::models::folder(dir, m).display().to_string()));
    *LOCAL.lock().unwrap() = chosen;
}

/// The model of this kind is being taken off the device: if it is the one listening, it is no longer.
pub(crate) fn stop_using(kind: &str) {
    let mut local = LOCAL.lock().unwrap();
    if local.as_ref().is_some_and(|(k, _)| k == kind) {
        *local = None;
    }
}

pub(crate) fn set_own(with: Option<Use>) {
    *OWN.lock().unwrap() = with.filter(|u| u.offer(Job::Ears).is_some());
}

/// A pass from a live service for a key. Err says why not, in words for him.
pub(crate) fn pass_for(kind: &str, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let l = listener(kind).ok_or("This version of airband does not know that listening service.")?;
    (l.pass)(set, key)
}

/// What a listening service makes of a recording, heard the way a phone would send it: live for a live
/// service (with how many times the words on screen changed while it played), in one upload for the others.
/// `pcm` is 16 kHz 16-bit mono. Blocks for as long as the recording is. For trying a service by hand.
pub fn try_listening(with: &Use, pcm: &[u8]) -> Result<(String, usize), String> {
    match with.offer(Job::Ears).map(|(_, o)| o.by) {
        Some(By::Listener(kind)) => {
            let l = listener(kind).ok_or("This version of airband does not know that listening service.")?;
            let set = with.settings(Job::Ears);
            let grant = EarsGrant { kind: kind.into(), token: (l.pass)(&set, &with.key)?, set, phrases: Vec::new() };
            let tape = crate::ears_stream::Tape::of(pcm, false);
            let said = (l.stream)(Arc::new(move || grant.clone()), &tape, RATE, wav_header(), false).map_err(|_| "The live connection to the listening service failed.".to_string())?;
            let changes = tape.heard.lock().unwrap().len();
            Ok((said.unwrap_or_default(), changes))
        }
        Some(_) => crate::hears::transcribe(with, &crate::hears::wav(RATE, &samples(pcm)), &[]).map(|said| (said, 0)),
        None => Err("This version of airband does not know that listening service.".into()),
    }
}

/// Does this service listen with this key and these settings? One real call to it.
pub(crate) fn check(with: &Use) -> Result<(), String> {
    match with.offer(Job::Ears).map(|(_, o)| o.by) {
        Some(By::Listener(kind)) => pass_for(kind, &with.settings(Job::Ears), &with.key).map(|_| ()),
        Some(_) => crate::hears::check(with),
        None => Err("This version of airband does not know that listening service.".into()),
    }
}

/// Google's short-lived token for its Live API: good for twenty minutes and for any number of connections in
/// that time, so one fetched ahead is still good when he opens the microphone, and again if a connection drops.
fn gemini_pass(_set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let until = crate::ears_stream::stamp_at(now_ms() + 20 * 60 * 1000);
    let body = serde_json::json!({ "uses": 0, "expireTime": until, "newSessionExpireTime": until });
    let mut res = crate::net::agent(Duration::from_secs(8)).post("https://generativelanguage.googleapis.com/v1beta/auth_tokens").header("x-goog-api-key", key.trim()).send_json(&body).map_err(|_| "Could not reach Google's speech service.".to_string())?;
    match res.status().as_u16() {
        200 => res.body_mut().read_json::<Value>().ok().and_then(|v| v["name"].as_str().map(String::from)).ok_or_else(|| "Google's speech service gave no pass.".to_string()),
        400 | 401 | 403 => Err("Google's speech service did not accept that key.".into()),
        code => Err(format!("Google's speech service said no ({code}).")),
    }
}

/// ElevenLabs' token for its realtime transcription: good for a quarter of an hour, and for one connection.
fn elevenlabs_pass(_set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let mut res = crate::net::agent(Duration::from_secs(8)).post("https://api.elevenlabs.io/v1/single-use-token/realtime_scribe").header("xi-api-key", key.trim()).header("Content-Length", "0").send_empty().map_err(|_| "Could not reach ElevenLabs.".to_string())?;
    match res.status().as_u16() {
        200 => res.body_mut().read_json::<Value>().ok().and_then(|v| v["token"].as_str().map(String::from)).ok_or_else(|| "ElevenLabs gave no pass.".to_string()),
        401 | 403 => Err("ElevenLabs did not accept that key.".into()),
        code => Err(format!("ElevenLabs said no ({code}).")),
    }
}

fn azure_pass(set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
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

#[derive(Default, Clone)]
pub(crate) struct Ears {
    grant: Arc<Mutex<Option<(EarsGrant, Instant)>>>,
}

impl Ears {
    fn grant(&self, link: &Link) -> EarsGrant {
        if let Some((g, when)) = self.grant.lock().unwrap().clone() {
            if when.elapsed() < PASS_LIFE && !once(&g.kind) {
                return g;
            }
        }
        // A model on the device comes first of all, then a key of his own: no computer and no relay is asked.
        if LOCAL.lock().unwrap().is_some() {
            return EarsGrant { kind: BY_MODEL.into(), token: String::new(), set: Default::default(), phrases: Vec::new() };
        }
        let own = OWN.lock().unwrap().clone();
        if let Some(own) = own {
            let set = own.settings(Job::Ears);
            let made = match own.offer(Job::Ears).map(|(_, o)| o.by) {
                Some(By::Listener(kind)) => pass_for(kind, &set, &own.key).ok().map(|token| EarsGrant { kind: kind.into(), token, set, phrases: PHRASES.lock().unwrap().clone() }),
                // by upload, with the key that is already here: nothing to fetch
                Some(_) => Some(EarsGrant { kind: BY_OWN_KEY.into(), token: String::new(), set, phrases: PHRASES.lock().unwrap().clone() }),
                None => None,
            };
            if let Some(g) = made {
                *self.grant.lock().unwrap() = Some((g.clone(), Instant::now()));
                return g;
            }
        }
        // His computer says what to listen with. When it cannot be reached, the relay lends the same service, so
        // he can still say what he has to say: it is kept for the computer until it is back.
        let g: EarsGrant = match link.request("GET", "/ears", None) {
            Ok((200, v)) => serde_json::from_value(v).unwrap_or_default(),
            _ => link.relay_ears().unwrap_or_default(),
        };
        if !g.phrases.is_empty() {
            *PHRASES.lock().unwrap() = g.phrases.clone();
        }
        let g = if g.listens() { g } else { link.relay_ears().unwrap_or(g) };
        // only a pass that can be used is kept: with none, the tower is asked again next time
        if g.listens() {
            *self.grant.lock().unwrap() = Some((g.clone(), Instant::now()));
        }
        g
    }

    /// Ask for the pass now, so it is there before he next opens the microphone.
    pub(crate) fn warm(&self, link: &Link) {
        let _ = self.grant(link);
        // with a key of his own the tower is still asked, when it is there, for the names he is likely to say
        if OWN.lock().unwrap().is_some() {
            if let Ok((200, v)) = link.request("GET", "/ears", None) {
                let phrases = serde_json::from_value::<EarsGrant>(v).map(|g| g.phrases).unwrap_or_default();
                if !phrases.is_empty() {
                    if let Some((g, _)) = self.grant.lock().unwrap().as_mut() {
                        g.phrases = phrases.clone();
                    }
                    *PHRASES.lock().unwrap() = phrases;
                }
            }
        }
    }

    /// Open the microphone and listen. The microphone opens first, at once: the pass for the listening service
    /// and the connection to it are made while he is already talking, and what he said meanwhile is sent when
    /// they are ready. With `manual` he opened it himself: it stays open, through any pause, until the host
    /// closes it, and everything he said comes back. Without, it is open for one reply and closes by itself
    /// when he stops talking.
    pub(crate) fn hear(&self, link: &Arc<Link>, host: &dyn RadioHost, manual: bool) -> Listened {
        let said = |t: Option<String>| t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(Listened::Said).unwrap_or(Listened::Nothing);
        if !host.record_start(RATE) {
            // no microphone to read: a terminal's typed line will do; a phone's own recognition will not
            return if host.own_voice() { said(host.listen()) } else { Listened::Cannot };
        }
        let pass: Arc<dyn Fn() -> EarsGrant + Send + Sync> = {
            let (ears, link) = (self.clone(), link.clone());
            Arc::new(move || ears.grant(&link))
        };
        // Which way: the pass says. By upload, the recording is made here and sent off whole, to the service
        // with his own key or to his computer, which has the key.
        let granted = self.grant(link);
        if granted.kind == BY_MODEL {
            let folder = LOCAL.lock().unwrap().clone().map(|(_, folder)| folder);
            let heard = folder.ok_or(()).and_then(|folder| by_model(host, folder, manual));
            host.record_stop();
            return match heard {
                Ok(text) => said(text),
                // the model would not run: it is no longer the one listening, and the next try asks elsewhere
                Err(()) => {
                    *LOCAL.lock().unwrap() = None;
                    *self.grant.lock().unwrap() = None;
                    Listened::Cannot
                }
            };
        }
        if granted.kind == BY_OWN_KEY || granted.kind == BY_TOWER {
            let clip = record(host, manual);
            host.record_stop();
            let heard = match clip {
                Ok(None) => return Listened::Nothing,
                Err(()) => Err(()),
                Ok(Some(pcm)) => {
                    host.status(RadioStatus::Sending);
                    match (granted.kind == BY_OWN_KEY, OWN.lock().unwrap().clone()) {
                        (true, Some(own)) => crate::hears::transcribe(&own, &crate::hears::wav(RATE, &pcm), &granted.phrases).map_err(|_| ()),
                        (true, None) => Err(()),
                        (false, _) => by_tower(link, &pcm),
                    }
                }
            };
            return match heard {
                Ok(text) => said(Some(text)),
                Err(()) => {
                    *self.grant.lock().unwrap() = None;
                    Listened::Cannot
                }
            };
        }
        // Live: one this build has no code for cannot be listened with.
        let Some(with) = listener(&granted.kind) else {
            host.record_stop();
            *self.grant.lock().unwrap() = None;
            return if host.own_voice() { said(host.listen()) } else { Listened::Cannot };
        };
        // as he speaks, with the words coming back while he talks. If that connection cannot be made, the same
        // microphone is listened to again the plain way (one upload, the words at the end), by a service that has one.
        let heard = (with.stream)(pass, host, RATE, wav_header(), manual);
        host.record_stop();
        let heard = match (heard, with.upload) {
            (Err(()), Some(upload)) if !manual && host.record_start(RATE) => {
                let plain = upload(&self.grant(link), host, manual);
                host.record_stop();
                plain
            }
            (heard, _) => heard,
        };
        match heard {
            Ok(text) => said(text),
            Err(_) => {
                // the service or the line failed, or there is none: ask for a fresh pass next time
                *self.grant.lock().unwrap() = None;
                if host.own_voice() { said(host.listen()) } else { Listened::Cannot }
            }
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

/// The microphone until he stops, as samples: for a service that takes a finished recording. Ok(None) when he
/// said nothing. A little from before he was judged to be speaking is kept, so the first word is not clipped.
fn record(host: &dyn RadioHost, manual: bool) -> Result<Option<Vec<i16>>, ()> {
    record_with(host, manual, &mut |_| {})
}

/// The same, telling `so_far` what has been kept each time more is, while he is still talking.
fn record_with(host: &dyn RadioHost, manual: bool, so_far: &mut dyn FnMut(&[i16])) -> Result<Option<Vec<i16>>, ()> {
    let mut end = Endpointer::new(RATE);
    let mut lead: std::collections::VecDeque<Vec<i16>> = std::collections::VecDeque::new();
    let mut kept: Vec<i16> = Vec::new();
    let mut speaking = false;
    loop {
        let bytes = host.record_read();
        if bytes.is_empty() {
            break; // the host stopped the microphone
        }
        let heard = samples(&bytes);
        let mut verdict = Heard::Waiting;
        for frame in heard.chunks(320) {
            verdict = end.feed(frame);
            if matches!(verdict, Heard::Finished | Heard::Silence) {
                break;
            }
        }
        // he opened the microphone himself: it is all kept, and only he ends it
        if manual {
            verdict = Heard::Waiting;
        }
        if (end.speaking || manual) && !speaking {
            speaking = true;
            lead.drain(..).for_each(|early| kept.extend(early));
        }
        if speaking {
            kept.extend(heard);
            so_far(&kept);
        } else {
            lead.push_back(heard);
            while lead.len() > 5 {
                lead.pop_front();
            }
        }
        match verdict {
            Heard::Finished => break,
            Heard::Silence => return Ok(None),
            _ => {}
        }
    }
    Ok((speaking && !kept.is_empty()).then_some(kept))
}

/// Listen with a model on the device, which the host runs. The model takes a whole recording, so to show his
/// words while he talks the recording so far is read again about twice a second, on a thread of its own: the
/// microphone is never kept waiting, and a reading that is still going when the next is due is let finish
/// and the next one skipped. When he stops, all of it is read once more, and that is what he said.
fn by_model(host: &dyn RadioHost, folder: String, manual: bool) -> Result<Option<String>, ()> {
    let bytes = |pcm: &[i16]| pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let clip = thread::scope(|scope| {
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        let reading = folder.clone();
        scope.spawn(move || {
            while let Ok(pcm) = rx.recv() {
                if let Some(now) = host.transcribe(reading.clone(), pcm) {
                    host.hearing(now);
                }
            }
        });
        let mut read_at = 0usize;
        // dropping `tx` when this returns ends the thread above, once the reading it is on has finished
        record_with(host, manual, &mut |kept| {
            if kept.len() >= read_at + RATE as usize / 2 && tx.try_send(bytes(kept)).is_ok() {
                read_at = kept.len();
            }
        })
    })?;
    let Some(pcm) = clip else { return Ok(None) };
    host.status(RadioStatus::Sending);
    host.transcribe(folder, bytes(&pcm)).map(|text| Some(text.trim().to_string()).filter(|t| !t.is_empty())).ok_or(())
}

/// Send a recording to the tower, which has the key for the listening service, and take back the words. It goes
/// in parts of ten seconds, each small enough to pass a relay sealed, in the compressed form speech comes in.
fn by_tower(link: &Link, pcm: &[i16]) -> Result<String, ()> {
    let clip = airband_core::relay::random_hex(8);
    let parts: Vec<&[i16]> = pcm.chunks(RATE as usize * 10).collect();
    let mut encoder = Encoder::default();
    for (i, part) in parts.iter().enumerate() {
        let blocks: Vec<u8> = part.chunks(BLOCK_SAMPLES).flat_map(|b| encoder.block(b)).collect();
        let sent = HearPart { clip: clip.clone(), data: base64_encode(&blocks), samples: part.len() as u32, last: i + 1 == parts.len() };
        let answer = link.post("/hear", &sent).map_err(|_| ())?;
        if sent.last {
            return serde_json::from_value::<HeardText>(answer).map(|h| h.text).map_err(|_| ());
        }
    }
    Err(())
}

/// Microsoft's speech-to-text, fed while he is still talking so the answer is ready almost as he stops.
/// Ok(None) is silence; Err is a failure of the service or the line.
fn azure(grant: &EarsGrant, host: &dyn RadioHost, manual: bool) -> Result<Option<String>, ()> {
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
        // he opened the microphone himself: it is all sent, and only he ends it
        if manual {
            verdict = Heard::Waiting;
        }
        if (end.speaking || manual) && upload.is_none() {
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
