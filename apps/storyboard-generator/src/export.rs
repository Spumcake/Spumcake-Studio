//! The animatic video this workspace exports itself: the film's frames, as
//! playback shows them, with its sound, encoded by `media`. An export is an
//! item of the storyboard, made from the cut and each clip's frames; what it
//! was made from is kept beside the film, never in its undo history, so an
//! undo never loses it.

use serde_json::{Value, json};

/// What one finished export was made from, as `.smx` keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    /// Its file, relative to the project.
    pub file: String,
    /// The fingerprint of the file's bytes when it was written, so a render
    /// can tell the file was replaced.
    pub bytes: String,
    /// Each item it was made from, by id, with the fingerprint it had when
    /// the export began.
    pub sources: Vec<(String, String)>,
}

impl Exported {
    /// The animatic's own fingerprint: of this record, so it changes only
    /// when an export finishes.
    pub fn fingerprint(&self) -> String {
        protocol::fingerprint(self.to_value().to_string().as_bytes())
    }

    pub fn to_value(&self) -> Value {
        json!({
            "file": self.file,
            "bytes": self.bytes,
            "sources": self.sources.iter().map(|(item, fingerprint)| json!({ "item": item, "fingerprint": fingerprint })).collect::<Vec<_>>(),
        })
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let mut sources = Vec::new();
        for source in value.get("sources")?.as_array()? {
            sources.push((source.get("item")?.as_str()?.to_string(), source.get("fingerprint")?.as_str()?.to_string()));
        }
        Some(Exported {
            file: value.get("file")?.as_str()?.to_string(),
            bytes: value.get("bytes")?.as_str()?.to_string(),
            sources,
        })
    }
}

use crate::film::Film;
use crate::look::Looker;
use crate::record::Record;
use draw::document::SceneMark;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The project's folder exports go in.
pub const FOLDER: &str = "exports";
/// A keyframe each second, so the file scrubs well.
const KEYFRAMES: u32 = crate::film::FPS;
/// Frames mixed at once while exporting.
const SOUND_CHUNK: u32 = crate::film::FPS;

/// The field's white, where a hidden Shots track shows nothing.
const WHITE: [u8; 3] = [255, 255, 255];
/// A gap: black, as the stage shows it.
const BLACK: [u8; 3] = [0, 0, 0];

/// One image the animatic shows: what a clip's frame renders, inside its
/// field. A hold shows one image for all its frames.
pub struct Image {
    marks: Vec<SceneMark>,
    min: [f64; 2],
    max: [f64; 2],
    /// An image key's image under the marks, by its path in the project.
    key: Option<String>,
}

/// What one frame of the film shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Gap,
    /// The Shots track is hidden.
    White,
    Image(usize),
}

/// Everything an export needs, taken from the storyboard when it starts,
/// so the person keeps working while it runs.
pub struct Plan {
    /// Even, as 4:2:0 video needs.
    pub size: (u32, u32),
    pub frames: Vec<Shown>,
    /// Each taken when it is first drawn.
    pub images: Vec<Option<Image>>,
    pub tracks: Vec<media::Track>,
    /// The project image keys are found in.
    pub project: Option<std::path::PathBuf>,
}

/// The even frame size an export of `film` is written at.
pub fn size_of(film: &Film) -> (u32, u32) {
    ((film.size[0] & !1).max(2), (film.size[1] & !1).max(2))
}

/// What each frame of the film shows, each distinct image listed once.
pub fn plan(record: &Record, tracks: Vec<media::Track>, project: Option<&std::path::Path>) -> Plan {
    let film = record.film();
    let length = film.length();
    let mut frames = vec![if film.shots_visible { Shown::Gap } else { Shown::White }; length as usize];
    let mut images = Vec::new();
    if film.shots_visible {
        for clip in &film.clips {
            // Where what the clip shows can change: its start and each key.
            let mut changes: Vec<u32> = vec![0];
            for layer in &clip.layers {
                changes.extend(layer.keys.iter().map(|key| key.frame).filter(|frame| *frame < clip.length));
            }
            changes.sort_unstable();
            changes.dedup();
            let [x, y, width, height] = crate::sheet::field_in(film, clip);
            for (index, change) in changes.iter().enumerate() {
                let end = changes.get(index + 1).copied().unwrap_or(clip.length);
                images.push(Some(Image {
                    marks: crate::board::marks(record, clip, *change),
                    min: [x, y],
                    max: [x + width, y + height],
                    key: crate::images::image_at(clip, *change).map(str::to_string),
                }));
                for frame in clip.start + change..clip.start + end {
                    if let Some(shown) = frames.get_mut(frame as usize) {
                        *shown = Shown::Image(images.len() - 1);
                    }
                }
            }
        }
    }
    Plan { size: size_of(film), frames, images, tracks, project: project.map(std::path::Path::to_path_buf) }
}

