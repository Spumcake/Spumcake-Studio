//! Playing sound: tracks mixed from any frame, a clock of what is being
//! heard for the frames to follow, a range played once or looped, and one
//! frame's slice at a time while the playhead is dragged or stepped. The
//! same `mix` makes an export's sound, so it sounds as playback did.

use crate::{Result, Sound, error};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One sound placed on the film, trimmed: it starts on the film at frame
/// `at`, from frame `from` of its own sound, for `length` frames, at
/// `volume` (1 as it is, 0 muted). Frames are the film's.
#[derive(Debug, Clone)]
pub struct Track {
    pub pcm: Arc<Sound>,
    pub at: u32,
    pub from: u32,
    pub length: u32,
    pub volume: f32,
}

/// The stereo samples the tracks make over `frames` frames from
/// `from_frame`, at `rate`, for a film at `fps`.
pub fn mix(tracks: &[Track], from_frame: u32, frames: u32, rate: u32, fps: (u32, u32)) -> Vec<f32> {
    let start = sample_at(from_frame, rate, fps);
    let end = sample_at(from_frame + frames, rate, fps);
    let mut out = vec![0.0; (end - start) * Sound::CHANNELS];
    mix_into(tracks, rate, fps, start, &mut out);
    out
}

/// Adds the tracks' samples from sample frame `position` into `out`. A
/// sound at another rate than the mix is read at its own, between its
/// samples, so it keeps its length and pitch rather than drifting.
fn mix_into(tracks: &[Track], rate: u32, fps: (u32, u32), position: usize, out: &mut [f32]) {
    let frames = out.len() / Sound::CHANNELS;
    for track in tracks {
        if track.volume == 0.0 {
            continue;
        }
        let own = track.pcm.rate.max(1);
        let start = sample_at(track.at, rate, fps);
        let skip = sample_at(track.from, own, fps);
        let available = track.pcm.len().saturating_sub(skip) as u64 * u64::from(rate) / u64::from(own);
        let length = sample_at(track.length, rate, fps).min(available as usize);
        let first = position.max(start);
        let last = (position + frames).min(start + length);
        let samples = &track.pcm.samples;
        for at in first..last {
            let to = (at - position) * Sound::CHANNELS;
            if own == rate {
                let from = (at - start + skip) * Sound::CHANNELS;
                for channel in 0..Sound::CHANNELS {
                    out[to + channel] += samples[from + channel] * track.volume;
                }
            } else {
                let exact = (at - start) as f64 * f64::from(own) / f64::from(rate) + skip as f64;
                let below = exact.floor() as usize;
                let part = (exact - below as f64) as f32;
                let above = (below + 1).min(track.pcm.len().saturating_sub(1));
                for channel in 0..Sound::CHANNELS {
                    let a = samples[below * Sound::CHANNELS + channel];
                    let b = samples[above * Sound::CHANNELS + channel];
                    out[to + channel] += (a + (b - a) * part) * track.volume;
                }
            }
        }
    }
    for sample in out {
        *sample = sample.clamp(-1.0, 1.0);
    }
}

/// The first sample frame of `frame`.
fn sample_at(frame: u32, rate: u32, fps: (u32, u32)) -> usize {
    (u64::from(frame) * u64::from(rate) * u64::from(fps.1.max(1)) / u64::from(fps.0.max(1))) as usize
}

/// What the sound's thread reads, and what it says it played. Positions
/// are sample frames from the film's start.
struct State {
    tracks: Vec<Track>,
    position: usize,
    /// Where playing stops, or loops back to `start` when `looping`.
    end: Option<usize>,
    start: usize,
    looping: bool,
    playing: bool,
    /// When the first sample of the last buffer reaches the speaker, and
    /// which sample frame it is.
    heard: Option<(Instant, usize)>,
    /// When play began, and from which sample frame, for a clock without
    /// sound.
    began: Option<(Instant, usize)>,
    /// Why the output stopped, when it did.
    failed: Option<String>,
    /// A silent player's time since play began, once a test has called
    /// `advance`: from then on only `advance` moves its clock.
    advanced: Option<Duration>,
    /// What was last played, and which frame was last scrubbed.
    played: Option<Played>,
    scrubbed: Option<u32>,
}

/// What a player was last asked to play: from a frame, to a frame or
/// until paused, once or looping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Played {
    pub from: u32,
    pub to: Option<u32>,
    pub looping: bool,
}

impl State {
    fn new() -> State {
        State {
            tracks: Vec::new(),
            position: 0,
            end: Some(0),
            start: 0,
            looping: false,
            playing: false,
            heard: None,
            began: None,
            failed: None,
            advanced: None,
            played: None,
            scrubbed: None,
        }
    }

