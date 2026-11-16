//! What the working area shows, drawn into the one raster image the host
//! draws: the shown take alone, two takes side by side at the same frame,
//! or every take as a row of eight frames. The image is the working area's
//! own size, and each frame is decoded at the size it is drawn, never
//! larger than the file's own, so a 4K take is never decoded whole to be
//! shown small. At 100% and more, the takes are drawn at that zoom about a
//! point the Move tool's drag moves; side by side, both show the same
//! region, the larger take at its own pixels, which is the before/after
//! crop.

use crate::decode::{Key, even};
use crate::gallery::{Gallery, Mode, Take};
use layers::Rgba;
use layers::layout::{self, Rect};

/// The room kept around the takes, as the host keeps around a fitted image.
pub const MARGIN: u32 = 40;
/// Between two takes side by side, and between the frames of the strips.
pub const SIDE_GAP: u32 = 16;
pub const STRIP_GAP: u32 = 8;
/// How many frames each take's strip shows, at equal times.
pub const STRIP_FRAMES: u32 = 8;
/// The working area's white, as the host draws around an image.
const GROUND: [u8; 4] = [255, 255, 255, 255];
/// A take whose file could not be read: the host's light checker grey.
const EMPTY: [u8; 4] = [224, 226, 230, 255];
/// The shape a take is drawn at until its file is known.
const WIDE: f32 = 16.0 / 9.0;

/// One frame drawn: which, where its whole is (`rect`), and the part of
/// the image it may cover (`clip`). No key draws the place empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub key: Option<Key>,
    pub rect: Rect,
    pub clip: Rect,
}

/// Everything the image shows, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub places: Vec<Place>,
    /// The frames come from the takes' seekers, as the frame stepped
    /// through; otherwise, as strips, from the strips' thread.
    pub live: bool,
    /// While zoomed, the point of the first take kept in the middle, as
    /// far as its edges let it be.
    pub focus: Option<[f64; 2]>,
}

impl Scene {
    pub fn keys(&self) -> Vec<Key> {
        self.places.iter().filter_map(|place| place.key.clone()).collect()
    }
}

/// How the shown takes are drawn: fitted, or at a zoom about `focus`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub frame: u32,
    /// None fits the takes in the area.
    pub zoom: Option<f64>,
    /// From 0 to 1 across and down the take.
    pub focus: [f64; 2],
}

/// The scene a gallery shows in an image of `size`.
pub fn scene(gallery: &Gallery, size: (u32, u32), look: Look) -> Scene {
    let (width, height) = size;
    let area = Rect {
        x: MARGIN as i32,
        y: MARGIN as i32,
        w: width.saturating_sub(2 * MARGIN).max(2),
        h: height.saturating_sub(2 * MARGIN).max(2),
    };
    let mut scene = Scene { width, height, places: Vec::new(), live: true, focus: None };
    match gallery.view() {
        Mode::Single => {
            if let Some(take) = gallery.take() {
                let (place, focus) = shown(take, area, look, None);
                scene.places.push(place);
                scene.focus = focus;
            }
        }
        Mode::Side { .. } => {
            let (Some(left), Some(right)) = (gallery.take(), gallery.other().map(|other| &gallery.takes[other])) else {
                return scene;
            };
            // Each half holds one take; at a zoom, both are drawn at the
            // larger one's scale, so they show the same region.
            let half = area.w.saturating_sub(SIDE_GAP) / 2;
            let widest = [left, right].iter().filter_map(|take| take.info.map(|info| info.width)).max();
            for (index, take) in [left, right].into_iter().enumerate() {
                let cell = Rect { x: area.x + (index as u32 * (half + SIDE_GAP)) as i32, y: area.y, w: half.max(2), h: area.h };
                let (place, focus) = shown(take, cell, look, widest);
                scene.places.push(place);
                if index == 0 {
                    scene.focus = focus;
                }
            }
        }
        Mode::Strips => {
            scene.live = false;
            let count = gallery.takes.len() as u32;
            let aspect = gallery.takes.iter().find_map(|take| take.info.map(aspect_of)).unwrap_or(WIDE);
            let cells = layout::cells(area, count, STRIP_FRAMES, STRIP_GAP, aspect);
            for (row, take) in gallery.takes.iter().enumerate() {
                for column in 0..STRIP_FRAMES {
                    let Some(cell) = cells.get(row * STRIP_FRAMES as usize + column as usize) else {
                        continue;
                    };
                    let index = strip_frame(take.info.map_or(0, |info| info.frames), column);
                    scene.places.push(fitted(take, *cell, index));
                }
            }
        }
    }
    scene
}

