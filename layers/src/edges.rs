//! The pre-vis check: the pre-vis render's edges drawn over an image, as
//! Spumcake-Skills' `build-animatic/scripts/overlay.py` draws them over a
//! take, or the pre-vis blended see-through. The edges are found as that
//! script finds them, so Studio and a harness show the same lines: the
//! render made grey, Pillow's 3 × 3 `FIND_EDGES` filter, and every value
//! above 18 an edge, which is mixed 85% toward the colour.

use crate::Rgba;

/// The pre-vis check's cyan, `overlay.py`'s `--previs-colour` default.
pub const CYAN: [u8; 3] = [0, 220, 255];
/// How far an edge pixel moves toward the colour, as `overlay.py` blends.
pub const STRENGTH: f32 = 0.85;
/// Above this, a filtered value is an edge.
pub const THRESHOLD: u8 = 18;

/// The image in grey, as Pillow's `convert("L")` makes it.
pub fn grey(image: &Rgba) -> Vec<u8> {
    image
        .data
        .chunks_exact(4)
        .map(|p| ((u32::from(p[0]) * 19595 + u32::from(p[1]) * 38470 + u32::from(p[2]) * 7471 + 0x8000) >> 16) as u8)
        .collect()
}

/// Which pixels of a grey image are edges. As Pillow's filter does, the
/// outermost rows and columns keep their own value, so a bright render's
/// border counts as an edge, as it does in `overlay.py`.
pub fn find(grey: &[u8], width: u32, height: u32) -> Vec<bool> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let value = if y == 0 || x == 0 || y + 1 == h || x + 1 == w {
                i32::from(grey[y * w + x])
            } else {
                let mut sum = 8 * i32::from(grey[y * w + x]);
                for dy in [-1i32, 0, 1] {
                    for dx in [-1i32, 0, 1] {
                        if dx != 0 || dy != 0 {
                            sum -= i32::from(grey[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize]);
                        }
                    }
                }
                sum.clamp(0, 255)
            };
            out[y * w + x] = value > i32::from(THRESHOLD);
        }
    }
    out
}

/// Draws `edges` over `image`, each edge pixel mixed `strength` toward
/// `colour`, as Pillow's `Image.blend` mixes, keeping the image's alpha.
pub fn draw(image: &mut Rgba, edges: &[bool], colour: [u8; 3], strength: f32) {
    for (pixel, edge) in image.data.chunks_exact_mut(4).zip(edges) {
        if *edge {
            for channel in 0..3 {
                pixel[channel] = blend(pixel[channel], colour[channel], strength);
            }
        }
    }
}

/// The pre-vis check over an image: the pre-vis's edges, found at the
/// image's size, in `colour`.
pub fn check(image: &mut Rgba, previs: &Rgba, colour: [u8; 3]) {
    let previs = crate::layout::fit_exactly(previs, image.width, image.height);
    let edges = find(&grey(&previs), previs.width, previs.height);
    draw(image, &edges, colour, STRENGTH);
}

/// The pre-vis blended over the image at `opacity`, from 0 to 1.
pub fn see_through(image: &mut Rgba, previs: &Rgba, opacity: f32) {
    let previs = crate::layout::fit_exactly(previs, image.width, image.height);
    for (pixel, over) in image.data.chunks_exact_mut(4).zip(previs.data.chunks_exact(4)) {
        for channel in 0..3 {
            pixel[channel] = blend(pixel[channel], over[channel], opacity.clamp(0.0, 1.0));
        }
    }
}

/// Pillow's blend of two bytes: `a + strength × (b − a)`, cut to a byte.
fn blend(a: u8, b: u8, strength: f32) -> u8 {
    (f32::from(a) + strength * (f32::from(b) - f32::from(a))).clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches an edge finder that marks flat areas or misses a step: a
    /// flat image has edges only at its border, as Pillow's filter keeps
    /// it, and a vertical step from dark to light is found where it is.
    #[test]
    fn edges_are_where_the_grey_changes() {
        let (w, h) = (8, 6);
        let flat = vec![100u8; w * h];
        let edges = find(&flat, w as u32, h as u32);
        for y in 0..h {
            for x in 0..w {
                let border = y == 0 || x == 0 || y == h - 1 || x == w - 1;
                assert_eq!(edges[y * w + x], border, "({x}, {y})");
            }
        }
        let step: Vec<u8> = (0..w * h).map(|i| if i % w < 4 { 0 } else { 200 }).collect();
        let edges = find(&step, w as u32, h as u32);
        assert!(edges[2 * w + 4], "the light side of the step");
        assert!(!edges[2 * w + 2] && !edges[2 * w + 6], "a flat area away from the step");
    }
}
