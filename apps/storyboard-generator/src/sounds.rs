//! The storyboard's sounds as sound: each file the Sound tracks name,
//! decoded once by `media` at one rate, with its fingerprint and the peaks
//! its waveform is drawn from. The film keeps only where each plays; this
//! keeps what plays. Nothing here is saved: a storyboard's sounds are
//! decoded again from the project's `sounds/` folder when it opens.

use crate::film::{FPS, Film};
use media::{Sound, Track};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The rate every sound is decoded at, and the film's sound is mixed at.
pub const RATE: u32 = 48_000;
/// Peaks kept for each frame of a sound: a waveform's finest detail.
pub const PEAKS_PER_FRAME: usize = 12;
/// The most peaks one item of the timeline carries.
pub const MOST_PEAKS: usize = 4000;
/// The project's folder imported sounds are copied into.
pub const FOLDER: &str = "sounds";
/// The file types Import sound offers.
pub const EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac", "mp4", "mov"];

/// The film's frame rate as `media` takes it.
pub fn fps() -> (u32, u32) {
    (FPS, 1)
}

/// One decoded sound.
#[derive(Debug)]
pub struct Loaded {
    pub sound: Arc<Sound>,
    /// The fingerprint of the file's bytes.
    pub fingerprint: String,
    /// The largest absolute sample of each twelfth of a frame, 0 to 1.
    pub peaks: Vec<f32>,
    /// Its length in film frames, rounded up.
    pub frames: u32,
    /// The file's size and when it was changed, when it was decoded: a
    /// file replaced or removed since is read again.
    pub stamp: Option<(u64, std::time::SystemTime)>,
}

/// A file's size and when it was changed.
fn stamp_of(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// What a sound's file gave.
#[derive(Clone, Debug)]
pub enum State {
    Loaded(Arc<Loaded>),
    /// Not at its path in the project.
    Missing,
    /// There, but `media` could not read it: why, as a sentence.
    Unreadable(String),
}

/// Every sound this program has decoded, by its path on disk.
#[derive(Default)]
pub struct Library {
    files: HashMap<PathBuf, State>,
}

impl Library {
    /// The sound of `file`, a path relative to `project`, decoded the first
    /// time it is asked for.
    pub fn get(&mut self, project: Option<&Path>, file: &str) -> State {
        let path = resolve(project, file);
        if let Some(found) = self.files.get(&path) {
            // What is known holds while the file is as it was: a file that
            // came back, was replaced, or went is read again.
            let current = match found {
                State::Loaded(loaded) => loaded.stamp.is_some() && loaded.stamp == stamp_of(&path),
                State::Missing => !path.is_file(),
                State::Unreadable(_) => true,
            };
            if current {
                return found.clone();
            }
        }
        let state = if path.is_file() {
            match decode(&path) {
                Ok(loaded) => State::Loaded(Arc::new(loaded)),
                Err(reason) => State::Unreadable(reason),
            }
        } else {
            State::Missing
        };
        self.files.insert(path, state.clone());
        state
    }

    /// What is known of `file` without reading it: none until it is asked
    /// for with `get`.
    pub fn peek(&self, project: Option<&Path>, file: &str) -> Option<&State> {
        self.files.get(&resolve(project, file))
    }

    /// The sound of `file` when it is decoded, and none when it is missing
    /// or unreadable.
    pub fn loaded(&mut self, project: Option<&Path>, file: &str) -> Option<Arc<Loaded>> {
        match self.get(project, file) {
            State::Loaded(loaded) => Some(loaded),
            _ => None,
        }
    }

    /// Keeps a sound already decoded under its path, as an import does.
    pub fn insert(&mut self, path: PathBuf, loaded: Arc<Loaded>) {
        self.files.insert(path, State::Loaded(loaded));
    }

    /// Forgets what is known of `file`, so the next ask reads it again.
    pub fn forget(&mut self, project: Option<&Path>, file: &str) {
        self.files.remove(&resolve(project, file));
    }

    /// The tracks `media` mixes for the film: every sound clip of every
    /// unmuted Sound track whose sound is decoded, at its own volume.
    pub fn tracks(&mut self, project: Option<&Path>, film: &Film) -> Vec<Track> {
        let mut out = Vec::new();
        for track in film.sound_tracks.iter().filter(|track| !track.muted) {
            for clip in &track.clips {
                let Some(loaded) = self.loaded(project, &clip.file) else {
                    continue;
                };
                out.push(Track {
                    pcm: loaded.sound.clone(),
                    at: clip.at,
                    from: clip.from,
                    length: clip.length,
                    volume: clip.volume as f32,
                });
            }
        }
        out
    }

    /// The files the film's sounds name that are not there, or cannot be
    /// read, each once.
    pub fn absent(&mut self, project: Option<&Path>, film: &Film) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, clip) in film.sounds() {
            if self.loaded(project, &clip.file).is_none() && !out.contains(&clip.file) {
                out.push(clip.file.clone());
            }
        }
        out
    }
}

