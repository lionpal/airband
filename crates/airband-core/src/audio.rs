//! Speech on the wire. The tower turns 16-bit PCM from a voice service into IMA ADPCM, four bits a sample, in
//! small blocks that each decode on their own; the radio turns them back. About a quarter of the size of the
//! PCM (12 KB a second at 24 kHz), no codec library on either side, and a block lost or late spoils a tenth of
//! a second and nothing after it.
//!
//! A block is: the first sample (i16, little-endian), the step index (u8), a zero byte, then the remaining
//! samples as 4-bit codes, low nibble first.

/// Samples in one block: a tenth of a second at 24 kHz.
pub const BLOCK_SAMPLES: usize = 2400;
/// Bytes in one full block.
pub const BLOCK_BYTES: usize = 4 + (BLOCK_SAMPLES - 1).div_ceil(2);

const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253,
    279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026,
    4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// Carries the encoder's step size from one block to the next, so quality does not dip at each block's start.
#[derive(Default, Clone, Copy)]
pub struct Encoder {
    index: i32,
}

fn step(predicted: &mut i32, index: &mut i32, code: u8) {
    let s = STEP[*index as usize];
    let mut diff = s >> 3;
    if code & 4 != 0 {
        diff += s;
    }
    if code & 2 != 0 {
        diff += s >> 1;
    }
    if code & 1 != 0 {
        diff += s >> 2;
    }
    *predicted = if code & 8 != 0 { *predicted - diff } else { *predicted + diff }.clamp(-32768, 32767);
    *index = (*index + INDEX[code as usize]).clamp(0, 88);
}

impl Encoder {
    /// One block from up to [`BLOCK_SAMPLES`] samples. A short last block is fine.
    pub fn block(&mut self, samples: &[i16]) -> Vec<u8> {
        let Some((&first, rest)) = samples.split_first() else { return Vec::new() };
        let mut out = Vec::with_capacity(4 + rest.len().div_ceil(2));
        out.extend_from_slice(&first.to_le_bytes());
        out.push(self.index as u8);
        out.push(0);
        let mut predicted = first as i32;
        let mut pending: Option<u8> = None;
        for &sample in rest {
            let s = STEP[self.index as usize];
            let mut diff = sample as i32 - predicted;
            let mut code = 0u8;
            if diff < 0 {
                code = 8;
                diff = -diff;
            }
            if diff >= s {
                code |= 4;
                diff -= s;
            }
            if diff >= s >> 1 {
                code |= 2;
                diff -= s >> 1;
            }
            if diff >= s >> 2 {
                code |= 1;
            }
            step(&mut predicted, &mut self.index, code);
            match pending.take() {
                None => pending = Some(code),
                Some(low) => out.push(low | (code << 4)),
            }
        }
        if let Some(low) = pending {
            out.push(low);
        }
        out
    }
}

/// One block back to samples. `samples` is how many the block holds (the last block of a clip may be short).
pub fn decode_block(block: &[u8], samples: usize) -> Vec<i16> {
    if block.len() < 4 || samples == 0 {
        return Vec::new();
    }
    let mut predicted = i16::from_le_bytes([block[0], block[1]]) as i32;
    let mut index = (block[2] as i32).clamp(0, 88);
    let mut out = Vec::with_capacity(samples);
    out.push(predicted as i16);
    'bytes: for &byte in &block[4..] {
        for code in [byte & 15, byte >> 4] {
            if out.len() == samples {
                break 'bytes;
            }
            step(&mut predicted, &mut index, code);
            out.push(predicted as i16);
        }
    }
    out
}

pub fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()
}

/// Has he finished speaking? Fed microphone audio a little at a time, it says when speech began and when it
/// ended. Shared by every host, so "how long a pause ends a reply" is decided once.
pub struct Endpointer {
    rate: u32,
    floor: f32,
    heard: u32,     // samples seen
    loud: u32,      // consecutive loud samples
    quiet: u32,     // consecutive quiet samples since speech began
    pub speaking: bool,
}

/// What the endpointer makes of the audio so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// Nothing yet; keep listening.
    Waiting,
    /// He is talking.
    Speaking,
    /// He talked and has stopped.
    Finished,
    /// He never started.
    Silence,
}

impl Endpointer {
    /// Seconds to wait for him to start, the pause that ends a reply, and the longest reply taken.
    pub const OPEN: f32 = 6.0;
    pub const PAUSE: f32 = 1.2;
    pub const LONGEST: f32 = 40.0;

    pub fn new(rate: u32) -> Self {
        Endpointer { rate, floor: 0.0, heard: 0, loud: 0, quiet: 0, speaking: false }
    }