    /// Fills `out` from the position on, looping back at the end of a
    /// looped range so the seam is never heard.
    fn fill(&mut self, rate: u32, fps: (u32, u32), out: &mut [f32]) {
        out.fill(0.0);
        let mut done = 0;
        let frames = out.len() / Sound::CHANNELS;
        while done < frames {
            let left = self.end.map_or(frames - done, |end| end.saturating_sub(self.position).min(frames - done));
            if left == 0 {
                if self.looping && self.end.is_some_and(|end| end > self.start) {
                    self.position = self.start;
                    continue;
                }
                break;
            }
            mix_into(&self.tracks, rate, fps, self.position, &mut out[done * Sound::CHANNELS..(done + left) * Sound::CHANNELS]);
            self.position += left;
            done += left;
        }
    }
}

/// The sound output, open at one rate for a film of one frame rate.
pub struct Player {
    state: Arc<Mutex<State>>,
    /// None when the computer has no sound output: the clock then runs on
    /// the wall's time, so the frames still play.
    stream: Option<cpal::Stream>,
    /// Why the output could not be opened.
    refused: Option<crate::Error>,
    rate: u32,
    fps: (u32, u32),
    lead: f64,
}

impl Player {
    /// Opens the computer's sound output at `rate`, for a film at `fps`.
    /// With no output, the player still keeps the clock. With
    /// `INPAINTER_NO_SOUND` set, as tests and screenshot scenarios set it,
    /// it is `silent` and opens nothing.
    pub fn open(rate: u32, fps: (u32, u32)) -> Player {
        if std::env::var_os("INPAINTER_NO_SOUND").is_some() {
            return Player::silent(rate, fps);
        }
        let state = Arc::new(Mutex::new(State::new()));
        let (stream, refused) = match output(state.clone(), rate, fps) {
            Ok(stream) => (Some(stream), None),
            Err(why) => (None, Some(why)),
        };
        Player { state, stream, rate, fps, lead: 0.0, refused }
    }

    /// A player that opens no sound output and keeps the clock on the
    /// wall's time, for tests and screenshots, which must never play sound
    /// on the computer. Once `advance` is called, only it moves the clock.
    pub fn silent(rate: u32, fps: (u32, u32)) -> Player {
        Player { state: Arc::new(Mutex::new(State::new())), stream: None, rate, fps, lead: 0.0, refused: None }
    }

    /// Moves a silent player's clock on by `milliseconds` while playing,
    /// and from then on keeps it still between calls.
    pub fn advance(&self, milliseconds: f64) {
        let mut state = self.lock();
        let step = Duration::from_secs_f64(milliseconds.max(0.0) / 1000.0);
        state.advanced = Some(state.advanced.unwrap_or(Duration::ZERO) + if state.playing { step } else { Duration::ZERO });
    }

    /// What was last played, from which frame, to which, and whether looped.
    pub fn last_played(&self) -> Option<Played> {
        self.lock().played
    }

    /// The frame the last play started from.
    pub fn last_played_from(&self) -> Option<u32> {
        self.lock().played.map(|played| played.from)
    }

    /// The frame whose slice was last scrubbed.
    pub fn last_scrubbed(&self) -> Option<u32> {
        self.lock().scrubbed
    }

    /// Whether sound is heard, or only the clock kept.
    pub fn has_output(&self) -> bool {
        self.stream.is_some() && self.lock().failed.is_none()
    }

    /// Why no sound is heard, as a sentence for a notice: the output could
    /// not be opened, or it stopped while playing.
    pub fn problem(&self) -> Option<crate::Error> {
        self.refused.clone().or_else(|| self.lock().failed.clone().map(crate::Error))
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The tracks to mix, replacing those before, while playing or not.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        self.lock().tracks = tracks;
    }

    /// Plays from the start of `frame` until paused.
    pub fn play_from(&self, frame: u32) {
        self.lock().played = Some(Played { from: frame, to: None, looping: false });
        self.play(sample_at(frame, self.rate, self.fps), None, false);
    }

    /// Plays frames `from` up to `to`, once or looping with no seam.
    pub fn play_range(&self, from: u32, to: u32, looping: bool) {
        let end = sample_at(to.max(from), self.rate, self.fps);
        self.lock().played = Some(Played { from, to: Some(to.max(from)), looping });
        self.play(sample_at(from, self.rate, self.fps), Some(end), looping);
    }

