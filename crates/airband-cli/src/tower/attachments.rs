//! Files he attaches from the radio: a photo, a video. They arrive in parts (one sealed message through a relay
//! carries about 250 KB) and are kept in a folder of the tower's own on this computer, where any agent can open
//! them by path. There is no limit on size from the model's side, because the model is handed a path, not the
//! bytes. A video cannot be watched by an agent, so a frame a second is pulled out beside it, with the program the
//! frames row names, when that program is here.
//!
//! A radio can write only here, only under names the tower makes safe, and only so much.
#![expect(clippy::disallowed_methods, reason = "writes each part where it belongs in a file still arriving: airband_io writes files whole")]
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
    airband_core::encode::is_hex(id, 8..=40)
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
    airband_io::files::make_folder(&dir).map_err(|e| e.to_string())?;
    let name = {
        let _guard = GOT.lock().unwrap();
        let named = dir.join(".name");
        match airband_io::files::read_text(&named) {
            Ok(Some(name)) if !name.is_empty() => name,
            _ => {
                let name = safe(&part.name);
                airband_io::files::write_atomic(&named, &name).map_err(|e| e.to_string())?;
                name
            }
        }
    };
    let path = dir.join(name);
    let bytes = airband_core::encode::unbase64(&part.data).ok_or("that part could not be read")?;
    // where this part ends, as a number that cannot wrap round: a part said to start past any file's end is refused
    let end = part.from.checked_add(bytes.len() as u64).ok_or("a part runs past the end of the file")?;
    let mut file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| e.to_string())?;
    if end > MOST || part.total > MOST {
        if let Err(e) = airband_io::files::remove_folder(&dir) {
            log(format!("a file too large to take could not be cleared away ({e})"));
        }
        return Err("that file is too large".into());
    }
    // The radio says how long the file is, and its parts come in any order, several at once. Each is written where
    // it belongs, and what has come is kept beside the file until all of it has. Whether it is whole, and its mark
    // saying so, are settled under one hold, so a part sent again meanwhile never writes a partial record back.
    if end > part.total {
        return Err("a part runs past the end of the file".into());
    }
    file.seek(SeekFrom::Start(part.from)).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let (next, first_whole) = {
        let _guard = GOT.lock().unwrap_or_else(|e| e.into_inner());
        // a part sent again after the file was whole: it is whole still, and what came is not counted afresh
        // from a note that went when it was
        if dir.join(".whole").exists() {
            return Ok(part.total);
        }
        let note = dir.join(".got");
        let mut got: Vec<(u64, u64)> = airband_io::files::read_text(&note).ok().flatten().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        got.push((part.from, end));
        got.sort();
        // how far it is whole from the start
        let mut have = 0u64;
        for (from, to) in &got {
            if *from > have {
                break;
            }
            have = have.max(*to);
        }
        if have >= part.total {
            // the file is named where it is first whole: what is read back is this name, never whatever the folder
            // lists first
            airband_io::files::write_atomic(&dir.join(".whole"), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).map_err(|e| e.to_string())?;
            airband_io::files::remove(&note).ok();
        } else {
            airband_io::files::write_atomic(&note, serde_json::to_string(&got).unwrap_or_default()).map_err(|e| e.to_string())?;
        }
        (have, have >= part.total)
    };
    if first_whole {
        // its name is his: the log has its size only
        log(format!("he attached a file of {} KB", next / 1024));
        if what(&path) == What::Video {
            frames(&path);
        } else {
            airband_io::files::write_atomic(&dir.join(".ready"), "").map_err(|e| e.to_string())?;
        }
    }
    Ok(next)
}

/// The videos whose frames are being pulled out now, by folder: each is told when its program has ended, so a
/// message that carries one waits for exactly that, never a guess at how long it takes.
static MAKING: std::sync::Mutex<Vec<(PathBuf, tokio::sync::watch::Receiver<bool>)>> = std::sync::Mutex::new(Vec::new());

