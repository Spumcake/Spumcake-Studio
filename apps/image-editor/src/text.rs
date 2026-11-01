//! Text layers: their words, style, and box, laid out and drawn to pixels
//! with ab_glyph from the fonts the app ships, never the computer's.
//!
//! The layout follows Simple Sketch's rules in `draw/src/text.rs`: words
//! break at spaces, and a word longer than the line breaks at a character;
//! justify stretches every wrapped line but a paragraph's last; letter
//! spacing goes after every character but the last on a line; and line
//! height multiplies the face's own line spacing. A wrapped line is placed
//! by its words, so the space it wrapped at does not push it aside; a
//! paragraph's last line counts every character, so a space typed at its end
//! moves the caret as it is typed.

use crate::picture::Pixels;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, VariableFont, point};
use serde::{Deserialize, Serialize};

/// Inter, its roman and italic variable files, bold through the weight axis.
const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");
const INTER_ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic-Variable.ttf");
/// Hack, egui's monospace face. It has no bold or italic of its own, so they
/// are drawn by thickening and slanting it.
const HACK: &[u8] = include_bytes!("../assets/fonts/Hack-Regular.ttf");

/// How far a slanted face leans: the tangent of about 11°.
const SLANT: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    #[default]
    Inter,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub const ALL: [Align; 4] = [Align::Left, Align::Center, Align::Right, Align::Justify];
}

/// A text's look. Size and letter spacing are in picture pixels; line
/// height multiplies the face's own line spacing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub face: Face,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: [u8; 3],
    pub align: Align,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub all_caps: bool,
}

impl Default for Style {
    /// As text.png shows: Inter, 96 px, black, line height 1.2.
    fn default() -> Self {
        Self {
            face: Face::Inter,
            size: 96.0,
            bold: false,
            italic: false,
            underline: false,
            color: [0, 0, 0],
            align: Align::Left,
            line_height: 1.2,
            letter_spacing: 0.0,
            all_caps: false,
        }
    }
}

impl Style {
    /// The same style with every value inside its range.
    pub fn clamped(mut self) -> Self {
        self.size = if self.size.is_finite() { self.size.clamp(1.0, 2000.0) } else { 96.0 };
        self.line_height = if self.line_height.is_finite() { self.line_height.clamp(0.5, 5.0) } else { 1.2 };
        self.letter_spacing = if self.letter_spacing.is_finite() { self.letter_spacing.clamp(-200.0, 1000.0) } else { 0.0 };
        self
    }
}

/// A text layer's words, style, and box. Its box's corner is its layer's
/// offset; `width` wraps the lines, or none sets one line per paragraph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub words: String,
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
}

/// One laid line: the characters `start..end` of the words, the place of
/// each boundary between them, `xs[k]` before character `start + k`, and
/// where the line sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub xs: Vec<f32>,
    pub top: f32,
    pub baseline: f32,
    /// Where the line's words begin and end, without the space it wrapped
    /// at, for its underline.
    pub left: f32,
    pub right: f32,
}

/// The words laid out in their box, in pixels from the box's corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// The box: the wrapping width, or the widest line.
    pub width: f32,
    pub height: f32,
    /// One line's height.
    pub advance: f32,
    chars: Vec<char>,
}

/// One face as the style asks for it, and what is drawn by hand.
struct Chosen {
    font: &'static FontRef<'static>,
    scale: PxScale,
    /// Hack's bold: each glyph drawn again this far to the right.
    thicken: f32,
    /// Hack's italic: slanted as it is drawn.
    slant: bool,
}

/// The five faces, made once: Inter regular, bold, italic, and bold italic,
/// and Hack.
fn faces() -> &'static [FontRef<'static>; 5] {
    static FACES: std::sync::OnceLock<[FontRef<'static>; 5]> = std::sync::OnceLock::new();
    FACES.get_or_init(|| {
        let weighted = |bytes: &'static [u8], weight: f32| {
            let mut font = FontRef::try_from_slice(bytes).expect("the shipped font reads");
            font.set_variation(b"wght", weight);
            font
        };
        [
            weighted(INTER, 400.0),
            weighted(INTER, 700.0),
            weighted(INTER_ITALIC, 400.0),
            weighted(INTER_ITALIC, 700.0),
            FontRef::try_from_slice(HACK).expect("the shipped font reads"),
        ]
    })
}