    fn play(&self, start: usize, end: Option<usize>, looping: bool) {
        let mut state = self.lock();
        state.position = start;
        state.start = start;
        state.end = end;
        state.looping = looping;
        state.playing = true;
        state.heard = None;
        state.began = Some((Instant::now(), start));
        if state.advanced.is_some() {
            state.advanced = Some(Duration::ZERO);
        }
    }

    pub fn pause(&self) {
        // With no output nothing moves the position, so it is taken from
        // the clock, where playing reached.
        if self.stream.is_none() {
            if let Some(seconds) = self.clock() {
                self.lock().position = (seconds * f64::from(self.rate)).round() as usize;
            }
        }
        let mut state = self.lock();
        state.end = Some(state.position);
        state.looping = false;
        state.playing = false;
        state.heard = None;
        state.began = None;
    }

    /// Plays `frame`'s slice of sound once, replacing any slice still
    /// playing, as the playhead is dragged or stepped.
    pub fn scrub(&self, frame: u32) {
        let mut state = self.lock();
        let start = sample_at(frame, self.rate, self.fps);
        state.scrubbed = Some(frame);
        state.position = start;
        state.start = start;
        state.end = Some(sample_at(frame + 1, self.rate, self.fps));
        state.looping = false;
        state.playing = false;
        state.heard = None;
        state.began = None;
    }

    /// The film's time being heard now, in seconds, while playing: from the
    /// device's own timestamps, which keep running with nothing to hear, or,
    /// with no output, the wall's time since play began. None when nothing
    /// plays, or before the first sound reaches the speaker.
    pub fn clock(&self) -> Option<f64> {
        let state = self.lock();
        if !state.playing {
            return None;
        }
        let device = self.stream.is_some() && state.failed.is_none();
        let (at, sample) = if device { state.heard? } else { state.began? };
        let since = if let (false, Some(advanced)) = (device, state.advanced) {
            advanced.as_secs_f64()
        } else {
            let now = Instant::now();
            if now >= at { (now - at).as_secs_f64() } else { -(at - now).as_secs_f64() }
        };
        let mut position = sample as f64 + since * f64::from(self.rate);
        if let Some(end) = state.end {
            let (start, end) = (state.start as f64, end as f64);
            if state.looping && end > start && position >= end {
                position = start + (position - start) % (end - start);
            } else {
                position = position.min(end);
            }
        }
        Some(position / f64::from(self.rate))
    }

    /// How far, in milliseconds, the frames are sent ahead of the sound,
    /// which the workspace keeps true from the host's `shown`.
    pub fn lead(&self) -> f64 {
        self.lead
    }

    pub fn set_lead(&mut self, milliseconds: f64) {
        self.lead = milliseconds;
    }

