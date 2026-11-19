//! The tower's voice: turning a sentence into audio the phone can play, as fast as it can be done.
//!
//! Two things make it fast. The audio is started the moment the sentence exists, which is usually before the
//! phone wants it, so by then the clip is simply there. And it is handed over in parts while the voice service is
//! still producing it, so the phone starts playing on the first part.
//!
//! A voice service is anything that implements [`Synth`]: text in, 16-bit PCM out, a piece at a time. Adding one
//! is one more `impl` and one more arm in [`synth_for`]. Clips are kept compressed (airband_core::audio), a
//! few dozen at most.
use super::*;
use airband_core::audio::{BLOCK_BYTES, BLOCK_SAMPLES, Encoder, base64_decode, base64_encode, samples};
use airband_core::radio::SpeechPart;
use config::Voice;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};

/// A voice service.
pub(super) trait Synth: Send + Sync {
    /// Samples a second of the PCM it produces.
    fn rate(&self) -> u32;
    /// Speak `text`, handing each piece of 16-bit little-endian mono PCM to `sink` as it arrives. A service that
    /// says when each word is spoken tells `mark` so, before the sound of that word: where the word starts in
    /// `text` (in characters) and how many milliseconds into the clip.
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String>;
}

struct Gemini {
    key: String,
    model: String,
    voice: String,
}

impl Synth for Gemini {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        // The words go in bare, and how to say them goes beside them in `speech_metadata`. Put the direction in the
        // text and this generation of model reads it aloud: the first version here opened every message with
        // "Read this aloud exactly as written", found by transcribing its own output. (Backoffice hit the same thing.)
        let body = json!({
            "contents": [{ "parts": [{ "text": text, "speech_metadata": { "style": "Calm and clear, at a brisk pace, like a radio dispatcher. Say the words exactly as written and nothing else." } }] }],
            "generationConfig": { "responseModalities": ["AUDIO"], "speechConfig": { "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": self.voice } } } },
        });
        let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse", self.model);
        let res = airband_radio::net::agent(Duration::from_secs(30)).post(&url).header("x-goog-api-key", &self.key).send_json(&body).map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("Gemini answered {}", res.status()));
        }
        // server-sent events: one JSON document per "data:" line, each with a piece of audio in base64
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Some(data) = line.strip_prefix("data: ") else { continue };
            let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
            for part in v["candidates"][0]["content"]["parts"].as_array().into_iter().flatten() {
                if let Some(pcm) = part["inlineData"]["data"].as_str().and_then(base64_decode) {
                    sink(&pcm);
                }
            }
        }
        Ok(())
    }
}

struct Azure {
    /// The service's key, or a pass made with it.
    key: String,
    pass: bool,
    region: String,
    voice: String,
}

impl Synth for Azure {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), _mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        let ssml = format!("<speak version='1.0' xml:lang='en-US'><voice name='{}'>{escaped}</voice></speak>", self.voice);
        let res = airband_radio::net::agent(Duration::from_secs(30))
            .post(format!("https://{}.tts.speech.microsoft.com/cognitiveservices/v1", self.region))
            .header(if self.pass { "Authorization" } else { "Ocp-Apim-Subscription-Key" }, if self.pass { format!("Bearer {}", self.key) } else { self.key.clone() })
            .header("Content-Type", "application/ssml+xml")
            .header("X-Microsoft-OutputFormat", "raw-24khz-16bit-mono-pcm")
            .header("User-Agent", "airband")
            .send(ssml.as_bytes())
            .map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("Microsoft answered {}", res.status()));
        }
        let mut reader = res.into_body().into_reader();
        let mut buf = [0u8; 4800]; // a tenth of a second
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => sink(&buf[..n]),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

struct Elevenlabs {
    key: String,
    voice: String,
    model: String,
}