/// Pull a frame a second out of a video, beside it, so an agent has something it can look at, with the program
/// the frames row names. Without it the video is still there and the agent is told to pull frames itself. Its
/// folder is in [`MAKING`] until the program has ended and `.ready` says so.
fn frames(video: &Path) -> tokio::sync::watch::Receiver<bool> {
    let (video, dir) = (video.to_path_buf(), video.parent().map(Path::to_path_buf).unwrap_or_default());
    let (done, waiting) = tokio::sync::watch::channel(false);
    MAKING.lock().unwrap_or_else(|e| e.into_inner()).push((dir.clone(), waiting.clone()));
    std::thread::spawn(move || {
        let out = dir.join("frames");
        // each program its rows name is tried in turn; the first that runs and succeeds is the one that did it
        let ran = airband_io::files::make_folder(&out).is_ok()
            && airband_core::catalog::frames_programs().iter().any(|f| {
                airband_io::child::command(&f.program)
                    .args(f.run(&video.display().to_string(), FRAMES, &out.display().to_string()))
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            });
        if !ran && let Err(e) = airband_io::files::remove_folder(&out) {
            // its name is his: never in the log
            log(format!("the frames of a video that could not be read could not be cleared away ({e})"));
        }
        // without its mark the file is never offered to an agent after a restart, so a mark that cannot be made is
        // said; those waiting now are told all the same, since the program has ended
        if let Err(e) = airband_io::files::write_atomic(&dir.join(".ready"), "") {
            log(format!("a video he attached could not be marked ready ({e})"));
        }
        MAKING.lock().unwrap_or_else(|e| e.into_inner()).retain(|(d, _)| *d != dir);
        done.send(true).ok();
    });
    waiting
}

/// Wait until the frames of the video in `dir` are all there: at once when it is marked ready; else until the
/// program making them ends; else (the tower stopped while they were being made) they are made again, and waited
/// for. Never a folder half written.
async fn frames_done(dir: &Path, video: &Path) {
    let waiting = {
        let making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
        if dir.join(".ready").exists() {
            return;
        }
        making.iter().find(|(d, _)| d == dir).map(|(_, w)| w.clone())
    };
    let mut waiting = match waiting {
        Some(w) => w,
        None => {
            airband_io::files::remove_folder(&dir.join("frames")).ok();
            frames(video)
        }
    };
    // its sender dropping is its program having ended too
    waiting.wait_for(|done| *done).await.ok();
}

pub(super) struct Attached {
    pub(super) path: PathBuf,
    pub(super) what: What,
    pub(super) bytes: u64,
    pub(super) frames: Vec<PathBuf>,
}

/// The file behind an id, once all of it has arrived. A video's frames are waited for until all are there.
pub(super) async fn find(id: &str) -> Option<Attached> {
    find_in(&root(), id).await
}

/// [`find`], in the attachments folder `root`.
async fn find_in(root: &Path, id: &str) -> Option<Attached> {
    if !hex(id) {
        return None;
    }
    let dir = root.join(id);
    let name = airband_io::files::read_text(&dir.join(".whole")).ok().flatten().filter(|n| !n.is_empty())?;
    let path = Some(dir.join(safe(&name))).filter(|p| p.is_file())?;
    if what(&path) == What::Video {
        frames_done(&dir, &path).await;
    }
    // no folder of frames is no frames; one that cannot be read is said (never his file's name), and the file is
    // offered without them
    let mut frames: Vec<PathBuf> = airband_io::files::entries(&dir.join("frames")).unwrap_or_else(|e| {
        if e.kind() != std::io::ErrorKind::NotFound {
            log(format!("the frames beside a video he attached could not be read ({e})"));
        }
        Vec::new()
    });
    frames.sort();
    let bytes = std::fs::metadata(&path).ok()?.len();
    Some(Attached { what: what(&path), bytes, path, frames })
}

/// What an agent is told about the files that came with a message, and the pictures to show beside his words
/// in the chat (an image itself; for a video, its first frame).
/// A file that has not all arrived is not left out without a word: his words wait, and he is told.
pub(super) async fn describe(ids: &[String]) -> Result<(String, Vec<airband_core::radio::Picture>), String> {
    if ids.len() > MOST_AT_ONCE {
        return Err(format!("Send at most {MOST_AT_ONCE} files at once."));
    }
    let (mut lines, mut pictures) = (Vec::new(), Vec::new());
    for id in ids {
        let Some(a) = find(id).await else {
            return Err(if ids.len() == 1 { "Your file had not arrived. Send it again.".into() } else { "One of your files had not arrived. Send them again.".into() });
        };
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
            What::Video => lines.push(match airband_core::catalog::frames_programs().iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" or ") {
                with if with.is_empty() => format!("- {} (a video, {size}). You cannot watch it: pull frames out of it to look at.", a.path.display()),
                with => format!("- {} (a video, {size}). You cannot watch it: pull frames out of it to look at, with {with} if it is installed.", a.path.display()),
            }),
            What::Other => lines.push(format!("- {} ({size})", a.path.display())),
        }
    }
    if lines.is_empty() {
        return Ok((String::new(), pictures));
    }
    Ok((format!("\n\nHe attached {}, saved on this computer:\n{}", if lines.len() == 1 { "a file" } else { "files" }, lines.join("\n")), pictures))
}

