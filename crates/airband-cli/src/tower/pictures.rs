//! Pictures an agent looked at or made for him. The tower keeps a small copy of each (a JPEG, 1600 pixels at
//! most on its long side) in `~/.airband/pictures`, named by what the original was, so the same file seen twice
//! is kept once. A copy goes to the phone only when it asks, in parts.
use super::*;
use airband_core::radio::{PICTURE_PART, Picture, PicturePart};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The longest side of a kept copy, in pixels.
const LONG_SIDE: u32 = 1600;
/// An original larger than this is not read: it is not a screenshot or a chart.
const TOO_LARGE: u64 = 40 * 1024 * 1024;
/// How many pictures one message carries.
pub(super) const PER_MESSAGE: usize = 4;
/// Copies are dropped after this long.
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);

/// A picture an agent touched since it last spoke.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Seen {
    pub(super) picture: Picture,
    /// The agent meant him to see it.
    pub(super) shown: bool,
}

fn folder() -> PathBuf {
    config::dir().join("pictures")
}

/// Where the copy with this id is. Nothing for an id that is not one of ours, so a request cannot name another file.
fn file(id: &str) -> Option<PathBuf> {
    (id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| folder().join(format!("{id}.jpg")))
}

/// Keep a copy of the picture at `path`. Nothing if it is not there, not a picture, or too large. Slow (it
/// decodes and shrinks the picture): not for a thread that others wait on.
pub(super) fn keep(path: &Path) -> Option<Picture> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= TOO_LARGE)?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    let digest = Sha256::digest(format!("{}\n{}\n{modified}", path.display(), meta.len()));
    keep_as(airband_core::relay::hex(&digest[..12]), || image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok())
}

/// Keep a copy of a picture he pasted in, given as its bytes.
pub(super) fn keep_bytes(bytes: &[u8]) -> Option<Picture> {
    if bytes.len() as u64 > TOO_LARGE {
        return None;
    }
    keep_as(airband_core::relay::hex(&Sha256::digest(bytes)[..12]), || image::load_from_memory(bytes).ok())
}

/// The small copy kept under `id`: the one already there, or one made now from the picture `decode` gives.
fn keep_as(id: String, decode: impl FnOnce() -> Option<image::DynamicImage>) -> Option<Picture> {
    let out = file(&id)?;
    // seen before: read its size from the copy's header and not the whole picture
    if let Ok(kept) = std::fs::metadata(&out) {
        let (width, height) = image::image_dimensions(&out).ok()?;
        return Some(Picture { id, width, height, bytes: kept.len() as u32 });
    }
    let img = decode()?;
    let img = if img.width().max(img.height()) > LONG_SIDE { img.resize(LONG_SIDE, LONG_SIDE, image::imageops::FilterType::Triangle) } else { img };
    let rgb = img.to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 75).encode_image(&rgb).ok()?;
    airband_core::kept::write_atomic(&out, &bytes).ok()?;
    Some(Picture { id, width: rgb.width(), height: rgb.height(), bytes: bytes.len() as u32 })
}

/// May a picture that is only named in an agent's text be copied for the phone? Only from where an agent makes
/// pictures: its own project, and the temporary folders. A path in a message is words, not an action the
/// agent took, so it does not reach into the rest of the disk. (A picture the agent read or sent him is
/// reported by its harness as an event and is not limited this way.)
/// Could this path, going by how it is written, be within reach? Decided without touching the disk.
fn near(path: &Path, cwd: Option<&str>) -> bool {
    let mut roots: Vec<PathBuf> = vec![std::env::temp_dir(), "/tmp".into(), "/private/tmp".into(), "/var/folders".into(), "/private/var/folders".into()];
    roots.extend(cwd.map(PathBuf::from));
    !path.components().any(|c| c == std::path::Component::ParentDir) && roots.iter().any(|root| path.starts_with(root))
}

