//! One open storyboard: its film, the `draw::Drawing` of every keyframe, and
//! what this workspace owns about it: its id, name, save state, where the
//! person is in it, and the undo and redo history.
//! Every edit to a drawing goes through `commit_step` or `record`, and every
//! edit to the film through `edit_film`.
//!
//! `drawing` is the drawing being worked on: the active layer's key at the
//! playhead inside a clip. Everywhere else, at the film's level, over a
//! folder, or with no clip open, it is a blank that nothing is drawn on.

use crate::film::{Film, NewDrawing, Onion};
use draw::document::{Allocator, Camera, Drawing, DrawingId, HeldIds, Step, ToolColor};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// Where the person is in a storyboard: the film's level, or inside a clip
/// at a frame and a layer. Moving there is never an undo step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The clip being edited; none at the film's level.
    pub clip: Option<u64>,
    /// The playhead inside that clip.
    pub frame: u32,
    /// The playhead on the film.
    pub film_frame: u32,
    pub layer: Option<u64>,
}

/// One undo step: a change to one drawing, a change to the film, or one
/// request's changes together. Each keeps where it was made, so undo and
/// redo can bring it back into view.
#[derive(Clone, Debug)]
enum Entry {
    Draw { drawing: u64, step: Step, at: View },
    /// `assembly`, when the change also changed what the storyboard read
    /// from the project's animatic, as Follow does: before and after.
    Film { before: Box<Film>, after: Box<Film>, at: View, assembly: Option<Box<Reads>> },
    Batch(Vec<Entry>),
}

impl Entry {
    fn at(&self) -> Option<View> {
        match self {
            Entry::Draw { at, .. } | Entry::Film { at, .. } => Some(*at),
            Entry::Batch(entries) => entries.first().and_then(Entry::at),
        }
    }
}

/// What a storyboard read from the project's animatic, before and after a
/// change.
type Reads = (Option<crate::assembly::Read>, Option<crate::assembly::Read>);

/// The fields that left `draw`'s drawing, wrapped around the film.
pub struct Record {
    pub drawing: Drawing,
    film: Film,
    /// Every keyframe's drawing but the one in `drawing`. Drawings a change
    /// let go of stay, so undo can bring them back; only those the film
    /// names are saved.
    drawings: HashMap<u64, Drawing>,
    /// Which drawing `drawing` is; none while it is the blank.
    active: Option<u64>,
    /// The blank, set aside while a keyframe's drawing is shown.
    blank: Option<Drawing>,
    view: View,
    /// The view of the film's level and of each clip, kept apart so moving
    /// the playhead never moves it.
    cameras: HashMap<Option<u64>, Camera>,
    /// The onion skin's settings, a view aid kept with the storyboard.
    onion: Onion,
    /// How loud the film plays here, 0 to 1. Kept with the storyboard,
    /// never in the export, and never an undo step.
    listening: f64,
    /// What the last finished animatic export was made from.
    exported: Option<crate::export::Exported>,
    /// The project's assembly this storyboard was opened from or last saved
    /// as, with each time as read.
    assembly: Option<crate::assembly::Read>,
    /// Holds published as rough boards before the storyboard read any
    /// animatic, as a new film's are until its first save takes them.
    loose_boards: Vec<crate::assembly::RoughBoard>,
    id: DrawingId,
    name: String,
    generated_name: String,
    /// A save of this id has succeeded. Later edits set `dirty`.
    stored: bool,
    /// Content, the name, or the camera differs from the last successful save.
    dirty: bool,
    /// The `.smx` later saves write. Absent until Save or Save as succeeds.
    path: Option<PathBuf>,
    /// Recovery file id, when a restore kept a file named with the old id.
    recovery_file: Option<DrawingId>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// The assistant's request whose changes merge into one undo step.
    group: Option<Group>,
}

/// An open request: its id, the undo length when it began, and whether the
/// top undo step is already the request's merged batch.
struct Group {
    id: String,
    base: usize,
    merged: bool,
}