impl Synth for Elevenlabs {
    fn rate(&self) -> u32 {
        24000
    }
    fn speak(&self, text: &str, sink: &mut dyn FnMut(&[u8]), mark: &mut dyn FnMut(u32, u32)) -> Result<(), String> {
        // the streaming endpoint with timings: a line of JSON at a time, each with a piece of the sound and
        // when each character of it is spoken, counted from the start of the clip
        let res = airband_radio::net::agent(Duration::from_secs(60))
            .post(format!("https://api.elevenlabs.io/v1/text-to-speech/{}/stream/with-timestamps?output_format=pcm_24000", self.voice))
            .header("xi-api-key", &self.key)
            .send_json(json!({ "text": text, "model_id": self.model }))
            .map_err(|e| e.to_string())?;
        if res.status().as_u16() != 200 {
            return Err(format!("ElevenLabs answered {}", res.status()));
        }
        // where in the text the service has got to, and whether the character before was a gap between words
        let (mut at, mut gap) = (0u32, true);
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let (characters, starts) = (v["alignment"]["characters"].as_array(), v["alignment"]["character_start_times_seconds"].as_array());
            for (c, start) in characters.into_iter().flatten().zip(starts.into_iter().flatten()) {
                let space = c.as_str().is_none_or(|c| c.trim().is_empty());
                if gap && !space {
                    mark(at, (start.as_f64().unwrap_or(0.0) * 1000.0) as u32);
                }
                gap = space;
                at += c.as_str().map(|c| c.chars().count() as u32).unwrap_or(1);
            }
            if let Some(pcm) = v["audio_base64"].as_str().and_then(base64_decode) {
                sink(&pcm);
            }
        }
        Ok(())
    }
}

fn synth_for(voice: &Voice) -> Option<Box<dyn Synth>> {
    match voice {
        Voice::Device => None,
        Voice::Gemini { key, model, voice } => Some(Box::new(Gemini { key: key.clone(), model: model.clone(), voice: voice.clone() })),
        Voice::Azure { key, region, voice } => Some(Box::new(Azure { key: key.clone(), pass: false, region: region.clone(), voice: voice.clone() })),
        Voice::Elevenlabs { key, voice, model } => Some(Box::new(Elevenlabs { key: key.clone(), voice: voice.clone(), model: model.clone() })),
        Voice::AzurePass { region, token } => Some(Box::new(Azure { key: token.clone(), pass: true, region: region.clone(), voice: "en-US-AvaMultilingualNeural".into() })),
    }
}

#[derive(Default)]
struct Clip {
    rate: u32,
    /// Compressed blocks, one after another; all full-sized but the last.
    blocks: Vec<u8>,
    samples: u32,
    done: bool,
    failed: bool,
    /// What is being said, and when each word of it starts, if the service says.
    text: String,
    marks: Vec<(u32, u32)>,
}

/// How many clips are kept. Each is a few tens of kilobytes, unless it is a whole message read aloud.
const KEEP: usize = 40;
/// And how much sound in all.
const KEEP_BYTES: usize = 4 * 1024 * 1024;
/// Blocks in one answer: four seconds. Small enough to pass the relay sealed, large enough to stay ahead of playback.
const PART_BLOCKS: usize = 40;

#[derive(Default)]
pub struct Speech {
    clips: Mutex<(HashMap<String, Arc<Mutex<Clip>>>, VecDeque<String>)>,
}

