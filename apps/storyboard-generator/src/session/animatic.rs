//! The storyboard in a Skills project: opening the project's animatic from
//! its assembly file, saving the film's timing as a new one, image keys, and
//! holds published as rough boards. Filing a file into the project and
//! pointing the animatic's step at it are the host's, through Skills'
//! scripts; this keeps what the storyboard read and what it would write.

use super::Session;
use crate::assembly::{Assembly, Hold, Notes};
use draw::document::DrawingId;
use std::collections::HashMap;
use std::path::Path;

impl Session {
    /// Opens an assembly file of the project as a new storyboard, shown.
    pub fn open_assembly(&mut self, path: &Path) -> Result<DrawingId, String> {
        let bytes = std::fs::read(path).map_err(|error| crate::store::reason(&error))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "It is not an animatic's assembly, or it is damaged.".to_string())?;
        let assembly = Assembly::from_value(&value)?;
        let relative = match self.project.as_deref() {
            Some(project) => path.strip_prefix(project).unwrap_or(path).to_string_lossy().replace('\\', "/"),
            None => path.display().to_string(),
        };
        let title = self.project.as_deref().and_then(Path::file_name).map_or_else(|| "Animatic".to_string(), |name| name.to_string_lossy().into_owned());
        let project = self.project.clone();
        let sounds = &mut self.sounds;
        let (film, made, read) = crate::assembly::open(&assembly, &title, &relative, &protocol::fingerprint(&bytes), |file| {
            sounds.loaded(project.as_deref(), file).map(|loaded| loaded.frames)
        });
        let palette = self.brush_palette.clone();
        let mut drawings = HashMap::new();
        for drawing in made {
            drawings.insert(drawing.id, crate::record::blank_drawing(&mut self.ids, &palette));
        }
        let id = self.allocate_storyboard();
        let name = self.fresh_name(None);
        let mut record = crate::record::Record::from_film(id, name, film, drawings, &mut self.ids, &palette);
        record.set_assembly(Some(read));
        record.set_dirty(true);
        self.settle();
        self.storyboards.push(record);
        self.active = self.storyboards.len() - 1;
        Ok(id)
    }

    /// The stage with the image key the playhead shows under the drawing,
    /// at its clip's field, when there is one.
    pub fn keyed_stage(&mut self, mut stage: protocol::Stage) -> protocol::Stage {
        let film = self.film();
        let view = self.view();
        let (clip, frame) = match view.clip.and_then(|clip| film.clip(clip)) {
            Some(clip) => (clip, view.frame),
            None => match film.clip_at(view.film_frame) {
                Some(clip) if film.shots_visible => (clip, view.film_frame - clip.start),
                _ => return stage,
            },
        };
        let Some(image) = crate::images::image_at(clip, frame).map(str::to_string) else {
            return stage;
        };
        let side = film.size[0].max(film.size[1]).min(1920);
        let [x, y, width, height] = crate::sheet::field_in(film, clip);
        let (w, h, _) = crate::look::size_for([x, y], [x + width, y + height], side);
        let project = self.project.clone();
        let folder = match &project {
            Some(project) => project.join(protocol::PROJECT_DIR).join("cache").join("storyboard-generator"),
            None => std::env::temp_dir().join(format!("storyboard-generator-keys-{}", std::process::id())),
        };
        let thumbnail = self
            .key_images
            .entry((image.clone(), w, h))
            .or_insert_with(|| {
                let pixels = crate::images::fitted(project.as_deref(), &image, (w, h))?;
                let name = format!("key-{}", protocol::fingerprint(format!("{image}|{w}x{h}").as_bytes()));
                std::fs::create_dir_all(&folder).ok()?;
                let path = media::write_frame(&folder, &name, &pixels, (w, h)).ok()?;
                Some(protocol::Thumbnail { path: path.display().to_string(), width: w, height: h, revision: u64::from_str_radix(&protocol::fingerprint(&pixels)[..12], 16).unwrap_or(1) })
            })
            .clone();
        if let Some(thumbnail) = thumbnail {
            stage.image = Some(thumbnail);
            stage.fill = None;
        }
        stage
    }

    /// A clip's holds, each with the image it would be saved as: an image
    /// key's image alone, or the rough board it was published as; none
    /// while it has drawing that is not published.
    pub(crate) fn holds_of(&self, clip: u64) -> Vec<Hold> {
        let record = self.active_storyboard();
        let Some(clip) = record.film().clip(clip) else {
            return Vec::new();
        };
        let mut changes: Vec<u32> = vec![0];
        for layer in clip.layers.iter().filter(|layer| !layer.folder) {
            changes.extend(layer.keys.iter().map(|key| key.frame).filter(|frame| *frame < clip.length));
        }
        changes.sort_unstable();
        changes.dedup();
        let boards = record.assembly().map(|read| read.boards.clone()).unwrap_or_default();
        changes
            .iter()
            .enumerate()
            .map(|(index, start)| {
                let end = changes.get(index + 1).copied().unwrap_or(clip.length);
                let showing: Vec<_> = clip.layers.iter().filter(|layer| !layer.folder && clip.shows(layer)).collect();
                let drawn = showing.iter().any(|layer| {
                    layer.key_at(*start).and_then(|key| record.drawing_of(key.drawing)).is_some_and(|drawing| !drawing.model_is_blank())
                });
                let images: Vec<&str> = showing.iter().filter_map(|layer| layer.image_at(*start)).collect();
                let published = boards.iter().find(|(at, frame, _)| *at == clip.id && frame == start).map(|(_, _, file)| file.clone());
                let image = match (drawn, images.as_slice()) {
                    (false, [only]) => Some(only.to_string()),
                    _ => published,
                };
                Hold { start: *start, length: end - start, image }
            })
            .collect()
    }

    /// The film's timing as an assembly, or why it cannot be one.
    /// `canon_length` is the film's canon length in seconds, when the
    /// project has one.
    pub fn timing(&self, canon_length: Option<f64>) -> Result<(Assembly, Notes), String> {
        let record = self.active_storyboard();
        crate::assembly::save(record.film(), record.assembly(), |clip| self.holds_of(clip), canon_length)
    }

    /// The host filed the saved timing as `path`: the storyboard reads it
    /// as what it last saved, keeping the holds it published.
    pub fn timing_saved(&mut self, assembly: Assembly, notes: &Notes, path: &str, fingerprint: &str) {
        let was = self.active_storyboard().assembly().cloned();
        let boards = was.as_ref().map(|read| read.boards.clone()).unwrap_or_default();
        let mut read = notes.read(assembly, path, fingerprint, boards);
        read.step = was.and_then(|read| read.step);
        self.active_storyboard_mut().set_assembly(Some(read));
    }

    /// A hold of a clip, from its frame `frame`, was published as a rough
    /// board whose file is `file`.
    pub fn hold_published(&mut self, clip: u64, frame: u32, file: &str) {
        let record = self.active_storyboard_mut();
        let Some(mut read) = record.assembly().cloned() else {
            return;
        };
        read.boards.retain(|(at, from, _)| !(*at == clip && *from == frame));
        read.boards.push((clip, frame, file.to_string()));
        record.set_assembly(Some(read));
    }

    /// Whether the project is a Skills project, as the host says.
    pub fn set_skills(&mut self, skills: bool) {
        self.skills = skills;
    }

    pub fn is_skills(&self) -> bool {
        self.skills
    }

    /// Publish hold: the hold of the open clip under the playhead, composited
    /// as the export draws it, written as a PNG in the project, and asked to
    /// be published as "<shot>: rough board". Returns what to send the host:
    /// the artifact, the file, and where it came from. None outside a Skills
    /// project, with no clip open, or for a hold that has nothing to publish.
    pub fn publish_hold(&mut self) -> Option<String> {
        let project = self.project.clone().filter(|_| self.skills)?;
        let view = self.view();
        let clip_id = view.clip?;
        let holds = self.holds_of(clip_id);
        let hold = holds.iter().rev().find(|hold| hold.start <= view.frame)?.clone();
        let clip = self.film().clip(clip_id)?.clone();
        let tracks = Vec::new();
        let mut plan = crate::export::plan(self.active_storyboard(), tracks, Some(project.as_path()));
        let shown = *plan.frames.get((clip.start + hold.start) as usize)?;
        let pixels = crate::export::pixels(&mut crate::look::Looker::new(), &mut plan, shown);
        let folder = project.join(protocol::PROJECT_DIR).join("cache").join("storyboard-generator");
        std::fs::create_dir_all(&folder).ok()?;
        let name = format!("{}-{}-rough-board-{}.png", self.active_id().get(), clip.name.replace(' ', "-").to_lowercase(), hold.start);
        let path = folder.join(&name);
        media::write_png(&path, &pixels, plan.size).ok()?;
        let source = path.strip_prefix(&project).ok()?.to_string_lossy().replace('\\', "/");
        let from: Vec<String> = self
            .file_path(self.active_id())
            .and_then(|file| Some(file.strip_prefix(&project).ok()?.to_string_lossy().replace('\\', "/")))
            .into_iter()
            .collect();
        let artifact = format!("{}: rough board", clip.name);
        self.publishing.insert(artifact.clone(), (self.active_id(), clip_id, hold.start));
        self.to_publish.push((artifact.clone(), source, from));
        Some(artifact)
    }

    /// The publishes asked for since the last call, for the host.
    pub fn take_publishes(&mut self) -> Vec<(String, String, Vec<String>)> {
        std::mem::take(&mut self.to_publish)
    }

    /// The host's answer to a publish: a hold asked for is now its rough
    /// board's file, or a card says why not.
    pub fn published(&mut self, artifact: &str, file: &str, error: Option<&str>) {
        let Some((storyboard, clip, frame)) = self.publishing.remove(artifact) else {
            return;
        };
        if let Some(error) = error {
            self.keep_failed(self.next_notice, "Hold not published", error);
            self.next_notice += 1;
            return;
        }
        self.note_published(artifact, file);
        if let Some(index) = self.storyboards.iter().position(|record| record.id() == storyboard) {
            let shown = self.active;
            self.active = index;
            self.hold_published(clip, frame, file);
            self.active = shown;
        }
    }

    /// Insert image key…: asks the host for an image of the project.
    pub fn ask_image_key(&mut self) {
        if self.open_clip().is_some() {
            self.file_intent = Some(crate::session::FileIntent::ImageKey);
        }
    }

    /// Makes the open clip's active layer show an image of the project from
    /// the playhead: a key that shows that image, fitted to the field.
    /// One undo step.
    pub fn insert_image_key(&mut self, image: &str) -> bool {
        let view = self.view();
        let (Some(clip), Some(layer)) = (view.clip, view.layer) else {
            return false;
        };
        let frame = view.frame;
        let image = image.to_string();
        self.change_film(|film| {
            let made = if film.clip(clip)?.layer(layer)?.keys.iter().any(|key| key.frame == frame) {
                Vec::new()
            } else {
                film.insert_key(clip, layer, frame, true)?
            };
            let row = film.clip_mut(clip)?.layer_mut(layer)?;
            let key = row.keys.iter().find(|key| key.frame == frame)?.id;
            (row.images.get(&key) != Some(&image)).then(|| {
                row.images.insert(key, image);
                made
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sounds::{RATE, clicks, wav_bytes};

    /// A project holding Last order's assembly, its keyframes' names, and
    /// sounds of its sounds' lengths.
    fn project(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("storyboard-animatic-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("library")).unwrap();
        std::fs::create_dir_all(dir.join("prompts/resources")).unwrap();
        std::fs::write(dir.join("library/2026-11-12_20-27-scene-3.wav"), wav_bytes(RATE, &clicks(192, &[]))).unwrap();
        std::fs::write(dir.join("library/2026-11-12_20-03-otis-line-1.wav"), wav_bytes(RATE, &clicks(98, &[2, 50]))).unwrap();
        let assembly = serde_json::json!({
            "length": 8.0, "fps": 24, "size": [1536, 864],
            "frames": [
                { "image": "library/2026-11-12_20-48-keyframe-wiping-2.png", "in": 0.0, "out": 3.0 },
                { "image": "library/2026-11-14_16-04-keyframe-the-bell.png", "in": 3.0, "out": 4.2 },
                { "image": "library/2026-11-14_15-37-keyframe-the-line.png", "in": 4.2, "out": 6.3 },
                { "image": "library/2026-11-14_15-40-keyframe-waiting.png", "in": 6.3, "out": 8.0 }
            ],
            "bed": { "audio": "library/2026-11-12_20-27-scene-3.wav", "gain_db": 0 },
            "clips": [
                { "audio": "library/2026-11-12_20-03-otis-line-1.wav", "from": 0.05, "to": 0.65, "at": 4.2, "gain_db": 0 },
                { "audio": "library/2026-11-12_20-03-otis-line-1.wav", "from": 2.05, "to": 3.35, "at": 4.8, "gain_db": 0 }
            ]
        });
        std::fs::write(dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json"), serde_json::to_vec_pretty(&assembly).unwrap()).unwrap();
        dir
    }

    fn opened(label: &str) -> (Session, Assembly) {
        let dir = project(label);
        let path = dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        let read = Assembly::from_value(&serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()).unwrap();
        let mut session = Session::new();
        session.set_project(&dir);
        session.open_assembly(&path).unwrap();
        (session, read)
    }

    #[test]
    fn the_project_s_assembly_opens_and_saves_back_unchanged() {
        let (session, read) = opened("open");
        let film = session.film();
        assert_eq!(film.clips.len(), 4);
        // The sounds are decoded from the project: the take is 98 frames.
        assert_eq!(film.sound_tracks[1].clips[0].source, 98);
        let record = session.active_storyboard();
        assert_eq!(record.assembly().unwrap().path, "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.to_value(), read.to_value());
    }

    #[test]
    fn drawing_over_a_keyframe_waits_for_its_board() {
        let (mut session, _) = opened("drawn");
        let second = session.film().clips[1].id;
        session.enter_clip(second);
        session.set_tool(crate::registry::Tool::Brush);
        session.begin().unwrap();
        for step in 0..4 {
            session.add_sample(draw::document::Sample { x: 100.0 + f64::from(step) * 8.0, y: 100.0, pressure: Some(1.0) }).unwrap();
        }
        session.commit().unwrap();
        session.leave_clip();
        assert_eq!(session.timing(Some(8.0)).unwrap_err(), "Shot 2 has drawing that is not published yet. Publish it first.");
        session.hold_published(second, 0, "library/shot-2-rough-board.png");
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "library/shot-2-rough-board.png");
        // Undo takes the stroke back, and the hold is the keyframe's again.
        session.undo().unwrap();
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "library/2026-11-14_16-04-keyframe-the-bell.png");
    }

    #[test]
    fn a_saved_timing_is_read_as_the_storyboard_s_own() {
        let (mut session, _) = opened("saved");
        let (second, last) = (session.film().clips[1].id, session.film().clips[3].id);
        session.resize_clip(second, true, 2);
        session.resize_clip(last, true, -2);
        let (saved, notes) = session.timing(Some(8.0)).unwrap();
        session.timing_saved(saved.clone(), &notes, "prompts/resources/2026-11-17_10-00-animatic-assembly.json", "abc");
        // Saved again untouched, it is the same file.
        let (again, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(again.to_value(), saved.to_value());
        assert_eq!(session.active_storyboard().assembly().unwrap().fingerprint, "abc");
    }

    #[test]
    fn in_a_skills_project_holds_are_published_and_sounds_stay_in_place() {
        let (mut session, _) = opened("skills");
        session.set_skills(true);
        let dir = session.project().unwrap().to_path_buf();
        // A take already in the project is placed where it is.
        let take = session.import_sound(&dir.join("library/2026-11-12_20-03-otis-line-1.wav"), None, Some(0)).unwrap();
        assert_eq!(session.film().sound(take).unwrap().1.file, "library/2026-11-12_20-03-otis-line-1.wav");
        assert!(!dir.join("sounds").exists());
        // One from outside is Skills' scripts' to file.
        let outside = std::env::temp_dir().join(format!("storyboard-animatic-outside-{}.wav", std::process::id()));
        std::fs::write(&outside, wav_bytes(RATE, &clicks(24, &[]))).unwrap();
        assert!(session.import_sound(&outside, None, None).unwrap_err().starts_with("In a Skills project"));
        // Drawing over a keyframe, then Publish hold: a PNG, asked to be published.
        let second = session.film().clips[1].id;
        session.enter_clip(second);
        session.set_tool(crate::registry::Tool::Brush);
        session.begin().unwrap();
        for step in 0..4 {
            session.add_sample(draw::document::Sample { x: 100.0 + f64::from(step) * 8.0, y: 100.0, pressure: Some(1.0) }).unwrap();
        }
        session.commit().unwrap();
        assert_eq!(session.publish_hold().as_deref(), Some("Shot 2: rough board"));
        let asked = session.take_publishes();
        assert_eq!(asked.len(), 1);
        assert!(dir.join(&asked[0].1).is_file(), "the hold was not written as a PNG");
        // The host's answer makes it the hold's image.
        session.published("Shot 2: rough board", "prompts/resources/2026-11-17_10-00-shot-2-rough-board.png", None);
        session.leave_clip();
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "prompts/resources/2026-11-17_10-00-shot-2-rough-board.png");
        // The export is the step's to make here.
        assert!(session.start_export().unwrap_err().starts_with("In a Skills project"));
    }

    #[test]
    fn an_image_key_shows_from_the_playhead() {
        let (mut session, _) = opened("image-key");
        let first = session.film().clips[0].id;
        session.enter_clip(first);
        session.seek_clip(36);
        assert!(session.insert_image_key("library/other.png"));
        let clip = session.film().clip(first).unwrap();
        assert_eq!(clip.layers[0].image_at(40), Some("library/other.png"));
        assert_eq!(clip.layers[0].image_at(10), Some("library/2026-11-12_20-48-keyframe-wiping-2.png"));
        // Two holds now: the clip is saved as two frames.
        session.leave_clip();
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames.len(), 5);
        assert_eq!((saved.frames[1].start, saved.frames[1].image.as_str()), (1.5, "library/other.png"));
        session.undo().unwrap();
        assert_eq!(session.film().clip(first).unwrap().layers[0].image_at(40), Some("library/2026-11-12_20-48-keyframe-wiping-2.png"));
    }
}
