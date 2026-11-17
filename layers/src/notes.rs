//! Notes marked on an image: numbered pins, each with its words, and
//! freehand marks, kept apart from the image's own layers. The marked image,
//! the take with its notes on it, is what a next round sends, with the
//! pins' words in its prompt. On a video, a note also belongs to a moment,
//! its `at`, and shows only near it.

use crate::Rgba;
use serde::{Deserialize, Serialize};

/// The notes' red.
pub const RED: [u8; 4] = [226, 36, 36, 255];

/// A numbered pin at a point of the image, in its pixels, and, on a video,
/// at a time in seconds. Kept in a file as the Image Generator's `.ipx`
/// keeps a pin, with `at` only when it has one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub number: u32,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
}

/// A freehand mark: the points it was drawn through, its width in image
/// pixels, its colour, and, on a video, the time in seconds it was drawn
/// at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
    pub colour: [u8; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
}

impl Pin {
    /// A pin at a time as vg-notes-at-time names it, on its cue and its
    /// card: its number and its time to a tenth, "1 — 3.4 s". None without
    /// a time.
    pub fn moment(&self) -> Option<String> {
        self.at.map(|at| format!("{} — {at:.1} s", self.number))
    }
}

/// One image's notes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub pins: Vec<Pin>,
    #[serde(default)]
    pub marks: Vec<Mark>,
}

/// How near a note's time the time shown must be for it to show over a
/// video's frame, in seconds: a note belongs to a moment as well as a
/// place.
pub const NEAR: f64 = 0.5;

/// A pin's radius on an image of this size, in its pixels: larger on a
/// larger image, so a pin reads at any size, and the same wherever it is
/// drawn.
pub fn pin_radius(width: u32, height: u32) -> f32 {
    (width.max(height) as f32 / 120.0).clamp(8.0, 48.0)
}

/// Whether a note at `at` shows at `time`: always when it has no time,
/// as on an image.
pub fn shows_at(at: Option<f64>, time: f64) -> bool {
    at.is_none_or(|at| (at - time).abs() <= NEAR)
}

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty() && self.marks.is_empty()
    }

    /// The number the next pin takes: one past the highest, so a removed
    /// pin's number is not given again while later ones remain.
    pub fn next_number(&self) -> u32 {
        self.pins.iter().map(|pin| pin.number).max().unwrap_or(0) + 1
    }

    /// The pins' words as a next round's prompt carries them, in number
    /// order, each "<n>. <words>", leaving out pins with none.
    pub fn as_words(&self) -> Vec<String> {
        let mut pins: Vec<&Pin> = self.pins.iter().filter(|pin| !pin.text.trim().is_empty()).collect();
        pins.sort_by_key(|pin| pin.number);
        pins.iter().map(|pin| format!("{}. {}", pin.number, pin.text.trim())).collect()
    }

    /// The pins' words as a video's next round carries them: as
    /// `as_words`, each pin with a time ending "(3.4 s)".
    pub fn as_timed_words(&self) -> Vec<String> {
        let mut pins: Vec<&Pin> = self.pins.iter().filter(|pin| !pin.text.trim().is_empty()).collect();
        pins.sort_by_key(|pin| pin.number);
        pins.iter()
            .map(|pin| match pin.at {
                Some(at) => format!("{}. {} ({at:.1} s)", pin.number, pin.text.trim()),
                None => format!("{}. {}", pin.number, pin.text.trim()),
            })
            .collect()
    }

    /// The notes that show at `time`, as `shows_at` has it.
    pub fn at_time(&self, time: f64) -> Notes {
        Notes {
            pins: self.pins.iter().filter(|pin| shows_at(pin.at, time)).cloned().collect(),
            marks: self.marks.iter().filter(|mark| shows_at(mark.at, time)).cloned().collect(),
        }
    }

    /// The time of the first note, by number, with a time; or the first
    /// mark's.
    pub fn first_time(&self) -> Option<f64> {
        let mut pins: Vec<&Pin> = self.pins.iter().filter(|pin| pin.at.is_some()).collect();
        pins.sort_by_key(|pin| pin.number);
        pins.first().and_then(|pin| pin.at).or_else(|| self.marks.iter().find_map(|mark| mark.at))
    }

    /// The pin under a point, within `radius`, the topmost first.
    pub fn pin_at(&self, x: f32, y: f32, radius: f32) -> Option<u32> {
        self.pins.iter().rev().find(|pin| (pin.x - x).powi(2) + (pin.y - y).powi(2) <= radius * radius).map(|pin| pin.number)
    }
}

