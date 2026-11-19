//! A tab's gallery: its images in order, which one is shown, its cells,
//! and the filmstrip's own history. Each image keeps its own layers,
//! selection, crop box, and history in its `Doc`. Adding, duplicating,
//! removing, renaming, and moving images are steps of the strip's history
//! instead, undone while the strip has the keyboard, as the owner set on
//! 2026-10-30, and so are changes to the tab's cells: a prompt edited on
//! the Prompt page, and a next round made from a take.

use crate::doc::Doc;
use crate::files::PngCache;
use protocol::{Cell, CellPart, CellState, DocumentCamera, StepRef, Thumbnail};
use std::collections::HashMap;

/// One image of a gallery: a frame of its filmstrip.
pub struct Frame {
    /// Unique within its tab, for the files written for the host.
    pub uid: u64,
    pub name: String,
    pub doc: Doc,
    /// The crop box while the crop tool is chosen: `[x, y, width, height]`.
    pub crop: Option<[f64; 4]>,
    /// The camera as the host last reported it, or as this sent it.
    pub camera: Option<DocumentCamera>,
    /// The host has reported moving this image's camera.
    pub host_camera: bool,
    /// Each layer's thumbnail, and the bytes written for it.
    pub thumbs: HashMap<u64, (Vec<u8>, Thumbnail)>,
    /// The composite revision last written to this image's file for the
    /// host, and the revision the host was told.
    pub written: Option<(u64, u64)>,
    /// The image file is older than the revision the host was told: a
    /// stroke being drawn sent only its rows.
    pub behind: bool,
    /// The files of rows sent while a stroke was drawn, oldest first, and
    /// the last change sent, sent again until the image file catches up.
    pub patches: std::collections::VecDeque<std::path::PathBuf>,
    pub last_patch: Option<protocol::RasterPatch>,
    /// The filmstrip's thumbnail: the composite revision it was made from,
    /// its bytes, and where they are.
    pub film: Option<(u64, Vec<u8>, Thumbnail)>,
    pub pngs: PngCache,
}

/// One step of the strip's history. An image taken out of the gallery is
/// held by its step, with its own history, until the step is put back.
enum Change {
    /// An image put in at `at`; held here while the step is undone.
    Add { at: usize, frame: Option<Box<Frame>> },
    /// An image taken out from `at`; held here while the step is done.
    Remove { at: usize, frame: Option<Box<Frame>> },
    Rename { at: usize, before: String, after: String },
    /// `to` counted after the moved image is taken out.
    Move { from: usize, to: usize },
    /// The tab's cells and the one the Prompt page shows, before and after.
    Cells { before: (Vec<Cell>, Option<String>), after: (Vec<Cell>, Option<String>) },
}

struct Step {
    change: Change,
    /// The image shown before the step, by position then.
    shown_before: usize,
}

pub struct Gallery {
    /// Never empty.
    pub frames: Vec<Frame>,
    pub shown: usize,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Rises with every change to the strip, done or undone.
    pub edits: u64,
    next_uid: u64,
    /// The cell and image this gallery was opened from as new work, kept
    /// in its file.
    pub source: Option<protocol::OpenedFrom>,
    /// The tab's cells, each a pointer to a chain step or a draft before
    /// its first run, kept in its file, and the one the Prompt page shows.
    pub cells: Vec<Cell>,
    pub cell: Option<String>,
}

impl Gallery {
    /// A gallery of `images`, named, showing `shown`.
    pub fn new(images: Vec<(String, Doc)>, shown: usize) -> Self {
        let mut gallery = Self {
            frames: Vec::new(),
            shown: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            edits: 0,
            next_uid: 1,
            source: None,
            cells: Vec::new(),
            cell: None,
        };
        for (name, doc) in images {
            let frame = gallery.frame_of(&name, doc);
            gallery.frames.push(frame);
        }
        assert!(!gallery.frames.is_empty(), "a gallery holds at least one image");
        gallery.shown = shown.min(gallery.frames.len() - 1);
        gallery
    }

    pub fn one(name: &str, doc: Doc) -> Self {
        Self::new(vec![(name.to_string(), doc)], 0)
    }

