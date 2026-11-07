//! Raw RGBA image files a workspace writes for the host to draw: a stage's
//! decoded frame, an image on a sheet, a timeline item's thumbnail. The
//! workspace decodes; the host only reads `width × height × 4` bytes, RGBA
//! with straight alpha, rows top to bottom, as raster tabs write theirs, and
//! keeps one texture for each file until a newer revision has been read,
//! which happens off the window's thread.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{LazyLock, Mutex};

use eframe::egui;
use protocol::Thumbnail;

/// One file's textures: the one shown, and a read under way of a newer
/// revision, on a thread of its own so the window never waits for it.
struct Loader {
    shown: Option<(u64, egui::TextureHandle)>,
    pending: Option<(u64, Receiver<Option<egui::ColorImage>>)>,
}

static LOADERS: LazyLock<Mutex<HashMap<String, Loader>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The texture for `image`. A new revision is read on a thread, and until
/// it is ready the last one read stays shown, so a frame that takes long to
/// read, such as a 4K frame of a shot being played, never stalls the
/// window. None until the first read of a file has finished, or when the
/// file is missing or not the size it says.
pub(crate) fn texture(ctx: &egui::Context, image: &Thumbnail) -> Option<egui::TextureHandle> {
    let mut loaders = LOADERS.lock().ok()?;
    let loader = loaders.entry(image.path.clone()).or_insert(Loader { shown: None, pending: None });
    if let Some((revision, rx)) = &loader.pending {
        match rx.try_recv() {
            Ok(pixels) => {
                if let Some(pixels) = pixels {
                    let texture = ctx.load_texture(format!("frame:{}", image.path), pixels, egui::TextureOptions::LINEAR);
                    loader.shown = Some((*revision, texture));
                }
                loader.pending = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint(),
            Err(TryRecvError::Disconnected) => loader.pending = None,
        }
    }
    let shown = loader.shown.as_ref().map(|(revision, _)| *revision);
    let asked = loader.pending.as_ref().map(|(revision, _)| *revision);
    if shown != Some(image.revision) && asked != Some(image.revision) && asked.is_none() {
        let (tx, rx) = channel();
        let (path, width, height) = (image.path.clone(), image.width, image.height);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(read(&path, width, height));
            wake.request_repaint();
        });
        loader.pending = Some((image.revision, rx));
    }
    loader.shown.as_ref().map(|(_, texture)| texture.clone())
}

/// The revision of `image` that `texture` shows now, if any.
pub(crate) fn shown_revision(image: &Thumbnail) -> Option<u64> {
    LOADERS.lock().ok()?.get(&image.path)?.shown.as_ref().map(|(revision, _)| *revision)
}

static SHOWN: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static TOLD: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Notes that this frame draws `revision` of `tab`'s image, for `shown`.
/// The window notes a stage's image; `raster.rs` notes its own image as it
/// draws a revision.
pub(crate) fn note_shown(tab: &str, revision: u64) {
    if let Ok(mut shown) = SHOWN.lock() {
        shown.insert(tab.to_string(), revision);
    }
}

/// The revisions drawn since the last call that no workspace was told of
/// yet, each as a tab and its revision.
pub(crate) fn take_shown() -> Vec<(String, u64)> {
    let (Ok(mut shown), Ok(mut told)) = (SHOWN.lock(), TOLD.lock()) else {
        return Vec::new();
    };
    let mut new = Vec::new();
    for (tab, revision) in shown.drain() {
        if told.get(&tab) != Some(&revision) {
            told.insert(tab.clone(), revision);
            new.push((tab, revision));
        }
    }
    new
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
