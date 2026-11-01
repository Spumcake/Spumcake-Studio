//! The brush, the eraser, the clone stamp, the healing brush, and the
//! selection: what a stroke covers, how it changes a layer's pixels, and the
//! selection's coverage mask and outline.

use crate::picture::{Pixels, Rect, in_bands, run_apart, shares};
use std::sync::{Mutex, PoisonError};

/// A brush or eraser as the toolbar sets it. Size is in picture pixels;
/// hardness and opacity run from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    pub rgb: [u8; 3],
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
    pub pressure: bool,
    pub erase: bool,
}

/// How much of each pixel one stroke covers, from 0 to 1: the most any of
/// its dabs covers there, so overlapping dabs never build past the brush's
/// opacity.
pub struct Coverage {
    pub rect: Rect,
    pub values: Vec<f32>,
}

/// The stroke's coverage: every pixel's distance to the nearest part of the
/// line through the points, against the brush's radius there, which pen
/// pressure scales. Measured against the line itself rather than round dabs
/// along it, so a hard edge is smooth, without the ridges dabs leave between
/// them, as the owner saw on 2026-10-30.
#[cfg(test)]
pub fn stroke_coverage(points: &[[f64; 3]], brush: &Brush) -> Option<Coverage> {
    stroke_coverage_within(points, brush, Rect::new(i32::MIN / 4, i32::MIN / 4, i32::MAX / 2, i32::MAX / 2))
}

/// The stroke's coverage inside `clip`, the only pixels it can change.
pub fn stroke_coverage_within(points: &[[f64; 3]], brush: &Brush, clip: Rect) -> Option<Coverage> {
    let bounds = stroke_bounds(points, brush)?.intersect(&clip);
    if bounds.is_empty() {
        return None;
    }
    let mut values = vec![0.0f32; bounds.w as usize * bounds.h as usize];
    cover(points, brush, bounds, bounds, &mut values);
    Some(Coverage { rect: bounds, values })
}

