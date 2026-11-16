//! Notes on an image, as the notes tool makes them: numbered pins, each
//! with its words on a card while it is open, and freehand marks. They are
//! drawn over the image the host shows, and onto the marked image a next
//! round sends, never into the image's layers, its file's pixels, or an
//! export. The pins' discs and the marks are drawn by `layers::notes`, the
//! numbers and words with this application's own fonts.

use crate::layered::{Pixels, Rect};
use crate::text::{self, Style, Text};
use layers::Rgba;
use layers::notes::{Mark, Notes, Pin};

/// The words' colour on a card, the card's, and its edge's.
const INK: [u8; 3] = [28, 28, 30];
const CARD: [u8; 4] = [255, 255, 255, 240];
const EDGE: [u8; 4] = [200, 200, 204, 255];
/// A freehand mark's width, in screen pixels: at 100% it is this many
/// image pixels.
pub const MARK_WIDTH: f32 = 4.0;

/// A pin's radius, in image pixels: larger on a larger image, so a pin
/// reads at any size, and the same wherever it is drawn.
pub fn pin_radius(width: u32, height: u32) -> f32 {
    (width.max(height) as f32 / 120.0).clamp(8.0, 48.0)
}

/// The size of a card's words.
pub fn word_size(width: u32, height: u32) -> f32 {
    pin_radius(width, height) * 1.25
}

/// A pin's card: its words as a text, the corner they sit at, and the card
/// around them, `[x, y, width, height]`, all in image pixels. It sits to
/// the pin's right, or its left when the right has no room.
pub struct Card {
    pub text: Text,
    pub corner: [f32; 2],
    pub rect: [f32; 4],
}

pub fn card(pin: &Pin, width: u32, height: u32) -> Card {
    let radius = pin_radius(width, height);
    let size = word_size(width, height);
    let text = Text { words: pin.text.clone(), style: Style { size, color: INK, ..Style::default() }, width: None };
    let layout = text::lay(&text);
    let pad = (size * 0.5).round();
    let w = layout.width.max(size * 6.0) + 2.0 * pad;
    let h = layout.height.max(layout.advance) + 2.0 * pad;
    let mut left = (pin.x + radius * 1.6).round();
    if left + w > width as f32 && pin.x - radius * 1.6 - w >= 0.0 {
        left = (pin.x - radius * 1.6 - w).round();
    }
    let top = (pin.y - h / 2.0).clamp(0.0, (height as f32 - h).max(0.0)).round();
    Card { text, corner: [left + pad, top + pad], rect: [left, top, w, h] }
}

/// Draws the notes onto `image`: the marks, then each pin's disc and
/// number in turn, then the card of the pin `open`, with its words.
pub fn draw(image: &mut Rgba, notes: &Notes, open: Option<u32>) {
    for mark in &notes.marks {
        draw_mark(image, mark);
    }
    let radius = pin_radius(image.width, image.height);
    for pin in &notes.pins {
        layers::notes::draw_pins(image, std::slice::from_ref(pin), radius);
        number(image, pin, radius);
    }
    if let Some(pin) = open.and_then(|open| notes.pins.iter().find(|pin| pin.number == open)) {
        let card = card(pin, image.width, image.height);
        let [x, y, w, h] = card.rect;
        fill(image, [x, y, w, h], EDGE);
        fill(image, [x + 1.0, y + 1.0, w - 2.0, h - 2.0], CARD);
        paint(image, &text::draw(&card.text), card.corner);
    }
}

/// The image with its notes on it, as a next round sends it: every mark,
/// and every pin with its number, but no card.
pub fn marked(composite: &[u8], width: u32, height: u32, notes: &Notes) -> Rgba {
    let mut image = Rgba { width, height, data: composite.to_vec() };
    draw(&mut image, notes, None);
    image
}

