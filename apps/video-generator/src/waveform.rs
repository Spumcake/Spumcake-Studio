//! A take's sound, decoded once for each file on a thread of its own, off
//! the message loop: the samples the Player plays, and its peaks, from
//! which the transport row's waveform is drawn at any width. A take with
//! no sound is kept as silent, so it is never asked for again. A sound
//! file's preview is drawn the same way, and a WAV's cue points are read
//! from its RIFF chunks.

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
/// it against the sound's loudest, on clear.
pub fn draw(peaks: &[f32], width: u32) -> Rgba {
    draw_sized(peaks, width, HEIGHT)
}

/// A waveform `width` × `height`, drawn as `draw` draws the transport
/// row's, as a sound's preview is.
pub fn draw_sized(peaks: &[f32], width: u32, height: u32) -> Rgba {
    let height = height.max(2);
    let mut image = Rgba::new(width.max(1), height, [0, 0, 0, 0]);
    if peaks.is_empty() {
        return image;
    }
    // Drawn against the sound's own loudest peak, so a quiet track, a room
    // tone with a door in it, still shows where its sounds are.
    let loudest_of_all = peaks.iter().fold(0.0f32, |most, peak| most.max(*peak));
    let scale = if loudest_of_all > 0.0 { 1.0 / loudest_of_all } else { 1.0 };
    let middle = height as f32 / 2.0;
    for x in 0..image.width {
        let (from, to) = (x as usize * peaks.len() / image.width as usize, ((x as usize + 1) * peaks.len() / image.width as usize).min(peaks.len()));
        let loudest = peaks[from.min(peaks.len() - 1)..to.max(from + 1).min(peaks.len())].iter().fold(0.0f32, |most, peak| most.max(*peak));
        let reach = ((loudest * scale).clamp(0.0, 1.0) * middle).max(0.5);
        let (top, bottom) = ((middle - reach).floor().max(0.0) as u32, ((middle + reach).ceil() as u32).min(height));
        for y in top..bottom {
            image.set(x, y, INK);
        }
    }
    image
}

/// A waveform row `width` × `height` with the sound drawn only from `from`
/// to `to`, fractions of the width, as a sound layer starting on the take
/// sits under the scrubber: clear elsewhere, and cut at the row's ends.
pub fn draw_span(peaks: &[f32], width: u32, height: u32, from: f64, to: f64) -> Rgba {
    let mut image = Rgba::new(width.max(1), height.max(2), [0, 0, 0, 0]);
    let (left, right) = ((from * f64::from(width)).round(), (to * f64::from(width)).round());
    let span = (right - left).max(1.0).min(f64::from(u32::MAX / 8)) as u32;
    let drawn = draw_sized(peaks, span, image.height);
    for x in 0..span {
        let at = left as i64 + i64::from(x);
        if at < 0 || at >= i64::from(image.width) {
            continue;
        }
        for y in 0..image.height {
            image.set(at as u32, y, drawn.pixel(x, y));
        }
    }
    image
}

/// A WAV's cue points, from its RIFF `cue ` chunk: each point's time in
/// seconds, its sample offset over the `fmt ` chunk's rate, and its label,
/// the `LIST` `adtl` chunk's `labl` with the same id, or none; in time
/// order. Empty for a file that is no WAV, or has no cue or no rate.
pub fn wav_cues(bytes: &[u8]) -> Vec<(f64, String)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Vec::new();
    }
    let mut rate = 0u32;
    let mut points: Vec<(u32, u32)> = Vec::new();
    let mut labels: HashMap<u32, String> = HashMap::new();
    for (id, data) in chunks(&bytes[12..]) {
        match id {
            b"fmt " if data.len() >= 8 => rate = u32_at(data, 4),
            b"cue " if data.len() >= 4 => {
                let count = u32_at(data, 0) as usize;
                // Each point: its id, its place, its chunk, the chunk's
                // and the block's start, and its sample offset.
                for point in data[4..].chunks_exact(24).take(count) {
                    points.push((u32_at(point, 0), u32_at(point, 20)));
                }
            }
            b"LIST" if data.len() >= 4 && &data[0..4] == b"adtl" => {
                for (kind, body) in chunks(&data[4..]) {
                    if kind == b"labl" && body.len() >= 4 {
                        let words = &body[4..];
                        let end = words.iter().position(|byte| *byte == 0).unwrap_or(words.len());
                        labels.insert(u32_at(body, 0), String::from_utf8_lossy(&words[..end]).into_owned());
                    }
                }
            }
            _ => {}
        }
    }
    if rate == 0 {
        return Vec::new();
    }
    let mut cues: Vec<(f64, String)> =
        points.iter().map(|(id, offset)| (f64::from(*offset) / f64::from(rate), labels.get(id).cloned().unwrap_or_default())).collect();
    cues.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    cues
}

