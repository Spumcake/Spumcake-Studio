//! `.smx` storyboards and recovery copies. A `.smx` file is the snapshot JSON at
//! a path the person chooses. Recovery lives in `recovery/` next to
//! preferences. Leftover files named `LEFTOVER_PREFIX<id>.json` stay where they
//! are: their ids are reserved, and they are neither opened nor deleted.

use crate::film::{Film, Onion};
use crate::record::Record;
use draw::document::snapshot::{SnapshotDrawing, drawing_from_value};
use draw::document::{Allocator, Camera, Drawing, DrawingId, ToolColor};
use std::collections::HashMap;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The stored spelling of the leftover files of an earlier recovery,
/// `<prefix><id>.json`, whose ids are still reserved.
const LEFTOVER_PREFIX: &str = "sketch-"; // The stored spelling.

pub(crate) fn normalize(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The same file, including two paths that canonicalize together.
pub(crate) fn same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(crate) fn ensure_smx(path: PathBuf) -> PathBuf {
    let already = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("smx"));
    if already {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".smx");
        PathBuf::from(name)
    }
}

/// The tab title: the file's name without `.smx`.
pub(crate) fn file_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Untitled")
        .to_string()
}

/// A storyboard as a file holds it, before the session makes it a record.
pub(crate) enum Stored {
    /// A version-1 file: one drawing, from before storyboards held a film.
    Drawing(SnapshotDrawing),
    /// A version-2 file: the film and its drawings.
    Film {
        id: DrawingId,
        name: String,
        film: Film,
        drawings: HashMap<u64, Drawing>,
        cameras: Vec<(Option<u64>, Camera)>,
        onion: Onion,
    },
}

impl Stored {
    pub(crate) fn id(&self) -> DrawingId {
        match self {
            Stored::Drawing(snapshot) => snapshot.id,
            Stored::Film { id, .. } => *id,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Stored::Drawing(snapshot) => &snapshot.name,
            Stored::Film { name, .. } => name,
        }
    }

    fn reserve_ids(&self, ids: &mut Allocator) {
        match self {
            Stored::Drawing(snapshot) => snapshot.drawing.reserve_ids(ids),
            Stored::Film { drawings, .. } => {
                for drawing in drawings.values() {
                    drawing.reserve_ids(ids);
                }
            }
        }
    }

    pub(crate) fn into_record(self, ids: &mut Allocator, palette: &[ToolColor]) -> Record {
        match self {
            Stored::Drawing(snapshot) => Record::from_snapshot(snapshot, ids, palette),
            Stored::Film { id, name, film, drawings, cameras, onion } => {
                let mut record = Record::from_film(id, name, film, drawings, ids, palette);
                record.set_cameras(cameras);
                record.set_onion(onion);
                record.set_dirty(false);
                record
            }
        }
    }
}

/// The version the `.smx` holding a film is written as.
const FILM_FORMAT: u64 = 2;

/// Reads either version of a storyboard file.
pub(crate) fn stored_from_text(text: &str) -> Option<Stored> {
    let value: Value = serde_json::from_str(text).ok()?;
    stored_from_value(&value)
}

fn stored_from_value(value: &Value) -> Option<Stored> {
    let object = value.as_object()?;
    match object.get("format_version")?.as_u64()? {
        1 => Some(Stored::Drawing(drawing_from_value(value)?)),
        FILM_FORMAT => {
            let film = Film::from_value(object.get("film")?)?;
            let mut drawings = HashMap::new();
            for (key, drawing) in object.get("drawings")?.as_object()? {
                let id: u64 = key.parse().ok()?;
                drawings.insert(id, drawing_from_value(drawing)?.drawing);
            }
            // Every key's drawing is in the file, or the file is damaged.
            if film.drawings().iter().any(|id| !drawings.contains_key(id)) {
                return None;
            }
            let mut cameras = Vec::new();
            for camera in object.get("cameras").and_then(Value::as_array).cloned().unwrap_or_default() {
                let clip = camera.get("clip").and_then(Value::as_u64);
                let origin = camera.get("origin").and_then(Value::as_array)?;
                let zoom = camera.get("zoom").and_then(Value::as_f64)?;
                let origin = [origin.first()?.as_f64()?, origin.get(1)?.as_f64()?];
                if !origin.iter().all(|value| value.is_finite()) || !(zoom.is_finite() && zoom > 0.0) {
                    continue;
                }
                cameras.push((clip, Camera { origin, zoom }));
            }
            Some(Stored::Film {
                id: DrawingId::new(object.get("id")?.as_u64().filter(|id| *id > 0)?),
                name: object.get("name")?.as_str()?.to_string(),
                film,
                drawings,
                cameras,
                onion: Onion::from_value(object.get("onion")),
            })
        }
        _ => None,
    }
}

