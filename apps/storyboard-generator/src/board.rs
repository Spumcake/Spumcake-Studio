//! A clip's images for other workspaces: its first frame, the clip's
//! board, and its last frame, the images a shot is rendered from. Each is
//! what that frame renders inside the field: every layer that shows, in
//! layer order, at its opacity. The eye and the outline are viewing aids,
//! except that a hidden layer is left out, as the owner set on 2026-11-06.
//!
//! An image's fingerprint is the fingerprint of its pixels, so it changes
//! exactly when the image does: not when the clip moves or is renamed,
//! when the view zooms, or when ink outside the field changes. The pixels
//! are drawn again only when what the frame holds changed.

use crate::film::{Clip, Film};
use crate::look::Looker;
use crate::record::Record;
use draw::document::SceneMark;
use protocol::Item;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The field's white.
const PAPER: [u8; 3] = [255, 255, 255];
/// The longest side an image's fingerprint is drawn at.
const FINGERPRINT_SIDE: u32 = 480;
/// The longest side a rendered image is drawn at.
const RENDER_SIDE: u32 = 1920;
/// The longest side of a clip's thumbnail on the timeline.
const THUMBNAIL_SIDE: u32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    First,
    Last,
}

impl Which {
    pub fn word(self) -> &'static str {
        match self {
            Which::First => "first",
            Which::Last => "last",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "first" => Some(Which::First),
            "last" => Some(Which::Last),
            _ => None,
        }
    }
}

/// The cut's item id: the order and timing of the shots, and their sound.
pub const CUT: &str = "cut";
/// The exported animatic's item id.
pub const ANIMATIC: &str = "animatic";

/// A clip's frames item's id: every frame it shows.
pub fn frames_id(clip: u64) -> String {
    format!("{clip}-frames")
}

/// The cut's fingerprint: each drawn clip's place and length, the Shots
/// track's eye, and each Sound track's mute and each sound's place, trim,
/// volume, and file. Nothing a person only looks at enters it.
pub fn cut_fingerprint(film: &Film, sounds: &mut crate::sounds::Library, project: Option<&Path>) -> String {
    let mut parts = vec![format!("shots {}", film.shots_visible)];
    for clip in &film.clips {
        parts.push(format!("clip {} {} {}", clip.id, clip.start, clip.length));
    }
    for track in &film.sound_tracks {
        parts.push(format!("track {} {}", track.id, track.muted));
        for clip in &track.clips {
            let file = sounds.loaded(project, &clip.file).map_or_else(|| "missing".to_string(), |loaded| loaded.fingerprint.clone());
            parts.push(format!("sound {} {} {} {} {} {file}", clip.id, clip.at, clip.from, clip.length, clip.volume));
        }
    }
    protocol::fingerprint(parts.join("\n").as_bytes())
}

/// A clip's frames' fingerprint: its length and, from each frame where
/// what it shows can change, that frame's description, as its first and
/// last frames are described.
pub fn frames_fingerprint(record: &Record, clip: &Clip) -> String {
    let mut changes: Vec<u32> = vec![0];
    for layer in &clip.layers {
        changes.extend(layer.keys.iter().map(|key| key.frame).filter(|frame| *frame < clip.length));
    }
    changes.sort_unstable();
    changes.dedup();
    let mut parts = vec![format!("length {}", clip.length)];
    for frame in changes {
        parts.push(format!("{frame}|{}", description(record, record.film(), clip, frame)));
    }
    protocol::fingerprint(parts.join("\n").as_bytes())
}

/// The name a storyboard's items and export go by: its title, or its
/// file's name when the title is empty.
pub fn title_of(record: &Record) -> String {
    let title = record.film().title.trim();
    if title.is_empty() { record.name().to_string() } else { title.to_string() }
}

/// An item's id: the clip's id and which frame.
pub fn item_id(clip: u64, which: Which) -> String {
    format!("{clip}-{}", which.word())
}

pub fn parse_item(id: &str) -> Option<(u64, Which)> {
    let (clip, which) = id.split_once('-')?;
    Some((clip.parse().ok()?, Which::from_word(which)?))
}