/// The pixels the stroke through `points` can reach.
pub fn stroke_bounds(points: &[[f64; 3]], brush: &Brush) -> Option<Rect> {
    let pieces = pieces(points, brush)?;
    let mut bounds = Rect::new(0, 0, 0, 0);
    for (a, ra, b, rb) in &pieces {
        bounds = bounds.union(&piece_box(a, *ra, b, *rb));
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// Raises `values`, the coverage of every pixel of `frame` row by row, to
/// what the stroke through `points` gives each pixel inside `clip`, and
/// returns the part it reached. Pixels already fully covered are passed by,
/// so a stroke that goes over itself, or a piece added to a stroke being
/// drawn, costs only its new pixels.
pub fn cover(points: &[[f64; 3]], brush: &Brush, frame: Rect, clip: Rect, values: &mut [f32]) -> Rect {
    let mut reached = Rect::new(0, 0, 0, 0);
    let Some(pieces) = pieces(points, brush) else {
        return reached;
    };
    let clip = clip.intersect(&frame);
    let hardness = brush.hardness.clamp(0.0, 1.0);
    for (a, ra, b, rb) in &pieces {
        let area = piece_box(a, *ra, b, *rb).intersect(&clip);
        if area.is_empty() {
            continue;
        }
        reached = reached.union(&area);
        let along = [b[0] - a[0], b[1] - a[1]];
        let length2 = along[0] * along[0] + along[1] * along[1];
        for y in area.y..area.bottom() {
            let py = f64::from(y) + 0.5;
            let row = (y - frame.y) as usize * frame.w as usize;
            for x in area.x..area.right() {
                let cell = &mut values[row + (x - frame.x) as usize];
                if *cell >= 1.0 {
                    continue;
                }
                let px = f64::from(x) + 0.5;
                // The nearest place on this piece, and the radius there.
                let t = if length2 > 0.0 {
                    (((px - a[0]) * along[0] + (py - a[1]) * along[1]) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest = [a[0] + along[0] * t, a[1] + along[1] * t];
                let radius = ra + (rb - ra) * t;
                let distance2 = (px - nearest[0]).powi(2) + (py - nearest[1]).powi(2);
                if distance2 >= (radius + 0.5) * (radius + 0.5) {
                    continue;
                }
                let distance = distance2.sqrt();
                // The edge is smoothed over one pixel, whatever the hardness.
                let edge = (radius - distance + 0.5).clamp(0.0, 1.0);
                let inner = radius * hardness;
                let soft = if distance <= inner || radius - inner <= f64::EPSILON {
                    1.0
                } else {
                    let t = ((distance - inner) / (radius - inner)).clamp(0.0, 1.0);
                    1.0 - t * t * (3.0 - 2.0 * t)
                };
                let value = (edge.min(soft)) as f32;
                if value > *cell {
                    *cell = value;
                }
            }
        }
    }
    reached
}

/// Each piece of the line: its two ends, with the radius at each. A stroke
/// of one point is one piece of no length, a round dot.
type Piece = ([f64; 2], f64, [f64; 2], f64);

fn pieces(points: &[[f64; 3]], brush: &Brush) -> Option<Vec<Piece>> {
    let points: Vec<[f64; 3]> = points
        .iter()
        .copied()
        .filter(|point| point.iter().all(|value| value.is_finite()))
        .collect();
    if points.is_empty() || !brush.size.is_finite() || brush.size <= 0.0 {
        return None;
    }
    let radius_at = |pressure: f64| {
        let scale = if brush.pressure { pressure.clamp(0.0, 1.0) } else { 1.0 };
        ((brush.size * scale).max(1.0) / 2.0).max(0.5)
    };
    Some(if points.len() == 1 {
        let point = points[0];
        vec![([point[0], point[1]], radius_at(point[2]), [point[0], point[1]], radius_at(point[2]))]
    } else {
        points
            .windows(2)
            .map(|pair| ([pair[0][0], pair[0][1]], radius_at(pair[0][2]), [pair[1][0], pair[1][1]], radius_at(pair[1][2])))
            .collect()
    })
}

fn piece_box(a: &[f64; 2], ra: f64, b: &[f64; 2], rb: f64) -> Rect {
    let reach = ra.max(rb) + 1.0;
    let x = (a[0].min(b[0]) - reach).floor() as i32;
    let y = (a[1].min(b[1]) - reach).floor() as i32;
    let right = (a[0].max(b[0]) + reach).ceil() as i32;
    let bottom = (a[1].max(b[1]) + reach).ceil() as i32;
    Rect::new(x, y, right - x, bottom - y)
}

/// Paints or erases a stroke's coverage into a layer whose pixels sit at
/// `offset`, inside `clip` (the picture, in picture pixels) and the
/// selection when there is one. `keep_alpha` paints colour only, as the
/// transparency lock asks. Returns the part of the layer that changed, in
/// the layer's own pixels.
pub fn apply(
    pixels: &mut Pixels,
    offset: [i32; 2],
    coverage: &Coverage,
    clip: Rect,
    selection: Option<&Mask>,
    brush: &Brush,
    keep_alpha: bool,
) -> Option<Rect> {
    let layer = Rect::new(offset[0], offset[1], pixels.width as i32, pixels.height as i32);
    let area = coverage.rect.intersect(&layer).intersect(&clip);
    if area.is_empty() {
        return None;
    }
    let opacity = brush.opacity.clamp(0.0, 1.0) as f32;
    let color = brush.rgb.map(|value| f32::from(value) / 255.0);
    let mut changed = Rect::new(0, 0, 0, 0);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let mut amount = coverage.values[(y - coverage.rect.y) as usize * coverage.rect.w as usize + (x - coverage.rect.x) as usize] * opacity;
            if let Some(mask) = selection {
                amount *= f32::from(mask.at(x, y)) / 255.0;
            }
            if amount <= 0.0 {
                continue;
            }
            let at = ((y - layer.y) as usize * pixels.width as usize + (x - layer.x) as usize) * 4;
            let before = [pixels.data[at], pixels.data[at + 1], pixels.data[at + 2], pixels.data[at + 3]];
            let below = f32::from(before[3]) / 255.0;
            let mut after = before;
            if brush.erase {
                after[3] = to_byte(below * (1.0 - amount));
            } else if keep_alpha {
                for channel in 0..3 {
                    let old = f32::from(before[channel]) / 255.0;
                    after[channel] = to_byte(old + (color[channel] - old) * amount);
                }
            } else {
                let alpha = amount + below * (1.0 - amount);
                for channel in 0..3 {
                    let old = f32::from(before[channel]) / 255.0;
                    after[channel] = to_byte((color[channel] * amount + old * below * (1.0 - amount)) / alpha);
                }
                after[3] = to_byte(alpha);
            }
            if after != before {
                pixels.data[at..at + 4].copy_from_slice(&after);
                changed = changed.union(&Rect::new(x - layer.x, y - layer.y, 1, 1));
            }
        }
    }
    (!changed.is_empty()).then_some(changed)
}

fn to_byte(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Paints a colour of its own at each pixel of `area`, in picture pixels,
/// into a layer whose pixels sit at `offset`: straight RGBA from 0 to 1,
/// mixed in by the coverage there, the selection, `opacity`, and the
/// colour's own alpha. The clone stamp and the healing brush paint so.
/// Returns the part of the layer that changed, in its own pixels.
#[allow(clippy::too_many_arguments)]
pub fn apply_colours(
    pixels: &mut Pixels,
    offset: [i32; 2],
    area: Rect,
    coverage: &[f32],
    colours: &[Option<[f32; 4]>],
    selection: Option<&Mask>,
    opacity: f32,
    keep_alpha: bool,
) -> Option<Rect> {
    let colour = |x: i32, y: i32| colours[(y - area.y) as usize * area.w as usize + (x - area.x) as usize];
    apply_with(pixels, offset, area, coverage, colour, selection, opacity, keep_alpha)
}

/// [`apply_colours`] with each pixel's colour worked out as it is painted,
/// by `colour(x, y)`, in bands of rows, one per core.
#[allow(clippy::too_many_arguments)]
pub fn apply_with(
    pixels: &mut Pixels,
    offset: [i32; 2],
    area: Rect,
    coverage: &[f32],
    colour: impl Fn(i32, i32) -> Option<[f32; 4]> + Sync,
    selection: Option<&Mask>,
    opacity: f32,
    keep_alpha: bool,
) -> Option<Rect> {
    let layer = Rect::new(offset[0], offset[1], pixels.width as i32, pixels.height as i32);
    let shared = area.intersect(&layer);
    if shared.is_empty() {
        return None;
    }
    let changed = Mutex::new(Rect::new(0, 0, 0, 0));
    let row_bytes = pixels.width as usize * 4;
    let rows = &mut pixels.data[(shared.y - layer.y) as usize * row_bytes..(shared.bottom() - layer.y) as usize * row_bytes];
    // In bands of rows, a few per core, as each pixel is painted by itself.
    let band = (shared.h as usize).div_ceil(shares(shared));
    in_bands(rows, band * row_bytes, |index, rows| {
        let mut own = Rect::new(0, 0, 0, 0);
        for (row, line) in rows.chunks_exact_mut(row_bytes).enumerate() {
            let y = shared.y + (index * band + row) as i32;
            for x in shared.x..shared.right() {
                let cell = (y - area.y) as usize * area.w as usize + (x - area.x) as usize;
                let Some(colour) = colour(x, y) else {
                    continue;
                };
                let mut amount = coverage[cell] * opacity * colour[3];
                if let Some(mask) = selection {
                    amount *= f32::from(mask.at(x, y)) / 255.0;
                }
                if amount <= 0.0 {
                    continue;
                }
                let at = (x - layer.x) as usize * 4;
                let before = [line[at], line[at + 1], line[at + 2], line[at + 3]];
                let below = f32::from(before[3]) / 255.0;
                let mut after = before;
                if keep_alpha {
                    for channel in 0..3 {
                        let old = f32::from(before[channel]) / 255.0;
                        after[channel] = to_byte(old + (colour[channel] - old) * amount);
                    }
                } else {
                    let alpha = amount + below * (1.0 - amount);
                    for channel in 0..3 {
                        let old = f32::from(before[channel]) / 255.0;
                        after[channel] = to_byte((colour[channel] * amount + old * below * (1.0 - amount)) / alpha);
                    }
                    after[3] = to_byte(alpha);
                }
                if after != before {
                    line[at..at + 4].copy_from_slice(&after);
                    own = own.union(&Rect::new(x - layer.x, y - layer.y, 1, 1));
                }
            }
        }
        let mut changed = changed.lock().unwrap_or_else(PoisonError::into_inner);
        *changed = changed.union(&own);
    });
    let changed = changed.into_inner().unwrap_or_else(PoisonError::into_inner);
    (!changed.is_empty()).then_some(changed)
}

/// What the clone stamp copies at each pixel of `area`: the picture
/// `source`, straight RGBA8 the picture's size, at the pixel plus `offset`.
pub fn stamp_colours(source: &[u8], width: u32, height: u32, area: Rect, offset: [i32; 2]) -> Vec<Option<[f32; 4]>> {
    let mut out = Vec::with_capacity(area.w as usize * area.h as usize);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            out.push(sample(source, width, height, x + offset[0], y + offset[1]));
        }
    }
    out
}

fn sample(source: &[u8], width: u32, height: u32, x: i32, y: i32) -> Option<[f32; 4]> {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return None;
    }
    let at = (y as usize * width as usize + x as usize) * 4;
    if source[at + 3] == 0 {
        return None;
    }
    Some([0, 1, 2, 3].map(|channel| f32::from(source[at + channel]) / 255.0))
}