    /// A frame that is no image of the gallery, such as a take shown in
    /// the working area, with a number no image of the tab has.
    pub fn frame_for(&mut self, name: &str, doc: Doc) -> Frame {
        self.frame_of(name, doc)
    }

    fn frame_of(&mut self, name: &str, doc: Doc) -> Frame {
        let uid = self.next_uid;
        self.next_uid += 1;
        Frame {
            uid,
            name: name.to_string(),
            doc,
            crop: None,
            camera: None,
            host_camera: false,
            thumbs: HashMap::new(),
            written: None,
            behind: false,
            patches: std::collections::VecDeque::new(),
            last_patch: None,
            film: None,
            pngs: PngCache::default(),
        }
    }

    /// The image shown.
    pub fn frame(&self) -> &Frame {
        &self.frames[self.shown]
    }

    pub fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frames[self.shown]
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Shows the image at `index`. Choosing an image is not a step.
    pub fn select(&mut self, index: usize) -> Result<(), String> {
        self.check(index)?;
        self.shown = index;
        Ok(())
    }

    fn check(&self, index: usize) -> Result<(), String> {
        if index < self.frames.len() {
            Ok(())
        } else {
            Err(format!("There is no image {index}; the gallery holds {}.", self.frames.len()))
        }
    }

    fn commit(&mut self, change: Change, shown_before: usize) {
        self.undo.push(Step { change, shown_before });
        self.redo.clear();
        self.edits += 1;
    }

    /// Adds an image, named `name` or the first name after it that no
    /// image of the gallery has, at `at`, or at the end, and shows it. One
    /// step. Returns its position.
    pub fn add(&mut self, name: &str, doc: Doc, at: Option<usize>) -> usize {
        let name = protocol::free_name(name.trim(), |name| self.frames.iter().any(|frame| frame.name == name));
        let frame = self.frame_of(&name, doc);
        let at = at.unwrap_or(self.frames.len()).min(self.frames.len());
        let before = self.shown;
        self.put(at, frame);
        self.commit(Change::Add { at, frame: None }, before);
        at
    }

    /// A copy of an image, its layers as they are now with a history of
    /// its own, beside it and shown, named "… copy". One step.
    pub fn duplicate(&mut self, index: usize) -> Result<usize, String> {
        self.check(index)?;
        let source = &mut self.frames[index];
        source.doc.finish_edit();
        let mut doc = Doc::new(source.doc.image.clone());
        doc.active = source.doc.active;
        doc.notes = source.doc.notes.clone();
        doc.notes_hidden = source.doc.notes_hidden;
        doc.previs = source.doc.previs.clone();
        let name = format!("{} copy", source.name);
        Ok(self.add(&name, doc, Some(index + 1)))
    }

    /// The cell the Prompt page shows.
    pub fn current_cell(&self) -> Option<&Cell> {
        let id = self.cell.as_deref()?;
        self.cells.iter().find(|cell| cell.id == id)
    }

    /// Makes the Prompt page's cell, an empty draft, when the tab has none.
    /// It is no step: nothing was asked for. Returns whether it made one.
    pub fn ensure_cell(&mut self) -> bool {
        if self.current_cell().is_some() {
            return false;
        }
        if let Some(last) = self.cells.last() {
            self.cell = Some(last.id.clone());
            return false;
        }
        let image = &self.frame().doc.image;
        let draft = empty_draft((image.width, image.height));
        self.cell = Some(draft.id.clone());
        self.cells.push(draft);
        true
    }

    /// Replaces the tab's cells and the one shown. One step; the same
    /// cells are none. Returns whether they changed.
    pub fn set_cells(&mut self, cells: Vec<Cell>, cell: Option<String>) -> bool {
        let after = (cells, cell);
        if after.0 == self.cells && after.1 == self.cell {
            return false;
        }
        let before = (std::mem::replace(&mut self.cells, after.0.clone()), std::mem::replace(&mut self.cell, after.1.clone()));
        let shown = self.shown;
        self.commit(Change::Cells { before, after }, shown);
        true
    }

