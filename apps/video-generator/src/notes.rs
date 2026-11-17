//! Notes on a take, as the notes tool makes them: numbered pins, each with
//! its words on a card while it is open, and freehand marks, as the Image
//! Generator's, each also at a time. They are drawn over the frame the
//! view shows, and onto the marked frame a next round sends, never into a
//! take's file. The pins' discs, their numbers, and the marks are drawn by
//! `layers::notes`; a card's words by `layers::label`, as the view's
//! captions are.

use layers::Rgba;
use layers::label::{self, Anchor, Look};
use layers::layout::Rect;
use layers::notes::{self, Mark, Notes, Pin};
use protocol::TextEditing;

/// The words' colour on a card, the card's, and its edge's, as the Image
/// Generator's.
const INK: [u8; 4] = [28, 28, 30, 255];
const CARD: [u8; 4] = [255, 255, 255, 240];
const EDGE: [u8; 4] = [200, 200, 204, 255];
/// A freehand mark's width, in the view's pixels: drawn at a zoom, it is
/// kept in the take's pixels at this width on the screen.
pub const MARK_WIDTH: f32 = 4.0;
/// The size of a card's words, and the widest they run before they wrap,
/// in the view's pixels, as ig-notes-pin shows a card: about 320.
const WORDS: f32 = 14.0;
const CARD_WORDS: f32 = 320.0;

/// The notes drawn over one take's frame: those that show at the time
/// shown, in the take's pixels; the take's size; where its whole frame is
/// drawn in the view, and the part it may cover; and the pin whose words
/// are being typed, whose card is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct NotesOver {
    pub notes: Notes,
    pub size: (u32, u32),
    pub rect: Rect,
    pub clip: Rect,
    pub open: Option<u32>,
}

impl NotesOver {
    /// The view's pixels to one of the take's.
    pub fn scale(&self) -> f32 {
        self.rect.w as f32 / self.size.0.max(1) as f32
    }

    /// A point of the take, in the view.
    pub fn to_view(&self, x: f32, y: f32) -> [f32; 2] {
        let scale = self.scale();
        [self.rect.x as f32 + x * scale, self.rect.y as f32 + y * scale]
    }

    /// A point of the view, in the take's pixels; none outside the part
    /// of the frame drawn.
    pub fn to_take(&self, at: [f64; 2]) -> Option<[f32; 2]> {
        let (x, y) = (at[0] as f32, at[1] as f32);
        let inside = |start: i32, room: u32, value: f32| value >= start as f32 && value <= (start + room as i32) as f32;
        if !at.iter().all(|value| value.is_finite())
            || !inside(self.clip.x, self.clip.w, x)
            || !inside(self.clip.y, self.clip.h, y)
            || !inside(self.rect.x, self.rect.w, x)
            || !inside(self.rect.y, self.rect.h, y)
        {
            return None;
        }
        let scale = self.scale().max(f32::EPSILON);
        Some([(x - self.rect.x as f32) / scale, (y - self.rect.y as f32) / scale])
    }

    /// A pin's radius on the take, in its pixels.
    pub fn radius(&self) -> f32 {
        notes::pin_radius(self.size.0, self.size.1)
    }

    /// A pin as the view draws it: at its place in the view.
    fn pin_in_view(&self, pin: &Pin) -> Pin {
        let [x, y] = self.to_view(pin.x, pin.y);
        Pin { x, y, ..pin.clone() }
    }

    /// The open pin's card, in the view's pixels.
    /// Its words start with the pin's moment, "1 — 3.4 s · ", when it has
    /// a time, as vg-notes-at-time shows; the caret stays after them.
    pub fn card(&self) -> Option<Card> {
        let pin = self.notes.pins.iter().find(|pin| Some(pin.number) == self.open)?;
        let mut shown = self.pin_in_view(pin);
        if let Some(moment) = pin.moment() {
            shown.text = format!("{moment} · {}", pin.text);
        }
        Some(card(&shown, self.radius() * self.scale(), self.clip))
    }

    /// The open pin's words, as the host draws them being typed: the
    /// words' box, and the caret after the last.
    pub fn editing(&self) -> Option<TextEditing> {
        let card = self.card()?;
        let last = card.lines.last().map_or(0.0, |line| label::measure(line, WORDS, false));
        let lines = card.lines.len().max(1) as f32;
        let [x, y] = card.corner;
        Some(TextEditing {
            rect: [f64::from(x), f64::from(y), f64::from(card.rect[2] - 2.0 * (x - card.rect[0])), f64::from(card.advance * lines)],
            caret: [f64::from(x + last), f64::from(y + card.advance * (lines - 1.0)), 1.0, f64::from(card.advance)],
            selection: Vec::new(),
        })
    }
}

