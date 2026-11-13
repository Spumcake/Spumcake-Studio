//! Converting between YUV 4:2:0 and RGBA by a stream's own colour tag, in
//! bands of rows on every core. FFmpeg's scaler, left at its defaults,
//! converts every stream as BT.601 whatever its tag says; the playback
//! experiment of 2026-11-06 found it nine levels off on BT.709 video.

/// A stream's colour matrix and range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tag {
    pub bt709: bool,
    pub full: bool,
}

impl Tag {
    /// The tag a stream carries, or, when it carries none, BT.709 for
    /// video taller than 576 rows and BT.601 below, as players assume.
    pub fn of(space: ffmpeg_next::color::Space, range: ffmpeg_next::color::Range, height: u32, full_format: bool) -> Self {
        use ffmpeg_next::color::{Range, Space};
        let bt709 = match space {
            Space::BT709 => true,
            Space::BT470BG | Space::SMPTE170M | Space::FCC | Space::SMPTE240M => false,
            _ => height > 576,
        };
        Tag { bt709, full: range == Range::JPEG || full_format }
    }

    /// Red from V, green from U and V, blue from U, as the matrix has them.
    fn weights(self) -> (f64, f64, f64, f64) {
        if self.bt709 { (1.5748, 0.1873, 0.4681, 1.8556) } else { (1.402, 0.3441, 0.7141, 1.772) }
    }
}

/// The three planes of a 4:2:0 image, each with its stride.
pub(crate) struct Planes<'a> {
    pub data: [&'a [u8]; 3],
    pub stride: [usize; 3],
}

const SHIFT: i32 = 16;
const HALF: i32 = 1 << (SHIFT - 1);

fn fixed(value: f64) -> i32 {
    (value * f64::from(1 << SHIFT)).round() as i32
}

/// How many rows each thread takes: even, so a band holds whole chroma
/// rows.
fn band(height: usize) -> usize {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    height.div_ceil(threads).next_multiple_of(2).max(2)
}

/// YUV 4:2:0 to RGBA with straight alpha, every pixel opaque.
pub(crate) fn to_rgba(planes: &Planes, width: usize, height: usize, tag: Tag, out: &mut [u8]) {
    let (rv, gu, gv, bu) = tag.weights();
    let (luma, chroma, black) = if tag.full { (1.0, 1.0, 0) } else { (255.0 / 219.0, 255.0 / 224.0, 16) };
    let (ky, krv, kgu, kgv, kbu) = (fixed(luma), fixed(rv * chroma), fixed(gu * chroma), fixed(gv * chroma), fixed(bu * chroma));
    let clamp = |value: i32| ((value + HALF) >> SHIFT).clamp(0, 255) as u8;
    let band = band(height);
    std::thread::scope(|scope| {
        for (index, chunk) in out[..width * height * 4].chunks_mut(band * width * 4).enumerate() {
            scope.spawn(move || {
                for (row, line) in chunk.chunks_exact_mut(width * 4).enumerate() {
                    let y = index * band + row;
                    let luma_row = &planes.data[0][y * planes.stride[0]..];
                    let u_row = &planes.data[1][(y / 2) * planes.stride[1]..];
                    let v_row = &planes.data[2][(y / 2) * planes.stride[2]..];
                    for (x, pixel) in line.chunks_exact_mut(4).enumerate() {
                        let l = (i32::from(luma_row[x]) - black) * ky;
                        let u = i32::from(u_row[x / 2]) - 128;
                        let v = i32::from(v_row[x / 2]) - 128;
                        pixel[0] = clamp(l + krv * v);
                        pixel[1] = clamp(l - kgu * u - kgv * v);
                        pixel[2] = clamp(l + kbu * u);
                        pixel[3] = 255;
                    }
                }
            });
        }
    });
}