fn frame_of(clip: &Clip, which: Which) -> u32 {
    match which {
        Which::First => 0,
        Which::Last => clip.length.saturating_sub(1),
    }
}

/// Every mark the frame renders, bottom layer first, at its layers' opacity.
pub(crate) fn marks(record: &Record, clip: &Clip, frame: u32) -> Vec<SceneMark> {
    let mut out = Vec::new();
    for layer in clip.layers.iter().rev() {
        if layer.folder || !clip.shows(layer) {
            continue;
        }
        let Some(drawing) = layer.key_at(frame).and_then(|key| record.drawing_of(key.drawing)) else {
            continue;
        };
        for mark in drawing.scene(None) {
            out.push(match mark {
                SceneMark::Ink(mut paint) => {
                    paint.opacity *= layer.opacity;
                    SceneMark::Ink(paint)
                }
                text => text,
            });
        }
    }
    out
}

/// What decides the image: the field, and in order each layer's opacity
/// and its drawing's content, without the view or the ids.
fn description(record: &Record, film: &Film, clip: &Clip, frame: u32) -> String {
    let mut parts = vec![format!("{:?}", film.size)];
    for layer in clip.layers.iter().rev() {
        if layer.folder || !clip.shows(layer) {
            continue;
        }
        let Some(drawing) = layer.key_at(frame).and_then(|key| record.drawing_of(key.drawing)) else {
            continue;
        };
        let mut value = drawing.snapshot_value(draw::document::DrawingId::new(1), "Frame");
        strip(&mut value);
        parts.push(format!("{}|{}", layer.opacity, value));
    }
    parts.join("\n")
}

/// The camera, the name, and every id leave the description.
fn strip(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["camera", "id", "name", "source"] {
                map.remove(key);
            }
            for child in map.values_mut() {
                strip(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip(item);
            }
        }
        _ => {}
    }
}

/// Fingerprints by description, so an image is drawn again only when what
/// its frame holds changed.
#[derive(Default)]
pub struct Boards {
    fingerprints: HashMap<String, String>,
}

impl Boards {
    pub fn fingerprint(&mut self, looker: &mut Looker, record: &Record, clip: &Clip, which: Which) -> Option<String> {
        let film = record.film();
        let frame = frame_of(clip, which);
        let description = description(record, film, clip, frame);
        if let Some(found) = self.fingerprints.get(&description) {
            return Some(found.clone());
        }
        let [x, y, width, height] = crate::sheet::field_in(film, clip);
        let (pixels, w, h) =
            looker.pixels(marks(record, clip, frame), [x, y], [x + width, y + height], FINGERPRINT_SIDE, PAPER)?;
        let mut bytes = format!("{w}x{h}").into_bytes();
        bytes.extend(pixels);
        let fingerprint = protocol::fingerprint(&bytes);
        if self.fingerprints.len() > 4096 {
            self.fingerprints.clear();
        }
        self.fingerprints.insert(description, fingerprint.clone());
        Some(fingerprint)
    }