/// The most files one message can carry.
const MOST_AT_ONCE: usize = 8;

/// Old attachments are cleared when the tower starts.
pub(super) fn prune() {
    airband_io::files::prune_older(&root(), Duration::from_secs(KEEP_DAYS * 86_400));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_that_has_not_arrived_holds_his_words_back_and_says_so() {
        assert!(describe(&[]).await.unwrap().0.is_empty(), "no files, nothing to say");
        assert!(describe(&["not-one-of-ours".into()]).await.is_err());
        assert!(describe(&vec!["x".to_string(); MOST_AT_ONCE + 1]).await.is_err(), "more than one message carries");
    }

    #[test]
    fn a_part_sent_again_after_the_file_is_whole_finds_it_whole() {
        let root = std::env::temp_dir().join(format!("airband-again-{}", crate::config::random_hex(4)));
        let id = "ab".repeat(16);
        let part = |from: u64, data: &[u8]| AttachmentPart { id: id.clone(), name: "note.txt".into(), from, total: 6, data: airband_core::encode::base64(data) };
        assert_eq!(take_in(&root, part(0, b"abc")).unwrap(), 3);
        assert_eq!(take_in(&root, part(3, b"def")).unwrap(), 6);
        // its answer was lost, and the radio sends the first part again
        assert_eq!(take_in(&root, part(0, b"abc")).unwrap(), 6);
        assert!(!root.join(&id).join(".got").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A video whose frames are still being made is offered only once its program has ended, with every frame,
    /// and nothing of his file's name is written in the log on the way.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_videos_frames_are_waited_for_until_their_program_has_ended() {
        let root = std::env::temp_dir().join(format!("airband-frames-{}", config::random_hex(4)));
        let id = "cd".repeat(8);
        let dir = root.join(&id);
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        std::fs::write(dir.join("secret-holiday.mp4"), b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom").unwrap();
        std::fs::write(dir.join(".whole"), "secret-holiday.mp4").unwrap();
        // its program is still running: one frame written so far
        let (done, waiting) = tokio::sync::watch::channel(false);
        MAKING.lock().unwrap().push((dir.clone(), waiting));
        std::fs::write(dir.join("frames").join("f001.jpg"), b"x").unwrap();
        let (found_root, found_id) = (root.clone(), id.clone());
        let found = tokio::spawn(async move { find_in(&found_root, &found_id).await });
        // until it waits on the program
        while done.receiver_count() < 2 {
            tokio::task::yield_now().await;
        }
        assert!(!found.is_finished(), "offered while its frames were still being made");
        std::fs::write(dir.join("frames").join("f002.jpg"), b"x").unwrap();
        std::fs::write(dir.join(".ready"), "").unwrap();
        MAKING.lock().unwrap().retain(|(d, _)| *d != dir);
        done.send(true).ok();
        let a = found.await.unwrap().unwrap();
        assert_eq!(a.frames.len(), 2, "every frame, never part of them");
        assert!(!crate::tower::logged::take().iter().any(|l| l.contains("secret-holiday")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_file_not_yet_whole_is_not_found_and_nothing_is_waited_for() {
        let root = std::env::temp_dir().join(format!("airband-notyet-{}", config::random_hex(4)));
        let part = AttachmentPart { id: "ef".repeat(8), name: "a.txt".into(), from: 0, total: 6, data: airband_core::encode::base64(b"abc") };
        assert_eq!(take_in(&root, part), Ok(3));
        assert!(find_in(&root, &"ef".repeat(8)).await.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

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

    #[test]
    fn a_part_said_to_start_past_any_end_is_refused_never_wrapped_round() {
        let root = std::env::temp_dir().join(format!("airband-parts-{}", config::random_hex(4)));
        let part = AttachmentPart { id: "a1b2c3d4e5f7".into(), name: "x.png".into(), from: u64::MAX - 1, total: 6, data: airband_core::encode::base64(b"abc") };
        assert_eq!(take_in(&root, part), Err("a part runs past the end of the file".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