/// A pin's card: its words as lines, the corner they start at, the card
/// around them, `[x, y, width, height]`, and a line's height, all in the
/// view's pixels. It grows with its words up to `CARD_WORDS` wide, or the
/// frame's, and then wraps them. It sits to the pin's right, or its left
/// when the right has no room, and inside the frame when neither has.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub lines: Vec<String>,
    pub corner: [f32; 2],
    pub rect: [f32; 4],
    pub advance: f32,
}

fn card(pin: &Pin, radius: f32, room: Rect) -> Card {
    let pad = (WORDS * 0.5).round();
    let advance = (label::height(WORDS) * 1.2).round();
    let most = CARD_WORDS.min(room.w as f32 - 2.0 * pad).max(WORDS * 6.0);
    let lines = wrap(&pin.text, most);
    let widest = lines.iter().map(|line| label::measure(line, WORDS, false)).fold(0.0, f32::max);
    let w = widest.max(WORDS * 6.0) + 2.0 * pad;
    let h = advance * lines.len().max(1) as f32 + 2.0 * pad;
    let (low, high) = (room.x as f32, (room.x + room.w as i32) as f32);
    let mut left = (pin.x + radius * 1.6).round();
    if left + w > high {
        left = if pin.x - radius * 1.6 - w >= low { (pin.x - radius * 1.6 - w).round() } else { (high - w).max(low).round() };
    }
    let (top, bottom) = (room.y as f32, (room.y + room.h as i32) as f32);
    let y = (pin.y - h / 2.0).clamp(top, (bottom - h).max(top)).round();
    Card { lines, corner: [left + pad, y + pad], rect: [left, y, w, h], advance }
}

/// Words broken into lines no wider than `most`, at spaces, each line
/// keeping its spaces, so the lines together are the words; a word wider
/// than a line has it to itself.
fn wrap(words: &str, most: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in words.split('\n') {
        let mut line = String::new();
        for piece in paragraph.split_inclusive(' ') {
            let longer = format!("{line}{}", piece.trim_end());
            if !line.is_empty() && label::measure(&longer, WORDS, false) > most {
                lines.push(std::mem::take(&mut line));
            }
            line.push_str(piece);
        }
        lines.push(line);
    }
    lines
}

