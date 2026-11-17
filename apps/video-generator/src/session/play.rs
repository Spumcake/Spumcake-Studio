//! Playing the take shown with its sound, as the Storyboard Generator plays
//! its film: the sound plays in `media`'s Player, and the frame shown always
//! follows the Player's clock (`frame_to_show`), never a timer of this
//! program's own. Frames come from the takes' seekers at the size drawn, as
//! a stepped frame does. The host's `shown` keeps the Player's lead true and
//! paces the frames written. Play, pause, a step, and a scrub never change
//! the gallery or its history, and any change to the gallery pauses first.
//! Side by side, both takes show the frame of one clock, with the sound of
//! the take shown first. Frame strips do not play.

use super::*;
use crate::waveform::{self, Heard, Sounds};
use media::{Player, Track};
use std::collections::VecDeque;

/// How many of the host's reports the lead is the mean of.
const LEAD_REPORTS: usize = 30;
/// How long a frame sent waits for the host's `shown` before the next is
/// written anyway, in milliseconds.
const PACE: f64 = 250.0;

/// The session's playback: one Player, as there is one sound output, the
/// sounds decoded, and the waveforms drawn.
pub(super) struct Playback {
    player: Option<Player>,
    /// The frame rate the Player was opened for.
    fps: (u32, u32),
    pub playing: bool,
    /// The last frame of the take playing, where it stops.
    last: u32,
    /// Each revision sent while playing and when, not yet shown.
    sent: VecDeque<(u64, f64)>,
    delays: VecDeque<f64>,
    pub sounds: Sounds,
    /// Each sound's waveform written for the host, by its key and width.
    waves: HashMap<(String, u32), Thumbnail>,
    next_wave: u64,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            player: None,
            fps: (24, 1),
            playing: false,
            last: 0,
            sent: VecDeque::new(),
            delays: VecDeque::new(),
            sounds: Sounds::new(),
            waves: HashMap::new(),
            next_wave: 1,
        }
    }
}

impl Playback {
    /// The host was sent this revision while playing.
    pub(super) fn sent(&mut self, revision: u64) {
        if self.playing {
            self.sent.push_back((revision, now_ms()));
            while self.sent.len() > LEAD_REPORTS * 4 {
                self.sent.pop_front();
            }
        }
    }

    /// Whether frames written are still waiting for the host to draw them:
    /// the next waits, for a while.
    fn paced(&self) -> bool {
        self.sent.len() >= 2 && self.sent.back().is_some_and(|(_, at)| now_ms() - at < PACE)
    }
}

/// Milliseconds since the Unix epoch, as `shown`'s `at` counts.
fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

/// The take that plays: its file, the key its sound is known by, and what
/// it is.
pub(super) struct Playing {
    pub file: PathBuf,
    pub key: String,
    pub info: media::Info,
}

impl Session {
    /// The Player, opened for `fps` on first use or when a take of another
    /// rate plays. With `INPAINTER_NO_SOUND` set, it is silent.
    fn player(&mut self, fps: (u32, u32)) -> &mut Player {
        let playback = &mut self.playback;
        if playback.player.is_none() || playback.fps != fps {
            playback.player = Some(Player::open(waveform::RATE, fps));
            playback.fps = fps;
        }
        playback.player.as_mut().expect("opened above")
    }

    /// Plays with `player` instead of the computer's sound output, as the
    /// tests do.
    #[cfg(test)]
    pub fn use_player(&mut self, player: Player, fps: (u32, u32)) {
        self.stop_playing();
        self.playback.player = Some(player);
        self.playback.fps = fps;
    }

    pub fn is_playing(&self) -> bool {
        self.playback.playing
    }

    /// The time the Player's clock gives while playing, in seconds.
    pub fn play_time(&self) -> Option<f64> {
        if !self.playback.playing {
            return None;
        }
        self.playback.player.as_ref()?.clock()
    }

    /// The take shown that plays, alone or first of two side by side: none
    /// for frame strips, or a take whose file did not open.
    pub(super) fn playing_take(&self) -> Option<Playing> {
        let tab = self.active_tab()?;
        let shown = tab.round_view();
        let gallery = shown.as_ref().unwrap_or(&tab.gallery);
        if gallery.view() == Mode::Strips {
            return None;
        }
        let take = gallery.take()?;
        let key = if take.link.sha256.is_empty() { take.file.display().to_string() } else { take.link.sha256.clone() };
        Some(Playing { file: take.file.clone(), key, info: take.info? })
    }

    /// Asks for the sound of the take that plays, once for each file.
    pub(super) fn want_sound(&mut self) {
        if let Some(take) = self.playing_take() {
            self.playback.sounds.want(&take.key, &take.file);
        }
    }