    /// A file filed for a draft arrived: `part` joins the draft `id`, in
    /// place of any part with its role, now and in every step of the
    /// history whose copy of the draft is still `made`, so the step that
    /// made the draft has it. No step of its own. Returns whether the
    /// draft was found.
    pub fn amend_draft(&mut self, id: &str, made: &[CellPart], part: CellPart) -> bool {
        let role = |own: &CellPart| match own {
            CellPart::Path { role, .. } | CellPart::Item { role, .. } => role.clone(),
            _ => None,
        };
        let wanted = role(&part);
        let amend = |cell: &mut Cell| {
            cell.input.retain(|own| wanted.is_none() || role(own) != wanted);
            cell.input.push(part.clone());
        };
        let mut found = false;
        for cell in self.cells.iter_mut().filter(|cell| cell.id == id && cell.step.is_none()) {
            amend(cell);
            found = true;
        }
        for kept in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if let Change::Cells { before, after } = &mut kept.change {
                for cell in before.0.iter_mut().chain(after.0.iter_mut()).filter(|cell| cell.id == id && cell.step.is_none() && cell.input == made) {
                    amend(cell);
                }
            }
        }
        if found {
            self.edits += 1;
        }
        found
    }

    /// The draft cell `id` is now that chain step: it keeps only the
    /// pointer, here and in every step of the history that holds it, so
    /// no undo brings its draft back. Not a step itself. Returns whether
    /// the tab has that cell.
    pub fn point_at_step(&mut self, id: &str, step: &StepRef) -> bool {
        let point = |cells: &mut Vec<Cell>| {
            for cell in cells.iter_mut().filter(|cell| cell.id == id) {
                cell.step = Some(step.clone());
                *cell = crate::files::stored_cell(cell);
            }
        };
        let found = self.cells.iter().any(|cell| cell.id == id);
        point(&mut self.cells);
        for kept in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if let Change::Cells { before, after } = &mut kept.change {
                point(&mut before.0);
                point(&mut after.0);
            }
        }
        if found {
            self.edits += 1;
        }
        found
    }

    /// Takes an image out. The gallery keeps at least one. One step.
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        self.check(index)?;
        if self.frames.len() == 1 {
            return Err("The gallery keeps at least one image.".into());
        }
        let before = self.shown;
        let frame = self.take(index);
        self.commit(Change::Remove { at: index, frame: Some(Box::new(frame)) }, before);
        Ok(())
    }

    /// Renames an image. One step; the same name is none.
    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), String> {
        self.check(index)?;
        let name = name.trim();
        if name.is_empty() {
            return Err("An image's name cannot be empty.".into());
        }
        let before = std::mem::replace(&mut self.frames[index].name, name.to_string());
        if before != name {
            let shown = self.shown;
            self.commit(Change::Rename { at: index, before, after: name.to_string() }, shown);
        }
        Ok(())
    }

    /// Moves an image to `to`, counted after it is taken out, as the strip's
    /// drag sends it. The image shown stays shown. One step.
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<(), String> {
        self.check(from)?;
        let to = to.min(self.frames.len() - 1);
        if from == to {
            return Ok(());
        }
        let before = self.shown;
        self.shift(from, to);
        self.shown = self.follow(before, from, to);
        self.commit(Change::Move { from, to }, before);
        Ok(())
    }

    /// Where the image at `index` is after the one at `from` moves to `to`.
    fn follow(&self, index: usize, from: usize, to: usize) -> usize {
        if index == from {
            to
        } else {
            let out = if index > from { index - 1 } else { index };
            if out >= to { out + 1 } else { out }
        }
    }

    fn shift(&mut self, from: usize, to: usize) {
        let frame = self.frames.remove(from);
        self.frames.insert(to, frame);
    }

    /// Puts an image in at `at` and shows it.
    fn put(&mut self, at: usize, frame: Frame) {
        self.frames.insert(at, frame);
        self.shown = at;
    }

    /// Takes the image at `at` out. The one shown stays shown; when it is
    /// the one taken, the next is shown, or the one before at the end.
    fn take(&mut self, at: usize) -> Frame {
        let frame = self.frames.remove(at);
        if self.shown > at {
            self.shown -= 1;
        } else if self.shown == at {
            self.shown = at.min(self.frames.len() - 1);
        }
        frame
    }

    /// Undoes the strip's last step. The image it touched is shown, or,
    /// for an added one, the image shown before it was added.
    pub fn undo(&mut self) -> bool {
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        match &mut step.change {
            Change::Add { at, frame } => {
                let shown_before = step.shown_before;
                *frame = Some(Box::new(self.take(*at)));
                self.shown = shown_before.min(self.frames.len() - 1);
            }
            Change::Remove { at, frame } => {
                if let Some(frame) = frame.take() {
                    self.put(*at, *frame);
                }
            }
            Change::Rename { at, before, .. } => {
                self.frames[*at].name = before.clone();
                self.shown = *at;
            }
            Change::Move { from, to } => {
                self.shift(*to, *from);
                self.shown = *from;
            }
            Change::Cells { before, .. } => {
                self.cells = before.0.clone();
                self.cell = before.1.clone();
            }
        }
        self.redo.push(step);
        self.edits += 1;
        true
    }

    /// Does the strip's last undone step again, showing the image it
    /// touched, or, for a removed one, the image shown after it went.
    pub fn redo(&mut self) -> bool {
        let Some(mut step) = self.redo.pop() else {
            return false;
        };
        match &mut step.change {
            Change::Add { at, frame } => {
                if let Some(frame) = frame.take() {
                    self.put(*at, *frame);
                }
            }
            Change::Remove { at, frame } => {
                *frame = Some(Box::new(self.take(*at)));
            }
            Change::Rename { at, after, .. } => {
                self.frames[*at].name = after.clone();
                self.shown = *at;
            }
            Change::Move { from, to } => {
                self.shift(*from, *to);
                self.shown = *to;
            }
            Change::Cells { after, .. } => {
                self.cells = after.0.clone();
                self.cell = after.1.clone();
            }
        }
        self.undo.push(step);
        self.edits += 1;
        true
    }
}

