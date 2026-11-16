//! The `.vgx` file: a tab's gallery as the person arranged it, the links to
//! its takes, the view, and the frame shown. It holds no video, and no
//! prompt, report, or record of what made a take; those are read from the
//! project's own records. The format is described in this application's
//! AGENTS.md.

use crate::gallery::{Gallery, Mode};
use layers::link::Link;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The first line of every `.vgx` file.
const MAGIC: &[u8] = b"VGX1\n";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    /// The take shown, by position from 0.
    #[serde(default)]
    shown: usize,
    takes: Vec<TakeFile>,
    #[serde(default)]
    view: ViewFile,
    /// The frame shown, from 0.
    #[serde(default)]
    frame: u32,
}

#[derive(Serialize, Deserialize)]
struct TakeFile {
    name: String,
    link: LinkFile,
}

/// A take's file: relative to the project's folder when it is inside it,
/// with `/` between folders, or else absolute; and its sha256.
#[derive(Serialize, Deserialize)]
struct LinkFile {
    path: String,
    sha256: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ViewFile {
    #[default]
    Single,
    Strips,
    /// Side by side, the shown take with the take at position `with`.
    Side { with: usize },
}

/// A gallery as its file keeps it.
pub struct Read {
    pub takes: Vec<(String, Link)>,
    pub shown: usize,
    pub view: ViewFile,
    pub frame: u32,
}

/// Whether the file starts as a `.vgx` file does.
pub fn is_vgx(path: &Path) -> bool {
    use std::io::Read as _;
    let mut start = [0u8; 5];
    std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut start)).is_ok() && start == MAGIC
}

/// How a file names a take's file: relative to `project` when inside it.
pub fn link_path(file: &Path, project: Option<&Path>) -> String {
    match project.and_then(|project| file.strip_prefix(project).ok()) {
        Some(inside) => inside.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"),
        None => file.display().to_string(),
    }
}

/// The file a link names: absolute as it is, or else inside `project`, or,
/// with none, beside the `.vgx` file.
pub fn resolve(path: &str, project: Option<&Path>, beside: Option<&Path>) -> PathBuf {
    let named = Path::new(path);
    if named.is_absolute() {
        return named.to_path_buf();
    }
    match project.or(beside) {
        Some(root) => root.join(named),
        None => named.to_path_buf(),
    }
}

/// Writes a gallery, its view, and its frame.
pub fn write_vgx(path: &Path, gallery: &Gallery, frame: u32, project: Option<&Path>) -> Result<(), String> {
    let view = match gallery.view() {
        Mode::Single => ViewFile::Single,
        Mode::Strips => ViewFile::Strips,
        Mode::Side { .. } => ViewFile::Side { with: gallery.other().unwrap_or(0) },
    };
    let header = Header {
        version: VERSION,
        shown: gallery.shown,
        takes: gallery
            .takes
            .iter()
            .map(|take| TakeFile {
                name: take.name.clone(),
                link: LinkFile { path: link_path(&take.file, project), sha256: take.link.sha256.clone() },
            })
            .collect(),
        view,
        frame,
    };
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(&header).map_err(|error| error.to_string())?);
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn read_vgx(path: &Path) -> Result<Read, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let Some(rest) = bytes.strip_prefix(MAGIC) else {
        return Err("This is not a Video Generator file.".into());
    };
    let line = rest.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let header: Header = serde_json::from_slice(line).map_err(|error| format!("The file is damaged: {error}."))?;
    if header.version > VERSION {
        return Err("This file was written by a newer Spumcake Studio.".into());
    }
    let takes: Vec<(String, Link)> =
        header.takes.into_iter().map(|take| (take.name, Link { path: take.link.path, sha256: take.link.sha256 })).collect();
    let count = takes.len();
    let view = match header.view {
        ViewFile::Side { with } if with >= count || with == header.shown => ViewFile::Single,
        view => view,
    };
    Ok(Read { takes, shown: header.shown.min(count.saturating_sub(1)), view, frame: header.frame })
}

/// Replaces a file whole: a scratch file beside it, then a rename, so no
/// reader ever sees half of one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, path).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        error.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("video-generator-files-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Catches a `.vgx` that loses a take, its link, the view, or the
    /// frame, or links a file inside the project by an absolute path.
    #[test]
    fn a_vgx_file_round_trips() {
        let dir = scratch("round-trip");
        let inside = dir.join("takes").join("shot 12.mp4");
        let outside = PathBuf::from("/elsewhere/take.mp4");
        let link = |path: &str| Link { path: path.into(), sha256: "ab".repeat(32) };
        let mut gallery = Gallery::new(
            vec![
                ("Shot 12".into(), link("takes/shot 12.mp4"), inside.clone(), None),
                ("Elsewhere".into(), link("/elsewhere/take.mp4"), outside.clone(), None),
            ],
            0,
        );
        let other = gallery.uid_at(1).unwrap();
        gallery.set_view(Mode::Side { other }, 0).unwrap();
        let path = dir.join("Untitled Video.vgx");
        write_vgx(&path, &gallery, 17, Some(&dir)).unwrap();
        assert!(is_vgx(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("VGX1\n{\"version\":1,"), "{text}");
        assert!(text.contains("\"path\":\"takes/shot 12.mp4\""), "{text}");
        let read = read_vgx(&path).unwrap();
        assert_eq!(read.takes.len(), 2);
        assert_eq!(read.takes[1].0, "Elsewhere");
        assert_eq!(read.takes[1].1.sha256, "ab".repeat(32));
        assert_eq!(read.view, ViewFile::Side { with: 1 });
        assert_eq!(read.frame, 17);
        assert_eq!(resolve(&read.takes[0].1.path, Some(&dir), None), inside);
        assert_eq!(resolve(&read.takes[1].1.path, Some(&dir), None), outside);
        assert!(!is_vgx(&dir.join("missing.vgx")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
