//! Notes marked on an image: numbered pins, each with its words, and
//! freehand marks, kept apart from the image's own layers. The marked image,
//! the take with its notes on it, is what a next round sends, with the
//! pins' words in its prompt.

use crate::Rgba;

/// The notes' red.
pub const RED: [u8; 4] = [226, 36, 36, 255];

/// A numbered pin at a point of the image, in its pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub number: u32,
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// A freehand mark: the points it was drawn through, its width in image
/// pixels, and its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
    pub colour: [u8; 4],
}

/// One image's notes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notes {
    pub pins: Vec<Pin>,
    pub marks: Vec<Mark>,
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

    /// Catches pins numbered again after one is removed, or words sent out
    /// of order or with empty notes: the next number is past the highest,
    /// and the words come in number order.
    #[test]
    fn pins_keep_their_numbers_and_order() {
        let mut notes = Notes::default();
        for (number, text) in [(3, "keep the lamp as it is"), (1, "the dial glows brighter"), (2, "  ")] {
            notes.pins.push(Pin { number, x: 0.0, y: 0.0, text: text.into() });
        }
        assert_eq!(notes.next_number(), 4);
        assert_eq!(notes.as_words(), vec!["1. the dial glows brighter", "3. keep the lamp as it is"]);
    }

    /// Catches marks drawn in the wrong place or not at all: a red line
    /// across the middle colours the middle row and leaves the corners.
    #[test]
    fn a_mark_is_drawn_where_it_was_made() {
        let mut image = Rgba::new(20, 10, [255, 255, 255, 255]);
        draw_marks(&mut image, &[Mark { points: vec![[2.0, 5.0], [18.0, 5.0]], width: 3.0, colour: RED }]);
        assert_eq!(image.pixel(10, 5), RED);
        assert_eq!(image.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(image.pixel(19, 9), [255, 255, 255, 255]);
    }
}
