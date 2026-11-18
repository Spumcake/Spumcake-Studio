//! The storyboard as the project's animatic, in every project, organized the
//! way the Last order benchmark is: opening the animatic from its assembly
//! file, saving the film's timing as a new one, image keys, and holds
//! published as rough boards. Filing a file into the project and pointing
//! the animatic's step at it are the host's, through Spumcake-Skills'
//! scripts; this keeps what the storyboard read and what it would write.

use super::Session;
use crate::assembly::{Assembly, Hold, Notes, RoughBoard};
use crate::film::Clip;
use crate::record::Record;
use draw::document::DrawingId;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

/// A stamp of what a clip's hold from `start` shows: each layer that shows,
/// its key there, the key's image, and its drawing. Drawing on the hold
/// again changes it.
pub(crate) fn hold_stamp(record: &Record, clip: &Clip, start: u32) -> String {
    let shown: Vec<Value> = clip
        .layers
        .iter()
        .filter(|layer| !layer.folder && clip.shows(layer))
        .map(|layer| {
            let key = layer.key_at(start);
            let nodes = key.and_then(|key| record.drawing_of(key.drawing)).map(|drawing| drawing.snapshot_value(DrawingId::new(1), "")["nodes"].clone());
            json!([layer.id, layer.opacity, layer.outline, key.map(|key| key.id), layer.image_at(start), nodes])
        })
        .collect();
    protocol::fingerprint(Value::Array(shown).to_string().as_bytes())
}

/// A hold's rough board, as `build-animatic` names it, under the film as
/// Last order names a shot's images: "Last order: rough board, Shot 2",
/// as the owner approved on 2026-11-17.
pub fn rough_board(film: &str, shot: &str) -> String {
    format!("{film}: rough board, {shot}")
}