/// The frame of a take's strip at `column`, of eight at equal times: the
/// last frame of each eighth of its length, so the eighth is its last.
pub fn strip_frame(frames: u32, column: u32) -> u32 {
    ((column + 1) * frames / STRIP_FRAMES).saturating_sub(1).min(frames.saturating_sub(1))
}

fn aspect_of(info: media::Info) -> f32 {
    info.width.max(1) as f32 / info.height.max(1) as f32
}

/// A take fitted into `cell`, never larger than its own pixels, centred,
/// at `index`.
fn fitted(take: &Take, cell: Rect, index: u32) -> Place {
    let Some(info) = take.info else {
        let (w, h) = layout::fit(cell.w, cell.h, WIDE);
        return Place { key: None, rect: centred(cell, w, h), clip: cell };
    };
    let (w, h) = layout::fit(cell.w, cell.h, aspect_of(info));
    let (w, h) = even((w.min(info.width), h.min(info.height)));
    let index = index.min(take.last_frame());
    Place { key: Some(Key::new(take.file.clone(), index, (w, h))), rect: centred(cell, w, h), clip: cell }
}

/// The shown take in `cell`: fitted, or at the look's zoom about its focus,
/// scaled as if it were `widest` pixels wide so two takes show one region.
fn shown(take: &Take, cell: Rect, look: Look, widest: Option<u32>) -> (Place, Option<[f64; 2]>) {
    let (Some(zoom), Some(info)) = (look.zoom, take.info) else {
        return (fitted(take, cell, look.frame), None);
    };
    let scale = zoom * f64::from(widest.unwrap_or(info.width).max(1)) / f64::from(info.width.max(1));
    let drawn = ((f64::from(info.width) * scale).round().max(2.0) as u32, (f64::from(info.height) * scale).round().max(2.0) as u32);
    // Decoded at the size drawn, and no larger than the file holds.
    let decoded = even((drawn.0.min(info.width), drawn.1.min(info.height)));
    let index = look.frame.min(take.last_frame());
    let along = |start: i32, room: u32, size: u32, focus: f64| -> (i32, f64) {
        if size <= room {
            return (start + ((room - size) / 2) as i32, 0.5);
        }
        let middle = f64::from(start) + f64::from(room) / 2.0;
        let at = (middle - focus.clamp(0.0, 1.0) * f64::from(size)).round() as i32;
        let at = at.clamp(start + room as i32 - size as i32, start);
        (at, (middle - f64::from(at)) / f64::from(size))
    };
    let (x, focus_x) = along(cell.x, cell.w, drawn.0, look.focus[0]);
    let (y, focus_y) = along(cell.y, cell.h, drawn.1, look.focus[1]);
    let rect = Rect { x, y, w: drawn.0, h: drawn.1 };
    (Place { key: Some(Key::new(take.file.clone(), index, decoded)), rect, clip: cell }, Some([focus_x, focus_y]))
}

fn centred(cell: Rect, w: u32, h: u32) -> Rect {
    Rect { x: cell.x + (cell.w.saturating_sub(w) / 2) as i32, y: cell.y + (cell.h.saturating_sub(h) / 2) as i32, w, h }
}

/// Draws the scene with the frames that are ready; a frame not ready, or
/// never to be, leaves its place empty.
pub fn compose(scene: &Scene, frames: &std::collections::HashMap<Key, Rgba>) -> Rgba {
    let mut image = Rgba::new(scene.width, scene.height, GROUND);
    for place in &scene.places {
        match place.key.as_ref().and_then(|key| frames.get(key)) {
            Some(frame) if (frame.width, frame.height) == (place.rect.w, place.rect.h) && inside(place.rect, place.clip) => {
                layout::place(&mut image, frame, place.rect);
            }
            Some(frame) => draw(&mut image, frame, place.rect, place.clip),
            None => fill(&mut image, place.rect, place.clip, EMPTY),
        }
    }
    image
}