/// Sums over a part of the picture, each pixel's colour times a weight and
/// the weight, so the average over any box inside it costs four reads.
struct Sums {
    rect: Rect,
    data: Vec<[f64; 4]>,
}

impl Sums {
    /// Over each part's rect, each pixel weighted by its alpha, times
    /// what the stroke leaves of it when the part has the stroke's
    /// coverage. Colours are summed as bytes and scaled once, in `mean`.
    /// Only the rows the part's `reach` says are read in each column are
    /// summed, each band of columns from its first such row, as a box's
    /// sum takes the same column's sum away at both its rows, so where the
    /// band starts makes no difference. One band of columns per share of
    /// the cores sums down its columns, each row starting from what the
    /// bands to its left hold in that row, worked out first in bands of
    /// rows; every part side by side. So each sum is written once, and adds
    /// as one thread would but for where it starts, which can move it by a
    /// rounding of the last bit.
    fn made(source: &[u8], width: u32, parts: &[Part<'_>]) -> Vec<Sums> {
        let cores = parts.iter().map(|part| shares(part.rect)).max().unwrap_or(1);
        // How wide each part's bands of columns are, and the rows each band
        // sums, first and last, which are clear when it sums none.
        let widths: Vec<usize> = parts.iter().map(|part| (part.rect.w as usize).div_ceil(cores).max(1)).collect();
        let spans: Vec<Vec<[usize; 2]>> = parts
            .iter()
            .zip(&widths)
            .map(|(part, wide)| {
                (0..cores)
                    .map(|band| {
                        // The band's columns, after the first, which is clear.
                        let columns = (1 + band * wide).min(part.reach.len())..(1 + (band + 1) * wide).min(part.reach.len());
                        part.reach[columns].iter().fold([usize::MAX, 0], |[first, last], reach| [first.min(reach[0]), last.max(reach[1])])
                    })
                    .collect()
            })
            .collect();
        // What each band of each row holds before it, for the rows a band
        // to its right sums from.
        let mut befores: Vec<Vec<[f64; 4]>> = parts.iter().map(|part| vec![[0.0; 4]; part.rect.h as usize * cores]).collect();
        {
            let mut pieces: Vec<Vec<(usize, usize, &mut [[f64; 4]])>> = (0..cores).map(|_| Vec::new()).collect();
            for (index, (before, part)) in befores.iter_mut().zip(parts).enumerate() {
                let band = (part.rect.h as usize).div_ceil(cores).max(1);
                for (share, rows) in before.chunks_mut(band * cores).enumerate() {
                    pieces[share].push((index, share * band, rows));
                }
            }
            run_apart(pieces, |_, pieces| {
                for (index, first, rows) in pieces {
                    let (part, wide) = (&parts[index], widths[index]);
                    for (local, row) in rows.chunks_exact_mut(cores).enumerate() {
                        let y = first + local;
                        let needed = spans[index].iter().rposition(|[from, to]| *from <= y && y < *to).map_or(0, |band| band + 1);
                        let py = part.rect.y + y as i32;
                        let mut sum = [0.0f64; 4];
                        for (band, before) in row.iter_mut().enumerate().take(needed) {
                            *before = sum;
                            let x0 = (band * wide).min(part.rect.w as usize);
                            let x1 = ((band + 1) * wide).min(part.rect.w as usize);
                            for add in terms(source, part.coverage, width, part.rect.x + x0 as i32, py, x1 - x0) {
                                for channel in 0..4 {
                                    sum[channel] += add[channel];
                                }
                            }
                        }
                    }
                }
            });
        }
        // Into the memory the last sums used, as a long stroke's are large.
        let mut tables: Vec<Vec<[f64; 4]>> = parts
            .iter()
            .map(|part| {
                // Every sum read is written, so it is only ever grown.
                let mut data = SUMS.lock().unwrap_or_else(PoisonError::into_inner).pop().unwrap_or_default();
                let size = (part.rect.w as usize + 1) * (part.rect.h as usize + 1);
                if data.len() < size {
                    data.resize(size, [0.0; 4]);
                }
                data
            })
            .collect();
        // Each band of columns of every part, its own part of the rows it
        // sums; the first column clear.
        let mut pieces: Vec<Vec<(usize, usize, Vec<&mut [[f64; 4]]>)>> = (0..cores).map(|_| Vec::new()).collect();
        for (index, (data, part)) in tables.iter_mut().zip(parts).enumerate() {
            let mut own: Vec<Vec<&mut [[f64; 4]]>> = (0..cores).map(|_| Vec::new()).collect();
            let columns = part.rect.w as usize + 1;
            for (y, line) in data[..columns * (part.rect.h as usize + 1)].chunks_exact_mut(columns).enumerate() {
                let (first, rest) = line.split_at_mut(1);
                first[0] = [0.0; 4];
                for (band, piece) in rest.chunks_mut(widths[index]).enumerate() {
                    let [from, to] = spans[index][band];
                    if from <= y && y <= to {
                        own[band].push(piece);
                    }
                }
            }
            for (band, lines) in own.into_iter().enumerate().filter(|(_, lines)| !lines.is_empty()) {
                pieces[band].push((index, spans[index][band][0], lines));
            }
        }
        run_apart(pieces, |band, pieces| {
            for (index, from, mut lines) in pieces {
                let part = &parts[index];
                let x0 = band * widths[index];
                lines[0].fill([0.0; 4]);
                for line in 1..lines.len() {
                    let (above, below) = lines.split_at_mut(line);
                    let y = from + line - 1;
                    let mut row = befores[index][y * cores + band];
                    let wide = below[0].len();
                    let added = terms(source, part.coverage, width, part.rect.x + x0 as i32, part.rect.y + y as i32, wide);
                    for ((cell, up), add) in below[0].iter_mut().zip(above[line - 1].iter()).zip(added) {
                        for channel in 0..4 {
                            row[channel] += add[channel];
                        }
                        *cell = [up[0] + row[0], up[1] + row[1], up[2] + row[2], up[3] + row[3]];
                    }
                }
            }
        });
        tables.into_iter().zip(parts).map(|(data, part)| Sums { rect: part.rect, data }).collect()
    }

    /// Over `rect`, as [`Sums::made`] does, on one thread, a row at a time:
    /// what the bands must equal.
    #[cfg(test)]
    fn alone(source: &[u8], width: u32, rect: Rect, weight: impl Fn(i32, i32) -> f64) -> Self {
        let columns = rect.w as usize + 1;
        let mut data = vec![[0.0f64; 4]; columns * (rect.h as usize + 1)];
        for y in 0..rect.h as usize {
            let mut row = [0.0f64; 4];
            let py = rect.y + y as i32;
            let line = py as usize * width as usize;
            for x in 0..rect.w as usize {
                let px = rect.x + x as i32;
                let at = (line + px as usize) * 4;
                let w = weight(px, py) * f64::from(source[at + 3]);
                row[0] += w * f64::from(source[at]);
                row[1] += w * f64::from(source[at + 1]);
                row[2] += w * f64::from(source[at + 2]);
                row[3] += w;
                let above = data[y * columns + x + 1];
                data[(y + 1) * columns + x + 1] = [above[0] + row[0], above[1] + row[1], above[2] + row[2], above[3] + row[3]];
            }
        }
        Self { rect, data }
    }

    /// The weighted average colour within `radius` of a pixel, and the
    /// weight it rests on.
    fn mean(&self, x: i32, y: i32, radius: i32) -> ([f64; 3], f64) {
        let x0 = (x - radius).max(self.rect.x) - self.rect.x;
        let y0 = (y - radius).max(self.rect.y) - self.rect.y;
        let x1 = (x + radius + 1).min(self.rect.right()) - self.rect.x;
        let y1 = (y + radius + 1).min(self.rect.bottom()) - self.rect.y;
        if x1 <= x0 || y1 <= y0 {
            return ([0.0; 3], 0.0);
        }
        let columns = self.rect.w as usize + 1;
        let get = |x: i32, y: i32| self.data[y as usize * columns + x as usize];
        let (a, b, c, d) = (get(x1, y1), get(x0, y1), get(x1, y0), get(x0, y0));
        let sum = [0, 1, 2, 3].map(|channel| a[channel] - b[channel] - c[channel] + d[channel]);
        if sum[3] <= 1e-9 {
            return ([0.0; 3], 0.0);
        }
        // The colours were summed as bytes, the weight with alpha as a byte.
        let scale = 255.0 * sum[3];
        ([sum[0] / scale, sum[1] / scale, sum[2] / scale], sum[3] / 255.0)
    }
}

impl Drop for Sums {
    fn drop(&mut self) {
        let mut kept = SUMS.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.len() < 2 {
            kept.push(std::mem::take(&mut self.data));
        }
    }
}

/// A part of the picture to sum: its rect, the stroke's coverage when the
/// stroke is left out, and in each column of the sums, the first and last
/// rows a box's average reads there.
struct Part<'a> {
    rect: Rect,
    coverage: Option<&'a [f32]>,
    reach: Vec<[usize; 2]>,
}