    /// The take's sound as the Player plays it, when it has one.
    fn tracks(&self, take: &Playing) -> Vec<Track> {
        match self.playback.sounds.pcm(&take.key) {
            Some(pcm) => vec![Track { pcm, at: 0, from: 0, length: take.info.frames, volume: 1.0 }],
            None => Vec::new(),
        }
    }

    /// The transport's play button: plays from the frame shown, or from
    /// the start at the last frame, with the take's sound, or silently
    /// with none; or pauses.
    pub(super) fn transport_play(&mut self, playing: bool) {
        if !playing {
            self.stop_playing();
            return;
        }
        if self.playback.playing {
            return;
        }
        let Some(take) = self.playing_take() else {
            return;
        };
        let last = take.info.frames.saturating_sub(1);
        let frame = self.active_tab().map_or(0, Tab::frame_shown);
        let from = if frame >= last { 0 } else { frame };
        let tracks = self.tracks(&take);
        let player = self.player(take.info.rate);
        player.set_tracks(tracks);
        player.play_from(from);
        let playback = &mut self.playback;
        playback.playing = true;
        playback.last = last;
        playback.sent.clear();
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.frame = from;
        }
    }

    /// Pauses at the frame shown, which the tab keeps.
    pub(super) fn stop_playing(&mut self) {
        let playback = &mut self.playback;
        if !playback.playing {
            return;
        }
        if let Some(player) = playback.player.as_ref() {
            player.pause();
        }
        playback.playing = false;
        playback.sent.clear();
    }

    /// While playing: the frame the Player's clock names, shown when it is
    /// new, and the end, where playing stops at the last frame. Paced by
    /// the host's `shown`. Returns whether the frame changed.
    pub(super) fn play_tick(&mut self) -> bool {
        if !self.playback.playing || self.playback.paced() {
            return false;
        }
        let Some(frame) = self.playback.player.as_ref().and_then(Player::frame_to_show) else {
            return false;
        };
        let last = self.playback.last;
        let frame = if frame >= last {
            self.stop_playing();
            last
        } else {
            frame
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        if tab.frame == frame {
            return !self.playback.playing;
        }
        tab.frame = frame;
        true
    }

    /// A step of the transport, a frame back or forward, heard as that
    /// frame's slice of sound.
    pub(super) fn transport_step(&mut self, by: i64) {
        self.set_frame(|frame, _| i64::from(frame) + by);
        self.hear_frame();
    }

    /// A scrub of the transport, to `time` in seconds, heard as that frame's
    /// slice of sound.
    pub(super) fn transport_time(&mut self, time: f64) {
        if !time.is_finite() {
            return;
        }
        self.set_frame(|_, (_, fps)| (time.max(0.0) * fps).round() as i64);
        self.hear_frame();
    }

    /// The frame shown's slice of sound, as a step or a scrub plays it;
    /// never for a take with no sound, so a silent take opens no output.
    fn hear_frame(&mut self) {
        let Some(take) = self.playing_take() else {
            return;
        };
        let tracks = self.tracks(&take);
        if tracks.is_empty() {
            return;
        }
        let frame = self.active_tab().map_or(0, Tab::frame_shown);
        let player = self.player(take.info.rate);
        player.set_tracks(tracks);
        player.scrub(frame);
    }

    /// The host drew `revision` at `at`: the delay since it was sent keeps
    /// the Player's lead, as the mean of the last reports.
    pub(super) fn frame_shown(&mut self, revision: u64, at: u64) {
        let playback = &mut self.playback;
        let Some(position) = playback.sent.iter().position(|(sent, _)| *sent == revision) else {
            return;
        };
        let (_, sent) = playback.sent[position];
        playback.sent.drain(..=position);
        playback.delays.push_back((at as f64 - sent).max(0.0));
        if playback.delays.len() > LEAD_REPORTS {
            playback.delays.pop_front();
        }
        let mean = playback.delays.iter().sum::<f64>() / playback.delays.len() as f64;
        if let Some(player) = playback.player.as_mut() {
            player.set_lead(mean);
        }
    }

    /// Takes in the sounds decoded since the last call. Returns whether
    /// any came.
    pub(super) fn poll_sounds(&mut self) -> bool {
        self.playback.sounds.poll()
    }

    /// Writes the waveform of the take that plays at `width`, once for
    /// each sound and width, as `wave-<n>.rgba` beside the images for the
    /// host.
    pub(super) fn prepare_wave(&mut self, width: u32) {
        let Some(take) = self.playing_take() else {
            return;
        };
        let playback = &mut self.playback;
        if playback.waves.contains_key(&(take.key.clone(), width)) {
            return;
        }
        let Some(Heard::Sound { peaks, .. }) = playback.sounds.ready.get(&take.key) else {
            return;
        };
        let image = waveform::draw(peaks, width);
        let name = format!("wave-{}.rgba", playback.next_wave);
        playback.next_wave += 1;
        let _ = std::fs::create_dir_all(&self.cache);
        if let Ok(path) = media::write_frame(&self.cache, &name, &image.data, (image.width, image.height)) {
            self.next_revision += 1;
            let thumb = Thumbnail { path: path.display().to_string(), width: image.width, height: image.height, revision: self.next_revision };
            self.playback.waves.insert((take.key, width), thumb);
        }
    }

    /// The waveform of the take that plays, at `width`, once written; none
    /// for a take with no sound.
    pub fn wave(&self, width: u32) -> Option<Thumbnail> {
        let take = self.playing_take()?;
        self.playback.waves.get(&(take.key, width)).cloned()
    }

    /// The width the transport's rows are drawn at: the working area's, as
    /// the scrubber runs under it.
    pub fn wave_width(&self) -> u32 {
        self.image_size().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: u32 = 48;

    /// A two-second take, 64 × 36 at 24 frames a second, each frame its own
    /// grey.
    fn take(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.mp4"));
        let settings = media::Settings { width: 64, height: 36, rate: (24, 1), keyframes: 12, sound: None };
        let mut encoder = media::Encoder::create(&path, settings).unwrap();
        for i in 0..FRAMES {
            encoder.frame(&[(i * 5) as u8, 100, 100, 255].repeat(64 * 36)).unwrap();
        }
        encoder.finish().unwrap();
        path
    }

    /// A session with `names` as takes, playing with a silent Player whose
    /// clock only the test moves.
    fn session(label: &str, names: &[&str]) -> (Session, PathBuf) {
        let dir = std::env::temp_dir().join(format!("video-generator-play-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = Session::launch();
        for name in names {
            let file = take(&dir, name);
            session.add_take(&file, None).unwrap();
        }
        session.change(|gallery| gallery.select(0));
        let player = Player::silent(waveform::RATE, (24, 1));
        player.advance(0.0);
        session.use_player(player, (24, 1));
        (session, dir)
    }

    fn frame(session: &Session) -> u32 {
        session.active_tab().unwrap().frame_shown()
    }

    /// Catches a frame counted by a timer of its own, or a pause that loses
    /// its place: played, the frame follows the Player's clock, half a
    /// second being frame 12; paused, it stays; played on to the end, it
    /// stops at the last frame; and none of it is a step of the gallery.
    #[test]
    fn playing_follows_the_players_clock() {
        let (mut session, dir) = session("clock", &["Dawn"]);
        let edits = session.active_tab().unwrap().gallery.edits;
        session.transport_play(true);
        assert!(session.is_playing());
        session.playback.player.as_ref().unwrap().advance(500.0);
        assert!(session.play_tick());
        assert_eq!(frame(&session), 12);
        session.transport_play(false);
        assert!(!session.is_playing());
        assert!(!session.play_tick());
        assert_eq!(frame(&session), 12, "pausing lost the frame");
        session.transport_play(true);
        session.playback.player.as_ref().unwrap().advance(5000.0);
        session.play_tick();
        assert_eq!(frame(&session), FRAMES - 1);
        assert!(!session.is_playing(), "playing went on past the end");
        assert_eq!(session.active_tab().unwrap().gallery.edits, edits, "playing changed the gallery");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches a step or a scrub that moves the wrong way, or a change to
    /// the gallery that keeps playing: a step forward and back, a scrub to
    /// a time, and a take added, which pauses.
    #[test]
    fn a_step_and_a_scrub_move_the_frame() {
        let (mut session, dir) = session("step", &["Dawn"]);
        session.transport_step(1);
        assert_eq!(frame(&session), 1);
        session.transport_step(-1);
        session.transport_step(-1);
        assert_eq!(frame(&session), 0);
        session.transport_time(1.0);
        assert_eq!(frame(&session), 24);
        session.transport_play(true);
        let file = take(&dir, "Dusk");
        session.add_take(&file, None).unwrap();
        assert!(!session.is_playing(), "a change to the gallery did not pause");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches two takes side by side at different times: both are drawn
    /// at the frame of the one clock.
    #[test]
    fn side_by_side_plays_one_clock() {
        let (mut session, dir) = session("side", &["Dawn", "Dusk"]);
        session.change(|gallery| {
            let other = gallery.uid_at(1).unwrap();
            gallery.set_view(Mode::Side { other }, 0)
        });
        session.transport_play(true);
        session.playback.player.as_ref().unwrap().advance(250.0);
        session.play_tick();
        let tab = session.active_tab().unwrap();
        let scene = view::scene(&tab.gallery, (1200, 800), tab.look());
        let indices: Vec<u32> = scene.keys().iter().map(|key| key.index).collect();
        assert_eq!(indices, [6, 6]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
