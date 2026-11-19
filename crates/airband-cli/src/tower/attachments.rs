//! Files he attaches from the radio: a photo, a video. They arrive in parts (one sealed message through a relay
//! carries about 250 KB) and are kept in a folder of the tower's own on this computer, where any agent can open
//! them by path. There is no limit on size from the model's side, because the model is handed a path, not the
//! bytes. A video cannot be watched by an agent, so when ffmpeg is here a frame a second is pulled out beside it.
//!
//! A radio can write only here, only under names the tower makes safe, and only so much.
use super::*;
use airband_core::radio::AttachmentPart;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The most one file may be.
const MOST: u64 = 600 * 1024 * 1024;
/// How many frames are pulled from a video, one a second from its start.
const FRAMES: usize = 30;
const KEEP_DAYS: u64 = 14;

fn root() -> PathBuf {
    config::dir().join("attachments")
}
fn hex(id: &str) -> bool {
    (8..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}
/// A file name that cannot leave its folder or surprise a shell: letters, digits, dot, dash and underscore.
fn safe(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
    let clean = clean.trim_matches(['.', '-']).to_string();
    if clean.is_empty() { "file".into() } else { clean.chars().rev().take(80).collect::<String>().chars().rev().collect() }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum What {
    Image,
    Video,
    Other,
}
fn what(path: &Path) -> What {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" => What::Image,
        "mp4" | "mov" | "m4v" | "webm" => What::Video,
        _ => What::Other,
    }
}

/// One part of a file. Answers with how many bytes the tower now has, which is where the next part starts; a
/// part sent again (its answer was lost) lands in the same place.
pub(super) fn take(part: AttachmentPart) -> Result<u64, String> {
    if !hex(&part.id) {
        return Err("not an attachment id".into());
    }
    let dir = root().join(&part.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(safe(&part.name));
    let bytes = airband_core::audio::base64_decode(&part.data).ok_or("that part could not be read")?;
    let mut file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| e.to_string())?;
    let have = file.metadata().map_err(|e| e.to_string())?.len();
    if part.from > have {
        return Err(format!("a part is missing: the tower has {have} bytes and this one starts at {}", part.from));
    }
    if part.from + bytes.len() as u64 > MOST {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("that file is too large".into());
    }
    file.set_len(part.from).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(part.from)).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let next = part.from + bytes.len() as u64;
    if part.done {
        log(format!("he attached {} ({} KB)", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), next / 1024));
        if what(&path) == What::Video {
            frames(&path);
        } else {
            let _ = std::fs::write(dir.join(".ready"), "");
        }
    }
    Ok(next)
}

/// Pull a frame a second out of a video, beside it, so an agent has something it can look at. Without ffmpeg
/// the video is still there and the agent is told to pull frames itself.
fn frames(video: &Path) {
    let (video, dir) = (video.to_path_buf(), video.parent().map(Path::to_path_buf).unwrap_or_default());
    std::thread::spawn(move || {
        let out = dir.join("frames");
        let _ = std::fs::create_dir_all(&out);
        let ran = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&video)
            .args(["-vf", "fps=1,scale='min(1280,iw)':-2", "-frames:v", &FRAMES.to_string()])
            .arg(out.join("%02d.jpg"))
            .stdin(std::process::Stdio::null())
            .output();
        if !matches!(&ran, Ok(o) if o.status.success()) {
            let _ = std::fs::remove_dir_all(&out);
        }
        let _ = std::fs::write(dir.join(".ready"), "");
    });
}

pub(super) struct Attached {
    pub(super) path: PathBuf,
    pub(super) what: What,
    pub(super) bytes: u64,
    pub(super) frames: Vec<PathBuf>,
}

/// The file behind an id, once all of it has arrived. Waits a little for a video's frames.
pub(super) async fn find(id: &str) -> Option<Attached> {
    if !hex(id) {
        return None;
    }
    let dir = root().join(id);
    for _ in 0..80 {
        if dir.join(".ready").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let path = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))?;
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir.join("frames")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    frames.sort();
    Some(Attached { what: what(&path), bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), path, frames })
}

/// What an agent is told about the files that came with a message, and the pictures to show beside his words
/// in the chat (an image itself; for a video, its first frame).
pub(super) async fn describe(ids: &[String]) -> (String, Vec<airband_core::radio::Picture>) {
    let (mut lines, mut pictures) = (Vec::new(), Vec::new());
    for id in ids.iter().take(8) {
        let Some(a) = find(id).await else { continue };
        let size = if a.bytes >= 1024 * 1024 { format!("{:.1} MB", a.bytes as f64 / 1_048_576.0) } else { format!("{} KB", a.bytes.div_ceil(1024)) };
        match a.what {
            What::Image => {
                lines.push(format!("- {} (an image, {size})", a.path.display()));
                pictures.extend(pictures::keep(&a.path));
            }
            What::Video if !a.frames.is_empty() => {
                lines.push(format!(
                    "- {} (a video, {size}). You cannot watch it, so a frame from each second of it is in {}: {} to {}.",
                    a.path.display(),
                    a.frames[0].parent().map(|p| p.display().to_string()).unwrap_or_default(),
                    a.frames[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    a.frames[a.frames.len() - 1].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                ));
                pictures.extend(pictures::keep(&a.frames[0]));
            }
            What::Video => lines.push(format!("- {} (a video, {size}). You cannot watch it: pull frames out of it to look at, with ffmpeg if it is installed.", a.path.display())),
            What::Other => lines.push(format!("- {} ({size})", a.path.display())),
        }
    }
    if lines.is_empty() {
        return (String::new(), pictures);
    }
    (format!("\n\nHe attached {}, saved on this computer:\n{}", if lines.len() == 1 { "a file" } else { "files" }, lines.join("\n")), pictures)
}

/// Old attachments are cleared when the tower starts.
pub(super) fn prune() {
    let Ok(dirs) = std::fs::read_dir(root()) else { return };
    for d in dirs.flatten() {
        let old = d.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(KEEP_DAYS * 86_400));
        if old {
            let _ = std::fs::remove_dir_all(d.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_leave_its_folder() {
        assert_eq!(safe("../../etc/passwd"), "passwd");
        assert_eq!(safe("IMG 0042 (1).HEIC"), "IMG-0042--1-.HEIC");
        assert_eq!(safe("..\\..\\x.mov"), "x.mov");
        assert_eq!(safe("...."), "file");
        assert_eq!(safe(""), "file");
        assert!(hex("a1b2c3d4e5f6") && !hex("../x") && !hex("abc"));
        assert_eq!((what(Path::new("a.JPG")), what(Path::new("a.mov")), what(Path::new("a.pdf"))), (What::Image, What::Video, What::Other));
    }
}