    /// The frame to send now: the one heard `lead` from now.
    pub fn frame_to_show(&self) -> Option<u32> {
        let seconds = self.clock()? + self.lead / 1000.0;
        Some((seconds * f64::from(self.fps.0) / f64::from(self.fps.1.max(1))).floor().max(0.0) as u32)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The computer's default output, fed from `state`.
fn output(state: Arc<Mutex<State>>, rate: u32, fps: (u32, u32)) -> Result<cpal::Stream> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| error("This computer has no sound output."))?;
    let failed = state.clone();
    let config = cpal::StreamConfig { channels: Sound::CHANNELS as u16, sample_rate: rate, buffer_size: cpal::BufferSize::Default };
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let now = Instant::now();
                let stamp = info.timestamp();
                let delay = stamp.playback.checked_duration_since(stamp.callback).unwrap_or(Duration::ZERO);
                let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let first = state.position;
                state.fill(rate, fps, out);
                if state.playing {
                    state.heard = Some((now + delay, first));
                }
            },
            move |why| {
                let mut state = failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                state.failed = Some(format!("The sound output stopped: {why}."));
            },
            None,
        )
        .map_err(|why| error(format!("Spumcake for Desktop could not open the sound output at {rate} Hz: {why}.")))?;
    stream.play().map_err(|why| error(format!("Spumcake for Desktop could not start the sound output: {why}.")))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: f32, frames: usize) -> Arc<Sound> {
        Arc::new(Sound { rate: 48, samples: vec![value; 2 * frames] })
    }

    fn left(samples: &[f32]) -> Vec<f32> {
        samples.chunks(2).map(|pair| pair[0]).collect()
    }

    /// Catches a track heard at the wrong frame, untrimmed, or at the wrong
    /// volume: at 48 samples a second and 24 frames, a frame is two
    /// samples.
    #[test]
    fn tracks_mix_where_they_are_placed() {
        let fps = (24, 1);
        let ramp = Arc::new(Sound { rate: 48, samples: (0..16).flat_map(|i| [i as f32 / 100.0; 2]).collect() });
        let tracks = vec![
            Track { pcm: flat(0.25, 8), at: 0, from: 0, length: 4, volume: 1.0 },
            // Trimmed: from its second frame, for two frames, at half volume.
            Track { pcm: ramp, at: 1, from: 1, length: 2, volume: 0.5 },
            Track { pcm: flat(0.9, 8), at: 0, from: 0, length: 4, volume: 0.0 },
        ];
        let out = left(&mix(&tracks, 0, 6, 48, fps));
        let expected = [0.25, 0.25, 0.26, 0.265, 0.27, 0.275, 0.25, 0.25, 0.0, 0.0, 0.0, 0.0];
        for (got, want) in out.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{out:?}");
        }
        assert_eq!(left(&mix(&tracks, 3, 2, 48, fps)), vec![0.25, 0.25, 0.0, 0.0]);
    }

    /// Catches a sound at another rate than the mix drifting: a ramp at
    /// half the mix's rate lasts as long and rises as it did.
    #[test]
    fn a_sound_at_another_rate_keeps_its_length() {
        let half = Arc::new(Sound { rate: 24, samples: (0..8).flat_map(|i| [i as f32 / 10.0; 2]).collect() });
        let tracks = vec![Track { pcm: half, at: 0, from: 0, length: 8, volume: 1.0 }];
        let out = left(&mix(&tracks, 0, 8, 48, (24, 1)));
        let expected = [0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.7];
        for (g, w) in out.iter().zip(expected) {
            assert!((g - w).abs() < 1e-6, "{out:?}");
        }
    }

    /// Catches a seam or a stop in a looped range: filling past its end
    /// carries on from its start in the same buffer.
    #[test]
    fn a_looped_range_has_no_seam() {
        let ramp = Arc::new(Sound { rate: 48, samples: (0..16).flat_map(|i| [i as f32 / 100.0; 2]).collect() });
        let mut state = State {
            tracks: vec![Track { pcm: ramp, at: 0, from: 0, length: 8, volume: 1.0 }],
            position: 2,
            end: Some(6),
            start: 2,
            looping: true,
            playing: true,
            ..State::new()
        };
        let mut out = vec![0.0; 2 * 10];
        state.fill(48, (24, 1), &mut out);
        let got = left(&out);
        let expected = [0.02, 0.03, 0.04, 0.05, 0.02, 0.03, 0.04, 0.05, 0.02, 0.03];
        for (g, w) in got.iter().zip(expected) {
            assert!((g - w).abs() < 1e-6, "{got:?}");
        }
    }

    /// Catches a silent player that opens a device, keeps running on the
    /// wall's time after `advance`, or loses what it was asked to play: the
    /// clock moves only by what was advanced, waits with the wall, starts
    /// again from the play's own frame, and pausing keeps where it reached.
    #[test]
    fn a_silent_player_moves_only_when_advanced() {
        let player = Player::silent(48_000, (24, 1));
        assert!(!player.has_output());
        assert!(player.problem().is_none());
        player.play_from(24);
        player.advance(500.0);
        assert!((player.clock().unwrap() - 1.5).abs() < 1e-9);
        std::thread::sleep(Duration::from_millis(30));
        assert!((player.clock().unwrap() - 1.5).abs() < 1e-9, "the wall moved a silent clock after advance");
        assert_eq!(player.frame_to_show(), Some(36));
        assert_eq!(player.last_played(), Some(Played { from: 24, to: None, looping: false }));
        assert_eq!(player.last_played_from(), Some(24));
        player.pause();
        assert_eq!(player.clock(), None);
        player.advance(1000.0);
        player.play_range(0, 48, true);
        assert!((player.clock().unwrap() - 0.0).abs() < 1e-9, "a new play kept the last play's advance");
        player.advance(2500.0);
        assert!((player.clock().unwrap() - 0.5).abs() < 1e-9, "the looped range did not wrap");
        assert_eq!(player.last_played(), Some(Played { from: 0, to: Some(48), looping: true }));
        player.scrub(7);
        assert_eq!(player.last_scrubbed(), Some(7));
        assert_eq!(player.clock(), None);
    }

    /// Catches a silent player that only ever runs by `advance`: before any
    /// call to it, the clock follows the wall, so frames still play.
    #[test]
    fn a_silent_player_keeps_wall_time_until_advanced() {
        let player = Player::silent(48_000, (24, 1));
        player.play_from(0);
        std::thread::sleep(Duration::from_millis(40));
        let seconds = player.clock().unwrap();
        assert!(seconds >= 0.04 && seconds < 1.0, "{seconds}");
    }
}