/// One mark, drawn as `layers::notes::draw_marks` draws it, over only the
/// part of the image it reaches, so a mark costs its own size.
pub fn draw_mark(image: &mut Rgba, mark: &Mark) {
    if mark.points.is_empty() {
        return;
    }
    let reach = mark.width / 2.0 + 2.0;
    let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
    for point in &mark.points {
        for axis in 0..2 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let x0 = (low[0] - reach).floor().max(0.0) as u32;
    let y0 = (low[1] - reach).floor().max(0.0) as u32;
    let x1 = ((high[0] + reach).ceil().max(0.0) as u32).min(image.width);
    let y1 = ((high[1] + reach).ceil().max(0.0) as u32).min(image.height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut part = Rgba { width: w, height: h, data: Vec::with_capacity((w * h * 4) as usize) };
    for y in y0..y1 {
        let start = ((y * image.width + x0) * 4) as usize;
        part.data.extend_from_slice(&image.data[start..start + (w * 4) as usize]);
    }
    let moved = Mark { points: mark.points.iter().map(|point| [point[0] - x0 as f32, point[1] - y0 as f32]).collect(), ..mark.clone() };
    layers::notes::draw_marks(&mut part, &[moved]);
    for (row, y) in (y0..y1).enumerate() {
        let start = ((y * image.width + x0) * 4) as usize;
        image.data[start..start + (w * 4) as usize].copy_from_slice(&part.data[row * (w * 4) as usize..(row + 1) * (w * 4) as usize]);
    }
}

/// A mark being drawn, over `rows`, the part `rect` of an image.
pub fn draw_mark_over(mark: &Mark, rect: Rect, rows: &mut Vec<u8>) {
    let mut part = Rgba { width: rect.w as u32, height: rect.h as u32, data: std::mem::take(rows) };
    let moved = Mark { points: mark.points.iter().map(|point| [point[0] - rect.x as f32, point[1] - rect.y as f32]).collect(), ..mark.clone() };
    draw_mark(&mut part, &moved);
    *rows = part.data;
}

/// Notes drawn on a clear image, as `draw` leaves them, laid over `rows`,
/// the part `rect` of an image: each colour, already scaled by its
/// coverage, over what is beneath for the rest.
pub fn lay_over(layer: &Rgba, rect: Rect, rows: &mut [u8]) {
    let width = layer.width as usize;
    for y in 0..rect.h.max(0) as usize {
        for x in 0..rect.w.max(0) as usize {
            let from = ((rect.y as usize + y) * width + rect.x as usize + x) * 4;
            let over = &layer.data[from..from + 4];
            if over[3] == 0 {
                continue;
            }
            let at = (y * rect.w as usize + x) * 4;
            let keep = 1.0 - f32::from(over[3]) / 255.0;
            for channel in 0..3 {
                rows[at + channel] = (f32::from(rows[at + channel]) * keep + f32::from(over[channel])).round().min(255.0) as u8;
            }
            rows[at + 3] = rows[at + 3].max(over[3]);
        }
    }
}

/// A pin's number, white and bold, in the middle of its disc.
fn number(image: &mut Rgba, pin: &Pin, radius: f32) {
    let text = Text {
        words: pin.number.to_string(),
        style: Style { size: radius * 1.15, bold: true, color: [255, 255, 255], ..Style::default() },
        width: None,
    };
    let layout = text::lay(&text);
    paint(image, &text::draw(&text), [pin.x - layout.width / 2.0, pin.y - layout.height / 2.0]);
}

/// Straight RGBA pixels drawn onto `image` with their corner at `corner`,
/// each by its alpha. The round views draw their words with it too.
pub fn paint(image: &mut Rgba, pixels: &Pixels, corner: [f32; 2]) {
    let (cx, cy) = (corner[0].round() as i64, corner[1].round() as i64);
    for y in 0..pixels.height as i64 {
        let ty = cy + y;
        if ty < 0 || ty >= i64::from(image.height) {
            continue;
        }
        for x in 0..pixels.width as i64 {
            let tx = cx + x;
            if tx < 0 || tx >= i64::from(image.width) {
                continue;
            }
            let at = ((y * i64::from(pixels.width) + x) * 4) as usize;
            let alpha = pixels.data[at + 3];
            if alpha > 0 {
                let colour = [pixels.data[at], pixels.data[at + 1], pixels.data[at + 2], 255];
                cover(image, tx as u32, ty as u32, f32::from(alpha) / 255.0, colour);
            }
        }
    }
}

/// A rectangle, `[x, y, width, height]`, filled with `colour`.
pub fn fill(image: &mut Rgba, rect: [f32; 4], colour: [u8; 4]) {
    let x0 = rect[0].round().max(0.0) as u32;
    let y0 = rect[1].round().max(0.0) as u32;
    let x1 = ((rect[0] + rect[2]).round().max(0.0) as u32).min(image.width);
    let y1 = ((rect[1] + rect[3]).round().max(0.0) as u32).min(image.height);
    for y in y0..y1 {
        for x in x0..x1 {
            cover(image, x, y, 1.0, colour);
        }
    }
}

/// One pixel moved `coverage` of the way to `colour`, as `layers::notes`
/// covers its discs and marks.
fn cover(image: &mut Rgba, x: u32, y: u32, coverage: f32, colour: [u8; 4]) {
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
    use layers::notes::RED;

    fn notes() -> Notes {
        Notes {
            pins: vec![Pin { number: 1, x: 100.0, y: 60.0, text: "less rain on the glass".into() }],
            marks: vec![Mark { points: vec![[20.0, 150.0], [180.0, 150.0]], width: 4.0, colour: RED }],
        }
    }

    /// Catches a marked image without its notes, or with its number or card
    /// missing: the mark is red where it was drawn, the pin's disc is red
    /// beside its white number, and no card is drawn, while the same notes
    /// with the pin open draw its card.
    #[test]
    fn the_marked_image_has_the_notes() {
        let (width, height) = (240, 200);
        let white = vec![255u8; (width * height * 4) as usize];
        let marked = marked(&white, width, height, &notes());
        assert_eq!(marked.pixel(100, 150), RED, "the mark is not drawn");
        assert_eq!(marked.pixel(10, 10), [255, 255, 255, 255]);
        let radius = pin_radius(width, height);
        let disc: Vec<[u8; 4]> = (0..radius as u32 - 1).map(|dx| marked.pixel(100 + dx, 60)).collect();
        assert!(disc.contains(&RED), "the pin's disc is not drawn");
        assert!(disc.iter().any(|pixel| pixel[1] > 200), "the pin's number is not drawn");
        let card = card(&notes().pins[0], width, height);
        let inside = [(card.rect[0] + 2.0) as u32, (card.rect[1] + 2.0) as u32];
        assert_eq!(marked.pixel(inside[0], inside[1]), [255, 255, 255, 255]);
        let mut open = Rgba::new(width, height, [0, 0, 0, 255]);
        draw(&mut open, &notes(), Some(1));
        assert!(open.pixel(inside[0], inside[1])[0] > 240, "the open pin's card is not drawn");
    }

    /// Catches notes laid over the view differently from notes drawn onto
    /// it: drawn on a clear image and laid over grey, they come within a
    /// step of being drawn onto grey.
    #[test]
    fn notes_laid_over_are_notes_drawn_on() {
        let (width, height) = (240, 200);
        let grey = vec![128u8; (width * height * 4) as usize];
        let direct = marked(&grey, width, height, &notes());
        let mut layer = Rgba::new(width, height, [0, 0, 0, 0]);
        draw(&mut layer, &notes(), None);
        let mut rows = grey.clone();
        lay_over(&layer, Rect::new(0, 0, width as i32, height as i32), &mut rows);
        for (got, want) in rows.iter().zip(&direct.data) {
            assert!((i32::from(*got) - i32::from(*want)).abs() <= 2, "{got} against {want}");
        }
    }
}
