//! Raw RGBA image files a workspace writes for the host to draw: a stage's
//! decoded frame, an image on a sheet, a timeline item's thumbnail. The
//! workspace decodes; the host only reads `width × height × 4` bytes, RGBA
//! with straight alpha, rows top to bottom, as raster tabs write theirs, and
//! keeps one texture for each file until its revision changes.

use eframe::egui;
use protocol::Thumbnail;

/// The texture for `image`, read again only when its revision changed.
/// None when the file is missing or not the size it says.
#[allow(dead_code)] // The stage and the timeline draw with it, once their drawing lands.
pub(crate) fn texture(ctx: &egui::Context, image: &Thumbnail) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("frame-file", &image.path));
    if let Some((revision, texture)) = ctx.data(|data| data.get_temp::<(u64, egui::TextureHandle)>(id))
        && revision == image.revision
    {
        return Some(texture);
    }
    let pixels = read(&image.path, image.width, image.height)?;
    let texture = ctx.load_texture(format!("frame:{}", image.path), pixels, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, (image.revision, texture.clone())));
    Some(texture)
}

/// Reads a whole raw RGBA file of `width × height` pixels.
pub(crate) fn read(path: &str, width: u32, height: u32) -> Option<egui::ColorImage> {
    let (width, height) = (width as usize, height as usize);
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if expected == 0 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() == expected).then(|| egui::ColorImage::from_rgba_unmultiplied([width, height], &bytes))
}

#[cfg(test)]
mod tests {
    /// Catches a frame read from a file of the wrong size, such as one cut
    /// short while it was written.
    #[test]
    fn a_frame_is_read_only_when_it_is_whole() {
        let dir = std::env::temp_dir().join(format!("frames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.rgba");
        std::fs::write(&path, [255u8; 2 * 2 * 4]).unwrap();
        let path = path.to_string_lossy();
        assert!(super::read(&path, 2, 2).is_some());
        assert!(super::read(&path, 3, 2).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