/// A sound's file on disk: relative to the project, or as it is when the
/// storyboard is in none.
pub fn resolve(project: Option<&Path>, file: &str) -> PathBuf {
    match project {
        Some(project) => project.join(file),
        None => PathBuf::from(file),
    }
}

/// Decodes a sound file, with its fingerprint and peaks.
pub fn decode(path: &Path) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|error| crate::store::reason(&error))?;
    let sound = Sound::open(path, RATE).map_err(|error| error.to_string())?;
    if sound.is_empty() {
        return Err(format!("{} has no sound.", file_name(path)));
    }
    let peaks = peaks(&sound);
    let frames = frames_of(&sound);
    Ok(Loaded { sound: Arc::new(sound), fingerprint: protocol::fingerprint(&bytes), peaks, frames, stamp: stamp_of(path) })
}

/// A sound's length in film frames, rounded up.
pub fn frames_of(sound: &Sound) -> u32 {
    let pairs = sound.samples.len() / Sound::CHANNELS;
    let per_frame = (RATE / FPS) as usize;
    pairs.div_ceil(per_frame) as u32
}

/// The largest absolute sample of each twelfth of a frame.
pub fn peaks(sound: &Sound) -> Vec<f32> {
    peaks_of(&sound.samples)
}

/// The same, for stereo samples from a mix.
pub fn peaks_of(samples: &[f32]) -> Vec<f32> {
    let slice = (RATE / FPS) as usize / PEAKS_PER_FRAME * Sound::CHANNELS;
    samples.chunks(slice).map(|chunk| chunk.iter().fold(0.0f32, |most, sample| most.max(sample.abs())).min(1.0)).collect()
}

/// The peaks of `from .. from + length` frames of a sound, merged pairwise
/// until there are at most `MOST_PEAKS`, as one timeline item carries them.
pub fn span_peaks(peaks: &[f32], from: u32, length: u32) -> Vec<f32> {
    let start = (from as usize * PEAKS_PER_FRAME).min(peaks.len());
    let end = ((from + length) as usize * PEAKS_PER_FRAME).min(peaks.len());
    let mut out = peaks[start..end].to_vec();
    while out.len() > MOST_PEAKS {
        out = out.chunks(2).map(|pair| pair.iter().copied().fold(0.0, f32::max)).collect();
    }
    out
}