fn within_reach(path: &Path, cwd: Option<&str>) -> bool {
    let Ok(real) = path.canonicalize() else { return false };
    let mut roots: Vec<PathBuf> = vec![std::env::temp_dir(), "/tmp".into(), "/var/folders".into()];
    roots.extend(cwd.map(PathBuf::from));
    roots.iter().filter_map(|r| r.canonicalize().ok()).any(|root| real.starts_with(root))
}

/// Picture files an agent named in what it wrote, in order. Only ones that exist and are [`within_reach`].
pub(super) fn named_in(text: &str, cwd: Option<&str>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || "`\"'()[]<>".contains(c)) {
        let word = word.trim_end_matches(['.', ',', ';', ':', '!', '?']).trim_start_matches("file://");
        if !agent::is_picture(word) {
            continue;
        }
        let path = if let Some(rest) = word.strip_prefix("~/") {
            std::env::var("HOME").map(|home| Path::new(&home).join(rest)).unwrap_or_else(|_| PathBuf::from(word))
        } else if Path::new(word).is_absolute() {
            PathBuf::from(word)
        } else {
            Path::new(cwd.unwrap_or(".")).join(word)
        };
        // by its name first, before the disk is touched: a path on another drive is never even looked at
        if near(&path, cwd) && path.is_file() && within_reach(&path, cwd) && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The pictures that go with what an agent just said: the ones it meant him to see, then the ones it named,
/// then the last ones it looked at, [`PER_MESSAGE`] at most.
pub(super) fn for_message(seen: Vec<Seen>, named: Vec<Picture>) -> Vec<Picture> {
    let mut out: Vec<Picture> = Vec::new();
    // What it sent him, and what it names in what it says. A picture it only looked at while working is not his
    // to be shown: most of those are his own screenshots coming back, or scraps it made for itself.
    let shown = seen.iter().filter(|s| s.shown).map(|s| s.picture.clone());
    for p in shown.chain(named) {
        if out.len() < PER_MESSAGE && !out.iter().any(|o| o.id == p.id) {
            out.push(p);
        }
    }
    out
}

/// One part of a kept copy, from byte `from`.
pub(super) fn part(id: &str, from: usize) -> Option<PicturePart> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(file(id)?).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    f.seek(SeekFrom::Start(from as u64)).ok()?;
    let mut buf = vec![0u8; PICTURE_PART.min(len.saturating_sub(from))];
    f.read_exact(&mut buf).ok()?;
    let next = from + buf.len();
    Some(PicturePart { data: airband_core::audio::base64_encode(&buf), next: next as u32, done: next >= len })
}

/// Drop copies nobody will ask for again.
pub(super) fn prune() {
    let Ok(dir) = std::fs::read_dir(folder()) else { return };
    for entry in dir.flatten() {
        if entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(id: &str) -> Picture {
        Picture { id: id.into(), width: 1, height: 1, bytes: 1 }
    }
    #[test]
    fn shown_then_named_and_nothing_it_only_looked_at() {
        let seen = vec![Seen { picture: pic("a"), shown: false }, Seen { picture: pic("b"), shown: true }, Seen { picture: pic("c"), shown: false }, Seen { picture: pic("d"), shown: false }];
        let ids: Vec<String> = for_message(seen, vec![pic("n"), pic("b")]).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["b", "n"]);
    }
    #[test]
    fn finds_pictures_named_in_a_message() {
        let dir = std::env::temp_dir().join(format!("airband-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chart.png"), b"x").unwrap();
        let abs = dir.join("chart.png").display().to_string();
        let found = named_in(&format!("See `chart.png`, also ({abs}). Not missing.png or notes.md."), Some(&dir.display().to_string()));
        assert_eq!(found, vec![dir.join("chart.png")]);
        // a picture elsewhere on the disk is not copied because a message mentions it
        let home = std::path::PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap()).join(format!(".airband-test-{}.png", std::process::id()));
        std::fs::write(&home, b"x").unwrap();
        assert!(named_in(&format!("Look at {}", home.display()), Some(&dir.display().to_string())).is_empty());
        std::fs::remove_file(&home).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn only_our_own_ids_name_a_file() {
        assert!(file("../../etc/passwd").is_none() && file("0123456789abcdef01234567").is_some());
    }
}