/// A RIFF body's chunks in order: each one's four-letter id and its data,
/// cut at the end of the bytes; each is padded to an even length.
fn chunks(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let size = u32_at(bytes, at + 4) as usize;
        let end = (at + 8).saturating_add(size).min(bytes.len());
        found.push((&bytes[at..at + 4], &bytes[at + 8..end]));
        at = (at + 8).saturating_add(size).saturating_add(size & 1);
    }
    found
}

/// A little-endian `u32` at `at`.
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV of `samples` silent 16-bit mono samples at `rate`, with a
    /// `cue ` chunk of `cues`, (id, sample offset), and a `LIST` `adtl`
    /// chunk of `labels`, (id, words).
    fn wav(rate: u32, samples: u32, cues: &[(u32, u32)], labels: &[(u32, &str)]) -> Vec<u8> {
        fn chunk(id: &[u8], data: Vec<u8>) -> Vec<u8> {
            let mut out = id.to_vec();
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            let odd = data.len() % 2 == 1;
            out.extend(data);
            if odd {
                out.push(0);
            }
            out
        }
        let mut fmt = Vec::new();
        for value in [1u16, 1] {
            fmt.extend_from_slice(&value.to_le_bytes());
        }
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * 2).to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let mut cue = (cues.len() as u32).to_le_bytes().to_vec();
        // Each point: its id, its place, its chunk, the chunk's and the
        // block's start, and its sample offset.
        for (id, offset) in cues {
            cue.extend_from_slice(&id.to_le_bytes());
            cue.extend_from_slice(&0u32.to_le_bytes());
            cue.extend_from_slice(b"data");
            cue.extend_from_slice(&[0; 8]);
            cue.extend_from_slice(&offset.to_le_bytes());
        }
        let mut list = b"adtl".to_vec();
        for (id, words) in labels {
            let mut body = id.to_le_bytes().to_vec();
            body.extend_from_slice(words.as_bytes());
            body.push(0);
            list.extend(chunk(b"labl", body));
        }
        let mut body = b"WAVE".to_vec();
        body.extend(chunk(b"fmt ", fmt));
        body.extend(chunk(b"data", vec![0; (samples * 2) as usize]));
        body.extend(chunk(b"cue ", cue));
        body.extend(chunk(b"LIST", list));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    /// Catches a sound layer's row drawn from the take's start rather than
    /// its own: a loud sound from a quarter to three quarters of the row is
    /// drawn there and nowhere else.
    #[test]
    fn a_span_is_drawn_where_it_starts() {
        let image = draw_span(&[1.0; 16], 40, HEIGHT, 0.25, 0.75);
        assert_eq!(image.pixel(5, 2), [0, 0, 0, 0], "drawn before its start");
        assert_eq!(image.pixel(20, 2), INK, "not drawn within its span");
        assert_eq!(image.pixel(35, HEIGHT / 2), [0, 0, 0, 0], "drawn after its end");
    }

    /// Catches cue points read at the wrong time or with the wrong words:
    /// two points at samples 12 000 and 4 000 of an 8 000 a second WAV
    /// are at 0.5 and 1.5 seconds, in time order, each with its own label
    /// by its id, and one with no label has none; a file that is no WAV
    /// has no cues.
    #[test]
    fn a_wavs_cues_are_read_from_its_chunks() {
        let bytes = wav(8000, 16_000, &[(7, 12_000), (3, 4_000), (9, 8_000)], &[(3, "Door"), (7, "Thunder")]);
        assert_eq!(wav_cues(&bytes), [(0.5, "Door".to_string()), (1.0, String::new()), (1.5, "Thunder".to_string())]);
        assert!(wav_cues(&wav(8000, 100, &[], &[])).is_empty());
        assert!(wav_cues(b"not a sound").is_empty());
    }

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
        // A quiet track is drawn against its own loudest, as tall.
        let quiet: Vec<f32> = peaks.iter().map(|peak| peak / 20.0).collect();
        assert_eq!(draw(&quiet, 40).pixel(5, 2), INK, "a quiet sound is drawn flat");
    }
}