/// Draws the notes over the view's image, within the frame's part: the
/// marks, then each pin's disc and number, then the open pin's card.
pub fn draw(image: &mut Rgba, over: &NotesOver) {
    let clip = over.clip;
    let x0 = clip.x.max(0) as u32;
    let y0 = clip.y.max(0) as u32;
    let x1 = ((clip.x + clip.w as i32).max(0) as u32).min(image.width);
    let y1 = ((clip.y + clip.h as i32).max(0) as u32).min(image.height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    // Drawn on the part alone, so nothing reaches past it.
    let (w, h) = (x1 - x0, y1 - y0);
    let mut part = Rgba { width: w, height: h, data: Vec::with_capacity((w * h * 4) as usize) };
    for y in y0..y1 {
        let start = ((y * image.width + x0) * 4) as usize;
        part.data.extend_from_slice(&image.data[start..start + (w * 4) as usize]);
    }
    let (dx, dy) = (x0 as f32, y0 as f32);
    let scale = over.scale();
    let marks: Vec<Mark> = over
        .notes
        .marks
        .iter()
        .map(|mark| Mark {
            points: mark.points.iter().map(|point| { let [x, y] = over.to_view(point[0], point[1]); [x - dx, y - dy] }).collect(),
            width: mark.width * scale,
            ..mark.clone()
        })
        .collect();
    notes::draw_marks(&mut part, &marks);
    let radius = over.radius() * scale;
    for pin in &over.notes.pins {
        let shown = over.pin_in_view(pin);
        let pin = Pin { x: shown.x - dx, y: shown.y - dy, ..shown };
        notes::draw_pins(&mut part, std::slice::from_ref(&pin), radius);
        notes::draw_number(&mut part, &pin, radius);
    }
    if let Some(card) = over.card() {
        let [x, y, cw, ch] = card.rect;
        fill(&mut part, [x - dx, y - dy, cw, ch], EDGE);
        fill(&mut part, [x - dx + 1.0, y - dy + 1.0, cw - 2.0, ch - 2.0], CARD);
        let look = Look { size: WORDS, colour: INK, bold: false };
        for (index, line) in card.lines.iter().enumerate() {
            let at = [card.corner[0] - dx, card.corner[1] - dy + card.advance * index as f32];
            label::draw(&mut part, line, at, look, Anchor::Left);
        }
    }
    for (row, y) in (y0..y1).enumerate() {
        let start = ((y * image.width + x0) * 4) as usize;
        image.data[start..start + (w * 4) as usize].copy_from_slice(&part.data[row * (w * 4) as usize..(row + 1) * (w * 4) as usize]);
    }
}

/// A frame with its notes on it, as a next round sends it: every mark and
/// every pin with its number, the take's `size` drawn over the whole
/// frame, but no card.
pub fn marked(frame: &mut Rgba, notes: &Notes, size: (u32, u32)) {
    let whole = Rect { x: 0, y: 0, w: frame.width, h: frame.height };
    let over = NotesOver { notes: notes.clone(), size, rect: whole, clip: whole, open: None };
    draw(frame, &over);
}

/// A rectangle, `[x, y, width, height]`, filled with `colour` over what is
/// beneath, by its alpha.
fn fill(image: &mut Rgba, rect: [f32; 4], colour: [u8; 4]) {
    let x0 = rect[0].round().max(0.0) as u32;
    let y0 = rect[1].round().max(0.0) as u32;
    let x1 = ((rect[0] + rect[2]).round().max(0.0) as u32).min(image.width);
    let y1 = ((rect[1] + rect[3]).round().max(0.0) as u32).min(image.height);
    let alpha = f32::from(colour[3]) / 255.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let under = image.pixel(x, y);
            let mut out = [0; 4];
            for channel in 0..3 {
                out[channel] = (f32::from(under[channel]) + alpha * (f32::from(colour[channel]) - f32::from(under[channel]))).round() as u8;
            }
            out[3] = under[3].max(colour[3]);
            image.set(x, y, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layers::notes::RED;

    fn over(open: Option<u32>) -> NotesOver {
        NotesOver {
            notes: Notes {
                pins: vec![Pin { number: 1, x: 64.0, y: 36.0, text: "less rain on the glass".into(), at: Some(0.5) }],
                marks: vec![Mark { points: vec![[10.0, 60.0], [120.0, 60.0]], width: 2.0, colour: RED, at: Some(0.5) }],
            },
            size: (128, 72),
            // Drawn at twice its size, its frame's left part shown.
            rect: Rect { x: 20, y: 10, w: 256, h: 144 },
            clip: Rect { x: 20, y: 10, w: 200, h: 144 },
            open,
        }
    }

    /// Catches notes drawn at the take's pixels rather than the view's, or
    /// past the part of the frame shown: the mark is red where its take's
    /// point is drawn, the pin's disc red around its place, the card drawn
    /// only while the pin is open, and nothing past the clip.
    #[test]
    fn notes_are_drawn_where_the_frame_is() {
        let mut image = Rgba::new(300, 200, [255, 255, 255, 255]);
        draw(&mut image, &over(None));
        assert_eq!(image.pixel(20 + 2 * 50, 10 + 2 * 60), RED, "the mark is not where the take's point is drawn");
        // Below the number, within the disc of twice the pin's radius.
        let disc = image.pixel(20 + 2 * 64, 10 + 2 * 36 + 13);
        assert_eq!(disc, RED, "the pin's disc is not where its point is drawn");
        assert_eq!(image.pixel(250, 10 + 2 * 60), [255, 255, 255, 255], "a mark reached past the part shown");
        let open = over(Some(1));
        let card = open.card().unwrap();
        assert!(card.lines.concat().starts_with("1 — 0.5 s · less rain"), "the card does not start with the pin's moment");
        let inside = [(card.rect[0] + 3.0) as u32, (card.rect[1] + 2.0) as u32];
        assert_eq!(image.pixel(inside[0], inside[1]), [255, 255, 255, 255]);
        let mut drawn = Rgba::new(300, 200, [0, 0, 0, 255]);
        draw(&mut drawn, &open);
        assert!(drawn.pixel(inside[0], inside[1])[0] > 240, "the open pin's card is not drawn");
        let editing = open.editing().unwrap();
        assert!(editing.caret[0] > editing.rect[0], "the caret is not after the words");
    }

    /// Catches a view point taken to the wrong place of the take, or a
    /// point off the frame taken as on it.
    #[test]
    fn a_view_point_is_a_point_of_the_take() {
        let over = over(None);
        assert_eq!(over.to_take([20.0 + 128.0, 10.0 + 72.0]), Some([64.0, 36.0]));
        assert_eq!(over.to_take([5.0, 50.0]), None);
        assert_eq!(over.to_take([250.0, 50.0]), None, "a point past the part shown was taken as on the frame");
    }

    /// Catches words that wrap by losing their spaces, or never wrap.
    #[test]
    fn words_wrap_and_keep_their_spaces() {
        let words = "the rain on the glass should be lighter, and the lamp warmer, as in the first round";
        let lines = wrap(words, 160.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), words);
        assert_eq!(wrap("one ", 160.0), ["one "]);
    }
}
