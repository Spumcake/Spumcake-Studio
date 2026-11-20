//! The `.vgx` file: a tab's gallery as the person arranged it, the links to
//! its takes, the view, the frame shown, the tab's cells, each a step
//! pointer or a draft, and its sound layers. It holds no video, no round or take a request made,
//! and no prompt sent, report, or record of what made a take; those are
//! read from the project's own records. The format is described in this
//! application's AGENTS.md.

use crate::gallery::{Gallery, Mode, TakeNotes, TakeSound};
use layers::notes::Notes;
use layers::link::Link;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The first line of every `.vgx` file.
const MAGIC: &[u8] = b"VGX1\n";
/// Version 3 keeps the tab's sound layers and its take's own sound;
/// version 2 the tab's cells; version 1, milestone 1's, had neither.
const VERSION: u32 = 3;

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
    /// From version 2: the tab's cells, as `layers::cells::stored_cell`
    /// keeps them, and the one the Prompt page shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cells: Vec<protocol::Cell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cell: Option<String>,
    /// From version 2: the notes marked on takes, as editing state, each by
    /// its take's file; never the take or its record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    take_notes: Vec<TakeNotesFile>,
    /// From version 3: the take's own sound, applying to whichever take is
    /// shown, when it is not at 0 dB and heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    take_sound: Option<TakeSoundFile>,
    /// From version 3: the tab's sound layers, top first, each a link to
    /// its file with its gain, start, and mute, as plain values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sounds: Vec<SoundFile>,
}

#[derive(Serialize, Deserialize)]
struct TakeSoundFile {
    #[serde(default)]
    gain_db: f64,
    #[serde(default)]
    muted: bool,
}

/// A sound layer: its name, its file as a take's link names one, its gain
/// in decibels, its start on the take in seconds, and whether it is muted.
#[derive(Serialize, Deserialize)]
struct SoundFile {
    name: String,
    link: LinkFile,
    #[serde(default)]
    gain_db: f64,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    muted: bool,
}

/// Notes on a take, by its file: its path, as a take's link names it, and
/// the sha256 of the bytes they were marked on.
#[derive(Serialize, Deserialize)]
struct TakeNotesFile {
    path: String,
    sha256: String,
    notes: Notes,
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
    pub cells: Vec<protocol::Cell>,
    pub cell: Option<String>,
    pub take_notes: Vec<TakeNotes>,
    pub take_sound: TakeSound,
    /// Each sound layer: its name, its link, its gain in decibels, its
    /// start in seconds, and whether it is muted.
    pub sounds: Vec<SoundRead>,
}

/// A sound layer as its file keeps it.
pub struct SoundRead {
    pub name: String,
    pub link: Link,
    pub gain_db: f64,
    pub start: f64,
    pub muted: bool,
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
        cells: gallery.cells.iter().map(layers::cells::stored_cell).collect(),
        cell: gallery.cell.clone().filter(|id| gallery.cells.iter().any(|cell| cell.id == *id)),
        take_notes: gallery
            .notes
            .iter()
            .filter(|kept| !kept.notes.is_empty())
            .map(|kept| TakeNotesFile { path: kept.path.clone(), sha256: kept.sha256.clone(), notes: kept.notes.clone() })
            .collect(),
        take_sound: (!gallery.take_sound.is_default())
            .then_some(TakeSoundFile { gain_db: gallery.take_sound.gain_db, muted: gallery.take_sound.muted }),
        sounds: gallery
            .sounds
            .iter()
            .map(|sound| SoundFile {
                name: sound.name.clone(),
                link: LinkFile { path: link_path(&sound.file, project), sha256: sound.link.sha256.clone() },
                gain_db: sound.gain_db,
                start: sound.start,
                muted: sound.muted,
            })
            .collect(),
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
        return Err("This file was written by a newer Spumcake for Desktop.".into());
    }
    let takes: Vec<(String, Link)> =
        header.takes.into_iter().map(|take| (take.name, Link { path: take.link.path, sha256: take.link.sha256 })).collect();
    let count = takes.len();
    let view = match header.view {
        ViewFile::Side { with } if with >= count || with == header.shown => ViewFile::Single,
        view => view,
    };
    // A cell is read as the file may keep it, whatever was written.
    let cells: Vec<protocol::Cell> = header.cells.iter().map(layers::cells::stored_cell).collect();
    let cell = header.cell.filter(|id| cells.iter().any(|cell| cell.id == *id));
    let take_notes = header
        .take_notes
        .into_iter()
        .filter(|kept| !kept.notes.is_empty())
        .map(|kept| TakeNotes { path: kept.path, sha256: kept.sha256, notes: kept.notes })
        .collect();
    let take_sound = header.take_sound.map_or_else(TakeSound::default, |kept| TakeSound { gain_db: kept.gain_db, muted: kept.muted });
    let sounds = header
        .sounds
        .into_iter()
        .map(|sound| SoundRead {
            name: sound.name,
            link: Link { path: sound.link.path, sha256: sound.link.sha256 },
            gain_db: sound.gain_db,
            start: sound.start,
            muted: sound.muted,
        })
        .collect();
    Ok(Read { takes, shown: header.shown.min(count.saturating_sub(1)), view, frame: header.frame, cells, cell, take_notes, take_sound, sounds })
}

