//! Laying out a round's takes in the working area: a grid of numbered
//! takes, rounds one above another, frame strips, and two takes side by
//! side. Each layout gives the rectangle every image is drawn in, kept at
//! the images' shape and centred in its cell.

use crate::Rgba;
use image::{ImageBuffer, imageops};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// `rows` × `columns` cells over `area`, with `gap` between them, each
/// holding the largest rectangle of `aspect` (width over height) that fits,
/// centred. Listed row by row.
pub fn cells(area: Rect, rows: u32, columns: u32, gap: u32, aspect: f32) -> Vec<Rect> {
    if rows == 0 || columns == 0 {
        return Vec::new();
    }
    let cell_w = area.w.saturating_sub(gap * (columns - 1)) / columns;
    let cell_h = area.h.saturating_sub(gap * (rows - 1)) / rows;
    let (w, h) = fit(cell_w, cell_h, aspect);
    let mut out = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let x = area.x + (column * (cell_w + gap)) as i32 + ((cell_w - w) / 2) as i32;
            let y = area.y + (row * (cell_h + gap)) as i32 + ((cell_h - h) / 2) as i32;
            out.push(Rect { x, y, w, h });
        }
    }
    out
}

/// A round of `count` takes as a grid: as close to square as `area` and
/// the takes' shape allow, filled row by row, picking the column count
/// that makes the takes largest.
pub fn grid(area: Rect, count: u32, gap: u32, aspect: f32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let best = (1..=count)
        .max_by_key(|&columns| {
            let rows = count.div_ceil(columns);
            cells(area, rows, columns, gap, aspect).first().map_or(0, |cell| cell.w * cell.h)
        })
        .unwrap_or(1);
    let mut out = cells(area, count.div_ceil(best), best, gap, aspect);
    out.truncate(count as usize);
    out
}

/// The largest `aspect` rectangle inside `w` × `h`.
pub fn fit(w: u32, h: u32, aspect: f32) -> (u32, u32) {
    let aspect = if aspect > 0.0 { aspect } else { 1.0 };
    if w as f32 / (h.max(1) as f32) > aspect {
        (((h as f32) * aspect).round() as u32, h)
    } else {
        (w, ((w as f32) / aspect).round() as u32)
    }
}

/// `image` scaled to exactly `width` × `height`, or as it is when it
/// already is that size.
pub fn fit_exactly(image: &Rgba, width: u32, height: u32) -> Rgba {
    if image.width == width && image.height == height {
        return image.clone();
    }
    let buffer: ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(image.width, image.height, image.data.clone()).expect("an image's bytes match its size");
    let scaled = imageops::resize(&buffer, width.max(1), height.max(1), imageops::FilterType::Triangle);
    Rgba { width: scaled.width(), height: scaled.height(), data: scaled.into_raw() }
}

/// Draws `image` scaled into `rect` on `onto`, cut at its edges.
pub fn place(onto: &mut Rgba, image: &Rgba, rect: Rect) {
    let scaled = fit_exactly(image, rect.w, rect.h);
    for y in 0..scaled.height {
        let ty = rect.y + y as i32;
        if ty < 0 || ty >= onto.height as i32 {
            continue;
        }
        for x in 0..scaled.width {
            let tx = rect.x + x as i32;
            if tx < 0 || tx >= onto.width as i32 {
                continue;
            }
            onto.set(tx as u32, ty as u32, scaled.pixel(x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a grid that spills out of its area, overlaps, or wastes it:
    /// four 16:9 takes in a wide area are two by two, inside it.
    #[test]
    fn four_takes_make_two_by_two() {
        let area = Rect { x: 10, y: 20, w: 1000, h: 600 };
        let rects = grid(area, 4, 8, 16.0 / 9.0);
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0].y, rects[1].y);
        assert!(rects[2].y > rects[0].y + rects[0].h as i32 - 1);
        for rect in &rects {
            assert!(rect.x >= area.x && rect.y >= area.y);
            assert!(rect.x + rect.w as i32 <= area.x + area.w as i32);
            assert!(rect.y + rect.h as i32 <= area.y + area.h as i32);
            assert!((rect.w as f32 / rect.h as f32 - 16.0 / 9.0).abs() < 0.02);
        }
    }

    /// Catches frame strips or side-by-side takes laid out of order:
    /// cells run row by row, left to right.
    #[test]
    fn cells_run_row_by_row() {
        let rects = cells(Rect { x: 0, y: 0, w: 800, h: 300 }, 3, 8, 4, 16.0 / 9.0);
        assert_eq!(rects.len(), 24);
        assert!(rects[1].x > rects[0].x && rects[1].y == rects[0].y);
        assert!(rects[8].y > rects[7].y && rects[8].x == rects[0].x);
    }
}