/// Each pixel's colour times its weight, and the weight, along row `py` of
/// the picture from column `x`, `count` pixels: the weight is its alpha,
/// times what the stroke leaves of it when `coverage` is given.
fn terms<'a>(source: &'a [u8], coverage: Option<&'a [f32]>, width: u32, x: i32, py: i32, count: usize) -> impl Iterator<Item = [f64; 4]> + 'a {
    let start = py as usize * width as usize + x as usize;
    let left = coverage.map(|coverage| &coverage[start..start + count]);
    source[start * 4..(start + count) * 4].chunks_exact(4).enumerate().map(move |(index, pixel)| {
        let kept = left.map_or(1.0, |left| 1.0 - f64::from(left[index]).clamp(0.0, 1.0));
        let w = kept * f64::from(pixel[3]);
        [w * f64::from(pixel[0]), w * f64::from(pixel[1]), w * f64::from(pixel[2]), w]
    })
}

/// The memory of the last sums, kept for the next: healing makes two.
static SUMS: Mutex<Vec<Vec<[f64; 4]>>> = Mutex::new(Vec::new());

/// What the healing brush paints at each pixel `p` of an area: the
/// source's detail and the light around where it lands,
/// `src(p + o) − blur(src)(p + o) + light(p)`, where `light` is the
/// destination's blur with the stroke's own pixels left out, as `coverage`
/// (the picture's size, row by row) says how much each is the stroke's.
/// Both blurs are normalised boxes of `radius`; the source and the
/// destination are the same picture, as the stroke began. Only the region
/// the area and its source reach, with the blur's reach around them, is
/// summed.
pub struct Healer<'a> {
    source: &'a [u8],
    width: u32,
    height: u32,
    coverage: &'a [f32],
    offset: [i32; 2],
    radius: i32,
    light: Option<Sums>,
    blurred: Option<Sums>,
}