/// The version-2 value of a storyboard: its film, the drawings it names, and
/// the view of the film and of each clip.
pub(crate) fn storyboard_value(storyboard: &Record, name: &str) -> Value {
    let mut drawings = Map::new();
    for (id, drawing) in storyboard.named_drawings() {
        drawings.insert(id.to_string(), drawing.snapshot_value(DrawingId::new(id), "Drawing"));
    }
    let mut cameras = Vec::new();
    let mut clips: Vec<Option<u64>> = vec![None];
    clips.extend(storyboard.film().clips.iter().map(|clip| Some(clip.id)));
    for clip in clips {
        if let Some(camera) = storyboard.camera_of(clip) {
            cameras.push(serde_json::json!({ "clip": clip, "origin": camera.origin, "zoom": camera.zoom }));
        }
    }
    serde_json::json!({
        "format_version": FILM_FORMAT,
        "id": storyboard.id().get(),
        "name": name,
        "film": storyboard.film().to_value(),
        "drawings": drawings,
        "cameras": cameras,
        "onion": storyboard.onion().to_value(),
    })
}

/// Reads a storyboard. The error is a sentence for the person: why it could not
/// be read, or that the file is not a storyboard.
pub(crate) fn read_smx(path: &Path) -> Result<Stored, String> {
    let bytes = std::fs::read(path).map_err(|error| reason(&error))?;
    String::from_utf8(bytes)
        .ok()
        .and_then(|text| stored_from_text(&text))
        .ok_or_else(|| "It is not a storyboard file, or it is damaged.".to_string())
}

pub(crate) fn readable_smx(path: &Path) -> bool {
    read_smx(path).is_ok()
}

/// Writes the snapshot. The stored name is `name`. The path is not in the file.
/// A failed write leaves any previous file as it was, and the error is a
/// sentence for the person.
pub(crate) fn write_smx(path: &Path, storyboard: &Record, name: &str) -> Result<(), String> {
    let bytes = smx_bytes(storyboard, name).ok_or_else(|| "The storyboard could not be encoded.".to_string())?;
    write_atomic_bytes(path, &bytes).map_err(|error| reason(&error))
}

/// Why a file operation failed, in words for the person.
pub(crate) fn reason(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::PermissionDenied => "You do not have permission to use this location.".to_string(),
        ErrorKind::NotFound => "The file or its folder no longer exists.".to_string(),
        ErrorKind::StorageFull => "The disk is full.".to_string(),
        ErrorKind::ReadOnlyFilesystem => "This location is read-only.".to_string(),
        _ => {
            // "Input/output error (os error 5)" becomes "Input/output error."
            let text = error.to_string();
            let text = text.split(" (os error").next().unwrap_or(&text).trim();
            format!("{text}.")
        }
    }
}

fn smx_bytes(storyboard: &Record, name: &str) -> Option<Vec<u8>> {
    serde_json::to_vec_pretty(&storyboard_value(storyboard, name)).ok()
}

pub(crate) fn recovery_path(dir: &Path, id: DrawingId) -> PathBuf {
    dir.join("recovery").join(format!("{}.json", id.get()))
}

/// One recovery copy. The file name is the storyboard id. `path` is included when
/// the storyboard has a file. The storyboard's `.smx` is not opened or written.
pub(crate) fn write_recovery(dir: &Path, storyboard: &Record) -> bool {
    let Some(bytes) = recovery_bytes(storyboard) else {
        return false;
    };
    write_atomic_bytes(&recovery_path(dir, storyboard.id()), &bytes).is_ok()
}

pub(crate) fn recovery_bytes(storyboard: &Record) -> Option<Vec<u8>> {
    let mut value = storyboard_value(storyboard, storyboard.name());
    let object = value.as_object_mut()?;
    if let Some(path) = storyboard.path().and_then(|path| path.to_str()) {
        object.insert("path".to_string(), Value::String(path.to_string()));
    } else {
        object.remove("path");
    }
    serde_json::to_vec_pretty(&value).ok()
}

