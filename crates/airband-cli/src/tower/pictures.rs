//! Pictures an agent looked at or made for him. The tower keeps a small copy of each (a JPEG, 1600 pixels at
//! most on its long side) in `~/.airband/pictures`, named by what the original was, so the same file seen twice
//! is kept once. A copy goes to the phone only when it asks, in parts.
use super::*;
use airband_core::radio::{PICTURE_PART, Picture, PicturePart, wire::Refusal};
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
    airband_core::encode::is_hex(id, 24..=24).then(|| folder().join(format!("{id}.jpg")))
}

/// Keep a copy of the picture at `path`. Nothing if it is not there, not a picture, or too large. Slow (it
/// decodes and shrinks the picture): not for a thread that others wait on.
pub(super) fn keep(path: &Path) -> Option<Picture> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= TOO_LARGE)?;
    let modified = crate::adapters::written(path)?;
    let digest = Sha256::digest(format!("{}\n{}\n{modified}", path.display(), meta.len()));
    keep_as(airband_core::encode::hex(&digest[..12]), || image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok())
}

/// The pictures he pasted in with each thing he typed at an agent's keyboard, kept: one list per thing typed,
/// empty for what was not his.
pub(super) fn pasted(told: &[crate::adapters::Told]) -> Vec<Vec<airband_core::radio::Picture>> {
    told.iter().map(|t| if t.his { t.images.iter().filter_map(|bytes| keep_bytes(bytes)).collect() } else { Vec::new() }).collect()
}

fn keep_bytes(bytes: &[u8]) -> Option<Picture> {
    if bytes.len() as u64 > TOO_LARGE {
        return None;
    }
    keep_as(airband_core::encode::hex(&Sha256::digest(bytes)[..12]), || image::load_from_memory(bytes).ok())
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
    airband_io::files::write_atomic(&out, &bytes).ok()?;
    Some(Picture { id, width: rgb.width(), height: rgb.height(), bytes: bytes.len() as u32 })
}

/// The picture files an agent put in what it wrote as pictures: Markdown images (`![chart](out/chart.png)`),
/// as the Markdown parser finds them, in order. A path that is only mentioned is words, and stays words: an
/// agent that means him to see a picture sends it (its tool's own way) or writes it as an image.
pub(super) fn in_markdown(text: &str, cwd: Option<&str>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for named in airband_core::prose::images(text) {
        // only a file that is here is one: a picture on the web is not looked for
        // a path from its own folder needs that folder: with none known, only a whole path is one
        let path = PathBuf::from(&named);
        let path = match cwd {
            _ if path.is_absolute() => path,
            Some(cwd) => Path::new(cwd).join(path),
            None => continue,
        };
        if path.is_file() && !out.contains(&path) {
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

/// One part of a kept copy, from byte `from`. A copy that is not there (never kept, or pruned) is gone; one that
/// is there and cannot be read is trouble on this computer, said as such: the phone may ask again.
pub(super) fn part(id: &str, from: usize) -> Result<PicturePart, Refusal> {
    let path = file(id).ok_or(Refusal::Gone)?;
    let trouble = |e: std::io::Error| Refusal::NotNow { reason: format!("airband could not read that picture ({e}).") };
    let gone_or = |e: std::io::Error| if e.kind() == std::io::ErrorKind::NotFound { Refusal::Gone } else { trouble(e) };
    let len = std::fs::metadata(&path).map_err(gone_or)?.len() as usize;
    if from > len {
        return Err(Refusal::BadAsk { reason: format!("the picture is {len} bytes long, and the part asked for starts at {from}") });
    }
    let buf = airband_io::files::read_part(&path, from as u64, PICTURE_PART.min(len - from) as u64).map_err(gone_or)?;
    let next = from + buf.len();
    Ok(PicturePart { data: airband_core::encode::base64(&buf), next: next as u32, done: next >= len })
}

/// Drop copies nobody will ask for again.
pub(super) fn prune() {
    airband_io::files::prune_older(&folder(), KEEP);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_that_is_not_there_is_gone_and_an_id_not_ours_names_no_file() {
        assert_eq!(part("not-an-id", 0).unwrap_err(), Refusal::Gone);
        assert_eq!(part(&"0".repeat(24), 0).unwrap_err(), Refusal::Gone, "never kept, or pruned");
    }

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
    fn a_picture_written_as_a_markdown_image_is_found_and_one_only_mentioned_is_not() {
        let dir = std::env::temp_dir().join(format!("airband-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chart.png"), b"x").unwrap();
        let abs = dir.join("chart.png").display().to_string();
        let cwd = dir.display().to_string();
        assert_eq!(in_markdown("Here it is: ![the chart](chart.png)", Some(&cwd)), vec![dir.join("chart.png")]);
        assert_eq!(in_markdown(&format!("![]({abs}) and again ![]({abs})"), Some(&cwd)), vec![dir.join("chart.png")]);
        // words that name a file, a file that is not there, and a picture on the web
        assert!(in_markdown(&format!("See `chart.png`, also ({abs}). ![gone](missing.png) ![web](https://example.com/a.png)"), Some(&cwd)).is_empty());
        // with no folder known for the agent, a path from its folder is not looked for anywhere else; a whole one is
        assert!(in_markdown("![the chart](chart.png)", None).is_empty());
        assert_eq!(in_markdown(&format!("![]({abs})"), None), vec![dir.join("chart.png")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn only_our_own_ids_name_a_file() {
        assert!(file("../../etc/passwd").is_none() && file("0123456789abcdef01234567").is_some());
    }
}