impl Record {
    /// A new untitled storyboard: an empty film.
    pub fn untitled(id: DrawingId, name: String, ids: &mut Allocator, palette: &[ToolColor]) -> Self {
        Self::of_film(id, name.clone(), Film::new(&name), HashMap::new(), Drawing::untitled(ids, palette))
    }

    fn of_film(id: DrawingId, name: String, film: Film, drawings: HashMap<u64, Drawing>, blank: Drawing) -> Self {
        Self {
            drawing: blank,
            film,
            drawings,
            active: None,
            blank: None,
            view: View::default(),
            cameras: HashMap::new(),
            onion: Onion::default(),
            listening: 1.0,
            exported: None,
            assembly: None,
            loose_boards: Vec::new(),
            id,
            generated_name: name.clone(),
            name,
            stored: false,
            dirty: false,
            path: None,
            recovery_file: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
        }
    }

    /// A storyboard read from a version-1 `.smx` or recovery file: a film with
    /// one clip whose one layer's first key is the old drawing, unchanged.
    /// Stored, clean, its name as the generated name, no history.
    pub fn from_snapshot(snapshot: draw::document::snapshot::SnapshotDrawing, ids: &mut Allocator, palette: &[ToolColor]) -> Self {
        let mut film = Film::new(&snapshot.name);
        let (clip, made) = film.add_clip(Some(0), crate::film::NEW_CLIP_LENGTH);
        // The field goes where the old drawing is, centred on what it holds,
        // so the drawing itself is not moved.
        let [_, _, width, height] = crate::sheet::field(&film);
        if let Some((min, max)) = ink_bounds(&snapshot.drawing)
            && let Some(at) = film.clip_mut(clip)
        {
            at.origin = [(min[0] + max[0] - width) / 2.0, (min[1] + max[1] - height) / 2.0];
        }
        let camera = snapshot.drawing.camera();
        let mut drawings = HashMap::new();
        drawings.insert(made[0].id, snapshot.drawing);
        let mut record = Self::of_film(snapshot.id, snapshot.name, film, drawings, Drawing::untitled(ids, palette));
        record.stored = true;
        record.cameras.insert(record.film.clips.first().map(|clip| clip.id), camera);
        record.cameras.insert(None, camera);
        record
    }

    /// A storyboard read from a version-2 `.smx`: its film and drawings.
    pub fn from_film(
        id: DrawingId,
        name: String,
        film: Film,
        drawings: HashMap<u64, Drawing>,
        ids: &mut Allocator,
        palette: &[ToolColor],
    ) -> Self {
        let mut record = Self::of_film(id, name, film, drawings, Drawing::untitled(ids, palette));
        record.stored = true;
        record
    }