pub(crate) fn delete_recovery(dir: &Path, id: DrawingId) {
    let _ = std::fs::remove_file(recovery_path(dir, id));
}

pub(crate) struct LoadedRecovery {
    pub storyboard: Stored,
    pub path: Option<PathBuf>,
}

/// A recovery file whose snapshot id matches its name. Anything else is absent.
pub(crate) fn read_recovery(dir: &Path, id: DrawingId) -> Option<LoadedRecovery> {
    let text = std::fs::read_to_string(recovery_path(dir, id)).ok()?;
    let (storyboard, path) = snapshot_and_path(&text)?;
    if storyboard.id() != id {
        return None;
    }
    Some(LoadedRecovery { storyboard, path })
}

/// Valid recovery files, ascending id. Unreadable files and id mismatches are
/// absent. The caller offers them one at a time.
pub(crate) fn recovery_offers(dir: &Path) -> Vec<(DrawingId, String)> {
    let mut offers = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir.join("recovery")) else {
        return offers;
    };
    for entry in entries.flatten() {
        let Some(id) = recovery_id(&entry.path()) else {
            continue;
        };
        let Some(loaded) = read_recovery(dir, id) else {
            continue;
        };
        let name = loaded
            .path
            .as_deref()
            .map(file_title)
            .unwrap_or_else(|| loaded.storyboard.name().to_string());
        offers.push((id, name));
    }
    offers.sort_by_key(|(id, _)| id.get());
    offers
}

/// The next storyboard id moves above every leftover `LEFTOVER_PREFIX` name and
/// every recovery file name, including a file that cannot be read. Node ids
/// move above those in a readable snapshot. Nothing is deleted or opened.
pub(crate) fn reserve(dir: &Path, ids: &mut Allocator, next_storyboard: &mut u64) {
    reserve_named(dir, LEFTOVER_PREFIX, ".json", ids, next_storyboard);
    reserve_named(&dir.join("recovery"), "", ".json", ids, next_storyboard);
}

fn reserve_named(
    dir: &Path,
    prefix: &str,
    suffix: &str,
    ids: &mut Allocator,
    next_storyboard: &mut u64,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name_id) = id_from_name(&entry.path(), prefix, suffix) else {
            continue;
        };
        *next_storyboard = (*next_storyboard).max(name_id.get() + 1);
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some((storyboard, _)) = snapshot_and_path(&text) else {
            continue;
        };
        *next_storyboard = (*next_storyboard).max(storyboard.id().get() + 1);
        storyboard.reserve_ids(ids);
    }
}

fn recovery_id(path: &Path) -> Option<DrawingId> {
    id_from_name(path, "", ".json")
}

fn id_from_name(path: &Path, prefix: &str, suffix: &str) -> Option<DrawingId> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id: u64 = rest.parse().ok()?;
    if id == 0 {
        return None;
    }
    Some(DrawingId::new(id))
}

fn snapshot_and_path(text: &str) -> Option<(Stored, Option<PathBuf>)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let path = value.as_object().and_then(path_from);
    Some((stored_from_value(&value)?, path))
}

fn path_from(object: &Map<String, Value>) -> Option<PathBuf> {
    let text = object.get("path")?.as_str()?;
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

/// Writes a temporary file beside `path`, flushes it to the disk, then
/// renames it over `path`, so a crash or power loss leaves either the old
/// file or the new one, never a partial one.
fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".to_string(),
    });
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // The rename itself reaches the disk with the folder. Not every
    // platform can open a folder for this, so a failure here is ignored.
    if let Ok(folder) = std::fs::File::open(parent) {
        let _ = folder.sync_all();
    }
    Ok(())
}

static EMERGENCY: Mutex<Vec<(PathBuf, Vec<u8>)>> = Mutex::new(Vec::new());

/// Replaces the copies a panic hook would write. Does not touch the disk.
pub(crate) fn publish_emergency(files: Vec<(PathBuf, Vec<u8>)>) {
    let mut guard = EMERGENCY.lock().unwrap_or_else(|err| err.into_inner());
    *guard = files;
}

/// Writes the last published recovery copies. The panic hook calls this, and
/// a test can call it instead of panicking.
pub(crate) fn flush_emergency() {
    let files = {
        let guard = EMERGENCY.lock().unwrap_or_else(|err| err.into_inner());
        guard.clone()
    };
    for (path, bytes) in files {
        let _ = write_atomic_bytes(&path, &bytes);
    }
}