impl Session {
    /// Opens an assembly file of the project as a new storyboard, its film
    /// titled `title`, the film's, shown in place of an untouched blank. It
    /// is named as every new storyboard is.
    pub fn open_assembly(&mut self, path: &Path, title: &str) -> Result<DrawingId, String> {
        let bytes = std::fs::read(path).map_err(|error| crate::store::reason(&error))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "It is not an animatic's assembly, or it is damaged.".to_string())?;
        let assembly = Assembly::from_value(&value)?;
        let relative = match self.project.as_deref() {
            Some(project) => path.strip_prefix(project).unwrap_or(path).to_string_lossy().replace('\\', "/"),
            None => path.display().to_string(),
        };
        let project = self.project.clone();
        let sounds = &mut self.sounds;
        let (film, made, read) = crate::assembly::open(&assembly, title, &relative, &protocol::fingerprint(&bytes), |file| {
            sounds.loaded(project.as_deref(), file).map(|loaded| loaded.frames)
        });
        let palette = self.brush_palette.clone();
        let mut drawings = HashMap::new();
        for drawing in made {
            drawings.insert(drawing.id, crate::record::blank_drawing(&mut self.ids, &palette));
        }
        let blank = self.untouched_blank();
        let name = self.fresh_name(blank);
        let id = self.allocate_storyboard();
        let mut record = crate::record::Record::from_film(id, name, film, drawings, &mut self.ids, &palette);
        record.set_assembly(Some(read));
        record.set_dirty(true);
        self.settle();
        self.storyboards.push(record);
        self.active = self.storyboards.len() - 1;
        self.drop_blank(blank);
        Ok(id)
    }

    /// The only storyboard, when it is a blank nothing was done to, which a
    /// storyboard opened in its place replaces.
    pub(crate) fn untouched_blank(&self) -> Option<DrawingId> {
        let [only] = self.storyboards.as_slice() else {
            return None;
        };
        (only.is_clean() && only.path().is_none() && self.text_draft.is_none()).then(|| only.id())
    }

    /// Puts the storyboard just opened in the place of the blank it was
    /// opened in place of, under the blank's own id, so the window's tab
    /// for it stays the same tab and shows it.
    pub(crate) fn drop_blank(&mut self, blank: Option<DrawingId>) {
        let Some(index) = blank.and_then(|id| self.index_of(id)) else {
            return;
        };
        if self.storyboards.len() < 2 || index == self.active {
            return;
        }
        let mut opened = self.storyboards.remove(self.active);
        let index = self.index_of(blank.expect("found above")).expect("still there");
        opened.set_id(self.storyboards[index].id());
        // The window fitted the blank's view once for this tab and will not
        // again: a storyboard with no view of its own keeps the blank's.
        if !opened.has_camera(None)
            && let Some(camera) = self.storyboards[index].camera_of(None)
        {
            opened.set_cameras(vec![(None, camera)]);
        }
        self.storyboards[index] = opened;
        self.active = index;
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
                let image = match (drawn, images.as_slice()) {
                    (false, [only]) => Some(only.to_string()),
                    _ => {
                        let stamp = hold_stamp(record, clip, *start);
                        record.assembly().and_then(|read| read.board(clip.id, *start, &stamp)).map(|board| board.file.clone())
                    }
                };
                Hold { start: *start, length: end - start, image, blank: !drawn && images.is_empty() }
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

    /// The host filed the saved timing of `storyboard` as `path`: the
    /// storyboard reads it as what it last saved, keeping the holds it
    /// published. False when the storyboard was closed.
    pub fn timing_saved(&mut self, storyboard: DrawingId, assembly: Assembly, notes: &Notes, path: &str, fingerprint: &str) -> bool {
        let Some(index) = self.index_of(storyboard) else {
            return false;
        };
        let record = &mut self.storyboards[index];
        let was = record.assembly().cloned();
        let boards = was.as_ref().map(|read| read.boards.clone()).unwrap_or_default();
        let mut read = notes.read(assembly, path, fingerprint, boards);
        read.step = was.and_then(|read| read.step);
        record.set_assembly(Some(read));
        true
    }

    /// A hold of a clip of `storyboard`, from its frame `frame`, showing
    /// what `stamp` stamps, was published as a rough board whose file is
    /// `file`.
    pub fn hold_published(&mut self, storyboard: DrawingId, clip: u64, frame: u32, stamp: &str, file: &str) {
        let Some(index) = self.index_of(storyboard) else {
            return;
        };
        let record = &mut self.storyboards[index];
        let Some(mut read) = record.assembly().cloned() else {
            return;
        };
        read.boards.retain(|board| !(board.clip == clip && board.frame == frame));
        read.boards.push(RoughBoard { clip, frame, file: file.to_string(), stamp: stamp.to_string() });
        record.set_assembly(Some(read));
    }

    /// A sound's gain above 0 dB, as the animatic's assembly has it, while
    /// it plays at 100% and is kept as read.
    pub fn kept_gain(&self, sound: u64) -> Option<f64> {
        let read = self.active_storyboard().assembly()?;
        let gain_db = if read.bed == sound {
            read.assembly.bed.gain_db
        } else {
            let index = read.voices.iter().position(|id| *id == sound)?;
            read.assembly.clips.get(index)?.gain_db
        };
        let (_, playing) = self.film().sound(sound)?;
        (gain_db > 0.0 && playing.volume >= 1.0).then_some(gain_db)
    }

    /// Publish hold: the hold of the open clip under the playhead, composited
    /// as the export draws it, written as a PNG in `work/`, and asked to
    /// be published as its rough board, made from the images its keys
    /// show. The storyboard in `work/` is never named as an input. Returns
    /// the artifact asked for, or why not.
    pub fn publish_hold(&mut self) -> Result<String, String> {
        let project = self.project.clone().ok_or("No project is open.")?;
        let nothing = || "that hold has nothing to publish".to_string();
        let view = self.view();
        let clip_id = view.clip.ok_or_else(nothing)?;
        let holds = self.holds_of(clip_id);
        let hold = holds.iter().rev().find(|hold| hold.start <= view.frame).ok_or_else(nothing)?.clone();
        let clip = self.film().clip(clip_id).ok_or_else(nothing)?.clone();
        let tracks = Vec::new();
        let mut plan = crate::export::plan(self.active_storyboard(), tracks, Some(project.as_path()));
        let shown = *plan.frames.get((clip.start + hold.start) as usize).ok_or_else(nothing)?;
        let pixels = crate::export::pixels(&mut crate::look::Looker::new(), &mut plan, shown);
        // Written in `work/`, the setup's own folder, which the host files
        // into the project with `request.py resource` before publishing.
        let folder = project.join("work");
        std::fs::create_dir_all(&folder).map_err(|error| crate::store::reason(&error))?;
        let base = format!("{}-{}-rough-board-{}", self.active_id().get(), clip.name.replace(' ', "-").to_lowercase(), hold.start + 1);
        let name = (1..1000).map(|n| if n == 1 { format!("{base}.png") } else { format!("{base}-{n}.png") }).find(|name| !folder.join(name).exists()).unwrap_or_else(|| format!("{base}.png"));
        // As RGB, as keyframes are: `assemble.py` joins its frames with
        // ffmpeg's concat, which cuts the video short where one frame's
        // pixel format differs from the others'.
        let rgb: Vec<u8> = pixels.chunks_exact(4).flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
        let image = image::RgbImage::from_raw(plan.size.0, plan.size.1, rgb).ok_or("that hold has nothing to publish")?;
        image.save(folder.join(&name)).map_err(|error| error.to_string())?;
        let source = format!("work/{name}");
        // Its inputs are the images under its drawing, such as the canon
        // keyframe, so `touches` follows keyframe to rough board.
        let mut from: Vec<String> = Vec::new();
        for image in clip.layers.iter().filter(|layer| !layer.folder && clip.shows(layer)).filter_map(|layer| layer.image_at(hold.start)) {
            if !from.iter().any(|seen| seen == image) {
                from.push(image.to_string());
            }
        }
        let stamp = hold_stamp(self.active_storyboard(), &clip, hold.start);
        let artifact = rough_board(&self.film_name(), &clip.name);
        self.publishing.insert(artifact.clone(), (self.active_id(), clip_id, hold.start, stamp, folder.join(&name)));
        self.to_publish.push((artifact.clone(), source, from));
        Ok(artifact)
    }

    /// The publishes asked for since the last call, for the host.
    pub fn take_publishes(&mut self) -> Vec<(String, String, Vec<String>)> {
        std::mem::take(&mut self.to_publish)
    }

    /// The host's answer to a publish: a hold asked for is now its rough
    /// board's file, or a card says why not.
    pub fn published(&mut self, artifact: &str, file: &str, error: Option<&str>) {
        // Whatever was published, a hold or a sound, the animatic can name it.
        if error.is_none() && !file.is_empty() {
            self.note_published(artifact, file);
        }
        let Some((storyboard, clip, frame, stamp, written)) = self.publishing.remove(artifact) else {
            return;
        };
        // Filed, or not wanted: the copy in work/ has done its work.
        let _ = std::fs::remove_file(&written);
        if let Some(error) = error {
            self.keep_failed(self.next_notice, "Hold not published", error);
            self.next_notice += 1;
            return;
        }
        // Cancelled on the publish card: nothing was published.
        if file.is_empty() {
            return;
        }
        self.hold_published(storyboard, clip, frame, &stamp, file);
    }

    /// An image's path in the project, from a path a dialog or a tool
    /// gave, or why it cannot be an image key: it must be a file inside the
    /// project's folder.
    pub fn image_in_project(&self, image: &Path) -> Result<String, String> {
        let outside = || "Only images inside the project's folder can be image keys.".to_string();
        let project = self.project.as_deref().ok_or_else(outside)?;
        let full = if image.is_absolute() { image.to_path_buf() } else { project.join(image) };
        let (Ok(full), Ok(root)) = (full.canonicalize(), project.canonicalize()) else {
            return Err(outside());
        };
        let relative = full.strip_prefix(&root).map_err(|_| outside())?;
        if !full.is_file() {
            return Err(outside());
        }
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    /// Insert image key… by hand: the image the dialog chose, or a card
    /// saying why it cannot be one.
    pub fn image_key_chosen(&mut self, image: &Path) {
        match self.image_in_project(image) {
            Ok(relative) => {
                self.insert_image_key(&relative);
            }
            Err(reason) => {
                self.keep_failed(self.next_notice, &reason, "");
                self.next_notice += 1;
            }
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
        session.open_assembly(&path, "Last order").unwrap();
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
        stroke(&mut session, 100.0);
        session.leave_clip();
        assert_eq!(session.timing(Some(8.0)).unwrap_err(), "Shot 2 has drawing that is not published yet. Publish it first.");
        let stamp = stamp_of(&session, second);
        session.hold_published(session.active_id(), second, 0, &stamp, "library/shot-2-rough-board.png");
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "library/shot-2-rough-board.png");
        // Drawn on again, the board is out of date until it is published again.
        session.enter_clip(second);
        stroke(&mut session, 200.0);
        session.leave_clip();
        assert_eq!(session.timing(Some(8.0)).unwrap_err(), "Shot 2 has drawing that is not published yet. Publish it first.");
        // Undo takes that stroke back, and the board is the hold's again.
        session.undo().unwrap();
        assert_eq!(session.timing(Some(8.0)).unwrap().0.frames[1].image, "library/shot-2-rough-board.png");
        // Undo takes the first stroke back, and the hold is the keyframe's again.
        session.undo().unwrap();
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "library/2026-11-14_16-04-keyframe-the-bell.png");
    }

    fn stroke(session: &mut Session, y: f64) {
        session.set_tool(crate::registry::Tool::Brush);
        session.begin().unwrap();
        for step in 0..4 {
            session.add_sample(draw::document::Sample { x: 100.0 + f64::from(step) * 8.0, y, pressure: Some(1.0) }).unwrap();
        }
        session.commit().unwrap();
    }

    fn stamp_of(session: &Session, clip: u64) -> String {
        hold_stamp(session.active_storyboard(), session.film().clip(clip).unwrap(), 0)
    }

    #[test]
    fn a_saved_timing_is_read_as_the_storyboard_s_own() {
        let (mut session, _) = opened("saved");
        let (second, last) = (session.film().clips[1].id, session.film().clips[3].id);
        session.resize_clip(second, true, 2);
        session.resize_clip(last, true, -2);
        let (saved, notes) = session.timing(Some(8.0)).unwrap();
        session.timing_saved(session.active_id(), saved.clone(), &notes, "prompts/resources/2026-11-17_10-00-animatic-assembly.json", "abc");
        // Saved again untouched, it is the same file.
        let (again, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(again.to_value(), saved.to_value());
        assert_eq!(session.active_storyboard().assembly().unwrap().fingerprint, "abc");
    }

    #[test]
    fn holds_are_published_and_sounds_are_filed_into_the_project() {
        let (mut session, _) = opened("skills");
        // The project's film, as its canon names it.
        session.film_element = Some("Last order".to_string());
        let dir = session.project().unwrap().to_path_buf();
        // A take already in the project is placed where it is.
        let take = session.import_sound(&dir.join("library/2026-11-12_20-03-otis-line-1.wav"), None, Some(0)).unwrap();
        assert_eq!(session.film().sound(take).unwrap().1.file, "library/2026-11-12_20-03-otis-line-1.wav");
        assert!(!dir.join("sounds").exists());
        // One from outside is copied into work/ and filed through the host,
        // then placed where it was filed.
        let outside = std::env::temp_dir().join(format!("storyboard-animatic-outside-{}.wav", std::process::id()));
        std::fs::write(&outside, wav_bytes(RATE, &clicks(24, &[]))).unwrap();
        session.start_import(outside.clone());
        let Some(protocol::AppMessage::FileResource { path, id: Some(id), .. }) = session.take_outgoing().pop() else {
            panic!("the sound was not asked to be filed");
        };
        assert_eq!(path, format!("work/{}", outside.file_name().unwrap().to_string_lossy()));
        assert!(dir.join(&path).is_file());
        std::fs::create_dir_all(dir.join("prompts/resources")).unwrap();
        std::fs::copy(&outside, dir.join("prompts/resources/2026-11-17_10-00-outside.wav")).unwrap();
        session.filed(&id, "prompts/resources/2026-11-17_10-00-outside.wav", None);
        // Filed, it is offered as canon under the film, after its file.
        let Some(protocol::AppMessage::Publish { artifact, source, .. }) = session.take_outgoing().pop() else {
            panic!("the sound was not offered for publishing");
        };
        let stem = outside.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(artifact, crate::session::sound::sound_artifact(&session.film_name(), &stem));
        assert_eq!(source, "prompts/resources/2026-11-17_10-00-outside.wav");
        // Placed under the name it had outside, so Publish sound offers it
        // again under the same artifact, after a Cancel on the card.
        let started = std::time::Instant::now();
        while !session.poll_imports() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let placed = session.film().sounds().find(|(_, clip)| clip.file == "prompts/resources/2026-11-17_10-00-outside.wav").map(|(_, clip)| clip.id).unwrap();
        assert_eq!(session.publish_sound(placed).unwrap(), artifact);
        let Some(protocol::AppMessage::Publish { artifact: again, source, .. }) = session.take_outgoing().pop() else {
            panic!("Publish sound offered nothing");
        };
        assert_eq!((again, source.as_str()), (artifact.clone(), "prompts/resources/2026-11-17_10-00-outside.wav"));
        // A take already in the project, after its own file; once it is
        // canon, it has nothing to publish.
        assert_eq!(session.publish_sound(take).unwrap(), "Last order: 2026-11-12_20-03-otis-line-1, sound");
        session.published("Last order: 2026-11-12_20-03-otis-line-1, sound", "library/2026-11-12_20-03-otis-line-1.wav", None);
        assert!(session.is_canon("library/2026-11-12_20-03-otis-line-1.wav"));
        assert!(session.publish_sound(take).unwrap_err().starts_with("that sound is already canon"));
        assert!(!dir.join(&path).exists(), "the copy in work/ was kept");
        // Drawing over a keyframe, then Publish hold: a PNG, asked to be published.
        let second = session.film().clips[1].id;
        session.enter_clip(second);
        stroke(&mut session, 100.0);
        assert_eq!(session.publish_hold().unwrap(), "Last order: rough board, Shot 2");
        let asked = session.take_publishes();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].1.starts_with("work/") && dir.join(&asked[0].1).is_file(), "the hold was not written as a PNG in work/");
        // Made from the keyframe under it, never from the storyboard in work/.
        assert_eq!(asked[0].2, ["library/2026-11-14_16-04-keyframe-the-bell.png"]);
        // The host's answer makes it the hold's image.
        session.published("Last order: rough board, Shot 2", "prompts/resources/2026-11-17_10-00-shot-2-rough-board.png", None);
        session.leave_clip();
        let (saved, _) = session.timing(Some(8.0)).unwrap();
        assert_eq!(saved.frames[1].image, "prompts/resources/2026-11-17_10-00-shot-2-rough-board.png");
        // Nothing was made outside the project's own folders.
        assert!(!dir.join("sounds").exists() && !dir.join("exports").exists());
    }

    #[test]
    fn the_export_runs_the_step_and_offers_its_take() {
        let mut session = from_its_step("export-step");
        // Untouched: the step runs at once.
        let step = session.start_export().unwrap();
        assert_eq!(step, "the animatic: animatic");
        let Some(protocol::AppMessage::RunStep { id, step, .. }) = session.take_outgoing().pop() else {
            panic!("the step was not run");
        };
        assert_eq!((step.chain.as_str(), step.step.as_str()), ("the animatic", "animatic"));
        assert_eq!(session.animatic_state()["state"], "exporting");
        // Pressed again while it runs: no second run, and no card says so.
        assert!(session.start_export().is_err());
        assert!(session.take_outgoing().is_empty());
        assert!(!session.kept_notices.iter().any(|notice| notice.text == "Animatic video not exported"));
        let dir = session.project().unwrap().to_path_buf();
        std::fs::write(dir.join("library/2026-11-17_10-01-animatic.mp4"), b"video").unwrap();
        session.step_ran(&id, Some("library/2026-11-17_10-01-animatic.mp4"), None);
        let Some(protocol::AppMessage::Publish { artifact, source, .. }) = session.take_outgoing().pop() else {
            panic!("the take was not offered for publishing");
        };
        assert_eq!((artifact.as_str(), source.as_str()), ("Last order: animatic", "library/2026-11-17_10-01-animatic.mp4"));
        assert!(session.kept_notices.iter().any(|notice| notice.text == "Animatic video exported" && notice.detail == "library/2026-11-17_10-01-animatic.mp4"));
        // The take is the animatic's file now, for list_items and renders.
        assert_eq!(session.animatic_state()["state"], "done");
        assert_eq!(session.animatic_state()["file"], "library/2026-11-17_10-01-animatic.mp4");
        assert!(!dir.join("exports").exists());
        // Retimed in another tool: the storyboard's own timing is saved
        // over it before the step runs.
        session.retimed = Some((session.active_id(), "prompts/resources/other.json".into()));
        session.start_export().unwrap();
        assert!(matches!(session.take_outgoing().pop(), Some(protocol::AppMessage::RunSkillScript { .. })));
        session.saving = None;
        session.retimed = None;
        // Retimed: the timing is saved first, then the step runs.
        let (second, last) = (session.film().clips[1].id, session.film().clips[3].id);
        session.resize_clip(second, true, 2);
        session.resize_clip(last, true, -2);
        session.start_export().unwrap();
        let Some(protocol::AppMessage::RunSkillScript { id, .. }) = session.take_outgoing().pop() else {
            panic!("the timing was not saved first");
        };
        std::fs::copy(dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json"), dir.join("prompts/resources/new.json")).unwrap();
        session.skill_script_ran(&id, "{\"path\": \"prompts/resources/new.json\"}", None);
        let Some(protocol::AppMessage::EditStep { id, .. }) = session.take_outgoing().pop() else {
            panic!("the step's edit was not asked");
        };
        session.step_edited(&id, None);
        let Some(protocol::AppMessage::RunStep { id, .. }) = session.take_outgoing().pop() else {
            panic!("the step did not run after the save");
        };
        // A step that is not approved says why on the export's card.
        session.step_ran(&id, None, Some("The user did not approve it, so nothing ran."));
        assert!(session.kept_notices.iter().any(|notice| notice.text == "Animatic video not exported" && notice.detail == "The user did not approve it, so nothing ran."));
        assert!(session.take_outgoing().is_empty());
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

    #[test]
    fn only_an_image_inside_the_project_is_an_image_key() {
        let (session, _) = opened("inside");
        let dir = session.project().unwrap().to_path_buf();
        std::fs::write(dir.join("library/key.png"), b"png").unwrap();
        assert_eq!(session.image_in_project(Path::new("library/key.png")).unwrap(), "library/key.png");
        assert_eq!(session.image_in_project(&dir.join("library/key.png")).unwrap(), "library/key.png");
        let outside = "Only images inside the project's folder can be image keys.";
        assert_eq!(session.image_in_project(Path::new("../outside.png")).unwrap_err(), outside);
        let elsewhere = std::env::temp_dir().join(format!("storyboard-animatic-elsewhere-{}.png", std::process::id()));
        std::fs::write(&elsewhere, b"png").unwrap();
        assert_eq!(session.image_in_project(&elsewhere).unwrap_err(), outside);
        assert_eq!(session.image_in_project(Path::new("library/missing.png")).unwrap_err(), outside);
    }

    /// An animatic opened as from its step, with every file it names
    /// published.
    fn from_its_step(label: &str) -> Session {
        let (mut session, _) = opened(label);
        let script = "python3 assemble.py prompts/resources/2026-11-14_16-07-animatic-assembly.json .build/animatic.mp4";
        let step = crate::session::project::AnimaticStep { chain: "the animatic".into(), name: "animatic".into(), artifact: "Last order: animatic".into(), script: script.into() };
        session.steps.insert(step.key(), step);
        let record = session.active_storyboard_mut();
        let mut read = record.assembly().unwrap().clone();
        read.step = Some(("the animatic".into(), "animatic".into()));
        for file in read.assembly.frames.iter().map(|frame| frame.image.clone()).chain([read.assembly.bed.audio.clone()]).chain(read.assembly.clips.iter().map(|clip| clip.audio.clone())) {
            session.canon_files.insert(file.clone(), format!("Last order: {file}"));
        }
        session.active_storyboard_mut().set_assembly(Some(read));
        session
    }

    #[test]
    fn a_save_reaches_the_storyboard_that_asked() {
        let mut session = from_its_step("save-tab");
        let animatic = session.active_id();
        session.save_timing().unwrap();
        let Some(protocol::AppMessage::RunSkillScript { id, .. }) = session.take_outgoing().pop() else {
            panic!("Skills' writer was not asked");
        };
        // The person shows another storyboard while the save waits.
        let other = session.new_storyboard();
        let dir = session.project().unwrap().to_path_buf();
        std::fs::copy(dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json"), dir.join("prompts/resources/new.json")).unwrap();
        session.skill_script_ran(&id, "{\"path\": \"prompts/resources/new.json\", \"sha256\": \"x\"}", None);
        let Some(protocol::AppMessage::EditStep { id, tab, script, .. }) = session.take_outgoing().pop() else {
            panic!("the step's edit was not asked");
        };
        assert_eq!(tab, animatic.get().to_string());
        assert!(script.unwrap().contains("prompts/resources/new.json"));
        session.step_edited(&id, None);
        assert_eq!(session.active_id(), other);
        assert!(session.active_storyboard().assembly().is_none());
        session.activate(animatic);
        assert_eq!(session.active_storyboard().assembly().unwrap().path, "prompts/resources/new.json");
        let state = session.project_animatic().unwrap();
        assert_eq!(state["saved"]["assembly"], "prompts/resources/new.json");
        assert_eq!(state["saved"]["step_uses_it"], true);
        assert_eq!(state["differs"], false);
    }

    #[test]
    fn a_save_with_no_answer_gives_up() {
        let mut session = from_its_step("save-wait");
        session.save_timing().unwrap();
        assert_eq!(session.save_timing().unwrap_err(), "The timing is already being saved.");
        session.check_retimed(std::time::Instant::now() + crate::session::project::SAVE_WAIT);
        assert!(session.kept_notices.iter().any(|notice| notice.text == "Timing not saved" && notice.detail == "The project did not answer."));
        assert!(session.save_timing().is_ok());
    }

    #[test]
    fn follow_keeps_each_hold_on_its_clip_and_undo_takes_it_back() {
        let mut session = from_its_step("follow");
        let first = session.film().clips[0].id;
        // Shot 1 split into two holds, then saved: the assembly has five frames.
        session.enter_clip(first);
        session.seek_clip(36);
        assert!(session.insert_image_key("library/other.png"));
        let layer = session.film().clip(first).unwrap().layers[0].id;
        session.add_mark(layer, 40, false, "turns").unwrap();
        session.leave_clip();
        let (saved, notes) = session.timing(Some(8.0)).unwrap();
        session.timing_saved(session.active_id(), saved.clone(), &notes, "prompts/resources/saved.json", "a");
        let before = session.film().clone();
        let read_before = session.active_storyboard().assembly().unwrap().clone();
        // Another tool moves shot 1's second hold two frames later.
        let mut retimed = saved.clone();
        retimed.frames[0].end = 1.5 + 2.0 / 24.0;
        retimed.frames[1].start = retimed.frames[0].end;
        let dir = session.project().unwrap().to_path_buf();
        std::fs::write(dir.join("prompts/resources/retimed.json"), serde_json::to_vec(&retimed.to_value()).unwrap()).unwrap();
        session.retimed = Some((session.active_id(), "prompts/resources/retimed.json".into()));
        session.follow_retime().unwrap();
        let film = session.film();
        assert_eq!(film.clips.len(), 4);
        let clip = film.clip(first).unwrap();
        assert_eq!(clip.layers[0].image_at(38), Some("library/other.png"));
        assert_eq!(clip.layers[0].image_at(37), Some("library/2026-11-12_20-48-keyframe-wiping-2.png"));
        // A mark moves with its hold.
        assert_eq!(clip.layers[0].marks[0].frame, 42);
        // No id is used twice: clips, layers, keys, tracks, and sounds share them.
        let mut ids: Vec<u64> = film.clips.iter().flat_map(|clip| std::iter::once(clip.id).chain(clip.layers.iter().flat_map(|layer| std::iter::once(layer.id).chain(layer.keys.iter().map(|key| key.id))))).collect();
        ids.extend(film.sound_tracks.iter().flat_map(|track| std::iter::once(track.id).chain(track.clips.iter().map(|sound| sound.id))));
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "an id is used twice");
        let read = session.active_storyboard().assembly().unwrap().clone();
        assert_eq!(read.path, "prompts/resources/retimed.json");
        assert_eq!(read.clips[..2], [first, first]);
        assert!(session.film().sound(read.bed).is_some() && read.voices.iter().all(|voice| session.film().sound(*voice).is_some()));
        // Saved untouched, it is the retimed assembly.
        assert_eq!(session.timing(Some(8.0)).unwrap().0, retimed);
        // A board published after Follow is kept through undo and redo.
        let stamp = stamp_of(&session, first);
        session.hold_published(session.active_id(), first, 0, &stamp, "library/board.png");
        // One undo takes the film and the timing it read back.
        session.undo().unwrap();
        assert_eq!(*session.film(), before);
        let undone = session.active_storyboard().assembly().unwrap().clone();
        assert_eq!((undone.path.as_str(), &undone.assembly, &undone.clips), (read_before.path.as_str(), &read_before.assembly, &read_before.clips));
        assert!(undone.boards.iter().any(|board| board.file == "library/board.png"));
        session.redo().unwrap();
        assert_eq!(session.active_storyboard().assembly().unwrap().path, "prompts/resources/retimed.json");
        assert!(session.active_storyboard().assembly().unwrap().boards.iter().any(|board| board.file == "library/board.png"));
    }

    /// Answers the reads `show_animatic` sent: no published files, and one
    /// animatic step naming `assembly`.
    fn answer_steps(session: &mut Session, assembly: &str) {
        let script = format!("python3 assemble.py {assembly} .build/animatic.mp4");
        for message in session.take_outgoing() {
            if let protocol::AppMessage::ReadCanon { id, query } = message {
                let rows = if query.contains("FROM step") {
                    vec![Value::String(format!("the animatic\u{1f}animatic\u{1f}Last order: animatic\u{1f}{script}"))]
                } else {
                    Vec::new()
                };
                session.canon_rows(&id, &rows, None);
            }
        }
    }

    #[test]
    fn the_project_s_animatic_is_its_storyboard() {
        let dir = project("show");
        let mut session = Session::new();
        session.set_project(&dir);
        assert_eq!(session.storyboards.len(), 1);
        let blank = session.active_id();
        session.fit_view([-40.0, -30.0], 0.5);
        let fitted = session.active_storyboard().camera_of(None).unwrap();
        // Told the project, it shows the animatic in place of the blank,
        // as the same tab, which the window keeps showing.
        session.show_animatic();
        answer_steps(&mut session, "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        assert_eq!(session.storyboards.len(), 1);
        assert_eq!(session.active_id(), blank, "the animatic opened as another tab");
        // It keeps the view the window fitted for the blank's tab, so coming
        // back from a shot shows the film fitted, not a default view.
        let kept = session.active_storyboard().camera_of(None).unwrap();
        assert_eq!((kept.origin, kept.zoom), (fitted.origin, fitted.zoom));
        // The film is Last order; the storyboard is named as every new one.
        assert_eq!(session.film().title, "Last order");
        assert_eq!(session.active_storyboard().name(), "Untitled Storyboard");
        let read = session.active_storyboard().assembly().unwrap().clone();
        assert_eq!(read.step, Some(("the animatic".to_string(), "animatic".to_string())));
        // Shown again after another tool retimed it, the same storyboard
        // takes the step's timing, with no card.
        let mut retimed = read.assembly.clone();
        retimed.frames[0].end = 2.5;
        retimed.frames[1].start = 2.5;
        std::fs::write(dir.join("prompts/resources/retimed.json"), serde_json::to_vec(&retimed.to_value()).unwrap()).unwrap();
        session.show_animatic();
        answer_steps(&mut session, "prompts/resources/retimed.json");
        assert_eq!(session.storyboards.len(), 1);
        assert_eq!(session.active_storyboard().assembly().unwrap().path, "prompts/resources/retimed.json");
        assert_eq!(session.film().clips[0].length, 60);
        assert!(session.retimed().is_none());
    }

    #[test]
    fn the_kept_animatic_opens_under_the_name_it_was_given() {
        let dir = project("renamed");
        let mut first = Session::new();
        first.set_project(&dir);
        first.show_animatic();
        answer_steps(&mut first, "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        // Renamed by the person, and kept in work/.
        std::fs::create_dir_all(dir.join("work")).unwrap();
        crate::store::write_smx(&dir.join("work/My cut.smx"), first.active_storyboard(), "My cut").unwrap();
        let mut again = Session::new();
        again.set_project(&dir);
        again.show_animatic();
        answer_steps(&mut again, "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        assert_eq!(again.storyboards.len(), 1);
        assert_eq!(again.active_storyboard().name(), "My cut");
        assert!(again.active_storyboard().assembly().is_some());
        assert_eq!(again.active_storyboard().undo_len(), 0, "opening left an undo step");
    }

    #[test]
    fn a_storyboard_that_read_no_timing_is_left_alone() {
        let dir = project("kept-other");
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::write(dir.join("work/Last order.smx"), b"{}").unwrap();
        let mut session = Session::new();
        session.set_project(&dir);
        session.show_animatic();
        answer_steps(&mut session, "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        assert_eq!(session.storyboards.len(), 1);
        assert!(session.active_storyboard().assembly().is_some(), "the animatic did not open");
        assert!(session.kept_notices.is_empty(), "a card was shown");
        assert_eq!(std::fs::read(dir.join("work/Last order.smx")).unwrap(), b"{}");
    }

    #[test]
    fn a_failed_read_on_opening_shows_no_card() {
        let dir = project("read-failed");
        let mut session = Session::new();
        session.set_project(&dir);
        session.show_animatic();
        for message in session.take_outgoing() {
            if let protocol::AppMessage::ReadCanon { id, .. } = message {
                session.canon_rows(&id, &[], Some("sqlite3.OperationalError: no such table: step"));
            }
        }
        assert!(session.kept_notices.is_empty(), "the host's error reached a card");
        assert_eq!(session.storyboards.len(), 1);
    }

    #[test]
    fn a_new_film_s_first_save_records_its_animatic_step() {
        let (mut session, _) = opened("new-film");
        let dir = session.project().unwrap().to_path_buf();
        // A storyboard of the film's frames and sounds that no step made
        // yet, in a project whose films and chains were read.
        session.active_storyboard_mut().set_assembly(None);
        let read_files = ["library/2026-11-12_20-48-keyframe-wiping-2.png", "library/2026-11-14_16-04-keyframe-the-bell.png", "library/2026-11-14_15-37-keyframe-the-line.png", "library/2026-11-14_15-40-keyframe-waiting.png", "library/2026-11-12_20-27-scene-3.wav", "library/2026-11-12_20-03-otis-line-1.wav"];
        for file in read_files {
            session.canon_files.insert(file.to_string(), format!("Harbor: {file}"));
        }
        // No film element yet, and no chain for the animatic.
        session.chain_orders = Some(crate::session::project::ChainOrders { chain_there: false, last_step: 0, last_chain: 4 });
        session.film_element = None;
        session.steps_read = true;
        // Not until the film's read is answered.
        assert_eq!(session.save_timing().unwrap_err(), "The project's animatic step is not known yet.");
        session.film_read = true;
        let film = session.film_name();
        session.save_timing().unwrap();
        let Some(protocol::AppMessage::RunSkillScript { id, script, .. }) = session.take_outgoing().pop() else {
            panic!("Skills' writer was not asked");
        };
        assert_eq!(script, "write-assembly.py");
        std::fs::copy(dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json"), dir.join("prompts/resources/first.json")).unwrap();
        session.skill_script_ran(&id, "{\"path\": \"prompts/resources/first.json\"}", None);
        // The film's element first, named after the project, then the chain
        // after the last one, then the step, each once the host answers.
        let Some(protocol::AppMessage::EditElement { id, name, kind, .. }) = session.take_outgoing().pop() else {
            panic!("the film was not recorded");
        };
        assert_eq!((name.as_str(), kind.as_str()), (film.as_str(), "film"));
        session.canon_edited(&id, None);
        let Some(protocol::AppMessage::EditChain { id, chain, order, .. }) = session.take_outgoing().pop() else {
            panic!("the chain was not recorded");
        };
        assert_eq!((chain.as_str(), order), ("the animatic", 5));
        session.canon_edited(&id, None);
        let Some(protocol::AppMessage::EditStep { id, step, script, order, skill, artifact, refs, endpoint, .. }) = session.take_outgoing().pop() else {
            panic!("the step was not recorded");
        };
        assert_eq!((step.chain.as_str(), step.step.as_str()), ("the animatic", "animatic"));
        assert_eq!((order, skill.as_deref(), artifact), (Some(1), Some("build-animatic"), Some(format!("{film}: animatic"))));
        assert_eq!(script.as_deref(), Some("python3 \"${SPUMCAKE_AGENT}/skills/build-animatic/scripts/assemble.py\" prompts/resources/first.json .build/animatic.mp4"));
        assert_eq!(endpoint.as_deref(), Some(r#"{"kind": "script", "outputs": [".build/animatic.mp4"]}"#));
        assert!(refs.unwrap().contains("frame, 0.0 to 3.0 s"));
        session.step_edited(&id, None);
        assert_eq!(session.film_name(), film);
        // The storyboard is the project's animatic from now on.
        let read = session.active_storyboard().assembly().unwrap();
        assert_eq!((read.path.as_str(), read.step.clone()), ("prompts/resources/first.json", Some(("the animatic".to_string(), "animatic".to_string()))));
    }

    #[test]
    fn a_project_with_no_animatic_keeps_its_blank() {
        let dir = project("none");
        let mut session = Session::new();
        session.set_project(&dir);
        session.show_animatic();
        for message in session.take_outgoing() {
            if let protocol::AppMessage::ReadCanon { id, .. } = message {
                session.canon_rows(&id, &[], None);
            }
        }
        assert_eq!(session.storyboards.len(), 1);
        assert!(session.active_storyboard().assembly().is_none());
        assert!(session.kept_notices.is_empty(), "a card was shown");
        // Its save waits for where the project's chains stand.
        assert_eq!(session.save_timing().unwrap_err(), "The project's animatic step is not known yet.");
        // With a shot to export, it says why it cannot.
        session.add_clip(None, 24).unwrap();
        assert_eq!(session.start_export().unwrap_err(), "This project has no animatic yet.");
    }

    #[test]
    fn a_gain_above_0_db_is_kept_as_read() {
        let dir = project("gain");
        let path = dir.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json");
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["clips"][0]["gain_db"] = serde_json::json!(3);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut session = Session::new();
        session.set_project(&dir);
        session.open_assembly(&path, "Last order").unwrap();
        let read = session.active_storyboard().assembly().unwrap().clone();
        assert_eq!(session.kept_gain(read.voices[0]), Some(3.0));
        assert_eq!(session.kept_gain(read.voices[1]), None);
        assert_eq!(session.timing(Some(8.0)).unwrap().0.clips[0].gain_db, 3.0);
        // Turned down, it is the volume's gain.
        session.set_sound_volume(read.voices[0], 0.5);
        assert_eq!(session.kept_gain(read.voices[0]), None);
        assert_eq!(session.timing(Some(8.0)).unwrap().0.clips[0].gain_db, -6.02);
    }
}