/// A new image cell with no prompt, as the Prompt page starts, its Size
/// the image's it is for.
pub fn empty_draft(size: (u32, u32)) -> Cell {
    let mut cell = Cell::text(protocol::cell_id(), "");
    cell.kind = String::from(protocol::IMAGE_KIND);
    cell.input = Vec::new();
    cell.state = CellState::Idle;
    cell.settings = Some(size_setting(size));
    cell
}

/// A draft's Size setting, as ig-request-anchor writes it: "1536 × 864".
pub fn size_setting(size: (u32, u32)) -> serde_json::Value {
    serde_json::json!({ "size": format!("{} × {}", size.0, size.1) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layered::LayeredImage;

    fn blank() -> Doc {
        Doc::new(LayeredImage::blank(8, 6))
    }

    fn names(gallery: &Gallery) -> Vec<&str> {
        gallery.frames.iter().map(|frame| frame.name.as_str()).collect()
    }

    /// Catches a strip step that undo does not reverse, or redo does not
    /// do again: add, duplicate, remove, rename, and move, with the shown
    /// image following each.
    #[test]
    fn the_strips_undo_reverses_each_change() {
        let mut gallery = Gallery::one("Untitled Image", blank());
        assert_eq!(gallery.add("Untitled Image", blank(), None), 1);
        assert_eq!(names(&gallery), ["Untitled Image", "Untitled Image 2"]);
        assert_eq!(gallery.shown, 1);
        gallery.select(0).unwrap();
        assert_eq!(gallery.duplicate(0).unwrap(), 1);
        assert_eq!(names(&gallery), ["Untitled Image", "Untitled Image copy", "Untitled Image 2"]);
        gallery.rename(2, " Beach ").unwrap();
        gallery.move_to(2, 0).unwrap();
        assert_eq!(names(&gallery), ["Beach", "Untitled Image", "Untitled Image copy"]);
        assert_eq!(gallery.frame().name, "Untitled Image copy", "a move changed the image shown");
        gallery.select(2).unwrap();
        gallery.remove(2).unwrap();
        assert_eq!(names(&gallery), ["Beach", "Untitled Image"]);
        assert_eq!(gallery.shown, 1);

        // Each undo puts one change back, and shows the image it touched.
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Beach", "Untitled Image", "Untitled Image copy"]);
        assert_eq!(gallery.shown, 2);
        gallery.select(1).unwrap();
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Untitled Image", "Untitled Image copy", "Beach"]);
        assert_eq!(gallery.shown, 2, "the moved image is not shown after its move is undone");
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Untitled Image", "Untitled Image copy", "Untitled Image 2"]);
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Untitled Image", "Untitled Image 2"]);
        assert_eq!(gallery.shown, 0, "undoing a duplicate does not show what was shown before it");
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Untitled Image"]);
        assert!(!gallery.undo());

        for _ in 0..5 {
            assert!(gallery.redo());
        }
        assert_eq!(names(&gallery), ["Beach", "Untitled Image"]);
        assert!(!gallery.can_redo());
    }

    /// Catches the last image deleted, and a removed image that comes
    /// back without its own history.
    #[test]
    fn the_last_image_stays_and_a_removed_one_keeps_its_history() {
        let mut gallery = Gallery::one("Only", blank());
        assert!(gallery.remove(0).is_err());
        assert_eq!(gallery.frames.len(), 1);
        assert!(!gallery.can_undo(), "a refused delete became a step");
        gallery.add("Second", blank(), None);
        gallery.frame_mut().doc.add_layer("Ink", None, None).unwrap();
        gallery.remove(1).unwrap();
        assert!(gallery.remove(0).is_err());
        gallery.undo();
        assert_eq!(gallery.frame().name, "Second");
        assert!(gallery.frame().doc.can_undo(), "the image came back without its history");
        assert!(gallery.frame_mut().doc.undo());
        assert_eq!(gallery.frame().doc.image.nodes.len(), 1);
    }

    /// Catches a move that loses track of the image shown.
    #[test]
    fn a_move_keeps_the_shown_image_shown() {
        let mut gallery = Gallery::new(
            ["A", "B", "C", "D"].into_iter().map(|name| (name.to_string(), blank())).collect(),
            1,
        );
        gallery.move_to(0, 3).unwrap();
        assert_eq!(names(&gallery), ["B", "C", "D", "A"]);
        assert_eq!(gallery.frame().name, "B");
        gallery.move_to(3, 1).unwrap();
        assert_eq!(names(&gallery), ["B", "A", "C", "D"]);
        assert_eq!(gallery.frame().name, "B");
        gallery.select(2).unwrap();
        gallery.move_to(1, 2).unwrap();
        assert_eq!(names(&gallery), ["B", "C", "A", "D"]);
        assert_eq!(gallery.frame().name, "C");
    }

    /// Catches a cell edit that is no step, or a tab with no cell: a new
    /// tab makes an empty draft, which is no step; a change to the cells
    /// is one step that undo puts back and redo does again, and the same
    /// cells are none.
    #[test]
    fn the_cells_change_as_steps_of_the_strip() {
        let mut gallery = Gallery::one("Only", blank());
        assert!(gallery.ensure_cell());
        assert!(!gallery.ensure_cell());
        assert!(!gallery.can_undo(), "making the first draft was a step");
        let draft = gallery.current_cell().unwrap().clone();
        assert_eq!((draft.kind.as_str(), draft.prompt().as_str()), (protocol::IMAGE_KIND, ""));
        assert_eq!(draft.settings, Some(serde_json::json!({ "size": "8 × 6" })), "a new draft's Size is not its image's");
        let mut edited = draft.clone();
        edited.input = vec![protocol::CellPart::Text { text: "A lighthouse.".into() }];
        assert!(gallery.set_cells(vec![edited.clone()], Some(draft.id.clone())));
        assert!(!gallery.set_cells(vec![edited], Some(draft.id.clone())), "the same cells were a step");
        assert!(gallery.undo());
        assert_eq!(gallery.current_cell().unwrap().prompt(), "");
        assert!(gallery.redo());
        assert_eq!(gallery.current_cell().unwrap().prompt(), "A lighthouse.");
    }
}
