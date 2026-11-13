//! Frames handed on: as the raw RGBA files the host draws, and as PNG.

use crate::{Result, error};
use ffmpeg_next as ff;
use std::path::{Path, PathBuf};

/// Writes `rgba`, `size.0 × size.1 × 4` bytes, as `dir/name`, the raw file
/// a stage's image and a raster tab name: whole, by a scratch file and a
/// rename, so the host never reads half of one. The caller keeps the
/// revision it sends with the path.
pub fn write_frame(dir: &Path, name: &str, rgba: &[u8], size: (u32, u32)) -> Result<PathBuf> {
    let expected = size.0 as usize * size.1 as usize * 4;
    if rgba.len() != expected {
        return Err(error(format!("A frame of {} × {} needs {expected} bytes, not {}.", size.0, size.1, rgba.len())));
    }
    let path = dir.join(name);
    let scratch = dir.join(format!("{name}.partial"));
    std::fs::write(&scratch, rgba).map_err(|e| error(format!("A frame could not be written to {}: {e}.", dir.display())))?;
    std::fs::rename(&scratch, &path).map_err(|e| error(format!("A frame could not be written to {}: {e}.", dir.display())))?;
    Ok(path)
}

/// Writes `rgba` as a PNG file at `path`, through a scratch file and a
/// rename, with FFmpeg's own PNG encoder.
pub fn write_png(path: &Path, rgba: &[u8], size: (u32, u32)) -> Result<()> {
    crate::init()?;
    let (width, height) = size;
    let failed = || error(format!("{} could not be written as an image.", crate::named(path)));
    if rgba.len() != width as usize * height as usize * 4 || width == 0 || height == 0 {
        return Err(failed());
    }
    let codec = ff::encoder::find(ff::codec::Id::PNG).ok_or_else(failed)?;
    let mut encoder = ff::codec::context::Context::new_with_codec(codec).encoder().video().map_err(|_| failed())?;
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(ff::format::Pixel::RGBA);
    encoder.set_time_base((1, 1));
    let mut encoder = encoder.open().map_err(|_| failed())?;
    let mut frame = ff::frame::Video::new(ff::format::Pixel::RGBA, width, height);
    let stride = frame.stride(0);
    let row = width as usize * 4;
    let data = frame.data_mut(0);
    for y in 0..height as usize {
        data[y * stride..y * stride + row].copy_from_slice(&rgba[y * row..(y + 1) * row]);
    }
    frame.set_pts(Some(0));
    encoder.send_frame(&frame).map_err(|_| failed())?;
    encoder.send_eof().map_err(|_| failed())?;
    let mut bytes = Vec::new();
    let mut packet = ff::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        bytes.extend_from_slice(packet.data().unwrap_or_default());
    }
    if bytes.is_empty() {
        return Err(failed());
    }
    let scratch = path.with_extension("png.partial");
    std::fs::write(&scratch, &bytes).map_err(|_| failed())?;
    std::fs::rename(&scratch, path).map_err(|_| failed())
}
