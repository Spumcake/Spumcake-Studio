//! Video, sound, and encoding for the workspaces that play and write them:
//! the Storyboard Generator and the Video Generator, as `draw` is shared.
//! Never used by the host, which only draws the raw frames a workspace
//! writes. Its interface is the lifecycle contract's "Media: one shared
//! library", in the team repository; the Image and Video role session
//! writes it.

mod colour;
mod encode;
mod frames;
mod player;
mod seeker;
mod sound;
mod video;

pub use encode::{Encoder, Settings};
pub use frames::{write_frame, write_png};
pub use player::{Player, Track, mix};
pub use seeker::Seeker;
pub use sound::Sound;
pub use video::{Frame, Frames, Info, Video};

/// What went wrong, as a sentence a person can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn error(text: impl Into<String>) -> Error {
    Error(text.into())
}

/// FFmpeg's libraries are set up once, before anything opens a file.
fn init() -> Result<()> {
    static DONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // FFmpeg's own messages would reach the person's terminal through the
    // host; what matters reaches them as errors instead.
    if *DONE.get_or_init(|| ffmpeg_next::init().inspect(|_| ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Fatal)).is_ok()) {
        Ok(())
    } else {
        Err(error("The video libraries could not start."))
    }
}

/// The name a person knows a file by, for sentences.
fn named(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}
