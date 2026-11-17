//! What the person does with the film's sound, by hand or through a tool:
//! importing a sound, placing, moving, and trimming it, its volume, the
//! tracks' lock, eye, and speaker, and the listening volume. Each change to
//! the film is one undo step; the listening volume never is.

use super::Session;
use super::film::Picked;
use crate::soundtrack::{TrackFlag, TrackRef};
use crate::sounds::{Imported, Library, Loaded, State};
use media::Track;
use protocol::{Action, Notice, NoticeState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// A sound being imported by hand: decoded and copied on a thread of its
/// own, then placed where the playhead was when it was chosen.
pub struct Import {
    id: u64,
    name: String,
    storyboard: draw::document::DrawingId,
    at: u32,
    track: Option<u64>,
    result: Receiver<Result<Imported, String>>,
}

impl Session {
    /// The decoded sounds, with the project they are found in.
    pub fn sound_library(&mut self) -> (Option<&Path>, &mut Library) {
        (self.project.as_deref(), &mut self.sounds)
    }

    /// Decodes every sound the active storyboard names that is not decoded
    /// yet, so its timeline can be declared.
    pub fn load_sounds(&mut self) {
        let files: Vec<String> = self.film().sounds().map(|(_, clip)| clip.file.clone()).collect();
        for file in files {
            self.sounds.get(self.project.as_deref(), &file);
        }
    }

    /// What is known of a sound's file, without reading it.
    pub fn peek_sound(&self, file: &str) -> Option<&State> {
        self.sounds.peek(self.project.as_deref(), file)
    }

    /// A sound clip's decoded sound, or why there is none.
    pub fn sound_state(&mut self, file: &str) -> State {
        self.sounds.get(self.project.as_deref(), file)
    }

    pub fn loaded_sound(&mut self, file: &str) -> Option<Arc<Loaded>> {
        self.sounds.loaded(self.project.as_deref(), file)
    }

    /// Every item of the active storyboard, whose file is `path` in the
    /// project.
    pub fn items(&mut self, boards: &mut crate::board::Boards, looker: &mut crate::look::Looker, path: &str) -> Vec<protocol::Item> {
        let record = &self.storyboards[self.active];
        boards.items(looker, record, &mut self.sounds, self.project.as_deref(), path)
    }

    /// The cut's fingerprint, now.
    pub fn cut_fingerprint(&mut self) -> String {
        let film = &self.storyboards[self.active].film().clone();
        crate::board::cut_fingerprint(film, &mut self.sounds, self.project.as_deref())
    }

    /// The tracks `media` mixes for the active storyboard's film.
    pub fn mix_tracks(&mut self) -> Vec<Track> {
        let film = self.film().clone();
        self.sounds.tracks(self.project.as_deref(), &film)
    }

    /// The files the active storyboard's sounds name that are missing or
    /// cannot be read.
    pub fn absent_sounds(&mut self) -> Vec<String> {
        let film = self.film().clone();
        self.sounds.absent(self.project.as_deref(), &film)
    }

    /// The peaks of the film's sound over `frames` frames from `from`, every
    /// unmuted Sound track mixed, from the sounds already decoded.
    pub fn film_sound_peaks(&self, from: u32, frames: u32) -> Vec<f32> {
        let mut tracks = Vec::new();
        for track in self.film().sound_tracks.iter().filter(|track| !track.muted) {
            for clip in &track.clips {
                if let Some(State::Loaded(loaded)) = self.sounds.peek(self.project.as_deref(), &clip.file) {
                    tracks.push(Track { pcm: loaded.sound.clone(), at: clip.at, from: clip.from, length: clip.length, volume: clip.volume as f32 });
                }
            }
        }
        if tracks.is_empty() {
            return vec![0.0; frames as usize * crate::sounds::PEAKS_PER_FRAME];
        }
        crate::sounds::peaks_of(&media::mix(&tracks, from, frames, crate::sounds::RATE, crate::sounds::fps()))
    }

    /// How loud the film's sound is on each frame from `from`.
    pub fn loudness(&mut self, from: u32, frames: u32) -> Vec<f32> {
        let tracks = self.mix_tracks();
        crate::sounds::loudness(&tracks, from, frames)
    }

    /// Decodes and copies a sound into the project, and places it, as one
    /// undo step: at `at` on the film, or the playhead, on `track` when it
    /// fits there. Returns the sound clip's id. A file that is not sound is
    /// neither copied nor placed, and the error says why.
    pub fn import_sound(&mut self, path: &Path, track: Option<u64>, at: Option<u32>) -> Result<u64, String> {
        let imported = crate::sounds::import(self.project.as_deref(), path)?;
        self.place_imported(imported, track, at).ok_or_else(|| "The sound could not be placed.".to_string())
    }

    /// Import sound…: asks the host for a file.
    pub fn ask_import_sound(&mut self) {
        self.file_intent = Some(crate::session::FileIntent::ImportSound);
    }

    /// Starts importing the sound the person chose. It is placed when it
    /// is decoded, at the playhead as it is now, and a notice says it is
    /// coming.
    pub fn start_import(&mut self, path: PathBuf) {
        let (send, result) = std::sync::mpsc::channel();
        let project = self.project.clone();
        let source = path.clone();
        std::thread::spawn(move || {
            let _ = send.send(crate::sounds::import(project.as_deref(), &source));
        });
        let id = self.next_notice;
        self.next_notice += 1;
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        self.imports.push(Import {
            id,
            name,
            storyboard: self.active_id(),
            at: self.view().film_frame,
            track: self.picked_track(),
            result,
        });
    }

    /// Places the imports that finished. Returns whether any did.
    pub fn poll_imports(&mut self) -> bool {
        let mut changed = false;
        let mut waiting = Vec::new();
        for import in std::mem::take(&mut self.imports) {
            match import.result.try_recv() {
                Err(TryRecvError::Empty) => waiting.push(import),
                Err(TryRecvError::Disconnected) => {
                    self.keep_failed(import.id, "Sound not imported", "The import stopped unexpectedly.");
                    changed = true;
                }
                Ok(Err(reason)) => {
                    self.keep_failed(import.id, "Sound not imported", &reason);
                    changed = true;
                }
                Ok(Ok(imported)) => {
                    // Placed in the storyboard it was chosen for, if it is still open.
                    if let Some(index) = self.storyboards.iter().position(|record| record.id() == import.storyboard) {
                        let shown = self.active;
                        self.active = index;
                        self.place_imported(imported, import.track, Some(import.at));
                        self.active = shown;
                    }
                    changed = true;
                }
            }
        }
        self.imports = waiting;
        changed
    }

    fn keep_failed(&mut self, id: u64, text: &str, detail: &str) {
        self.kept_notices.push(Notice {
            id: format!("notice-{id}"),
            text: text.to_string(),
            detail: detail.to_string(),
            progress: None,
            state: NoticeState::Failed,
            actions: Vec::new(),
            close: Some(Action::with_arg("close_notice", serde_json::json!(format!("notice-{id}")))),
        });
    }

    /// Keeps a notice until it is closed.
    pub(crate) fn keep_notice(&mut self, notice: Notice) {
        self.kept_notices.retain(|kept| kept.id != notice.id);
        self.kept_notices.push(notice);
    }

    pub fn close_notice(&mut self, id: &str) -> bool {
        let before = self.kept_notices.len();
        self.kept_notices.retain(|notice| notice.id != id);
        self.kept_notices.len() != before
    }

    /// The cards at the stage's bottom right: imports under way, then
    /// what stays until it is closed.
    pub fn sound_notices(&self) -> Vec<Notice> {
        let mut out: Vec<Notice> = self
            .imports
            .iter()
            .map(|import| Notice {
                id: format!("notice-{}", import.id),
                text: format!("Importing {}", import.name),
                detail: String::new(),
                progress: None,
                state: NoticeState::Working,
                actions: Vec::new(),
                close: None,
            })
            .collect();
        out.extend(self.playback_notice());
        out.extend(self.export_notice());
        out.extend(self.kept_notices.iter().cloned());
        out
    }

    /// Whether an import is under way.
    pub fn importing(&self) -> bool {
        !self.imports.is_empty()
    }

    /// Places a sound an import decoded, as one undo step.
    pub fn place_imported(&mut self, imported: Imported, track: Option<u64>, at: Option<u32>) -> Option<u64> {
        let Imported { file, name, loaded, path } = imported;
        let frames = loaded.frames;
        self.sounds.insert(path, loaded);
        let at = at.unwrap_or_else(|| self.view().film_frame);
        let track = track.or_else(|| self.picked_track());
        let mut made = None;
        self.change_film(|film| {
            made = film.place_sound(track, at, &file, &name, frames).map(|(_, clip)| clip);
            made.map(|_| Vec::new())
        });
        if let Some(clip) = made {
            self.select_sounds(vec![clip]);
        }
        made
    }

    /// The Sound track of the selected sound, where an import lands first.
    fn picked_track(&self) -> Option<u64> {
        let film = self.film();
        self.picked().sounds.iter().find_map(|sound| film.sound(*sound).map(|(track, _)| track.id))
    }

    pub fn add_sound_track(&mut self) -> Option<u64> {
        let mut made = None;
        self.change_film(|film| {
            made = Some(film.add_sound_track());
            Some(Vec::new())
        });
        made
    }

    pub fn rename_sound_track(&mut self, track: u64, name: &str) -> bool {
        self.change_film(|film| film.rename_sound_track(track, name).then(Vec::new))
    }

    pub fn delete_sound_track(&mut self, track: u64) -> bool {
        self.change_film(|film| film.delete_sound_track(track).then(Vec::new))
    }

    pub fn set_track_flag(&mut self, track: TrackRef, flag: TrackFlag, on: bool) -> bool {
        self.change_film(|film| film.set_track_flag(track, flag, on).then(Vec::new))
    }

    pub fn move_sounds(&mut self, sounds: &[u64], to: Option<u64>, frames: i64, push: bool) -> bool {
        self.change_film(|film| film.move_sounds(sounds, to, frames, push).then(Vec::new))
    }

    pub fn trim_sound(&mut self, sound: u64, end: bool, frames: i64) -> bool {
        self.change_film(|film| film.trim_sound(sound, end, frames).then(Vec::new))
    }

    pub fn set_sound_span(&mut self, sound: u64, at: Option<u32>, from: Option<u32>, length: Option<u32>) -> bool {
        self.change_film(|film| film.set_sound_span(sound, at, from, length).then(Vec::new))
    }

    pub fn set_sound_volume(&mut self, sound: u64, volume: f64) -> bool {
        self.change_film(|film| film.set_sound_volume(sound, volume).then(Vec::new))
    }

    pub fn delete_sound(&mut self, sound: u64) -> bool {
        self.change_film(|film| film.delete_sound(sound).then(Vec::new))
    }

    /// Selects sounds on the Sound tracks, as clicking them does. Clips and
    /// frames are let go.
    pub fn select_sounds(&mut self, sounds: Vec<u64>) {
        self.set_picked(Picked { sounds, ..Picked::default() });
    }

    /// Shows a Sound track's page in the panel, to rename it.
    pub fn pick_track(&mut self, track: u64) {
        self.set_picked(Picked { track: Some(track), ..Picked::default() });
        self.open_panel();
    }

    /// How loud the film plays here. A listening aid, saved with the
    /// storyboard but never an undo step, and never in the export.
    pub fn listening(&self) -> f64 {
        self.active_storyboard().listening()
    }

    pub fn set_listening(&mut self, volume: f64) -> bool {
        let changed = self.active_storyboard_mut().set_listening(volume);
        self.hear_listening();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sounds::{RATE, clicks, wav_bytes};

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("storyboard-session-sound-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_import_is_one_undo_step_and_its_file_stays() {
        let project = scratch("undo");
        let source = project.join("line.wav");
        std::fs::write(&source, wav_bytes(RATE, &clicks(24, &[5]))).unwrap();
        let mut session = Session::new();
        session.set_project(&project);
        // The playhead stays on the film, so the film needs a clip first.
        session.add_clip(None, 48);
        session.seek_film(12);
        let clip = session.import_sound(&source, None, None).unwrap();
        let (track, placed) = session.film().sound(clip).unwrap();
        assert_eq!((placed.at, placed.length, placed.file.as_str()), (12, 24, "sounds/line.wav"));
        assert_eq!(track.name, "Sound");
        // The click is heard on its frame.
        let loud = session.loudness(12, 24);
        assert!(loud[5] > 0.5 && loud[4] < 0.01, "{loud:?}");
        session.undo().unwrap();
        assert!(session.film().sound_tracks.is_empty());
        assert_eq!(session.film().clips.len(), 1);
        assert!(project.join("sounds/line.wav").is_file());
        session.redo().unwrap();
        assert!(session.film().sound(clip).is_some());
    }

    #[test]
    fn a_locked_shots_track_refuses_its_edits() {
        let mut session = Session::new();
        let clip = session.add_clip(None, 24).unwrap();
        assert!(session.set_track_flag(TrackRef::Shots, TrackFlag::Locked, true));
        assert!(session.add_clip(None, 24).is_none());
        assert!(!session.move_clips(&[clip], 10, true));
        assert!(!session.resize_clip(clip, true, 10));
        assert!(!session.delete_clip(clip));
        assert!(session.duplicate_clip(clip).is_none());
    }
}
