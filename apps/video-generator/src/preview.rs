//! The previews the host asks for with `preview`, as a video reference's
//! row shows it: the video's first frame, small, and its length for the
//! thumbnail's corner, "8.0 s"; and a sound's waveform with its length,
//! and a WAV's cue points. The host never decodes a video or a sound
//! itself.

use protocol::{AppMessage, Cue, Thumbnail};
use std::path::{Path, PathBuf};

/// The media types this workspace edits, as `edits` declares them, one for
/// each of the video files a take can be.
pub const VIDEO_TYPES: [&str; 6] = ["video/mp4", "video/quicktime", "video/x-m4v", "video/x-matroska", "video/webm", "video/x-msvideo"];

/// The sounds this workspace previews, as `previews` declares them beside
/// its videos; it edits none of them.
pub const AUDIO_TYPES: [&str; 3] = ["audio/wav", "audio/mpeg", "audio/flac"];

/// Every type `previews` declares: the videos, then the sounds.
pub fn previewed_types() -> Vec<String> {
    VIDEO_TYPES.iter().chain(AUDIO_TYPES.iter()).map(|kind| kind.to_string()).collect()
}

/// A sound's preview, a waveform as the transport row draws one.
const SOUND_SIZE: [u32; 2] = [160, 32];

/// The largest a preview is, as the Image Generator's: a reference row's
/// thumbnail, drawn sharp at twice its size.
const PREVIEW_SIZE: [u32; 2] = [160, 96];

/// The answer to `preview` for the file at `path`, read from `root`, the
/// project, unless it is whole. The frame is written as a raw image in
/// `cache`, named by the path, so asking again replaces it, with the next
/// revision.
pub fn preview(id: String, path: &str, media_type: &str, root: &Path, cache: &Path, revision: &mut u64) -> AppMessage {
    let failed = |id: String, error: String| AppMessage::Previewed { id, thumbnail: None, badge: String::new(), length: None, cues: Vec::new(), error: Some(error) };
    if !media_type.starts_with("video/") && !media_type.starts_with("audio/") {
        return failed(id, "Only videos and sounds are previewed here.".into());
    }
    let file = if Path::new(path).is_absolute() { PathBuf::from(path) } else { root.join(path) };
    if media_type.starts_with("audio/") {
        return match sound(&file, path, cache, revision) {
            Ok((thumbnail, badge, length, cues)) => AppMessage::Previewed { id, thumbnail: Some(thumbnail), badge, length: Some(length), cues, error: None },
            Err(error) => failed(id, error),
        };
    }
    let mut video = match media::Video::open(&file) {
        Ok(video) => video,
        Err(error) => return failed(id, error.0),
    };
    let info = video.info();
    let frame = match video.frame(0, crate::decode::even(fit(info.width, info.height))) {
        Ok(frame) => frame,
        Err(error) => return failed(id, error.0),
    };
    if let Err(error) = std::fs::create_dir_all(cache) {
        return failed(id, error.to_string());
    }
    let name = format!("preview-{}.rgba", protocol::fingerprint(path.as_bytes()));
    match media::write_frame(cache, &name, &frame.rgba, (frame.width, frame.height)) {
        Ok(written) => {
            *revision += 1;
            AppMessage::Previewed {
                id,
                thumbnail: Some(Thumbnail { path: written.display().to_string(), width: frame.width, height: frame.height, revision: *revision }),
                badge: length(&info),
                length: Some(f64::from(info.frames) / info.fps().max(f64::EPSILON)),
                cues: Vec::new(),
                error: None,
            }
        }
        Err(error) => failed(id, error.0),
    }
}

/// A sound's preview: its waveform, `SOUND_SIZE`, drawn from its peaks as
/// the transport row's is and written as a raw image in `cache`, named by
/// the path; its length, "8.0 s", and in seconds; and, for a WAV, its cue
/// points, each at its time with its label. Decoded on the message loop,
/// once for each `preview`.
fn sound(file: &Path, path: &str, cache: &Path, revision: &mut u64) -> Result<(Thumbnail, String, f64, Vec<Cue>), String> {
    let heard = media::Sound::open(file, crate::waveform::RATE).map_err(|error| error.0)?;
    let peaks = crate::waveform::peaks(&heard, SOUND_SIZE[0] as usize);
    let image = crate::waveform::draw_sized(&peaks, SOUND_SIZE[0], SOUND_SIZE[1]);
    std::fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let name = format!("preview-{}.rgba", protocol::fingerprint(path.as_bytes()));
    let written = media::write_frame(cache, &name, &image.data, (image.width, image.height)).map_err(|error| error.0)?;
    *revision += 1;
    let thumbnail = Thumbnail { path: written.display().to_string(), width: image.width, height: image.height, revision: *revision };
    let seconds = heard.len() as f64 / f64::from(heard.rate.max(1));
    let cues = std::fs::read(file)
        .map(|bytes| crate::waveform::wav_cues(&bytes))
        .unwrap_or_default()
        .into_iter()
        .map(|(at, label)| Cue { at, label, colour: None })
        .collect();
    Ok((thumbnail, format!("{seconds:.1} s"), seconds, cues))
}

/// A video's length, as a thumbnail's corner shows it: "8.0 s".
pub fn length(info: &media::Info) -> String {
    let seconds = f64::from(info.frames) / info.fps().max(f64::EPSILON);
    format!("{seconds:.1} s")
}

/// A preview's size: the video's shape fitted into `PREVIEW_SIZE`, and no
/// larger than its own pixels.
fn fit(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = (f64::from(PREVIEW_SIZE[0]) / w).min(f64::from(PREVIEW_SIZE[1]) / h).min(1.0);
    (((w * scale).round() as u32).max(2), ((h * scale).round() as u32).max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a length that is not the guide's "8.0 s": frames over the
    /// rate, to a tenth of a second.
    #[test]
    fn a_length_reads_in_seconds() {
        let info = |frames: u32, rate: (u32, u32)| media::Info { width: 128, height: 72, rate, frames, sound: false };
        assert_eq!(length(&info(192, (24, 1))), "8.0 s");
        assert_eq!(length(&info(48, (24, 1))), "2.0 s");
        assert_eq!(length(&info(100, (30000, 1001))), "3.3 s");
        assert_eq!(fit(3840, 2160), (160, 90));
        assert_eq!(fit(128, 72), (128, 72));
    }
}
