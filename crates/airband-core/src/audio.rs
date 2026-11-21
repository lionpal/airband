//! Sound and bytes shared by every host: when he has finished speaking ([`Endpointer`]), microphone bytes as
//! samples, and base64. The sound itself is made and heard on the phone; none crosses to his computer.

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
}
