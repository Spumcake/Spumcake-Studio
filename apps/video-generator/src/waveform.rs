//! A take's sound, decoded once for each file on a thread of its own, off
//! the message loop: the samples the Player plays, and its peaks, from
//! which the transport row's waveform is drawn at any width. A take with
//! no sound is kept as silent, so it is never asked for again.

use layers::Rgba;
use media::Sound;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

/// The rate every take's sound is decoded and played at, as the
/// Storyboard Generator's.
pub const RATE: u32 = 48_000;
/// How many peaks a sound keeps, the most columns a waveform is drawn at.
const PEAKS: usize = 4096;
/// A waveform row's height, in pixels.
pub const HEIGHT: u32 = 24;
/// The waveform's gray, on clear.
const INK: [u8; 4] = [120, 120, 128, 255];

/// A take's sound, once decoded.
pub enum Heard {
    /// Its samples, and the loudest sample of each of `PEAKS` spans.
    Sound { pcm: Arc<Sound>, peaks: Vec<f32> },
    /// It has no sound, or its sound could not be read.
    Silent,
}

/// The sounds decoded, by the key their file is known by (its sha256 or
/// fingerprint), and those still being decoded.
pub struct Sounds {
    jobs: Sender<(String, PathBuf)>,
    done: Receiver<(String, Heard)>,
    asked: HashSet<String>,
    pub ready: HashMap<String, Heard>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self::new()
    }
}

impl Sounds {
    pub fn new() -> Self {
        let (jobs, incoming) = std::sync::mpsc::channel::<(String, PathBuf)>();
        let (out, done) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(String::from("video-generator-sounds"))
            .spawn(move || {
                for (key, file) in incoming {
                    let heard = match Sound::open(&file, RATE) {
                        Ok(sound) => {
                            let peaks = peaks(&sound, PEAKS);
                            Heard::Sound { pcm: Arc::new(sound), peaks }
                        }
                        Err(_) => Heard::Silent,
                    };
                    if out.send((key, heard)).is_err() {
                        return;
                    }
                }
            })
            .expect("a thread for decoding sound");
        Self { jobs, done, asked: HashSet::new(), ready: HashMap::new() }
    }

    /// Asks for the sound of `file`, known by `key`, once.
    pub fn want(&mut self, key: &str, file: &PathBuf) {
        if self.ready.contains_key(key) || !self.asked.insert(key.to_string()) {
            return;
        }
        let _ = self.jobs.send((key.to_string(), file.clone()));
    }

    /// Takes in every sound decoded since the last call. Returns whether
    /// any came.
    pub fn poll(&mut self) -> bool {
        let mut came = false;
        while let Ok((key, heard)) = self.done.try_recv() {
            self.asked.remove(&key);
            self.ready.insert(key, heard);
            came = true;
        }
        came
    }

    /// Whether a sound asked for has not come yet.
    pub fn waiting(&self) -> bool {
        !self.asked.is_empty()
    }

    /// The samples of the sound known by `key`, once decoded, if it has any.
    pub fn pcm(&self, key: &str) -> Option<Arc<Sound>> {
        match self.ready.get(key)? {
            Heard::Sound { pcm, .. } => Some(pcm.clone()),
            Heard::Silent => None,
        }
    }
}

/// The loudest sample of each of `count` equal spans of a sound, from 0
/// to 1.
pub fn peaks(sound: &Sound, count: usize) -> Vec<f32> {
    let frames = sound.len();
    if frames == 0 || count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|span| {
            let (from, to) = (span * frames / count, ((span + 1) * frames / count).max(span * frames / count + 1).min(frames));
            sound.samples[from * Sound::CHANNELS..to * Sound::CHANNELS].iter().fold(0.0f32, |most, sample| most.max(sample.abs()))
        })
        .collect()
}

/// A waveform `width` wide and `HEIGHT` tall, drawn from `peaks`: each
/// column a gray bar about the middle, as tall as the loudest sample under
/// it, on clear.
pub fn draw(peaks: &[f32], width: u32) -> Rgba {
    let mut image = Rgba::new(width.max(1), HEIGHT, [0, 0, 0, 0]);
    if peaks.is_empty() {
        return image;
    }
    let middle = HEIGHT as f32 / 2.0;
    for x in 0..image.width {
        let (from, to) = (x as usize * peaks.len() / image.width as usize, ((x as usize + 1) * peaks.len() / image.width as usize).min(peaks.len()));
        let loudest = peaks[from.min(peaks.len() - 1)..to.max(from + 1).min(peaks.len())].iter().fold(0.0f32, |most, peak| most.max(*peak));
        let reach = (loudest.clamp(0.0, 1.0) * middle).max(0.5);
        let (top, bottom) = ((middle - reach).floor().max(0.0) as u32, ((middle + reach).ceil() as u32).min(HEIGHT));
        for y in top..bottom {
            image.set(x, y, INK);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a waveform that does not follow its sound: a sound loud in
    /// its first half and silent in its second draws tall bars at the left
    /// and a thin line at the right, across the width asked for.
    #[test]
    fn a_waveform_follows_its_sound() {
        let mut samples = vec![0.9f32; 2 * 100];
        samples.extend(vec![0.0f32; 2 * 100]);
        let sound = Sound { rate: RATE, samples };
        let peaks = peaks(&sound, 8);
        assert_eq!(peaks.len(), 8);
        assert!(peaks[0] > 0.8 && peaks[7] == 0.0);
        let image = draw(&peaks, 40);
        assert_eq!((image.width, image.height), (40, HEIGHT));
        assert_eq!(image.pixel(5, 2), INK, "the loud half is not tall");
        assert_eq!(image.pixel(35, 2), [0, 0, 0, 0], "the silent half is not flat");
        assert_eq!(image.pixel(35, HEIGHT / 2), INK, "the silent half has no line");
    }
}
