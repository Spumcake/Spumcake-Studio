//! The views' small labels: one line of words drawn into an image, such as
//! a round's label, a take's number, a frame strip's times, or a take's
//! name. Inter, which ships in this crate's `assets/fonts/`, is the one
//! face, regular or bold through its weight axis; a workspace never reads
//! the computer's fonts.

use crate::Rgba;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, VariableFont, point};

/// Inter's roman variable file, Inter 4.001, under the SIL Open Font
/// Licence in `assets/fonts/OFL.txt`.
pub const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");

/// Where a label's `at` is: its left end, or its middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Left,
    Centre,
}

/// A label's size in pixels, its colour, straight RGBA, and whether it is
/// bold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub size: f32,
    pub colour: [u8; 4],
    pub bold: bool,
}

/// Inter at its regular and bold weights, read once.
fn face(bold: bool) -> &'static FontRef<'static> {
    static FACES: std::sync::OnceLock<[FontRef<'static>; 2]> = std::sync::OnceLock::new();
    let faces = FACES.get_or_init(|| {
        let weighted = |weight: f32| {
            let mut font = FontRef::try_from_slice(INTER).expect("the shipped font reads");
            font.set_variation(b"wght", weight);
            font
        };
        [weighted(400.0), weighted(700.0)]
    });
    &faces[usize::from(bold)]
}

/// How wide `words` are on one line, in pixels.
pub fn measure(words: &str, size: f32, bold: bool) -> f32 {
    let font = face(bold).as_scaled(PxScale::from(size.max(1.0)));
    let mut width = 0.0;
    let mut last = None;
    for c in words.chars() {
        let id = font.glyph_id(c);
        if let Some(before) = last {
            width += font.kern(before, id);
        }
        width += font.h_advance(id);
        last = Some(id);
    }
    width
}

/// How tall a line of this size is, from its ascent to its descent.
pub fn height(size: f32) -> f32 {
    let font = face(false).as_scaled(PxScale::from(size.max(1.0)));
    font.ascent() - font.descent()
}

/// Draws `words` on one line into `image`, the line's top at `at[1]` and
/// its left end, or its middle, at `at[0]`: each pixel moved towards the
/// colour by how much the glyphs cover it, straight alpha, and cut at the
/// image's edges. Returns the line's width.
pub fn draw(image: &mut Rgba, words: &str, at: [f32; 2], look: Look, anchor: Anchor) -> f32 {
    let scale = PxScale::from(look.size.max(1.0));
    let font = face(look.bold);
    let scaled = font.as_scaled(scale);
    let width = measure(words, look.size, look.bold);
    let mut x = match anchor {
        Anchor::Left => at[0],
        Anchor::Centre => at[0] - width / 2.0,
    };
    let baseline = at[1] + scaled.ascent();
    let mut last = None;
    for c in words.chars() {
        let id = scaled.glyph_id(c);
        if let Some(before) = last {
            x += scaled.kern(before, id);
        }
        if let Some(outlined) = font.outline_glyph(id.with_scale_and_position(scale, point(x, baseline))) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let (px, py) = (bounds.min.x as i64 + i64::from(gx), bounds.min.y as i64 + i64::from(gy));
                if px >= 0 && py >= 0 && px < i64::from(image.width) && py < i64::from(image.height) {
                    cover(image, px as u32, py as u32, coverage.clamp(0.0, 1.0), look.colour);
                }
            });
        }
        x += scaled.h_advance(id);
        last = Some(id);
    }
    width
}

/// One pixel moved `coverage` of the way to `colour`, as the notes' discs
/// and marks cover theirs.
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

    const INK: Look = Look { size: 16.0, colour: [0, 0, 0, 255], bold: false };

    fn inked(image: &Rgba) -> Vec<(u32, u32)> {
        let mut found = Vec::new();
        for y in 0..image.height {
            for x in 0..image.width {
                if image.pixel(x, y)[0] < 128 {
                    found.push((x, y));
                }
            }
        }
        found
    }

    /// Catches a label drawn nowhere, past where it was put, or not as
    /// wide as it measures: "Take 3" covers pixels within its line and its
    /// width, and none elsewhere; centred, it sits about its point; bold
    /// is wider.
    #[test]
    fn a_label_covers_where_it_is_drawn() {
        let mut image = Rgba::new(120, 40, [255, 255, 255, 255]);
        let width = draw(&mut image, "Take 3", [10.0, 8.0], INK, Anchor::Left);
        assert!((width - measure("Take 3", 16.0, false)).abs() < 0.01);
        assert!(width > 30.0 && width < 70.0, "{width}");
        let ink = inked(&image);
        assert!(!ink.is_empty(), "the label covers nothing");
        let line = 8.0 + height(16.0);
        for (x, y) in &ink {
            assert!(*x as f32 >= 9.0 && *x as f32 <= 10.0 + width + 1.0, "ink at x {x}");
            assert!(*y as f32 >= 7.0 && *y as f32 <= line + 1.0, "ink at y {y}");
        }
        assert_eq!(image.pixel(110, 35), [255, 255, 255, 255]);

        let mut centred = Rgba::new(120, 40, [255, 255, 255, 255]);
        draw(&mut centred, "8.0 s", [60.0, 8.0], INK, Anchor::Centre);
        let xs: Vec<u32> = inked(&centred).iter().map(|(x, _)| *x).collect();
        let (left, right) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
        assert!((i64::from(left) + i64::from(right) - 120).abs() <= 4, "a centred label is not about its point: {left} to {right}");
        assert!(measure("Round 14", 16.0, true) > measure("Round 14", 16.0, false));
    }
}