impl<'a> Healer<'a> {
    pub fn new(source: &'a [u8], width: u32, height: u32, coverage: &'a [f32], area: Rect, offset: [i32; 2], radius: i32) -> Self {
        let radius = radius.max(1);
        let picture = Rect::new(0, 0, width as i32, height as i32);
        let grown = |rect: Rect| Rect::new(rect.x - radius, rect.y - radius, rect.w + 2 * radius, rect.h + 2 * radius).intersect(&picture);
        let around = grown(area);
        let moved = grown(Rect::new(area.x + offset[0], area.y + offset[1], area.w, area.h));
        // The rows the stroke reaches in each column of the area: only the
        // sums its pixels' averages read are made.
        let mut reached = vec![[i32::MAX, i32::MIN]; area.w.max(0) as usize];
        for y in area.y..area.bottom() {
            let start = y as usize * width as usize + area.x as usize;
            for (rows, covered) in reached.iter_mut().zip(&coverage[start..start + area.w as usize]) {
                if *covered > 0.0 {
                    *rows = [rows[0].min(y), y];
                }
            }
        }
        // In each column of the sums over `rect`, the first and last rows
        // the averages around the reached pixels, moved by `by`, read.
        let reach = |rect: Rect, by: [i32; 2]| {
            let mut reach = vec![[usize::MAX, 0]; rect.w as usize + 1];
            for (column, [top, bottom]) in reached.iter().enumerate().filter(|(_, rows)| rows[0] <= rows[1]) {
                let x = area.x + column as i32 + by[0];
                let first = ((top + by[1] - radius).clamp(rect.y, rect.bottom()) - rect.y) as usize;
                let last = ((bottom + by[1] + radius + 1).clamp(rect.y, rect.bottom()) - rect.y) as usize;
                for corner in [x - radius, x + radius + 1] {
                    let rows = &mut reach[(corner.clamp(rect.x, rect.right()) - rect.x) as usize];
                    *rows = [rows[0].min(first), rows[1].max(last)];
                }
            }
            reach
        };
        let mut parts = Vec::new();
        if !around.is_empty() {
            parts.push(Part { rect: around, coverage: Some(coverage), reach: reach(around, [0, 0]) });
        }
        if !moved.is_empty() {
            parts.push(Part { rect: moved, coverage: None, reach: reach(moved, offset) });
        }
        let mut made = Sums::made(source, width, &parts).into_iter();
        let light = if around.is_empty() { None } else { made.next() };
        let blurred = if moved.is_empty() { None } else { made.next() };
        Self { source, width, height, coverage, offset, radius, light, blurred }
    }

    /// What it paints at a pixel: nothing where the stroke does not reach
    /// or the source has nothing.
    pub fn colour(&self, x: i32, y: i32) -> Option<[f32; 4]> {
        // A pixel the stroke does not reach is not painted, so its colour
        // is not worked out.
        if outside(self.coverage, self.width, x, y) >= 1.0 {
            return None;
        }
        let (sx, sy) = (x + self.offset[0], y + self.offset[1]);
        let own = sample(self.source, self.width, self.height, sx, sy)?;
        let (detail_base, _) = self.blurred.as_ref().map_or(([0.0; 3], 0.0), |sums| sums.mean(sx, sy, self.radius));
        let (lit, weight) = self.light.as_ref().map_or(([0.0; 3], 0.0), |sums| sums.mean(x, y, self.radius));
        Some(lit_colour(own, detail_base, lit, weight))
    }
}

/// How much of a pixel the stroke leaves out: 1 where it does not reach.
fn outside(coverage: &[f32], width: u32, x: i32, y: i32) -> f64 {
    1.0 - f64::from(coverage[y as usize * width as usize + x as usize]).clamp(0.0, 1.0)
}

/// A healed pixel: the source's own colour less its blur, plus the light.
fn lit_colour(own: [f32; 4], detail_base: [f64; 3], lit: [f64; 3], weight: f64) -> [f32; 4] {
    // With the whole window under the stroke, nothing is left to light it,
    // and the source's own light stays.
    let lit = if weight > 0.5 { lit } else { detail_base };
    let mut colour = own;
    for channel in 0..3 {
        colour[channel] = (f64::from(own[channel]) - detail_base[channel] + lit[channel]).clamp(0.0, 1.0) as f32;
    }
    colour
}

/// What [`Healer`] paints at each pixel of `area`, on one thread, a row at
/// a time, as healing was first made: what the bands must equal.
#[cfg(test)]
pub fn heal_colours_alone(source: &[u8], width: u32, height: u32, coverage: &[f32], area: Rect, offset: [i32; 2], radius: i32) -> Vec<Option<[f32; 4]>> {
    let radius = radius.max(1);
    let picture = Rect::new(0, 0, width as i32, height as i32);
    let grown = |rect: Rect| Rect::new(rect.x - radius, rect.y - radius, rect.w + 2 * radius, rect.h + 2 * radius).intersect(&picture);
    let around = grown(area);
    let moved = grown(Rect::new(area.x + offset[0], area.y + offset[1], area.w, area.h));
    let light = (!around.is_empty()).then(|| Sums::alone(source, width, around, |x, y| outside(coverage, width, x, y)));
    let blurred = (!moved.is_empty()).then(|| Sums::alone(source, width, moved, |_, _| 1.0));
    let mut out = Vec::with_capacity(area.w as usize * area.h as usize);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if outside(coverage, width, x, y) >= 1.0 {
                out.push(None);
                continue;
            }
            let (sx, sy) = (x + offset[0], y + offset[1]);
            let Some(own) = sample(source, width, height, sx, sy) else {
                out.push(None);
                continue;
            };
            let (detail_base, _) = blurred.as_ref().map_or(([0.0; 3], 0.0), |sums| sums.mean(sx, sy, radius));
            let (lit, weight) = light.as_ref().map_or(([0.0; 3], 0.0), |sums| sums.mean(x, y, radius));
            out.push(Some(lit_colour(own, detail_base, lit, weight)));
        }
    }
    out
}


