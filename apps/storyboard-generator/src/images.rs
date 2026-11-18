//! Image keys' images: a project image, such as a canon keyframe in
//! `library/`, decoded here, never in the host, and fitted whole into the
//! clip's field on white, at the size a frame is drawn at.

use std::path::Path;

/// The image at `file`, relative to `project`, fitted whole and centred on
/// white at `width × height`, as opaque RGBA. None when it cannot be read.
pub fn fitted(project: Option<&Path>, file: &str, (width, height): (u32, u32)) -> Option<Vec<u8>> {
    let path = crate::sounds::resolve(project, file);
    let decoded = image::open(&path).ok()?.to_rgba8();
    let (w, h) = decoded.dimensions();
    if w == 0 || h == 0 || width == 0 || height == 0 {
        return None;
    }
    let scale = (f64::from(width) / f64::from(w)).min(f64::from(height) / f64::from(h));
    let (fit_w, fit_h) = (((f64::from(w) * scale).round() as u32).clamp(1, width), ((f64::from(h) * scale).round() as u32).clamp(1, height));
    let resized = image::imageops::resize(&decoded, fit_w, fit_h, image::imageops::FilterType::Triangle);
    let mut out = vec![255u8; width as usize * height as usize * 4];
    let (left, top) = ((width - fit_w) / 2, (height - fit_h) / 2);
    for (x, y, pixel) in resized.enumerate_pixels() {
        let at = (((top + y) * width + left + x) * 4) as usize;
        // Over white, so the frame stays opaque.
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            out[at + channel] = ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha)) / 255) as u8;
        }
        out[at + 3] = 255;
    }
    Some(out)
}

/// The image a clip shows at its frame: the bottom-most image key of a
/// layer that shows. Every layer's drawing is drawn over it.
pub fn image_at<'a>(clip: &'a crate::film::Clip, frame: u32) -> Option<&'a str> {
    clip.layers.iter().rev().filter(|layer| !layer.folder && clip.shows(layer)).find_map(|layer| layer.image_at(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_fits_whole_on_white() {
        let dir = std::env::temp_dir().join(format!("storyboard-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = image::RgbaImage::from_pixel(40, 10, image::Rgba([255, 0, 0, 255]));
        red.save(dir.join("red.png")).unwrap();
        let red_fitted = fitted(Some(&dir), "red.png", (20, 20)).unwrap();
        // 40 × 10 fits as 20 × 5, centred: rows 7 to 11 are red, the rest white.
        let pixel = |x: usize, y: usize| &red_fitted[(y * 20 + x) * 4..(y * 20 + x) * 4 + 3];
        assert_eq!(pixel(10, 9), [255, 0, 0]);
        assert_eq!(pixel(10, 2), [255, 255, 255]);
        assert!(fitted(Some(&dir), "missing.png", (20, 20)).is_none());
    }
}
