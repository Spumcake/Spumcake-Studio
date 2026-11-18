//! Playing, pausing, looping, and scrubbing the film, by hand: none of it
//! changes the storyboard, so none is an undo step or a tool's. Any change
//! to the storyboard pauses first.

use super::Session;
use crate::playback::{Frame, Playback, Span};
use protocol::{Notice, NoticeState, Stage};

impl Session {
    /// Where the stage's frames are written: the project's cache, or, in
    /// no project, a folder of this program's own.
    fn new_playback(&self) -> Playback {
        let folder = match &self.project {
            Some(project) => project.join(protocol::PROJECT_DIR).join("cache").join("storyboard-generator"),
            None => std::env::temp_dir().join(format!("storyboard-generator-stage-{}", std::process::id())),
        };
        Playback::new(folder, format!("{}-stage", self.active_id().get()))
    }

    fn playback_mut(&mut self) -> &mut Playback {
        if self.playback.is_none() {
            self.playback = Some(self.new_playback());
        }
        self.playback.as_mut().expect("made above")
    }

    /// Plays with `player` instead of the computer's sound output.
    pub fn use_player(&mut self, player: media::Player) {
        self.stop_playing();
        let playback = std::mem::take(&mut self.playback).unwrap_or_else(|| self.new_playback());
        self.playback = Some(playback.with_player(player));
    }

    pub fn player(&mut self) -> &mut media::Player {
        self.playback_mut().player()
    }

    pub fn is_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(Playback::playing)
    }

    /// Whether the stage shows the film's images: playing or scrubbing.
    pub fn showing_frames(&self) -> bool {
        self.playback.as_ref().is_some_and(|playback| playback.running.is_some())
    }

    /// The loop button inside a clip.
    pub fn looping(&self) -> bool {
        self.looping
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
        if self.is_playing() {
            self.stop_playing();
            self.start_playing();
        }
    }

    /// The tracks the Player plays: the export's, at the listening volume.
    fn heard_tracks(&mut self) -> Vec<media::Track> {
        let listening = self.listening() as f32;
        let mut tracks = self.mix_tracks();
        for track in &mut tracks {
            track.volume *= listening;
        }
        tracks
    }

    fn prepare_frames(&mut self) {
        if self.showing_frames() {
            return;
        }
        let tracks = self.heard_tracks();
        let plan = crate::export::plan(self.active_storyboard(), tracks, self.project.as_deref());
        self.playback_mut().prepare(plan);
    }

    pub fn toggle_play(&mut self) {
        if self.is_playing() {
            self.stop_playing();
        } else {
            self.start_playing();
        }
    }

    /// Plays the film from the playhead, or inside a clip the clip's span,
    /// looping when the loop button is on.
    pub fn start_playing(&mut self) {
        let film = self.film().clone();
        let view = self.view();
        let (span, at) = match view.clip.and_then(|clip| film.clip(clip)) {
            Some(clip) => {
                let span = Span { from: clip.start, to: clip.start + clip.length, looping: self.looping };
                (span, clip.start + view.frame)
            }
            None => {
                let length = film.length();
                let at = if view.film_frame + 1 >= length { 0 } else { view.film_frame };
                (Span { from: 0, to: length, looping: false }, at)
            }
        };
        if span.to <= span.from {
            return;
        }
        self.prepare_frames();
        self.playback_mut().play(span, at);
        self.playback_mut().show(at, true);
    }

    /// Pauses at the frame showing, which the playhead keeps.
    pub fn stop_playing(&mut self) {
        let Some(playback) = self.playback.as_mut() else {
            return;
        };
        let shown = playback.running.as_ref().and_then(|running| running.shown);
        let was_playing = playback.playing();
        playback.stop();
        if was_playing && let Some(frame) = shown {
            self.seek_to_film_frame(frame);
        }
    }

    /// Moves the playhead to a frame of the film, inside the open clip when
    /// one is open.
    fn seek_to_film_frame(&mut self, frame: u32) {
        let view = self.view();
        match view.clip.and_then(|clip| self.film().clip(clip)).map(|clip| clip.start) {
            Some(start) => self.seek_clip(frame.saturating_sub(start)),
            None => self.seek_film(frame),
        }
    }

    /// A tick while playing. Returns whether the stage changed.
    pub fn playback_tick(&mut self) -> bool {
        let Some(playback) = self.playback.as_mut() else {
            return false;
        };
        let (changed, ended) = playback.tick();
        let shown = playback.running.as_ref().and_then(|running| running.shown);
        if let Some(end) = ended {
            self.seek_to_film_frame(end);
        } else if changed && let Some(frame) = shown {
            self.seek_to_film_frame(frame);
        }
        changed || ended.is_some()
    }

    /// Plays the slice of sound of the frame under the playhead, as a step
    /// does. Never with no sound on the film, so a silent film opens no
    /// sound output.
    pub fn hear_playhead(&mut self) {
        if self.film().sound_tracks.is_empty() {
            return;
        }
        let view = self.view();
        let film_frame = match view.clip.and_then(|clip| self.film().clip(clip)).map(|clip| clip.start) {
            Some(start) => start + view.frame,
            None => view.film_frame,
        };
        let tracks = self.heard_tracks();
        let player = self.player();
        player.set_tracks(tracks);
        player.scrub(film_frame);
    }

    /// The host drew a revision of the stage's image.
    pub fn frame_shown(&mut self, revision: u64, at: f64) {
        if let Some(playback) = self.playback.as_mut() {
            playback.shown(revision, at);
        }
    }

    /// Dragging the playhead: `live` while it moves, scrubbing that frame's
    /// sound and showing its image; once more when let go, which shows the
    /// drawing at that frame again. `frame` is the timeline's: inside a clip,
    /// the clip's frame.
    pub fn scrub(&mut self, frame: u32, live: bool) {
        if self.is_playing() {
            self.stop_playing();
        }
        let film_frame = match self.open_clip().and_then(|clip| self.film().clip(clip)).map(|clip| clip.start) {
            Some(start) => start + frame,
            None => frame,
        };
        if live {
            self.prepare_frames();
            self.playback_mut().scrub(film_frame);
        } else if let Some(playback) = self.playback.as_mut() {
            playback.stop();
        }
        self.seek_to_film_frame(film_frame);
    }

    /// The stage while playing or scrubbing: only the film's image, with
    /// the drawing hidden and nothing over or under it.
    pub fn playing_stage(&self, mut stage: Stage) -> Stage {
        let Some(frame) = self.playback.as_ref().and_then(|playback| playback.frame.clone()) else {
            return stage;
        };
        stage.underlays.clear();
        stage.overlays.clear();
        stage.active.visible = false;
        match frame {
            Frame::Image(image) => {
                stage.image = Some(image);
                stage.fill = None;
            }
            Frame::Fill(rgb) => {
                stage.image = None;
                stage.fill = Some(rgb);
            }
        }
        stage
    }

    /// The listening volume, heard at once while playing.
    pub fn hear_listening(&mut self) {
        if self.is_playing() {
            let tracks = self.heard_tracks();
            self.player().set_tracks(tracks);
        }
    }

    /// Why there is no sound while playing, as a card.
    pub(crate) fn playback_notice(&self) -> Option<Notice> {
        let playback = self.playback.as_ref()?;
        if !playback.playing() {
            return None;
        }
        let problem = playback.problem()?;
        Some(Notice {
            id: "no-sound".to_string(),
            text: "Playing without sound".to_string(),
            detail: problem,
            progress: None,
            state: NoticeState::Working,
            actions: Vec::new(),
            close: None,
        })
    }
}