/// A frame's pixels at exactly `size`: the image, which is taken from
/// the plan, white, or black.
pub fn pixels(looker: &mut Looker, plan: &mut Plan, shown: Shown) -> Vec<u8> {
    let (width, height) = plan.size;
    let fill = |rgb: [u8; 3]| {
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width * height {
            out.extend([rgb[0], rgb[1], rgb[2], 255]);
        }
        out
    };
    let Shown::Image(index) = shown else {
        return fill(if shown == Shown::White { WHITE } else { BLACK });
    };
    let Some(image) = plan.images.get_mut(index).and_then(Option::take) else {
        return fill(WHITE);
    };
    let side = width.max(height);
    let base = keyed(plan.project.as_deref(), &image, side);
    let Some((rendered, w, h)) = looker.pixels_on(image.marks, image.min, image.max, side, WHITE, base.as_deref()) else {
        return fill(WHITE);
    };
    placed(&rendered, (w, h), plan.size)
}

/// A image's pixels at exactly `size`, as playback shows it: the same
/// as the export writes, its marks copied rather than taken.
pub fn render_image(looker: &mut Looker, image: &Image, size: (u32, u32), project: Option<&std::path::Path>) -> Vec<u8> {
    let side = size.0.max(size.1);
    let base = keyed(project, image, side);
    match looker.pixels_on(image.marks.clone(), image.min, image.max, side, WHITE, base.as_deref()) {
        Some((rendered, w, h)) => placed(&rendered, (w, h), size),
        None => placed(&[], (0, 0), size),
    }
}

/// An image key's image, fitted to the size the field is drawn at.
fn keyed(project: Option<&std::path::Path>, image: &Image, side: u32) -> Option<Vec<u8>> {
    let key = image.key.as_deref()?;
    let (width, height, _) = crate::look::size_for(image.min, image.max, side);
    crate::images::fitted(project, key, (width, height))
}