/// How a new selection shape joins the one there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    New,
    Add,
    Subtract,
    Intersect,
}

impl Combine {
    pub const ALL: [Combine; 4] = [Combine::New, Combine::Add, Combine::Subtract, Combine::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            Combine::New => "New",
            Combine::Add => "Add",
            Combine::Subtract => "Subtract",
            Combine::Intersect => "Intersect",
        }
    }
}

/// The selection: how much of each picture pixel is selected, 0 to 255.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Mask {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize],
        }
    }

    pub fn at(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    pub fn is_clear(&self) -> bool {
        self.data.iter().all(|value| *value == 0)
    }

    /// A rectangle, corner to corner, of the pixels whose centres it holds.
    pub fn rect(width: u32, height: u32, from: [f64; 2], to: [f64; 2]) -> Self {
        let mut mask = Self::empty(width, height);
        let x0 = from[0].min(to[0]).round().clamp(0.0, f64::from(width)) as usize;
        let x1 = from[0].max(to[0]).round().clamp(0.0, f64::from(width)) as usize;
        let y0 = from[1].min(to[1]).round().clamp(0.0, f64::from(height)) as usize;
        let y1 = from[1].max(to[1]).round().clamp(0.0, f64::from(height)) as usize;
        for y in y0..y1 {
            mask.data[y * width as usize + x0..y * width as usize + x1].fill(255);
        }
        mask
    }

    /// A freehand outline, closed by its last point: the pixels whose
    /// centres it holds, by the even-odd rule.
    pub fn lasso(width: u32, height: u32, points: &[[f64; 2]]) -> Self {
        let mut mask = Self::empty(width, height);
        let points: Vec<[f64; 2]> = points.iter().copied().filter(|point| point[0].is_finite() && point[1].is_finite()).collect();
        if points.len() < 3 {
            return mask;
        }
        let top = points.iter().map(|point| point[1]).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
        let bottom = points.iter().map(|point| point[1]).fold(f64::NEG_INFINITY, f64::max).ceil().min(f64::from(height)) as usize;
        let mut crossings = Vec::new();
        for y in top..bottom {
            let center = y as f64 + 0.5;
            crossings.clear();
            for index in 0..points.len() {
                let a = points[index];
                let b = points[(index + 1) % points.len()];
                if (a[1] <= center) != (b[1] <= center) {
                    crossings.push(a[0] + (center - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            crossings.sort_by(|a, b| a.total_cmp(b));
            for pair in crossings.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                // Pixels whose centre lies between the two crossings.
                let x0 = (pair[0] - 0.5).ceil().clamp(0.0, f64::from(width)) as usize;
                let x1 = ((pair[1] - 0.5).ceil()).clamp(0.0, f64::from(width)) as usize;
                if x1 > x0 {
                    mask.data[y * width as usize + x0..y * width as usize + x1].fill(255);
                }
            }
        }
        mask
    }

    /// Softens the edge over about `radius` pixels: three box blurs, which
    /// come close to a Gaussian.
    pub fn feather(&mut self, radius: f64) {
        if !radius.is_finite() || radius < 0.5 {
            return;
        }
        let pass = ((radius / 3.0).ceil() as usize).max(1);
        let width = self.width as usize;
        let height = self.height as usize;
        let mut values: Vec<f32> = self.data.iter().map(|value| f32::from(*value)).collect();
        let mut scratch = vec![0.0f32; width.max(height)];
        for _ in 0..3 {
            for y in 0..height {
                box_line(&mut values[y * width..(y + 1) * width], 1, width, pass, &mut scratch);
            }
            for x in 0..width {
                box_line(&mut values[x..], width, height, pass, &mut scratch);
            }
        }
        for (byte, value) in self.data.iter_mut().zip(values) {
            *byte = value.round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Joins `shape` into this selection.
    pub fn combine(&mut self, shape: &Mask, how: Combine) {
        for (value, other) in self.data.iter_mut().zip(shape.data.iter()) {
            *value = match how {
                Combine::New => *other,
                Combine::Add => (*value).max(*other),
                Combine::Subtract => (*value).min(255 - *other),
                Combine::Intersect => (*value).min(*other),
            };
        }
    }

    pub fn invert(&mut self) {
        for value in &mut self.data {
            *value = 255 - *value;
        }
    }

    /// The smallest rectangle holding every selected pixel.
    pub fn bounds(&self) -> Rect {
        let width = self.width as usize;
        let mut rect = Rect::new(0, 0, 0, 0);
        for (y, row) in self.data.chunks(width.max(1)).enumerate() {
            let Some(first) = row.iter().position(|value| *value > 0) else {
                continue;
            };
            let last = row.iter().rposition(|value| *value > 0).unwrap_or(first);
            rect = rect.union(&Rect::new(first as i32, y as i32, (last - first + 1) as i32, 1));
        }
        rect
    }

    /// The same selection moved by `dx` and `dy` pixels, as moving the
    /// selected pixels moves it.
    pub fn shifted(&self, dx: i32, dy: i32) -> Mask {
        let mut out = Mask::empty(self.width, self.height);
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                out.data[y as usize * self.width as usize + x as usize] = self.at(x - dx, y - dy);
            }
        }
        out
    }

    /// A part of the mask, as crop keeps it.
    pub fn cut(&self, rect: Rect) -> Mask {
        let mut out = Mask::empty(rect.w as u32, rect.h as u32);
        for y in 0..rect.h {
            for x in 0..rect.w {
                out.data[y as usize * rect.w as usize + x as usize] = self.at(rect.x + x, rect.y + y);
            }
        }
        out
    }

    /// The edges between selected and unselected pixels, at half coverage,
    /// as closed outlines in picture pixels.
    pub fn outlines(&self) -> Vec<Vec<[f64; 2]>> {
        let inside = |x: i32, y: i32| self.at(x, y) >= 128;
        // Each edge runs clockwise around the selected pixels, keyed by
        // where it starts.
        let mut edges: std::collections::HashMap<(i32, i32), Vec<(i32, i32)>> = std::collections::HashMap::new();
        let mut count = 0usize;
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                if !inside(x, y) {
                    continue;
                }
                let mut push = |from: (i32, i32), to: (i32, i32)| {
                    edges.entry(from).or_default().push(to);
                    count += 1;
                };
                if !inside(x, y - 1) {
                    push((x, y), (x + 1, y));
                }
                if !inside(x + 1, y) {
                    push((x + 1, y), (x + 1, y + 1));
                }
                if !inside(x, y + 1) {
                    push((x + 1, y + 1), (x, y + 1));
                }
                if !inside(x - 1, y) {
                    push((x, y + 1), (x, y));
                }
            }
        }
        let mut loops = Vec::new();
        while count > 0 {
            let Some((&start, _)) = edges.iter().find(|(_, ends)| !ends.is_empty()) else {
                break;
            };
            let mut points = vec![start];
            let mut at = start;
            loop {
                let Some(next) = edges.get_mut(&at).and_then(|ends| ends.pop()) else {
                    break;
                };
                count -= 1;
                if next == start {
                    break;
                }
                points.push(next);
                at = next;
            }
            loops.push(simplify(&points));
        }
        loops
    }
}

/// One row or column of a box blur, in place: `len` values `stride` apart.
fn box_line(values: &mut [f32], stride: usize, len: usize, radius: usize, scratch: &mut [f32]) {
    let get = |values: &[f32], index: isize| -> f32 {
        if index < 0 || index as usize >= len { 0.0 } else { values[index as usize * stride] }
    };
    let window = (2 * radius + 1) as f32;
    let mut sum = 0.0;
    for index in -(radius as isize)..=(radius as isize) {
        sum += get(values, index);
    }
    for index in 0..len {
        scratch[index] = sum / window;
        sum += get(values, index as isize + radius as isize + 1);
        sum -= get(values, index as isize - radius as isize);
    }
    for index in 0..len {
        values[index * stride] = scratch[index];
    }
}

/// An outline without the corners that lie on a straight run.
fn simplify(points: &[(i32, i32)]) -> Vec<[f64; 2]> {
    let count = points.len();
    let mut out = Vec::new();
    for index in 0..count {
        let before = points[(index + count - 1) % count];
        let here = points[index];
        let after = points[(index + 1) % count];
        let straight = (here.0 - before.0) * (after.1 - here.1) == (here.1 - before.1) * (after.0 - here.0);
        if !straight {
            out.push([f64::from(here.0), f64::from(here.1)]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush(size: f64, hardness: f64, opacity: f64) -> Brush {
        Brush { rgb: [255, 0, 0], size, hardness, opacity, pressure: true, erase: false }
    }

    /// Catches a stroke that builds past its opacity where dabs overlap.
    #[test]
    fn one_stroke_never_builds_past_its_opacity() {
        let mut pixels = Pixels::filled(40, 10, [255, 255, 255, 255]);
        let stroke = [[2.0, 5.0, 1.0], [38.0, 5.0, 1.0], [2.0, 5.0, 1.0]];
        let coverage = stroke_coverage(&stroke, &brush(6.0, 1.0, 0.5)).unwrap();
        apply(&mut pixels, [0, 0], &coverage, Rect::new(0, 0, 40, 10), None, &brush(6.0, 1.0, 0.5), false).unwrap();
        let at = (5 * 40 + 20) * 4;
        // Half red over white: 255, 128, 128.
        assert_eq!(&pixels.data[at..at + 4], &[255, 128, 128, 255]);

        // A soft stroke's fading edge is no stronger than one dab's there:
        // the dabs along it do not add up.
        let soft = brush(12.0, 0.0, 1.0);
        let value = |coverage: &Coverage, x: i32, y: i32| {
            coverage.values[((y - coverage.rect.y) * coverage.rect.w + (x - coverage.rect.x)) as usize]
        };
        let dab = stroke_coverage(&[[20.0, 20.0, 1.0]], &soft).unwrap();
        let line = stroke_coverage(&[[5.0, 20.0, 1.0], [35.0, 20.0, 1.0]], &soft).unwrap();
        let single = value(&dab, 20, 23);
        assert!(single > 0.05 && single < 0.9, "{single}");
        assert!(value(&line, 20, 23) <= single + 0.05, "{} against one dab's {single}", value(&line, 20, 23));
    }

    /// Catches pressure ignored: a light press makes a smaller dab.
    #[test]
    fn a_hard_edge_has_no_ridges() {
        // A long straight stroke drawn from few points: every column across
        // it covers the same, as a smooth edge does. Dabs a quarter of the
        // size apart left a ripple along the edge.
        let coverage = stroke_coverage(&[[10.0, 50.0, 1.0], [210.0, 50.3, 1.0]], &brush(41.0, 0.8, 1.0)).unwrap();
        let at = |x: i32, y: i32| coverage.values[((y - coverage.rect.y) * coverage.rect.w + (x - coverage.rect.x)) as usize];
        for y in [30, 31, 69, 70] {
            let row: Vec<f32> = (60..160).map(|x| at(x, y)).collect();
            let low = row.iter().cloned().fold(f32::MAX, f32::min);
            let high = row.iter().cloned().fold(f32::MIN, f32::max);
            assert!(high - low < 0.08, "row {y} ripples from {low} to {high}");
        }
    }

    #[test]
    fn pressure_scales_the_size() {
        let full = stroke_coverage(&[[20.0, 20.0, 1.0]], &brush(20.0, 1.0, 1.0)).unwrap();
        let light = stroke_coverage(&[[20.0, 20.0, 0.25]], &brush(20.0, 1.0, 1.0)).unwrap();
        let covered = |coverage: &Coverage| coverage.values.iter().filter(|value| **value > 0.5).count();
        assert!(covered(&light) * 8 < covered(&full), "{} vs {}", covered(&light), covered(&full));
        let mut flat = brush(20.0, 1.0, 1.0);
        flat.pressure = false;
        let ignored = stroke_coverage(&[[20.0, 20.0, 0.25]], &flat).unwrap();
        assert_eq!(covered(&ignored), covered(&full));
    }

    /// Catches hardness ignored: a soft brush's dab fades toward its edge.
    #[test]
    fn hardness_softens_the_edge() {
        let hard = stroke_coverage(&[[20.0, 20.0, 1.0]], &brush(20.0, 1.0, 1.0)).unwrap();
        let soft = stroke_coverage(&[[20.0, 20.0, 1.0]], &brush(20.0, 0.0, 1.0)).unwrap();
        let near_edge = |coverage: &Coverage| {
            let x = 27 - coverage.rect.x;
            let y = 20 - coverage.rect.y;
            coverage.values[y as usize * coverage.rect.w as usize + x as usize]
        };
        assert_eq!(near_edge(&hard), 1.0);
        assert!(near_edge(&soft) < 0.3, "{}", near_edge(&soft));
    }

    /// Catches a lasso filled by the wrong rule, and an outline that is not
    /// closed around the selection.
    #[test]
    fn lasso_and_outline() {
        let mask = Mask::lasso(20, 20, &[[2.0, 2.0], [12.0, 2.0], [12.0, 8.0], [2.0, 8.0]]);
        assert_eq!(mask.data.iter().filter(|value| **value == 255).count(), 60);
        assert_eq!(mask.at(2, 2), 255);
        assert_eq!(mask.at(12, 2), 0);
        let outlines = mask.outlines();
        assert_eq!(outlines.len(), 1);
        let mut corners = outlines[0].clone();
        corners.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        assert_eq!(corners, vec![[2.0, 2.0], [2.0, 8.0], [12.0, 2.0], [12.0, 8.0]]);
    }

    /// Catches the four ways of joining a selection mixed up, and a feather
    /// that does nothing.
    #[test]
    fn combine_and_feather() {
        let left = Mask::rect(10, 1, [0.0, 0.0], [6.0, 1.0]);
        let right = Mask::rect(10, 1, [4.0, 0.0], [10.0, 1.0]);
        let ones = |mask: &Mask| mask.data.iter().map(|value| u8::from(*value == 255)).collect::<Vec<_>>();
        let mut add = left.clone();
        add.combine(&right, Combine::Add);
        assert_eq!(ones(&add), vec![1; 10]);
        let mut subtract = left.clone();
        subtract.combine(&right, Combine::Subtract);
        assert_eq!(ones(&subtract), vec![1, 1, 1, 1, 0, 0, 0, 0, 0, 0]);
        let mut intersect = left.clone();
        intersect.combine(&right, Combine::Intersect);
        assert_eq!(ones(&intersect), vec![0, 0, 0, 0, 1, 1, 0, 0, 0, 0]);
        let mut new = left.clone();
        new.combine(&right, Combine::New);
        assert_eq!(new, right);
        let mut soft = Mask::rect(40, 40, [10.0, 10.0], [30.0, 30.0]);
        soft.feather(6.0);
        assert_eq!(soft.at(20, 20), 255);
        let edge = soft.at(10, 20);
        assert!(edge > 60 && edge < 200, "{edge}");
        assert!(soft.at(8, 20) > 0);
    }

    /// Catches healing in bands that paints otherwise than healing a row at
    /// a time: a long wavy stroke healed over a busy, partly clear picture,
    /// large enough to be split among the cores, paints within one level of
    /// what the sums made on one thread paint, and paints.
    #[test]
    fn healing_in_bands_paints_as_healing_alone() {
        let (width, height) = (700u32, 420u32);
        let mut picture = Pixels::filled(width, height, [0, 0, 0, 0]);
        for (index, pixel) in picture.data.chunks_exact_mut(4).enumerate() {
            let (x, y) = (index % width as usize, index / width as usize);
            let alpha = if (x / 50 + y / 40) % 7 == 0 { 0 } else { 255 - (x % 90) as u8 };
            pixel.copy_from_slice(&[(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x * y) % 251) as u8, alpha]);
        }
        let points: Vec<[f64; 3]> = (0..=160).map(|k| [30.0 + 4.0 * f64::from(k), 260.0 + 110.0 * (f64::from(k) / 16.0).sin(), 1.0]).collect();
        let stroke = stroke_coverage(&points, &Brush { size: 60.0, hardness: 0.5, ..brush(60.0, 0.5, 1.0) }).unwrap();
        let area = stroke.rect.intersect(&Rect::new(0, 0, width as i32, height as i32));
        let mut coverage = vec![0.0f32; (width * height) as usize];
        let mut values = Vec::new();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let value = stroke.values[((y - stroke.rect.y) * stroke.rect.w + x - stroke.rect.x) as usize];
                coverage[(y as u32 * width + x as u32) as usize] = value;
                values.push(value);
            }
        }
        assert!(area.w * area.h > 65_536, "the stroke is too small to be split");
        let (offset, radius) = ([0, -150], 30);
        let colours = heal_colours_alone(&picture.data, width, height, &coverage, area, offset, radius);
        let mut alone = picture.clone();
        apply_colours(&mut alone, [0, 0], area, &values, &colours, None, 1.0, false);
        // The memory sums reuse holds the last stroke's; any of it read
        // and not written again would paint here.
        {
            let mut kept = SUMS.lock().unwrap_or_else(PoisonError::into_inner);
            kept.push(vec![[f64::NAN; 4]; ((width + 1) * (height + 1)) as usize]);
            kept.push(vec![[f64::NAN; 4]; ((width + 1) * (height + 1)) as usize]);
        }
        let healer = Healer::new(&picture.data, width, height, &coverage, area, offset, radius);
        let mut banded = picture.clone();
        apply_with(&mut banded, [0, 0], area, &values, |x, y| healer.colour(x, y), None, 1.0, false);
        assert!(alone.data != picture.data, "healing painted nothing");
        let worst = banded.data.iter().zip(&alone.data).map(|(banded, alone)| banded.abs_diff(*alone)).max().unwrap();
        let differ = banded.data.iter().zip(&alone.data).filter(|(banded, alone)| banded != alone).count();
        assert!(worst <= 1, "healing in bands differs from healing alone by up to {worst} levels, in {differ} bytes");
    }
}