fn choose(style: &Style) -> Chosen {
    let faces = faces();
    let size = style.size.max(1.0);
    let (font, thicken, slant) = match style.face {
        Face::Inter => {
            let index = usize::from(style.bold) + 2 * usize::from(style.italic);
            (&faces[index], 0.0, false)
        }
        Face::Mono => (&faces[4], if style.bold { (size / 18.0).max(1.0) } else { 0.0 }, style.italic),
    };
    // The em is the size in pixels, as CSS and egui have it.
    let em = font.units_per_em().unwrap_or(1000.0);
    let scale = PxScale::from(size * font.height_unscaled() / em);
    Chosen { font, scale, thicken, slant }
}

/// What a character is drawn as: all caps changes only its case, one
/// character for one, so the caret's places stay the words'.
fn shown(c: char, caps: bool) -> char {
    if caps { c.to_uppercase().next().unwrap_or(c) } else { c }
}

/// Lays the words out in their box.
pub fn lay(text: &Text) -> Layout {
    let style = text.style.clamped();
    let chosen = choose(&style);
    let font = chosen.font.as_scaled(chosen.scale);
    let chars: Vec<char> = text.words.chars().collect();
    let glyph = |index: usize| font.glyph_id(shown(chars[index], style.all_caps));
    let advance_of = |index: usize| font.h_advance(glyph(index)) + chosen.thicken;
    let kern = |a: usize, b: usize| font.kern(glyph(a), glyph(b));
    let spacing = style.letter_spacing;
    let wrap = text.width.filter(|width| width.is_finite() && *width > 0.0);
    // How wide the characters `from..to` are on one line, with spacing
    // after every one but the last.
    let measure = |from: usize, to: usize| -> f32 {
        let mut x = 0.0;
        for index in from..to {
            if index > from {
                x += kern(index - 1, index) + spacing;
            }
            x += advance_of(index);
        }
        x
    };
    // The lines as character ranges, and whether each wrapped.
    let mut ranges: Vec<(usize, usize, bool)> = Vec::new();
    let mut start = 0;
    loop {
        let end = chars[start..].iter().position(|c| *c == '\n').map_or(chars.len(), |at| start + at);
        let mut line = start;
        while let Some(width) = wrap {
            // The furthest a line from `line` reaches before its words
            // pass the width; spaces at its end hang outside.
            let mut fits = line;
            let mut wrap_at = None;
            let mut index = line;
            while index < end {
                let c = chars[index];
                if c != ' ' {
                    let mut trimmed = index + 1;
                    while trimmed > line && chars[trimmed - 1] == ' ' {
                        trimmed -= 1;
                    }
                    if measure(line, trimmed) > width && index > line {
                        break;
                    }
                }
                index += 1;
                fits = index;
                if c == ' ' {
                    wrap_at = Some(index);
                }
            }
            if fits >= end {
                break;
            }
            // Words break at spaces; a word longer than the line breaks at
            // a character, keeping at least one on the line.
            let next = match wrap_at {
                Some(at) if at > line => at,
                _ => fits.max(line + 1),
            };
            ranges.push((line, next, true));
            line = next;
        }
        ranges.push((line, end, false));
        if end >= chars.len() {
            break;
        }
        start = end + 1;
    }
    let ascent = font.ascent();
    let descent = font.descent();
    let natural = ascent - descent + font.line_gap();
    let advance = (natural * style.line_height).max(1.0);
    // Each line's own width: a wrapped line by its words, a paragraph's
    // last by all its characters.
    let extent = |from: usize, to: usize, wrapped: bool| {
        let mut to = to;
        if wrapped {
            while to > from && chars[to - 1] == ' ' {
                to -= 1;
            }
        }
        measure(from, to)
    };
    let widest = ranges.iter().map(|(from, to, wrapped)| extent(*from, *to, *wrapped)).fold(0.0f32, f32::max);
    let width = wrap.unwrap_or(widest);
    let share = match style.align {
        Align::Left | Align::Justify => 0.0,
        Align::Center => 0.5,
        Align::Right => 1.0,
    };
    let mut lines = Vec::with_capacity(ranges.len());
    for (row, (from, to, wrapped)) in ranges.into_iter().enumerate() {
        let own = extent(from, to, wrapped);
        let mut inner_end = to;
        while inner_end > from && chars[inner_end - 1] == ' ' {
            inner_end -= 1;
        }
        // Justify stretches the spaces between a wrapped line's words.
        let spaces = (from..inner_end).filter(|index| chars[*index] == ' ').count();
        let stretch = if style.align == Align::Justify && wrapped && spaces > 0 { ((width - own) / spaces as f32).max(0.0) } else { 0.0 };
        let shift = (width - own).max(0.0) * share;
        let mut xs = Vec::with_capacity(to - from + 1);
        let mut x = shift;
        xs.push(x);
        for index in from..to {
            if index > from {
                x += kern(index - 1, index);
            }
            x += advance_of(index);
            if chars[index] == ' ' && index < inner_end {
                x += stretch;
            }
            // Spacing goes between characters, not after the line's last.
            if index + 1 < to {
                x += spacing;
            }
            xs.push(x);
        }
        let top = row as f32 * advance;
        let right = if inner_end > from { xs[inner_end - from] } else { shift };
        lines.push(Line { start: from, end: to, xs, top, baseline: top + (advance - (ascent - descent)) / 2.0 + ascent, left: shift, right });
    }
    let height = lines.len() as f32 * advance;
    Layout { lines, width, height, advance, chars }
}