/// RGBA to limited-range BT.709 YUV 4:2:0, each chroma sample the average
/// of its four pixels, as the encoder tags what it writes. Alpha is
/// ignored. `width` and `height` are even.
pub(crate) fn from_rgba(rgba: &[u8], width: usize, height: usize, planes: [&mut [u8]; 3], stride: [usize; 3]) {
    // Kr = 0.2126, Kb = 0.0722.
    let (kr, kb) = (0.2126, 0.0722);
    let kg = 1.0 - kr - kb;
    let luma = 219.0 / 255.0;
    let chroma = 224.0 / 255.0;
    let (yr, yg, yb) = (fixed(kr * luma), fixed(kg * luma), fixed(kb * luma));
    let (ur, ug, ub) = (fixed(-kr / (2.0 * (1.0 - kb)) * chroma), fixed(-kg / (2.0 * (1.0 - kb)) * chroma), fixed(0.5 * chroma));
    let (vr, vg, vb) = (fixed(0.5 * chroma), fixed(-kg / (2.0 * (1.0 - kr)) * chroma), fixed(-kb / (2.0 * (1.0 - kr)) * chroma));
    let band = band(height);
    let [luma_plane, u_plane, v_plane] = planes;
    std::thread::scope(|scope| {
        let lumas = luma_plane.chunks_mut(band * stride[0]);
        let us = u_plane.chunks_mut(band / 2 * stride[1]);
        let vs = v_plane.chunks_mut(band / 2 * stride[2]);
        for (index, ((luma_band, u_band), v_band)) in lumas.zip(us).zip(vs).enumerate() {
            scope.spawn(move || {
                let first = index * band;
                let rows = band.min(height.saturating_sub(first));
                for row in 0..rows {
                    let y = first + row;
                    let source = &rgba[y * width * 4..(y + 1) * width * 4];
                    let line = &mut luma_band[row * stride[0]..row * stride[0] + width];
                    for (x, out) in line.iter_mut().enumerate() {
                        let p = &source[x * 4..x * 4 + 3];
                        let value = yr * i32::from(p[0]) + yg * i32::from(p[1]) + yb * i32::from(p[2]);
                        *out = ((value + HALF) >> SHIFT).clamp(0, 255) as u8 + 16;
                    }
                }
                for row in 0..rows / 2 {
                    let y = first + row * 2;
                    for x in 0..width / 2 {
                        let (mut r, mut g, mut b) = (0, 0, 0);
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let at = ((y + dy) * width + x * 2 + dx) * 4;
                            r += i32::from(rgba[at]);
                            g += i32::from(rgba[at + 1]);
                            b += i32::from(rgba[at + 2]);
                        }
                        let u = (ur * r + ug * g + ub * b + 2 * HALF * 2) >> (SHIFT + 2);
                        let v = (vr * r + vg * g + vb * b + 2 * HALF * 2) >> (SHIFT + 2);
                        u_band[row * stride[1] + x] = (u + 128).clamp(0, 255) as u8;
                        v_band[row * stride[2] + x] = (v + 128).clamp(0, 255) as u8;
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches the two conversions disagreeing: a colour written and read
    /// back by the stream's tag comes back within a level or two.
    #[test]
    fn colours_survive_the_round_trip() {
        let (w, h) = (8usize, 4usize);
        let colours = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [200, 120, 40], [128, 128, 128], [16, 16, 16], [250, 250, 250], [30, 90, 200]];
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = colours[(x / 2 + (y / 2) * 4) % colours.len()];
                rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let (mut l, mut u, mut v) = (vec![0u8; w * h], vec![0u8; w * h / 4], vec![0u8; w * h / 4]);
        from_rgba(&rgba, w, h, [&mut l, &mut u, &mut v], [w, w / 2, w / 2]);
        let mut back = vec![0u8; w * h * 4];
        let planes = Planes { data: [&l, &u, &v], stride: [w, w / 2, w / 2] };
        to_rgba(&planes, w, h, Tag { bt709: true, full: false }, &mut back);
        for (a, b) in rgba.iter().zip(&back) {
            assert!((i32::from(*a) - i32::from(*b)).abs() <= 2, "{rgba:?}\n{back:?}");
        }
    }
}
