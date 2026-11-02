//! One open storyboard: the model, a `draw::Drawing`, wrapped with what this
//! workspace owns about it: its id, name, save state, and the undo and redo
//! history.
//! Every edit to the model goes through `commit_step` or `record`.

use draw::document::{
    Allocator, Camera, HeldIds, Drawing, DrawingId, Step, ToolColor,
};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// The fields that left `draw`'s drawing, wrapped around that model.
pub struct Record {
    pub drawing: Drawing,
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
    undo: Vec<Step>,
    redo: Vec<Step>,
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
    /// A new untitled storyboard, as the reference's `Drawing::untitled` made one.
    pub fn untitled(id: DrawingId, name: String, ids: &mut Allocator, palette: &[ToolColor]) -> Self {
        Self {
            drawing: Drawing::untitled(ids, palette),
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

    /// A storyboard read from a `.smx` or recovery file, as the reference's reader
    /// made it: stored, clean, its name as the generated name, no history.
    pub fn from_snapshot(snapshot: draw::document::snapshot::SnapshotDrawing) -> Self {
        Self {
            drawing: snapshot.drawing,
            id: snapshot.id,
            generated_name: snapshot.name.clone(),
            name: snapshot.name,
            stored: true,
            dirty: false,
            path: None,
            recovery_file: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
        }
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
        self.dirty = true;
        self.redo.clear();
        self.push_undo(step);
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
        self.name = name;
    }

    pub fn set_name(&mut self, name: String) {
        if self.name == name {
            return;
        }
        self.name = name;
        self.dirty = true;
    }

    /// Starting canvas only, no strokes, the default palette, the generated
    /// name, and never saved.
    pub fn is_clean(&self) -> bool {
        if self.stored || self.name != self.generated_name {
            return false;
        }
        self.drawing.model_is_blank()
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

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Applies `step` to the model, then pushes it onto this record's history.
    pub fn commit_step(&mut self, step: Step) {
        self.dirty = true;
        self.redo.clear();
        self.drawing.apply(&step);
        self.push_undo(step);
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
    fn push_undo(&mut self, step: Step) {
        self.undo.push(step);
        let Some(group) = self.group.as_mut() else {
            return;
        };
        if self.undo.len() < group.base + 2 {
            return;
        }
        let last = self.undo.pop().expect("two steps");
        let first = self.undo.pop().expect("two steps");
        let merged = match first {
            Step::Batch(mut steps) if group.merged => {
                steps.push(last);
                Step::Batch(steps)
            }
            first => Step::Batch(vec![first, last]),
        };
        group.merged = true;
        self.undo.push(merged);
    }

    pub fn undo(&mut self) -> bool {
        self.group = None;
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.dirty = true;
        self.drawing.revert(&step);
        self.redo.push(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.group = None;
        let Some(step) = self.redo.pop() else {
            return false;
        };
        self.dirty = true;
        self.drawing.apply(&step);
        self.undo.push(step);
        true
    }

    /// A `.smx` just opened. No selection and no highlighted canvas.
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

    pub fn collect_held(&self, held: &mut HeldIds) {
        held.drawings.insert(self.id.get());
        self.drawing.collect_held(held);
    }

    /// Ids this storyboard shares with `held` become new ones.
    pub fn separate_ids(&mut self, held: &HeldIds, ids: &mut Allocator, next_storyboard: &mut u64) {
        if held.drawings.contains(&self.id.get()) {
            self.id = DrawingId::new(*next_storyboard);
            *next_storyboard += 1;
        }
        self.drawing.separate_ids(held, ids);
    }

    /// A clean opening storyboard takes a palette loaded after it was made.
    pub fn adopt_palette(&mut self, palette: &[ToolColor], ids: &mut Allocator) {
        if palette.is_empty() || !self.is_clean() {
            return;
        }
        self.drawing.adopt_palette(palette, ids);
    }
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