impl Layout {
    /// The line a character place is on: a wrapped line's end is the next
    /// line's start, where the caret goes.
    pub fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|line| line.start <= index).unwrap_or(0)
    }

    /// Where a character place is on its line.
    pub fn x_of(&self, index: usize) -> f32 {
        let line = &self.lines[self.line_of(index)];
        line.xs[index.clamp(line.start, line.end) - line.start]
    }

    /// The caret before character `index`: `[x, y, width, height]`.
    pub fn caret(&self, index: usize, size: f32) -> [f32; 4] {
        let line = &self.lines[self.line_of(index)];
        [self.x_of(index), line.top, (size / 40.0).max(1.0), self.advance]
    }

    /// The place nearest `at` on line `row`.
    pub fn place_on(&self, row: usize, x: f32) -> usize {
        let line = &self.lines[row.min(self.lines.len() - 1)];
        let mut best = line.start;
        let mut distance = f32::MAX;
        for (k, place) in line.xs.iter().enumerate() {
            let gap = (place - x).abs();
            if gap < distance {
                distance = gap;
                best = line.start + k;
            }
        }
        best
    }

    /// The character place nearest a point in the box.
    pub fn hit(&self, at: [f32; 2]) -> usize {
        let row = (at[1] / self.advance).floor().max(0.0) as usize;
        self.place_on(row.min(self.lines.len() - 1), at[0])
    }

    /// The rectangles that show the characters `from..to` selected.
    pub fn spans(&self, from: usize, to: usize) -> Vec<[f32; 4]> {
        let mut out = Vec::new();
        for line in &self.lines {
            let a = from.max(line.start);
            let b = to.min(line.end);
            // A selection running past the line's end shows its break.
            let past = to > line.end && from <= line.end && line.end < self.chars.len();
            if a > b || (a == b && !past) {
                continue;
            }
            let x0 = line.xs[a - line.start];
            let mut x1 = line.xs[b - line.start];
            if past {
                x1 += self.advance / 4.0;
            }
            out.push([x0, line.top, x1 - x0, self.advance]);
        }
        out
    }
}

