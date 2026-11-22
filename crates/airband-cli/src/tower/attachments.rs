//! Files he attaches from the radio: a photo, a video. They arrive in parts (one sealed message through a relay
//! carries about 250 KB) and are kept in a folder of the tower's own on this computer, where any agent can open
//! them by path. There is no limit on size from the model's side, because the model is handed a path, not the
//! bytes. A video cannot be watched by an agent, so a frame a second is pulled out beside it, with the program the
//! frames row names, when that program is here.
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
    let base = std::path::Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
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
/// What a file is, by what its own first bytes say it is. Its name says nothing.
fn what(path: &Path) -> What {
    match infer::get_from_path(path).ok().flatten().map(|kind| kind.matcher_type()) {
        Some(infer::MatcherType::Image) => What::Image,
        Some(infer::MatcherType::Video) => What::Video,
        _ => What::Other,
    }
}

/// One file's record of what has come is read and written by one part at a time.
static GOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One part of a file. Answers with how many bytes the tower now has, which is where the next part starts; a
/// part sent again (its answer was lost) lands in the same place.
pub(super) fn take(part: AttachmentPart) -> Result<u64, String> {
    take_in(&root(), part)
}

/// [`take`], into the attachments folder `root`. The file is called what the first part to come says, made
/// safe, and written down beside it: every later part goes into that one file, whatever name it carries.
fn take_in(root: &Path, part: AttachmentPart) -> Result<u64, String> {
    if !hex(&part.id) {
        return Err("not an attachment id".into());
    }
    let dir = root.join(&part.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = {
        let _guard = GOT.lock().unwrap();
        let named = dir.join(".name");
        match std::fs::read_to_string(&named) {
            Ok(name) if !name.is_empty() => name,
            _ => {
                let name = safe(&part.name);
                std::fs::write(&named, &name).map_err(|e| e.to_string())?;
                name
            }
        }
    };
    let path = dir.join(name);
    let bytes = airband_core::encode::unbase64(&part.data).ok_or("that part could not be read")?;
    let mut file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| e.to_string())?;
    if part.from + bytes.len() as u64 > MOST || part.total > MOST {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("that file is too large".into());
    }
    let (next, whole) = {
        // The radio says how long the file is, and its parts come in any order, several at once. Each is
        // written where it belongs, and what has come is kept beside the file until all of it has.
        {
            let total = part.total;
            if part.from + bytes.len() as u64 > total {
                return Err("a part runs past the end of the file".into());
            }
            file.seek(SeekFrom::Start(part.from)).map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            let _guard = GOT.lock().unwrap();
            let note = dir.join(".got");
            let mut got: Vec<(u64, u64)> = std::fs::read_to_string(&note).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            got.push((part.from, part.from + bytes.len() as u64));
            got.sort();
            // how far it is whole from the start
            let mut have = 0u64;
            for (from, to) in &got {
                if *from > have {
                    break;
                }
                have = have.max(*to);
            }
            if have >= total {
                let _ = std::fs::remove_file(&note);
            } else {
                std::fs::write(&note, serde_json::to_string(&got).unwrap_or_default()).map_err(|e| e.to_string())?;
            }
            (have, have >= total)
        }
    };
    if whole && !dir.join(".ready").exists() && !dir.join(".whole").exists() {
        // the file is named where it is first whole: what is read back is this name, never whatever the folder lists first
        let _ = std::fs::write(dir.join(".whole"), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        log(format!("he attached {} ({} KB)", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), next / 1024));
        if what(&path) == What::Video {
            frames(&path);
        } else {
            let _ = std::fs::write(dir.join(".ready"), "");
        }
    }
    Ok(next)
}

/// Pull a frame a second out of a video, beside it, so an agent has something it can look at, with the program
/// the frames row names. Without it the video is still there and the agent is told to pull frames itself.
fn frames(video: &Path) {
    let (video, dir) = (video.to_path_buf(), video.parent().map(Path::to_path_buf).unwrap_or_default());
    std::thread::spawn(move || {
        let out = dir.join("frames");
        let _ = std::fs::create_dir_all(&out);
        let ran = airband_core::catalog::frames().ok_or(()).and_then(|f| {
            std::process::Command::new(&f.program)
                .args(f.run(&video.display().to_string(), FRAMES, &out.display().to_string()))
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|_| ())
        });
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
    let name = std::fs::read_to_string(dir.join(".whole")).ok().filter(|n| !n.is_empty())?;
    let path = Some(dir.join(safe(&name))).filter(|p| p.is_file())?;
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
            What::Video => lines.push(match airband_core::catalog::frames() {
                Some(f) => format!("- {} (a video, {size}). You cannot watch it: pull frames out of it to look at, with {} if it is installed.", a.path.display(), f.name),
                None => format!("- {} (a video, {size}). You cannot watch it: pull frames out of it to look at.", a.path.display()),
            }),
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
    airband_core::kept::prune_older(&root(), Duration::from_secs(KEEP_DAYS * 86_400));
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
        // a file is what its bytes say, whatever it is called
        let dir = std::env::temp_dir().join(format!("airband-what-{}", config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let png = file("a.txt", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        let mp4 = file("b.png", b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom");
        let text = file("c.jpg", b"only words");
        assert_eq!((what(&png), what(&mp4), what(&text)), (What::Image, What::Video, What::Other));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_parts_of_one_file_go_into_one_file_whatever_names_they_carry() {
        let root = std::env::temp_dir().join(format!("airband-parts-{}", config::random_hex(4)));
        let part = |name: &str, from: u64, data: &[u8]| AttachmentPart { id: "a1b2c3d4e5f6".into(), name: name.into(), from, total: 6, data: airband_core::encode::base64(data) };
        assert_eq!(take_in(&root, part("photo.png", 0, b"abc")), Ok(3));
        // the second part says another name: it is still the same file, and the file is now whole
        assert_eq!(take_in(&root, part("other.png", 3, b"def")), Ok(6));
        let dir = root.join("a1b2c3d4e5f6");
        assert_eq!(std::fs::read(dir.join("photo.png")).unwrap(), b"abcdef");
        assert!(!dir.join("other.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