/// Draws the marks onto `image`, each as a round-ended line through its
/// points. A mark's coverage is gathered over all its segments first, so
/// where they meet is drawn once, not darker.
pub fn draw_marks(image: &mut Rgba, marks: &[Mark]) {
    let (w, h) = (image.width as usize, image.height as usize);
    for mark in marks {
        let radius = (mark.width / 2.0).max(0.5);
        let mut coverage = vec![0f32; w * h];
        let points: Vec<[f32; 2]> = if mark.points.len() == 1 { vec![mark.points[0], mark.points[0]] } else { mark.points.clone() };
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (x0, x1) = ((a[0].min(b[0]) - radius - 1.0).floor().max(0.0) as usize, ((a[0].max(b[0]) + radius + 1.0).ceil().max(0.0) as usize).min(w));
            let (y0, y1) = ((a[1].min(b[1]) - radius - 1.0).floor().max(0.0) as usize, ((a[1].max(b[1]) + radius + 1.0).ceil().max(0.0) as usize).min(h));
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = dx * dx + dy * dy;
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let t = if length > 0.0 { (((px - a[0]) * dx + (py - a[1]) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
                    let d = ((px - a[0] - t * dx).powi(2) + (py - a[1] - t * dy).powi(2)).sqrt();
                    let c = &mut coverage[y * w + x];
                    *c = c.max((radius + 0.5 - d).clamp(0.0, 1.0));
                }
            }
        }
        for (i, c) in coverage.iter().enumerate() {
            if *c > 0.0 {
                cover(image, (i % w) as u32, (i / w) as u32, *c, mark.colour);
            }
        }
    }
}

/// A pin's number, white and bold, in the middle of its disc, drawn by
/// `crate::label` as the views' labels are.
pub fn draw_number(image: &mut Rgba, pin: &Pin, radius: f32) {
    let size = radius * 1.15;
    let look = crate::label::Look { size, colour: [255, 255, 255, 255], bold: true };
    crate::label::draw(image, &pin.number.to_string(), [pin.x, pin.y - crate::label::height(size) / 2.0], look, crate::label::Anchor::Centre);
}

/// Draws each pin as a filled disc of `radius` with a white ring, leaving
/// its number to the workspace, which draws text with its own fonts.
pub fn draw_pins(image: &mut Rgba, pins: &[Pin], radius: f32) {
    for pin in pins {
        disc(image, pin.x, pin.y, radius + radius * 0.15, [255, 255, 255, 255]);
        disc(image, pin.x, pin.y, radius, RED);
    }
}

/// A filled disc, its edge smoothed over one pixel.
pub fn disc(image: &mut Rgba, cx: f32, cy: f32, radius: f32, colour: [u8; 4]) {
    let (x0, x1) = ((cx - radius - 1.0).floor().max(0.0) as u32, ((cx + radius + 1.0).ceil() as u32).min(image.width));
    let (y0, y1) = ((cy - radius - 1.0).floor().max(0.0) as u32, ((cy + radius + 1.0).ceil() as u32).min(image.height));
    for y in y0..y1 {
        for x in x0..x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            cover(image, x, y, (radius + 0.5 - d).clamp(0.0, 1.0), colour);
        }
    }
}