/// Pixels rendered at the field's size, give or take a pixel, placed on
/// white at exactly `size`.
fn placed(rendered: &[u8], (w, h): (u32, u32), (width, height): (u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..width * height {
        out.extend([WHITE[0], WHITE[1], WHITE[2], 255]);
    }
    for row in 0..height.min(h) as usize {
        let from = row * w as usize * 4;
        let to = row * width as usize * 4;
        let count = width.min(w) as usize * 4;
        out[to..to + count].copy_from_slice(&rendered[from..from + count]);
    }
    out
}

/// An export under way on a thread of its own.
pub struct Job {
    pub storyboard: draw::document::DrawingId,
    /// Its file, relative to the project.
    pub file: String,
    pub total: u32,
    /// What it is made from, with the fingerprints they had at its start.
    pub sources: Vec<(String, String)>,
    written: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<String, String>>,
}

/// How a poll found an export.
pub enum Polled {
    Running,
    /// Finished: what it was made from, for the storyboard to keep.
    Done(Exported),
    Failed(String),
}

impl Job {
    pub fn written(&self) -> u32 {
        self.written.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn poll(&self) -> Polled {
        match self.result.try_recv() {
            Err(TryRecvError::Empty) => Polled::Running,
            Err(TryRecvError::Disconnected) => Polled::Failed("The export stopped unexpectedly.".to_string()),
            Ok(Err(reason)) => Polled::Failed(reason),
            Ok(Ok(bytes)) => Polled::Done(Exported { file: self.file.clone(), bytes, sources: self.sources.clone() }),
        }
    }
}

/// The export's file for a film titled `title`, relative to the project:
/// "exports/The Last Signal — animatic.mp4".
pub fn file_for(title: &str) -> String {
    let title: String = title.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { ' ' } else { c }).collect();
    let title = title.trim();
    let title = if title.is_empty() { "Untitled Storyboard" } else { title };
    format!("{FOLDER}/{title} — animatic.mp4")
}

/// Starts writing `plan` to `file` in `project`. The previous export of
/// that name stays whole until this one finishes.
pub fn start(
    plan: Plan,
    project: &Path,
    file: String,
    storyboard: draw::document::DrawingId,
    sources: Vec<(String, String)>,
) -> Job {
    let written = Arc::new(AtomicU32::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (send, result) = std::sync::mpsc::channel();
    let total = plan.frames.len() as u32;
    let path = project.join(&file);
    let (counter, stop) = (written.clone(), cancel.clone());
    std::thread::spawn(move || {
        let _ = send.send(write(plan, &path, &counter, &stop));
    });
    Job { storyboard, file, total, sources, written, cancel, result }
}

/// Writes the animatic. Returns its bytes' fingerprint.
fn write(mut plan: Plan, path: &Path, written: &AtomicU32, cancel: &AtomicBool) -> Result<String, String> {
    if plan.frames.is_empty() {
        return Err("The film has no clips to export.".to_string());
    }
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| crate::store::reason(&error))?;
    }
    let settings = media::Settings {
        width: plan.size.0,
        height: plan.size.1,
        rate: crate::sounds::fps(),
        keyframes: KEYFRAMES,
        sound: (!plan.tracks.is_empty()).then_some((crate::sounds::RATE, 2)),
    };
    let mut encoder = media::Encoder::create(path, settings).map_err(|error| error.to_string())?;
    let mut looker = Looker::new();
    // Each image is drawn once and kept while the frames that hold it are
    // written, which come one after another.
    let mut drawn: Option<(usize, Vec<u8>)> = None;
    let length = plan.frames.len() as u32;
    let frames = plan.frames.clone();
    for (frame, shown) in frames.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            // Dropping the encoder removes what was written.
            return Err("The export was cancelled.".to_string());
        }
        let frame = frame as u32;
        if !plan.tracks.is_empty() && frame % SOUND_CHUNK == 0 {
            let frames = SOUND_CHUNK.min(length - frame);
            let samples = media::mix(&plan.tracks, frame, frames, crate::sounds::RATE, crate::sounds::fps());
            encoder.sound(&samples).map_err(|error| error.to_string())?;
        }
        if let Shown::Image(index) = shown {
            if drawn.as_ref().is_none_or(|(at, _)| at != index) {
                drawn = Some((*index, pixels(&mut looker, &mut plan, *shown)));
            }
            let (_, pixels) = drawn.as_ref().expect("drawn above");
            encoder.frame(pixels).map_err(|error| error.to_string())?;
        } else {
            encoder.frame(&pixels(&mut looker, &mut plan, *shown)).map_err(|error| error.to_string())?;
        }
        written.store(encoder.frames_written(), Ordering::Relaxed);
    }
    encoder.finish().map_err(|error| error.to_string())?;
    let bytes = std::fs::read(path).map_err(|error| crate::store::reason(&error))?;
    Ok(protocol::fingerprint(&bytes))
}

/// The exported file, checked against what was recorded, copied to the
/// render path named by the animatic's fingerprint, never over an
/// existing one. Returns its path in the project and its fingerprint.
pub fn render(project: &Path, exported: &Exported) -> Result<(String, String), String> {
    let source = project.join(&exported.file);
    let bytes = std::fs::read(&source).ok().filter(|bytes| protocol::fingerprint(bytes) == exported.bytes);
    let Some(bytes) = bytes else {
        return Err("The animatic video has been moved or replaced. Export it again.".to_string());
    };
    let fingerprint = exported.fingerprint();
    let relative = format!("{}/renders/storyboard-generator/{}-{fingerprint}.mp4", protocol::PROJECT_DIR, crate::board::ANIMATIC);
    let path: PathBuf = project.join(&relative);
    if !path.is_file() {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| crate::store::reason(&error))?;
        }
        std::fs::write(&path, bytes).map_err(|error| crate::store::reason(&error))?;
    }
    Ok((relative, fingerprint))
}