/// RGBA pixels as a PNG, as the marked frame a next round sends is kept.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())?;
    Ok(out)
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
        assert!(text.starts_with("VGX1\n{\"version\":3,"), "{text}");
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

    /// Catches a draft's run or a step's copy kept, or a version 1 file that
    /// no longer opens: the cells come back as a pointer and a draft, and
    /// milestone 1's file opens with none.
    #[test]
    fn a_vgx_keeps_its_cells_and_reads_version_1() {
        let dir = scratch("cells");
        let mut gallery = Gallery::new(Vec::new(), 0);
        let mut draft = protocol::Cell::text("d1", "The door opens.");
        draft.output = vec![protocol::CellPart::Text { text: "a result".into() }];
        let mut step = protocol::Cell::text("s1", "a copy of the step");
        step.step = Some(protocol::StepRef { chain: "shot-12".into(), step: "take".into() });
        gallery.cells = vec![draft, step];
        gallery.cell = Some("s1".into());
        let path = dir.join("Cells.vgx");
        write_vgx(&path, &gallery, 0, Some(&dir)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("a result") && !text.contains("a copy"), "{text}");
        let read = read_vgx(&path).unwrap();
        assert_eq!(read.cell.as_deref(), Some("s1"));
        assert_eq!(read.cells[0].prompt(), "The door opens.");
        assert!(read.cells[1].step.is_some());
        let old = dir.join("Old.vgx");
        std::fs::write(&old, "VGX1\n{\"version\":1,\"shown\":0,\"takes\":[],\"view\":{\"mode\":\"single\"},\"frame\":0}\n").unwrap();
        let read = read_vgx(&old).unwrap();
        assert!(read.cells.is_empty() && read.cell.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catches a sound layer or the take's own sound lost, or kept as
    /// anything but plain values: the take's gain and mute, and each layer's
    /// name, link, gain, start, and mute come back; a take's own sound at
    /// 0 dB and heard is left out; and a version 2 file still opens, with
    /// none.
    #[test]
    fn a_vgx_keeps_its_sound_layers() {
        use crate::gallery::SoundLayer;
        let dir = scratch("sounds");
        let mut gallery = Gallery::new(Vec::new(), 0);
        let rain = dir.join("sounds").join("rain.wav");
        gallery.take_sound = TakeSound { gain_db: -3.0, muted: true };
        gallery.sounds = vec![SoundLayer {
            id: 1,
            name: "Rain".into(),
            link: Link { path: "sounds/rain.wav".into(), sha256: "cd".repeat(32) },
            file: rain.clone(),
            gain_db: -6.0,
            start: 1.5,
            muted: false,
        }];
        let path = dir.join("Sounds.vgx");
        write_vgx(&path, &gallery, 0, Some(&dir)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"take_sound\":{\"gain_db\":-3.0,\"muted\":true}"), "{text}");
        assert!(text.contains("\"gain_db\":-6.0,\"start\":1.5,\"muted\":false"), "{text}");
        let read = read_vgx(&path).unwrap();
        assert_eq!(read.take_sound, TakeSound { gain_db: -3.0, muted: true });
        assert_eq!(read.sounds.len(), 1);
        let sound = &read.sounds[0];
        assert_eq!((sound.name.as_str(), sound.link.path.as_str(), sound.gain_db, sound.start, sound.muted), ("Rain", "sounds/rain.wav", -6.0, 1.5, false));
        assert_eq!(resolve(&sound.link.path, Some(&dir), None), rain);
        gallery.take_sound = TakeSound::default();
        gallery.sounds.clear();
        write_vgx(&path, &gallery, 0, Some(&dir)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("take_sound") && !text.contains("sounds"), "{text}");
        let old = dir.join("Two.vgx");
        std::fs::write(&old, "VGX1\n{\"version\":2,\"shown\":0,\"takes\":[],\"view\":{\"mode\":\"single\"},\"frame\":0}\n").unwrap();
        let read = read_vgx(&old).unwrap();
        assert!(read.sounds.is_empty() && read.take_sound.is_default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