    pub fn film(&self) -> &Film {
        &self.film
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn onion(&self) -> Onion {
        self.onion
    }

    /// Saved with the storyboard, never an undo step.
    pub fn set_onion(&mut self, onion: Onion) -> bool {
        if onion == self.onion {
            return false;
        }
        self.onion = onion;
        self.dirty = true;
        true
    }

    pub fn listening(&self) -> f64 {
        self.listening
    }

    /// Saved with the storyboard, never an undo step.
    pub fn set_listening(&mut self, volume: f64) -> bool {
        let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { return false };
        if volume == self.listening {
            return false;
        }
        self.listening = volume;
        self.dirty = true;
        true
    }

    pub fn assembly(&self) -> Option<&crate::assembly::Read> {
        self.assembly.as_ref()
    }

    /// Kept beside the film, so undo never takes it back, except where a
    /// film change takes it with it, in `edit_film_reading`.
    pub fn set_assembly(&mut self, assembly: Option<crate::assembly::Read>) {
        if self.assembly != assembly {
            self.assembly = assembly;
            self.dirty = true;
        }
    }

    pub fn loose_boards(&self) -> &[crate::assembly::RoughBoard] {
        &self.loose_boards
    }

    /// Kept beside the film, as what it read is.
    pub fn set_loose_boards(&mut self, boards: Vec<crate::assembly::RoughBoard>) {
        if self.loose_boards != boards {
            self.loose_boards = boards;
            self.dirty = true;
        }
    }

    pub fn exported(&self) -> Option<&crate::export::Exported> {
        self.exported.as_ref()
    }

    /// Kept beside the film, so undo never takes it back.
    pub fn set_exported(&mut self, exported: Option<crate::export::Exported>) {
        if self.exported != exported {
            self.exported = exported;
            self.dirty = true;
        }
    }

    /// A drawing of this storyboard, the one being worked on included.
    pub fn drawing_of(&self, id: u64) -> Option<&Drawing> {
        if self.active == Some(id) {
            return Some(&self.drawing);
        }
        self.drawings.get(&id)
    }

    /// The drawing being worked on is a keyframe's, so it can be drawn on.
    pub fn editing(&self) -> Option<u64> {
        self.active
    }

    /// Whether drawing tools work now: inside a clip, on a layer that shows
    /// and is not locked.
    pub fn can_draw(&self) -> bool {
        let Some(clip) = self.view.clip.and_then(|clip| self.film.clip(clip)) else {
            return false;
        };
        let Some(layer) = self.view.layer.and_then(|layer| clip.layer(layer)) else {
            return false;
        };
        self.active.is_some() && clip.shows(layer) && !clip.locked(layer)
    }

    /// Goes to `view`, made valid for the film as it is, and swaps in the
    /// drawing it shows. Never an undo step.
    pub fn go(&mut self, view: View) {
        let mut view = view;
        match view.clip.and_then(|clip| self.film.clip(clip)) {
            Some(clip) => {
                view.frame = view.frame.min(clip.length.saturating_sub(1));
                if !view.layer.is_some_and(|layer| clip.layer(layer).is_some()) {
                    view.layer = clip.first_drawn_layer();
                }
            }
            None => {
                view.clip = None;
                view.layer = None;
                view.frame = 0;
            }
        }
        view.film_frame = view.film_frame.min(self.film.length());
        let target = view.clip.and_then(|clip| self.film.clip(clip)).and_then(|clip| {
            let layer = clip.layer(view.layer?)?;
            Some(layer.key_at(view.frame)?.drawing)
        });
        let leaving = self.view.clip;
        self.view = view;
        if target == self.active && leaving == view.clip {
            return;
        }
        self.cameras.insert(leaving, self.drawing.camera());
        // The drawing shown goes home, and the blank comes back.
        if let Some(id) = self.active.take()
            && let Some(blank) = self.blank.take()
        {
            let outgoing = std::mem::replace(&mut self.drawing, blank);
            self.drawings.insert(id, outgoing);
        }
        if let Some(id) = target
            && let Some(incoming) = self.drawings.remove(&id)
        {
            self.blank = Some(std::mem::replace(&mut self.drawing, incoming));
            self.active = Some(id);
            self.drawing.focus_origin();
        }
        if let Some(camera) = self.cameras.get(&view.clip).copied() {
            self.drawing.set_camera(camera);
        }
    }

    /// Changes the film. `change` returns the drawings it needs made, or
    /// `None` when nothing changed. One undo step; the person stays where
    /// they are, or as near as the film still allows.
    pub fn edit_film(
        &mut self,
        ids: &mut Allocator,
        palette: &[ToolColor],
        change: impl FnOnce(&mut Film) -> Option<Vec<NewDrawing>>,
    ) -> bool {
        self.edit_film_reading(ids, palette, change, None)
    }

    /// `edit_film`, which also makes `reading` what the storyboard read from
    /// the project's animatic, in the same undo step.
    pub fn edit_film_reading(
        &mut self,
        ids: &mut Allocator,
        palette: &[ToolColor],
        change: impl FnOnce(&mut Film) -> Option<Vec<NewDrawing>>,
        reading: Option<crate::assembly::Read>,
    ) -> bool {
        let before = self.film.clone();
        let Some(made) = change(&mut self.film) else {
            self.film = before;
            return false;
        };
        if self.film == before && made.is_empty() {
            return false;
        }
        let assembly = reading.map(|after| Box::new((self.assembly.clone(), Some(after))));
        if let Some(reads) = &assembly {
            self.assembly = reads.1.clone();
        }
        for drawing in made {
            let new = match drawing.copy_of.and_then(|source| self.drawing_of(source)) {
                Some(source) => copy_drawing(source, ids).unwrap_or_else(|| blank_drawing(ids, palette)),
                None => blank_drawing(ids, palette),
            };
            self.drawings.insert(drawing.id, new);
        }
        let at = self.view;
        self.dirty = true;
        self.redo.clear();
        self.push_undo(Entry::Film { before: Box::new(before), after: Box::new(self.film.clone()), at, assembly });
        self.go(at);
        true
    }

    /// An edit a `draw` method built and has not applied: applies it and
    /// records it. `None`, an edit that changed nothing, records nothing.
    pub fn commit(&mut self, step: Option<Step>) -> bool {
        match step {
            Some(step) => {
                self.commit_step(step);
                true
            }
            None => false,
        }
    }

    /// A camera change, as the reference's `note_view` noted it: the storyboard
    /// has unsaved changes only when the camera actually moved.
    pub fn move_view(&mut self, change: impl FnOnce(&mut Camera, &Drawing)) {
        let before = self.drawing.camera();
        let mut camera = before;
        change(&mut camera, &self.drawing);
        if camera.origin != before.origin || camera.zoom != before.zoom {
            self.drawing.set_camera(camera);
            self.cameras.insert(self.view.clip, camera);
            self.dirty = true;
        }
    }

    /// A live change with no history, such as a drag preview or a property
    /// the reference set directly. The release, when there is one, records it.
    pub fn preview(&mut self, step: Option<Step>) -> bool {
        let Some(step) = step else {
            return false;
        };
        self.drawing.apply(&step);
        true
    }

    /// An edit already applied to the model, such as a transform preview the
    /// release keeps. Records it without applying it again.
    pub fn record(&mut self, step: Option<Step>) -> bool {
        let Some(step) = step else {
            return false;
        };
        let Some(drawing) = self.active else {
            return false;
        };
        self.dirty = true;
        self.redo.clear();
        let at = self.view;
        self.push_undo(Entry::Draw { drawing, step, at });
        true
    }

    pub fn id(&self) -> DrawingId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Another name for an untouched storyboard, which stays untouched.
    pub fn rename_untitled(&mut self, name: String) {
        self.generated_name = name.clone();
        self.film.title = name.clone();
        self.name = name;
    }

    pub fn set_name(&mut self, name: String) {
        if self.name == name {
            return;
        }
        self.name = name;
        self.dirty = true;
    }

    /// An empty film, the generated name, and never saved.
    pub fn is_clean(&self) -> bool {
        !self.stored && self.name == self.generated_name && self.film.clips.is_empty()
    }

    /// Changes since the last save, or a renamed title that has not been saved.
    pub fn needs_keep(&self) -> bool {
        if self.stored {
            self.dirty
        } else {
            !self.is_clean()
        }
    }

    pub fn mark_saved(&mut self) {
        self.stored = true;
        self.dirty = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn recovery_file(&self) -> Option<DrawingId> {
        self.recovery_file
    }

    pub fn set_recovery_file(&mut self, id: DrawingId) {
        self.recovery_file = Some(id);
    }

    pub fn set_id(&mut self, id: DrawingId) {
        self.id = id;
    }

    /// Opened from a `.smx` or restored from recovery. The stored camera stays.
    pub fn keeps_stored_view(&self) -> bool {
        self.stored || self.path.is_some() || self.recovery_file.is_some()
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    /// Forgets every undo and redo step, as a storyboard just opened has
    /// none.
    pub fn clear_history(&mut self) {
        self.group = None;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Applies `step` to the drawing being worked on, then pushes it onto this
    /// record's history. With no keyframe's drawing open, nothing happens.
    pub fn commit_step(&mut self, step: Step) {
        let Some(drawing) = self.active else {
            return;
        };
        self.dirty = true;
        self.redo.clear();
        self.drawing.apply(&step);
        let at = self.view;
        self.push_undo(Entry::Draw { drawing, step, at });
    }

    /// Every change from now until another group, or until `close_group`,
    /// is one undo step: one undo takes the whole request back.
    pub fn join_group(&mut self, id: &str) {
        if self.group.as_ref().is_some_and(|group| group.id == id) {
            return;
        }
        self.group = Some(Group {
            id: id.to_string(),
            base: self.undo.len(),
            merged: false,
        });
    }

    /// A change by hand, or the end of a request: later changes are their own steps.
    pub fn close_group(&mut self) {
        self.group = None;
    }

    /// Pushes a step, folding it into the open group's step when it has one.
    fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
        let Some(group) = self.group.as_mut() else {
            return;
        };
        if self.undo.len() < group.base + 2 {
            return;
        }
        let last = self.undo.pop().expect("two steps");
        let first = self.undo.pop().expect("two steps");
        let merged = match first {
            Entry::Batch(mut entries) if group.merged => {
                entries.push(last);
                Entry::Batch(entries)
            }
            first => Entry::Batch(vec![first, last]),
        };
        group.merged = true;
        self.undo.push(merged);
    }

    /// Takes the last step back and brings where it was made into view.
    pub fn undo(&mut self) -> bool {
        self.group = None;
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        self.dirty = true;
        self.run(&entry, false);
        self.redo.push(entry);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.group = None;
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        self.dirty = true;
        self.run(&entry, true);
        self.undo.push(entry);
        true
    }

    fn run(&mut self, entry: &Entry, forward: bool) {
        self.apply_entry(entry, forward);
        if let Some(at) = entry.at() {
            self.go(at);
        }
    }

    fn apply_entry(&mut self, entry: &Entry, forward: bool) {
        match entry {
            Entry::Draw { drawing, step, .. } => {
                let target = if self.active == Some(*drawing) {
                    Some(&mut self.drawing)
                } else {
                    self.drawings.get_mut(drawing)
                };
                if let Some(target) = target {
                    if forward {
                        target.apply(step);
                    } else {
                        target.revert(step);
                    }
                }
            }
            Entry::Film { before, after, assembly, .. } => {
                self.film = if forward { (**after).clone() } else { (**before).clone() };
                // Only the timing Follow changed goes back: a save, a step,
                // or a board published since stays.
                if let Some(reads) = assembly {
                    let target = if forward { &reads.1 } else { &reads.0 };
                    self.assembly = match (self.assembly.take(), target) {
                        (Some(mut now), Some(target)) => {
                            now.take_timing(target);
                            Some(now)
                        }
                        (now, target) => now.or_else(|| target.clone()),
                    };
                }
                // The drawing shown may have left the film; settle on one it has.
                let view = self.view;
                self.go(view);
            }
            Entry::Batch(entries) => {
                if forward {
                    for entry in entries {
                        self.apply_entry(entry, true);
                    }
                } else {
                    for entry in entries.iter().rev() {
                        self.apply_entry(entry, false);
                    }
                }
            }
        }
    }

    /// A `.smx` just opened. Nothing selected, at the film's level.
    pub fn mark_opened(&mut self, path: PathBuf, title: String) {
        self.name = title.clone();
        self.generated_name = title;
        self.path = Some(path);
        self.stored = true;
        self.dirty = false;
        self.recovery_file = None;
        self.undo.clear();
        self.redo.clear();
        self.drawing.clear_open_state();
    }

    /// A recovery copy just opened. Still unsaved. The original path is kept
    /// when the recovery file stored one.
    pub fn mark_restored(&mut self, path: Option<PathBuf>, title: String, file_id: DrawingId) {
        self.name = title.clone();
        self.generated_name = title;
        self.stored = path.is_some();
        self.path = path;
        self.dirty = true;
        self.recovery_file = Some(file_id);
        self.undo.clear();
        self.redo.clear();
        self.drawing.clear_open_state();
    }

    /// Makes sure no id any of its drawings holds is handed out again.
    pub fn reserve_ids(&self, ids: &mut Allocator) {
        self.drawing.reserve_ids(ids);
        for drawing in self.drawings.values().chain(self.blank.iter()) {
            drawing.reserve_ids(ids);
        }
    }

    pub fn collect_held(&self, held: &mut HeldIds) {
        held.drawings.insert(self.id.get());
        self.drawing.collect_held(held);
        for drawing in self.drawings.values().chain(self.blank.iter()) {
            drawing.collect_held(held);
        }
    }

    /// Ids this storyboard shares with `held` become new ones.
    pub fn separate_ids(&mut self, held: &HeldIds, ids: &mut Allocator, next_storyboard: &mut u64) {
        if held.drawings.contains(&self.id.get()) {
            self.id = DrawingId::new(*next_storyboard);
            *next_storyboard += 1;
        }
        self.drawing.separate_ids(held, ids);
        for drawing in self.drawings.values_mut().chain(self.blank.iter_mut()) {
            drawing.separate_ids(held, ids);
        }
    }

    /// A clean opening storyboard takes a palette loaded after it was made.
    pub fn adopt_palette(&mut self, palette: &[ToolColor], ids: &mut Allocator) {
        if palette.is_empty() || !self.is_clean() {
            return;
        }
        self.drawing.adopt_palette(palette, ids);
    }

    /// Every drawing the film names, by id, for saving.
    pub fn named_drawings(&self) -> Vec<(u64, &Drawing)> {
        let mut out: Vec<(u64, &Drawing)> =
            self.film.drawings().into_iter().filter_map(|id| Some((id, self.drawing_of(id)?))).collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// The carried-over checks start drawing with no history, as its blank did.
    #[cfg(test)]
    pub fn forget_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
    }

    /// Whether the film's level or a clip has a view of its own yet.
    pub fn has_camera(&self, clip: Option<u64>) -> bool {
        self.cameras.contains_key(&clip)
    }

    /// The view a level first shows, fitted to the window. Not a change to
    /// the storyboard, so an opened file is not written for it.
    pub fn fit(&mut self, camera: Camera) {
        self.drawing.set_camera(camera);
        self.cameras.insert(self.view.clip, camera);
    }

    /// The camera kept for the film's level or a clip.
    pub fn camera_of(&self, clip: Option<u64>) -> Option<Camera> {
        if clip == self.view.clip {
            return Some(self.drawing.camera());
        }
        self.cameras.get(&clip).copied()
    }

    pub fn set_cameras(&mut self, cameras: Vec<(Option<u64>, Camera)>) {
        self.cameras.extend(cameras);
        if let Some(camera) = self.cameras.get(&self.view.clip).copied() {
            self.drawing.set_camera(camera);
        }
    }
}

/// The corners of everything a drawing paints, if it paints anything.
fn ink_bounds(drawing: &Drawing) -> Option<([f64; 2], [f64; 2])> {
    let mut points: Vec<[f64; 2]> = Vec::new();
    for mark in drawing.scene(None) {
        match mark {
            draw::document::SceneMark::Ink(paint) => points.extend(paint.triangles.iter().flatten().copied()),
            draw::document::SceneMark::Text(view) => {
                let at = view.world * kurbo::Point::new(view.origin[0], view.origin[1]);
                points.push([at.x, at.y]);
            }
        }
    }
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(min, max), point| {
        ([min[0].min(point[0]), min[1].min(point[1])], [max[0].max(point[0]), max[1].max(point[1])])
    }))
}

/// A copy of a drawing with ids of its own, as Insert keyframe and Paste
/// frames make one. A drawing is copied through its saved form.
pub fn copy_drawing(source: &Drawing, ids: &mut Allocator) -> Option<Drawing> {
    let value = source.snapshot_value(DrawingId::new(1), "Drawing");
    let mut copy = draw::document::snapshot::drawing_from_value(&value)?.drawing;
    let mut held = HeldIds::default();
    source.collect_held(&mut held);
    copy.separate_ids(&held, ids);
    copy.focus_origin();
    Some(copy)
}

/// A new keyframe's drawing: one canvas, with the brush's colours.
pub(crate) fn blank_drawing(ids: &mut Allocator, palette: &[ToolColor]) -> Drawing {
    let mut drawing = Drawing::untitled(ids, palette);
    drawing.focus_origin();
    drawing
}

impl Deref for Record {
    type Target = Drawing;

    fn deref(&self) -> &Drawing {
        &self.drawing
    }
}

impl DerefMut for Record {
    fn deref_mut(&mut self) -> &mut Drawing {
        &mut self.drawing
    }
}