    /// Every item of a storyboard: each clip's first and last frames and
    /// its frames, the cut, and the animatic once it is exported. `path` is
    /// the storyboard's file in the project, which the animatic's sources
    /// name.
    pub fn items(
        &mut self,
        looker: &mut Looker,
        record: &Record,
        sounds: &mut crate::sounds::Library,
        project: Option<&Path>,
        path: &str,
    ) -> Vec<Item> {
        let mut items = vec![Item {
            id: CUT.to_string(),
            name: format!("{} — cut", title_of(record)),
            media_types: Vec::new(),
            fingerprint: cut_fingerprint(record.film(), sounds, project),
            made: None,
        }];
        for clip in &record.film().clips {
            items.push(Item {
                id: frames_id(clip.id),
                name: format!("{} — frames", clip.name),
                media_types: Vec::new(),
                fingerprint: frames_fingerprint(record, clip),
                made: None,
            });
        }
        if let Some(exported) = record.exported() {
            items.push(Item {
                id: ANIMATIC.to_string(),
                name: format!("{} — animatic", title_of(record)),
                media_types: vec!["video/mp4".to_string()],
                fingerprint: exported.fingerprint(),
                made: Some(protocol::Made {
                    cell: None,
                    version: None,
                    used: Vec::new(),
                    sources: exported
                        .sources
                        .iter()
                        .map(|(item, fingerprint)| protocol::UsedItem {
                            item: protocol::ItemRef { path: path.to_string(), item: Some(item.clone()) },
                            fingerprint: fingerprint.clone(),
                        })
                        .collect(),
                }),
            });
        }
        for clip in &record.film().clips {
            for which in [Which::First, Which::Last] {
                let Some(fingerprint) = self.fingerprint(looker, record, clip, which) else {
                    continue;
                };
                items.push(Item {
                    id: item_id(clip.id, which),
                    name: format!("{} — {} frame", clip.name, which.word()),
                    media_types: vec!["image/png".to_string()],
                    fingerprint,
                    made: None,
                });
            }
        }
        items
    }
}

/// Writes a clip's frame as a PNG at
/// `.spumcake/renders/storyboard-generator/<item>-<fingerprint>.png`, never
/// over an existing one. Returns its path inside the project.
pub fn render(
    boards: &mut Boards,
    looker: &mut Looker,
    record: &Record,
    project: &Path,
    clip: u64,
    which: Which,
) -> Result<(String, String), String> {
    let film = record.film();
    let Some(clip) = film.clip(clip) else {
        return Err("That clip is no longer in the storyboard.".to_string());
    };
    let fingerprint = boards
        .fingerprint(looker, record, clip, which)
        .ok_or_else(|| "The frame could not be drawn.".to_string())?;
    let relative = format!("{}/renders/storyboard-generator/{}-{fingerprint}.png", protocol::PROJECT_DIR, item_id(clip.id, which));
    let path = project.join(&relative);
    if !path.is_file() {
        let [x, y, width, height] = crate::sheet::field_in(film, clip);
        let frame = frame_of(clip, which);
        let image = looker
            .plain_image(marks(record, clip, frame), [x, y], [x + width, y + height], RENDER_SIDE, PAPER)
            .ok_or_else(|| "The frame could not be drawn.".to_string())?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| crate::store::reason(&error))?;
        }
        std::fs::write(&path, image.png).map_err(|error| crate::store::reason(&error))?;
    }
    Ok((relative, fingerprint))
}

/// A clip's first frame as a small raw image for its timeline thumbnail,
/// written once per fingerprint in a scratch folder of this program's.
pub fn thumbnail(boards: &mut Boards, looker: &mut Looker, record: &Record, clip: &Clip) -> Option<protocol::Thumbnail> {
    let fingerprint = boards.fingerprint(looker, record, clip, Which::First)?;
    let folder = scratch_folder();
    let path = folder.join(format!("{fingerprint}.rgba"));
    let size = folder.join(format!("{fingerprint}.size"));
    let film = record.film();
    let [x, y, width, height] = crate::sheet::field_in(film, clip);
    let (w, h) = match std::fs::read_to_string(&size).ok().and_then(|text| {
        let (w, h) = text.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }) {
        Some(found) if path.is_file() => found,
        _ => {
            let (pixels, w, h) =
                looker.pixels(marks(record, clip, 0), [x, y], [x + width, y + height], THUMBNAIL_SIDE, PAPER)?;
            std::fs::create_dir_all(&folder).ok()?;
            std::fs::write(&path, pixels).ok()?;
            std::fs::write(&size, format!("{w}x{h}")).ok()?;
            (w, h)
        }
    };
    Some(protocol::Thumbnail {
        path: path.display().to_string(),
        width: w,
        height: h,
        revision: u64::from_str_radix(&fingerprint, 16).unwrap_or(0),
    })
}

fn scratch_folder() -> PathBuf {
    // Named by fingerprint alone, so any copy of this program finds the same file.
    std::env::temp_dir().join("storyboard-generator-thumbnails")
}