fn cover(image: &mut Rgba, x: u32, y: u32, coverage: f32, colour: [u8; 4]) {
    if coverage <= 0.0 {
        return;
    }
    let under = image.pixel(x, y);
    let alpha = coverage * f32::from(colour[3]) / 255.0;
    let mut out = [0; 4];
    for channel in 0..3 {
        out[channel] = (f32::from(under[channel]) + alpha * (f32::from(colour[channel]) - f32::from(under[channel]))).round() as u8;
    }
    out[3] = under[3].max((alpha * 255.0).round() as u8);
    image.set(x, y, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a pin's moment written other than as vg-notes-at-time shows
    /// it, "1 — 3.4 s", or one given to a pin with no time.
    #[test]
    fn a_pin_at_a_time_reads_as_its_moment() {
        let pin = |at: Option<f64>| Pin { number: 1, x: 0.0, y: 0.0, text: String::new(), at };
        assert_eq!(pin(Some(3.42)).moment().as_deref(), Some("1 — 3.4 s"));
        assert_eq!(pin(None).moment(), None);
    }

    /// Catches pins numbered again after one is removed, or words sent out
    /// of order or with empty notes: the next number is past the highest,
    /// and the words come in number order.
    #[test]
    fn pins_keep_their_numbers_and_order() {
        let mut notes = Notes::default();
        for (number, text) in [(3, "keep the lamp as it is"), (1, "the dial glows brighter"), (2, "  ")] {
            notes.pins.push(Pin { number, x: 0.0, y: 0.0, text: text.into(), at: None });
        }
        assert_eq!(notes.next_number(), 4);
        assert_eq!(notes.as_words(), vec!["1. the dial glows brighter", "3. keep the lamp as it is"]);
    }

    /// Catches marks drawn in the wrong place or not at all: a red line
    /// across the middle colours the middle row and leaves the corners.
    #[test]
    fn a_mark_is_drawn_where_it_was_made() {
        let mut image = Rgba::new(20, 10, [255, 255, 255, 255]);
        draw_marks(&mut image, &[Mark { points: vec![[2.0, 5.0], [18.0, 5.0]], width: 3.0, colour: RED, at: None }]);
        assert_eq!(image.pixel(10, 5), RED);
        assert_eq!(image.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(image.pixel(19, 9), [255, 255, 255, 255]);
    }

    /// Catches the Image Generator's `.ipx` notes changed by a note's time:
    /// a pin and a mark with none are written exactly as its file writes
    /// them, and its file's notes read back with none.
    #[test]
    fn notes_without_a_time_keep_the_image_generators_shape() {
        let pin = Pin { number: 2, x: 4.5, y: 3.0, text: "less rain".into(), at: None };
        assert_eq!(serde_json::to_string(&pin).unwrap(), r#"{"number":2,"x":4.5,"y":3.0,"text":"less rain"}"#);
        let mark = Mark { points: vec![[1.0, 1.0], [9.0, 2.0]], width: 4.0, colour: RED, at: None };
        assert_eq!(serde_json::to_string(&mark).unwrap(), r#"{"points":[[1.0,1.0],[9.0,2.0]],"width":4.0,"colour":[226,36,36,255]}"#);
        let stored = r#"{"pins":[{"number":2,"x":4.5,"y":3.0,"text":"less rain"}],"marks":[{"points":[[1.0,1.0],[9.0,2.0]],"width":4.0,"colour":[226,36,36,255]}],"hidden":false}"#;
        let read: Notes = serde_json::from_str(stored).unwrap();
        assert_eq!(read, Notes { pins: vec![pin], marks: vec![mark] });
    }

    /// Catches a video's note that shows away from its moment, or whose
    /// words lose their time: a note at 3.4 s shows within half a second
    /// of it, and its words end "(3.4 s)".
    #[test]
    fn a_note_at_a_time_shows_near_it() {
        let notes = Notes {
            pins: vec![
                Pin { number: 1, x: 0.0, y: 0.0, text: "the door".into(), at: Some(3.4) },
                Pin { number: 2, x: 0.0, y: 0.0, text: "the lamp".into(), at: None },
            ],
            marks: vec![Mark { points: vec![[0.0, 0.0]], width: 4.0, colour: RED, at: Some(1.0) }],
        };
        assert!(shows_at(Some(3.4), 3.0) && !shows_at(Some(3.4), 2.8) && shows_at(None, 9.0));
        let near = notes.at_time(3.6);
        assert_eq!((near.pins.len(), near.marks.len()), (2, 0));
        assert_eq!(notes.as_timed_words(), ["1. the door (3.4 s)", "2. the lamp"]);
        assert_eq!(notes.first_time(), Some(3.4));
    }
}