/// How loud the film's sound is on each frame of `from .. from + frames`,
/// 0 to 1: the largest absolute sample of the mix on that frame.
pub fn loudness(tracks: &[Track], from: u32, frames: u32) -> Vec<f32> {
    let mix = media::mix(tracks, from, frames, RATE, fps());
    let per_frame = (RATE / FPS) as usize * Sound::CHANNELS;
    let mut out: Vec<f32> = mix.chunks(per_frame).map(|chunk| chunk.iter().fold(0.0f32, |most, sample| most.max(sample.abs())).min(1.0)).collect();
    out.resize(frames as usize, 0.0);
    out
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A sound, decoded and copied into the project's `sounds/` folder under a
/// free name, ready to place.
pub struct Imported {
    /// Its file, relative to the project.
    pub file: String,
    /// The name its clip shows: the file's name.
    pub name: String,
    pub loaded: Arc<Loaded>,
    /// Where it was copied to.
    pub path: PathBuf,
}

/// Decodes `source` and, when it is sound, copies it into the project's
/// `sounds/` folder. A file that is not sound is not copied. Outside a
/// project the sound stays where it is.
pub fn import(project: Option<&Path>, source: &Path) -> Result<Imported, String> {
    let loaded = decode(source)?;
    let name = file_name(source);
    let Some(project) = project else {
        let file = source.display().to_string();
        return Ok(Imported { file, name, loaded: Arc::new(loaded), path: source.to_path_buf() });
    };
    let folder = project.join(FOLDER);
    // A sound already in the folder is used where it is.
    if source.parent().is_some_and(|parent| crate::store::same_file(parent, &folder)) {
        let file = format!("{FOLDER}/{name}");
        return Ok(Imported { file, name, loaded: Arc::new(loaded), path: source.to_path_buf() });
    }
    std::fs::create_dir_all(&folder).map_err(|error| crate::store::reason(&error))?;
    let target = free_path(&folder, &name);
    std::fs::copy(source, &target).map_err(|error| crate::store::reason(&error))?;
    let copied = file_name(&target);
    // Known by the copy, which is what the storyboard plays from now on.
    let loaded = Loaded { stamp: stamp_of(&target), ..loaded };
    Ok(Imported { file: format!("{FOLDER}/{copied}"), name: copied, loaded: Arc::new(loaded), path: target })
}

/// `name` in `folder`, or, when that is taken, "Storm and radio 2.wav" and on.
fn free_path(folder: &Path, name: &str) -> PathBuf {
    let path = folder.join(name);
    if !path.exists() {
        return path;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.to_string(), String::new()),
    };
    let mut n = 2;
    loop {
        let path = folder.join(format!("{stem} {n}{extension}"));
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

/// A WAV file of 16-bit stereo samples, for tests and the no-host check:
/// `samples` are left and right in turn, from -1 to 1.
pub fn wav_bytes(rate: u32, samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 4).to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for sample in samples {
        out.extend(((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Stereo silence of `frames` film frames with a click, a few loud
/// samples, on each frame of `clicks`.
pub fn clicks(frames: u32, clicks: &[u32]) -> Vec<f32> {
    let per_frame = (RATE / FPS) as usize;
    let mut samples = vec![0.0; frames as usize * per_frame * Sound::CHANNELS];
    for frame in clicks {
        let start = *frame as usize * per_frame;
        for pair in start..(start + per_frame / 4).min(samples.len() / Sound::CHANNELS) {
            samples[pair * 2] = 0.9;
            samples[pair * 2 + 1] = 0.9;
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("storyboard-sounds-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_import_is_copied_under_a_free_name_and_decoded() {
        let project = scratch("import");
        let outside = scratch("import-source");
        let source = outside.join("Storm and radio.wav");
        std::fs::write(&source, wav_bytes(RATE, &clicks(48, &[30]))).unwrap();
        let first = import(Some(&project), &source).unwrap();
        assert_eq!(first.file, "sounds/Storm and radio.wav");
        assert_eq!(first.loaded.frames, 48);
        let second = import(Some(&project), &source).unwrap();
        assert_eq!(second.file, "sounds/Storm and radio 2.wav");
        // The click is the loudest thing, on its frame.
        let peaks = span_peaks(&first.loaded.peaks, 0, 48);
        let loudest = peaks.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(loudest / PEAKS_PER_FRAME, 30);
    }

    #[test]
    fn a_file_that_is_not_sound_is_not_copied() {
        let project = scratch("not-sound");
        let source = project.join("notes.txt");
        std::fs::write(&source, "not sound").unwrap();
        assert!(import(Some(&project), &source).is_err());
        assert!(!project.join(FOLDER).exists());
    }

    #[test]
    fn long_waveforms_are_merged_to_what_an_item_carries() {
        let peaks = vec![0.5; 20_000];
        assert!(span_peaks(&peaks, 0, 1000).len() <= MOST_PEAKS);
        assert_eq!(span_peaks(&peaks, 10, 5).len(), 5 * PEAKS_PER_FRAME);
    }
}