impl Speech {
    /// Start making speech for `text` and return its id at once, or nothing when the phone is to use its own
    /// voice. The same text with the same voice is made once.
    pub(super) fn start(&self, voice: &Voice, text: &str) -> Option<String> {
        let synth = synth_for(voice)?;
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let id: String = Sha256::digest(format!("{}|{text}", serde_json::to_string(voice).unwrap_or_default()).as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
        let clip = {
            let mut clips = self.clips.lock().unwrap();
            if clips.0.get(&id).is_some_and(|c| !c.lock().unwrap().failed) {
                return Some(id);
            }
            let clip = Arc::new(Mutex::new(Clip { rate: synth.rate(), text: text.clone(), ..Clip::default() }));
            clips.0.insert(id.clone(), clip.clone());
            clips.1.retain(|k| k != &id);
            clips.1.push_back(id.clone());
            while clips.1.len() > KEEP {
                if let Some(old) = clips.1.pop_front() {
                    clips.0.remove(&old);
                }
            }
            // a whole message read aloud is minutes of sound: the oldest clips go when there are megabytes of it
            let held = |clips: &HashMap<String, Arc<Mutex<Clip>>>| clips.values().map(|c| c.lock().unwrap().blocks.len()).sum::<usize>();
            while clips.1.len() > 2 && held(&clips.0) > KEEP_BYTES {
                if let Some(old) = clips.1.pop_front() {
                    clips.0.remove(&old);
                }
            }
            clip
        };
        // a plain thread: the voice service is read with blocking I/O, a piece at a time
        std::thread::spawn(move || {
            let started = Instant::now();
            let mut first: Option<Duration> = None;
            let mut encoder = Encoder::default();
            let mut waiting: Vec<i16> = Vec::new();
            let mut odd: Option<u8> = None;
            let mut sink = |bytes: &[u8]| {
                first.get_or_insert_with(|| started.elapsed());
                // pieces can split a sample in two: carry the odd byte over
                let mut joined = Vec::with_capacity(bytes.len() + 1);
                joined.extend(odd.take());
                joined.extend_from_slice(bytes);
                if joined.len() % 2 == 1 {
                    odd = joined.pop();
                }
                waiting.extend(samples(&joined));
                let mut c = clip.lock().unwrap();
                while waiting.len() >= BLOCK_SAMPLES {
                    let block: Vec<i16> = waiting.drain(..BLOCK_SAMPLES).collect();
                    c.blocks.extend(encoder.block(&block));
                    c.samples += BLOCK_SAMPLES as u32;
                }
            };
            let marked = clip.clone();
            let mut mark = |at: u32, ms: u32| marked.lock().unwrap().marks.push((at, ms));
            let result = synth.speak(&text, &mut sink, &mut mark);
            let mut c = clip.lock().unwrap();
            if !waiting.is_empty() {
                c.samples += waiting.len() as u32;
                c.blocks.extend(encoder.block(&waiting));
            }
            c.done = true;
            match result {
                Ok(()) if c.samples > 0 => log(format!("voice: {:.1} s of speech, first sound after {:.2} s, all after {:.2} s", c.samples as f32 / c.rate as f32, first.unwrap_or_default().as_secs_f32(), started.elapsed().as_secs_f32())),
                Ok(()) => {
                    c.failed = true;
                    log("voice: the service sent no audio");
                }
                Err(e) => {
                    c.failed = c.samples == 0; // what was already spoken is still worth playing
                    log(format!("voice: {e}"));
                }
            }
        });
        Some(id)
    }

    /// The next part of a clip from block `from`, waiting a moment if the voice has not got that far yet.
    /// Nothing when the clip is unknown or failed before making a sound.
    pub(super) async fn part(&self, id: &str, from: usize) -> Option<SpeechPart> {
        let clip = self.clips.lock().unwrap().0.get(id).cloned()?;
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            {
                let c = clip.lock().unwrap();
                if c.failed {
                    return None;
                }
                let full = c.blocks.len() / BLOCK_BYTES;
                let have = if c.done { c.blocks.len().div_ceil(BLOCK_BYTES) } else { full };
                if have > from || c.done {
                    let to = (from + PART_BLOCKS).min(have);
                    let bytes = &c.blocks[(from * BLOCK_BYTES).min(c.blocks.len())..(to * BLOCK_BYTES).min(c.blocks.len())];
                    let last_included = c.done && to == have;
                    let samples = if last_included { c.samples.saturating_sub((from * BLOCK_SAMPLES) as u32) } else { ((to - from.min(to)) * BLOCK_SAMPLES) as u32 };
                    // the words that start within this stretch of sound (and, with the last of it, any after)
                    let ms = |block: usize| (block * BLOCK_SAMPLES) as u64 * 1000 / c.rate.max(1) as u64;
                    let (begins, ends) = (ms(from), if last_included { u64::MAX } else { ms(to) });
                    let marks = c.marks.iter().filter(|(_, at)| (*at as u64) >= begins && (*at as u64) < ends).copied().collect();
                    return Some(SpeechPart { rate: c.rate, data: base64_encode(bytes), samples, next: to as u32, done: last_included, text: (from == 0).then(|| c.text.clone()), marks });
                }
            }
            if Instant::now() >= until {
                // nothing yet: say so, and let the radio decide whether to keep waiting
                let rate = clip.lock().unwrap().rate;
                return Some(SpeechPart { rate, data: String::new(), samples: 0, next: from as u32, done: false, ..SpeechPart::default() });
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}
