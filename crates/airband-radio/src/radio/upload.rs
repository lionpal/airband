//! Sending a file he attached to his computer, in parts.
use super::*;

/// How sending a file ended when nothing went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadEnd {
    /// All of it is on his computer.
    Arrived,
    /// He took it off before it was all there.
    TakenOff,
}

impl Radio {
    /// Send a file to the tower in parts, under `id`. `sent` is told how far it has got after each part and
    /// returns false to give up. Blocks until it is all there, the tower refuses it, or it is given up on.
    pub(crate) fn upload(&self, id: &str, path: &str, name: &str, mut sent: impl FnMut(u64, u64) -> bool) -> Result<UploadEnd, RadioError> {
        // 90 KB of file is 120 KB of text, which fits one sealed message through a relay
        const PART: u64 = 90_000;
        // Every part is a round trip to his computer and back, so they go several at once: a video is hundreds
        // of them. The first goes alone: a file his computer will not take is refused before the rest is sent.
        const AT_ONCE: usize = 6;
        let unread = |_| RadioError::here("read that file");
        let total = airband_io::files::metadata(std::path::Path::new(path)).map_err(unread)?.len();
        let part_at = |from: u64| -> Result<airband_core::radio::AttachmentPart, RadioError> {
            let buf = airband_io::files::read_part(std::path::Path::new(path), from, PART).map_err(unread)?;
            Ok(airband_core::radio::AttachmentPart { id: id.into(), name: name.into(), from, data: airband_core::encode::base64(&buf), total })
        };
        // a part lost on a bad connection is sent again, to the same place, a few times before giving up
        let send = |part: &airband_core::radio::AttachmentPart| -> Result<u64, RadioError> {
            let mut tries = 0;
            loop {
                match self.link.attachment_part(part) {
                    Ok(answer) => return Ok(answer),
                    // a no, or what will never go however often it is sent (more than the relay carries): at once
                    Err(e) if e.retry() == Retry::Never => return Err(e.into()),
                    Err(_) if tries < 6 => {
                        tries += 1;
                        thread::sleep(Duration::from_secs(tries.min(4)));
                    }
                    // a file cannot be left with the relay: it needs his computer there to take it
                    Err(e) => return Err(e.into()),
                }
            }
        };
        send(&part_at(0)?)?;
        let mut arrived = PART.min(total);
        if !sent(arrived, total) {
            return Ok(UploadEnd::TakenOff);
        }
        if arrived >= total {
            return Ok(UploadEnd::Arrived);
        }
        let starts: Vec<u64> = (1..total.div_ceil(PART)).map(|k| k * PART).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failed: Mutex<Option<RadioError>> = Mutex::new(None);
        let taken_off = std::sync::atomic::AtomicBool::new(false);
        let (tx, rx) = channel::<u64>();
        thread::scope(|scope| {
            for _ in 0..AT_ONCE.min(starts.len()) {
                let tx = tx.clone();
                let (next, failed, starts, part_at, send, taken_off) = (&next, &failed, &starts, &part_at, &send, &taken_off);
                scope.spawn(move || {
                    while failed.lock().unwrap().is_none() && !taken_off.load(Ordering::SeqCst) {
                        let Some(from) = starts.get(next.fetch_add(1, Ordering::SeqCst)).copied() else { break };
                        match part_at(from).and_then(|part| send(&part).map(|_| part.data.len() as u64 * 3 / 4)) {
                            Ok(bytes) => {
                                tx.send(bytes).ok();
                            }
                            Err(e) => *failed.lock().unwrap() = Some(e),
                        }
                    }
                });
            }
            drop(tx);
            // progress, as each part lands; he can take the file off while it goes
            for bytes in rx {
                arrived = (arrived + bytes).min(total);
                if !sent(arrived, total) {
                    taken_off.store(true, Ordering::SeqCst);
                }
            }
        });
        match failed.into_inner().unwrap() {
            Some(e) => Err(e),
            None if taken_off.into_inner() => Ok(UploadEnd::TakenOff),
            None => Ok(UploadEnd::Arrived),
        }
    }
}