fn inside(rect: Rect, clip: Rect) -> bool {
    rect.x >= clip.x && rect.y >= clip.y && rect.x + rect.w as i32 <= clip.x + clip.w as i32 && rect.y + rect.h as i32 <= clip.y + clip.h as i32
}

/// The rows and columns of `rect` within `clip` and the image.
fn span(image: &Rgba, rect: Rect, clip: Rect) -> (std::ops::Range<i32>, std::ops::Range<i32>) {
    let x0 = rect.x.max(clip.x).max(0);
    let y0 = rect.y.max(clip.y).max(0);
    let x1 = (rect.x + rect.w as i32).min(clip.x + clip.w as i32).min(image.width as i32);
    let y1 = (rect.y + rect.h as i32).min(clip.y + clip.h as i32).min(image.height as i32);
    (x0..x1.max(x0), y0..y1.max(y0))
}

fn fill(image: &mut Rgba, rect: Rect, clip: Rect, colour: [u8; 4]) {
    let (xs, ys) = span(image, rect, clip);
    for y in ys {
        for x in xs.clone() {
            image.set(x as u32, y as u32, colour);
        }
    }
}

/// Draws `frame` scaled to `rect`, smoothly, only where `clip` lets it.
fn draw(image: &mut Rgba, frame: &Rgba, rect: Rect, clip: Rect) {
    let (xs, ys) = span(image, rect, clip);
    let sx = f64::from(frame.width) / f64::from(rect.w.max(1));
    let sy = f64::from(frame.height) / f64::from(rect.h.max(1));
    let (last_x, last_y) = (frame.width.saturating_sub(1) as f64, frame.height.saturating_sub(1) as f64);
    for y in ys {
        let v = ((f64::from(y - rect.y) + 0.5) * sy - 0.5).clamp(0.0, last_y);
        let (v0, fv) = (v.floor() as u32, v.fract());
        let v1 = (v0 + 1).min(frame.height - 1);
        for x in xs.clone() {
            let u = ((f64::from(x - rect.x) + 0.5) * sx - 0.5).clamp(0.0, last_x);
            let (u0, fu) = (u.floor() as u32, u.fract());
            let u1 = (u0 + 1).min(frame.width - 1);
            let (a, b, c, d) = (frame.pixel(u0, v0), frame.pixel(u1, v0), frame.pixel(u0, v1), frame.pixel(u1, v1));
            let mut out = [0u8; 4];
            for channel in 0..4 {
                let top = f64::from(a[channel]) * (1.0 - fu) + f64::from(b[channel]) * fu;
                let bottom = f64::from(c[channel]) * (1.0 - fu) + f64::from(d[channel]) * fu;
                out[channel] = (top * (1.0 - fv) + bottom * fv).round() as u8;
            }
            image.set(x as u32, y as u32, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layers::link::Link;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn info(width: u32, height: u32, frames: u32) -> media::Info {
        media::Info { width, height, rate: (24, 1), frames, sound: false }
    }

    fn gallery(takes: &[(&str, media::Info)]) -> Gallery {
        let takes = takes
            .iter()
            .map(|(name, info)| {
                let link = Link { path: format!("{name}.mp4"), sha256: String::new() };
                (name.to_string(), link, PathBuf::from(format!("/scratch/{name}.mp4")), Some(*info))
            })
            .collect();
        Gallery::new(takes, 0)
    }

    const FIT: Look = Look { frame: 10, zoom: None, focus: [0.5, 0.5] };

    /// Catches a take decoded larger than it is drawn, or larger than its
    /// file: a 4K take in a 1200 × 800 area is decoded at the area's size,
    /// and a small take at its own.
    #[test]
    fn a_take_is_decoded_at_the_size_it_is_drawn() {
        let big = scene(&gallery(&[("big", info(3840, 2160, 48))]), (1200, 800), FIT);
        let key = big.places[0].key.clone().unwrap();
        assert_eq!((key.width, key.height, key.index), (1120, 630, 10));
        assert_eq!((big.places[0].rect.w, big.places[0].rect.h), (1120, 630));
        let small = scene(&gallery(&[("small", info(128, 72, 4))]), (1200, 800), FIT);
        let key = small.places[0].key.clone().unwrap();
        assert_eq!((key.width, key.height, key.index), (128, 72, 3), "a take was enlarged, or its frame not kept within it");
        assert_eq!(small.places[0].rect, Rect { x: 536, y: 364, w: 128, h: 72 });
    }

    /// Catches strips out of order or at the wrong times: each take is a
    /// row, in filmstrip order, of eight frames ending at its last.
    #[test]
    fn strips_are_rows_of_eight_frames_in_order() {
        let mut gallery = gallery(&[("a", info(128, 72, 48)), ("b", info(128, 72, 24))]);
        gallery.set_view(Mode::Strips, 0).unwrap();
        let strips = scene(&gallery, (1200, 800), FIT);
        assert!(!strips.live);
        assert_eq!(strips.places.len(), 16);
        let indices: Vec<u32> = strips.places.iter().map(|place| place.key.as_ref().unwrap().index).collect();
        assert_eq!(&indices[..8], [5, 11, 17, 23, 29, 35, 41, 47]);
        assert_eq!(&indices[8..], [2, 5, 8, 11, 14, 17, 20, 23]);
        assert!(strips.places[8].rect.y > strips.places[7].rect.y);
        assert!(strips.places[0].key.as_ref().unwrap().file.ends_with("a.mp4"));
    }

    /// Catches side by side drawing different regions at 100%: a 720p and
    /// a 4K take show the same part, the 4K one at its own pixels.
    #[test]
    fn side_by_side_at_100_percent_shows_one_region() {
        let mut gallery = gallery(&[("low", info(1280, 720, 24)), ("high", info(3840, 2160, 24))]);
        let other = gallery.uid_at(1).unwrap();
        gallery.set_view(Mode::Side { other }, 0).unwrap();
        let look = Look { frame: 3, zoom: Some(1.0), focus: [0.25, 0.75] };
        let side = scene(&gallery, (1200, 800), look);
        let (low, high) = (&side.places[0], &side.places[1]);
        assert_eq!((high.rect.w, high.rect.h), (3840, 2160));
        assert_eq!((low.rect.w, low.rect.h), (3840, 2160), "the smaller take is not drawn at the larger one's scale");
        let low_key = low.key.as_ref().unwrap();
        assert_eq!((low_key.width, low_key.height), (1280, 720), "a take was decoded larger than its file");
        assert_eq!(low.rect.x - low.clip.x, high.rect.x - high.clip.x);
        assert_eq!(low.rect.y - low.clip.y, high.rect.y - high.clip.y);
        assert!(side.focus.is_some());
    }

    /// Catches a frame drawn outside its place, or a missing one drawn as
    /// another: a zoomed frame stays within its cell, scaled smoothly.
    #[test]
    fn a_zoomed_frame_stays_in_its_cell() {
        let mut frames = HashMap::new();
        let key = Key::new(PathBuf::from("/scratch/a.mp4"), 0, (4, 4));
        let mut frame = Rgba::new(4, 4, [0, 0, 0, 255]);
        frame.set(3, 3, [255, 0, 0, 255]);
        frames.insert(key.clone(), frame);
        let clip = Rect { x: 10, y: 10, w: 20, h: 20 };
        let scene = Scene {
            width: 50,
            height: 50,
            places: vec![Place { key: Some(key), rect: Rect { x: 0, y: 0, w: 40, h: 40 }, clip }],
            live: true,
            focus: None,
        };
        let image = compose(&scene, &frames);
        assert_eq!(image.pixel(5, 5), GROUND);
        assert_eq!(image.pixel(35, 35), GROUND);
        assert_eq!(image.pixel(10, 10), [0, 0, 0, 255]);
        assert!(image.pixel(29, 29)[0] > 0, "the frame was not scaled smoothly");
    }
}