    /// Feed the next samples. Call with about a fiftieth of a second at a time.
    pub fn feed(&mut self, frame: &[i16]) -> Heard {
        if frame.is_empty() {
            return if self.speaking { Heard::Speaking } else { Heard::Waiting };
        }
        let rms = (frame.iter().map(|s| (*s as f32) * (*s as f32)).sum::<f32>() / frame.len() as f32).sqrt();
        let n = frame.len() as u32;
        // The room's own noise: the quietest level heard lately. It drops at once to anything quieter and creeps
        // up slowly, so it settles on the background whether the recording opens on silence or on speech.
        if self.heard == 0 || rms < self.floor {
            self.floor = rms;
        } else {
            self.floor = self.floor * 1.004 + 0.5;
        }
        // no room is this loud: without the cap, a recording that opens on speech would take the speech for the room
        self.floor = self.floor.min(1200.0);
        self.heard += n;
        let is_loud = rms > (self.floor * 3.0).max(350.0);
        if is_loud {
            self.loud += n;
            self.quiet = 0;
            // a tenth of a second of it counts as speech, not a click
            if self.loud >= self.rate / 10 {
                self.speaking = true;
            }
        } else {
            self.loud = 0;
            if self.speaking {
                self.quiet += n;
            }
        }
        let secs = |samples: u32| samples as f32 / self.rate as f32;
        if self.speaking && (secs(self.quiet) >= Self::PAUSE || secs(self.heard) >= Self::LONGEST) {
            Heard::Finished
        } else if !self.speaking && secs(self.heard) >= Self::OPEN {
            Heard::Silence
        } else if self.speaking {
            Heard::Speaking
        } else {
            Heard::Waiting
        }
    }
}

/// Raw little-endian 16-bit PCM bytes to samples. An odd trailing byte is left for the next call.
pub fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> Vec<i16> {
        (0..n).map(|i| ((i as f32 * 0.05).sin() * 9000.0 + (i as f32 * 0.31).sin() * 3000.0) as i16).collect()
    }

    #[test]
    fn a_block_is_a_quarter_the_size_and_decodes_close_to_what_went_in() {
        let input = tone(BLOCK_SAMPLES * 3);
        let mut enc = Encoder::default();
        let mut worst = 0i32;
        for chunk in input.chunks(BLOCK_SAMPLES) {
            let block = enc.block(chunk);
            assert_eq!(block.len(), BLOCK_BYTES);
            assert!(block.len() * 4 <= chunk.len() * 2 + 16);
            let back = decode_block(&block, chunk.len());
            assert_eq!(back.len(), chunk.len());
            assert_eq!(back[0], chunk[0]);
            // skip the first few samples of the very first block, while the step size is still finding its level
            worst = worst.max(chunk.iter().zip(&back).skip(40).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap());
        }
        assert!(worst < 1500, "worst error {worst}");
    }

    #[test]
    fn endpointer_hears_speech_start_and_stop_and_plain_silence() {
        let rate = 16000;
        let frame = |amp: f32, n: usize| -> Vec<i16> { (0..n).map(|i| ((i as f32 * 0.3).sin() * amp) as i16).collect() };
        let run = |parts: &[(f32, f32)]| {
            let mut e = Endpointer::new(rate);
            let mut last = Heard::Waiting;
            for (amp, secs) in parts {
                for _ in 0..(secs * 50.0) as usize {
                    last = e.feed(&frame(*amp, 320));
                    if matches!(last, Heard::Finished | Heard::Silence) {
                        return last;
                    }
                }
            }
            last
        };
        assert_eq!(run(&[(40.0, 0.5), (6000.0, 1.0), (40.0, 2.0)]), Heard::Finished);
        assert_eq!(run(&[(40.0, 0.5), (6000.0, 1.0), (40.0, 0.5)]), Heard::Speaking); // a short pause is not the end
        assert_eq!(run(&[(40.0, 7.0)]), Heard::Silence);
        // steady road noise is not speech
        assert_eq!(run(&[(900.0, 7.0)]), Heard::Silence);
        // speech over road noise still starts and ends
        assert_eq!(run(&[(900.0, 0.6), (7000.0, 1.0), (900.0, 2.0)]), Heard::Finished);
        // a recording that opens on speech, with no quiet lead-in to learn the room from
        assert_eq!(run(&[(6000.0, 1.0), (40.0, 2.0)]), Heard::Finished);
    }

    #[test]
    fn a_short_last_block_and_an_empty_one() {
        let mut enc = Encoder::default();
        let input = tone(777);
        let block = enc.block(&input);
        assert_eq!(decode_block(&block, 777).len(), 777);
        assert!(enc.block(&[]).is_empty());
        assert!(decode_block(&[], 5).is_empty());
    }

    #[test]
    fn each_block_decodes_without_the_one_before() {
        let input = tone(BLOCK_SAMPLES * 2);
        let mut enc = Encoder::default();
        let _first = enc.block(&input[..BLOCK_SAMPLES]);
        let second = enc.block(&input[BLOCK_SAMPLES..]);
        let back = decode_block(&second, BLOCK_SAMPLES);
        let worst = input[BLOCK_SAMPLES..].iter().zip(&back).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
        assert!(worst < 1500, "worst error {worst}");
    }
}