/// The text drawn in its box: straight RGBA, clear where no glyph reaches.
/// It is a little wider and taller than the box, so slanted and bold
/// glyphs at its edge are not cut.
pub fn draw(text: &Text) -> Pixels {
    let style = text.style.clamped();
    let layout = lay(text);
    let chosen = choose(&style);
    let font = chosen.font.as_scaled(chosen.scale);
    let pad = (style.size * 0.3).ceil() + 2.0;
    let width = ((layout.width + pad).ceil() as u32).clamp(1, 16_384);
    let height = ((layout.height + pad / 2.0).ceil() as u32).clamp(1, 16_384);
    let mut cover = vec![0.0f32; width as usize * height as usize];
    let mut put = |x: f32, y: i32, value: f32| {
        if y < 0 || y >= height as i32 || value <= 0.0 {
            return;
        }
        // A slanted or fractional column shares its coverage between the
        // two pixels it falls across.
        let left = x.floor();
        let part = x - left;
        for (column, share) in [(left as i32, 1.0 - part), (left as i32 + 1, part)] {
            if column < 0 || column >= width as i32 || share <= 0.0 {
                continue;
            }
            let cell = &mut cover[y as usize * width as usize + column as usize];
            *cell = (*cell + value * share).min(1.0);
        }
    };
    for line in &layout.lines {
        for index in line.start..line.end {
            let c = shown(layout.chars[index], style.all_caps);
            if c.is_whitespace() {
                continue;
            }
            let x = line.xs[index - line.start];
            let id = font.glyph_id(c);
            let copies = if chosen.thicken > 0.0 { 3 } else { 1 };
            for copy in 0..copies {
                let shift = chosen.thicken * copy as f32 / (copies.max(2) - 1) as f32;
                let glyph = id.with_scale_and_position(chosen.scale, point(x + shift, line.baseline));
                let Some(outlined) = font.outline_glyph(glyph) else {
                    continue;
                };
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, value| {
                    let py = bounds.min.y + gy as f32;
                    let mut px = bounds.min.x + gx as f32;
                    if chosen.slant {
                        px += (line.baseline - (py + 0.5)) * SLANT;
                    }
                    put(px, py as i32, value);
                });
            }
        }
        if style.underline && line.right > line.left {
            let thickness = (style.size / 16.0).max(1.0);
            let y0 = line.baseline + style.size * 0.1;
            for y in y0.round() as i32..(y0 + thickness).round().max(y0.round() + 1.0) as i32 {
                let mut x = line.left.floor();
                while x < line.right {
                    put(x, y, 1.0);
                    x += 1.0;
                }
            }
        }
    }
    let mut data = Vec::with_capacity(cover.len() * 4);
    for value in cover {
        data.extend_from_slice(&[style.color[0], style.color[1], style.color[2], (value * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    Pixels { width, height, data }
}

/// Where a word starts and ends around a character place: letters and
/// digits make words, and anything else stands alone.
pub fn word_at(words: &str, index: usize) -> (usize, usize) {
    let chars: Vec<char> = words.chars().collect();
    let wordy = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    if chars.is_empty() {
        return (0, 0);
    }
    let at = index.min(chars.len() - 1);
    let kind = wordy(chars[at]);
    let mut start = at;
    while start > 0 && wordy(chars[start - 1]) == kind && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && wordy(chars[end]) == kind && chars[end] != '\n' {
        end += 1;
    }
    (start, end)
}

/// The place a word-wise move left or right reaches: past spaces, then
/// past the word.
pub fn word_step(words: &str, index: usize, forward: bool) -> usize {
    let chars: Vec<char> = words.chars().collect();
    let wordy = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = index.min(chars.len());
    if forward {
        while at < chars.len() && !wordy(chars[at]) {
            at += 1;
        }
        while at < chars.len() && wordy(chars[at]) {
            at += 1;
        }
    } else {
        while at > 0 && !wordy(chars[at - 1]) {
            at -= 1;
        }
        while at > 0 && wordy(chars[at - 1]) {
            at -= 1;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(words: &str, style: Style, width: Option<f32>) -> Text {
        Text { words: words.into(), style, width }
    }

    fn inked(pixels: &Pixels) -> usize {
        pixels.data.chunks(4).filter(|pixel| pixel[3] > 0).count()
    }

    /// Catches text that draws nothing, or draws where its box is not.
    #[test]
    fn words_draw_inside_their_box() {
        let hi = text("Hi", Style { size: 40.0, ..Style::default() }, None);
        let layout = lay(&hi);
        assert!(layout.width > 20.0 && layout.width < 80.0, "{}", layout.width);
        let pixels = draw(&hi);
        assert!(inked(&pixels) > 50, "the text drew nothing");
        // Nothing far right of the words.
        for y in 0..pixels.height as usize {
            for x in (layout.width as usize + 4)..pixels.width as usize {
                assert_eq!(pixels.data[(y * pixels.width as usize + x) * 4 + 3], 0, "ink past the words at {x},{y}");
            }
        }
        assert_ne!(draw(&text("Hello", hi.style, None)), pixels);
    }

    /// Catches bold drawn as regular, a monospace face whose characters
    /// differ in width, and a wrap that makes no more lines.
    #[test]
    fn bold_is_wider_monospace_is_even_and_wrapping_adds_lines() {
        let regular = Style { size: 30.0, ..Style::default() };
        let bold = Style { bold: true, ..regular };
        let words = "Golden hour";
        assert!(lay(&text(words, bold, None)).width > lay(&text(words, regular, None)).width + 1.0, "bold is not wider");
        assert!(inked(&draw(&text(words, bold, None))) > inked(&draw(&text(words, regular, None))));
        let mono = Style { face: Face::Mono, ..regular };
        let narrow = lay(&text("iiii", mono, None)).width;
        let wide = lay(&text("MMMM", mono, None)).width;
        assert!((narrow - wide).abs() < 0.01, "monospace advances differ: {narrow} and {wide}");
        assert!(lay(&text("iiii", regular, None)).width < lay(&text("MMMM", regular, None)).width - 5.0);
        // Hack's bold, drawn by thickening, is wider too.
        assert!(lay(&text(words, Style { bold: true, ..mono }, None)).width > lay(&text(words, mono, None)).width);

        let one = lay(&text("a few words that wrap", regular, None));
        let narrow = lay(&text("a few words that wrap", regular, Some(90.0)));
        assert_eq!(one.lines.len(), 1);
        assert!(narrow.lines.len() >= 3, "{} lines", narrow.lines.len());
        // A word longer than the line breaks at a character.
        let long = lay(&text("abcdefghijklmnop", regular, Some(40.0)));
        assert!(long.lines.len() >= 3);
        assert!(long.lines.iter().all(|line| line.end > line.start));
    }

    /// Catches justify that leaves a wrapped line ragged, alignment that
    /// does not move a line, and letter spacing after the line's last
    /// character.
    #[test]
    fn alignment_justify_and_spacing_follow_simple_sketch() {
        let style = Style { size: 20.0, ..Style::default() };
        let words = "one two three four five six";
        let justified = lay(&text(words, Style { align: Align::Justify, ..style }, Some(150.0)));
        assert!(justified.lines.len() >= 2);
        let first = &justified.lines[0];
        assert!((first.right - 150.0).abs() < 1.0, "a justified line ends at {}", first.right);
        let last = justified.lines.last().unwrap();
        assert!(last.right < 140.0, "the last line was stretched");
        let right = lay(&text("hi", Style { align: Align::Right, ..style }, Some(150.0)));
        assert!((right.lines[0].right - 150.0).abs() < 0.5);
        let centered = lay(&text("hi", Style { align: Align::Center, ..style }, Some(150.0)));
        assert!((centered.lines[0].left + centered.lines[0].right - 150.0).abs() < 0.5);
        let plain = lay(&text("abc", style, None)).width;
        let spaced_layout = lay(&text("abc", Style { letter_spacing: 10.0, ..style }, None));
        let spaced = spaced_layout.width;
        assert!((spaced - plain - 20.0).abs() < 0.01, "spacing {spaced} against {plain}");
        let end = spaced_layout.lines[0].xs[3];
        assert!((end - spaced).abs() < 0.01, "the caret after the last character sits at {end}, past the words' {spaced}");
        let taller = lay(&text("a\nb", Style { line_height: 2.0, ..style }, None));
        let normal = lay(&text("a\nb", style, None));
        assert!((taller.height - 2.0 * normal.height / 1.2).abs() < 0.5);
    }

    /// Catches a caret or hit test that loses its place across lines.
    #[test]
    fn places_and_carets_agree() {
        let laid = lay(&text("ab\ncd", Style { size: 20.0, ..Style::default() }, None));
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(laid.line_of(2), 0);
        assert_eq!(laid.line_of(3), 1);
        let caret = laid.caret(4, 20.0);
        assert_eq!(laid.hit([caret[0], caret[1] + 2.0]), 4);
        assert_eq!(word_at("hello there", 2), (0, 5));
        assert_eq!(word_step("hello there", 0, true), 5);
        assert_eq!(word_step("hello there", 11, false), 6);
    }
}
