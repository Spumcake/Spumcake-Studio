//! A tab's gallery: its takes in filmstrip order, the one shown, how the
//! working area shows them, and the gallery's own history. A take is a
//! link to a video file, which is never copied or written. Adding,
//! duplicating, removing, renaming, and moving takes, and changing the
//! view, are steps of the history; choosing a take on the strip and
//! stepping its frames are not, as in the Image Generator. So are changes
//! to the tab's cells: a prompt edited on the Prompt page, and a next round
//! made from a take; and to the notes marked on a take: a pin with its
//! words, a mark, a pin moved, and a pin taken away.

use layers::link::Link;
use layers::notes::Notes;
use protocol::{Cell, CellState, StepRef};
use std::path::PathBuf;

/// One take of a gallery: a frame of its filmstrip.
#[derive(Debug, Clone)]
pub struct Take {
    /// Unique within its tab for this run.
    pub uid: u64,
    pub name: String,
    /// The file as the `.vgx` names it, and its sha256.
    pub link: Link,
    /// The file itself, as an absolute path.
    pub file: PathBuf,
    /// What the file is, once it opened; none when it did not.
    pub info: Option<media::Info>,
}

impl Take {
    /// How long the take is, in seconds, for the filmstrip's badge.
    pub fn seconds(&self) -> Option<f64> {
        self.info.map(|info| f64::from(info.frames) / info.fps().max(1.0))
    }

    /// The last frame, from 0.
    pub fn last_frame(&self) -> u32 {
        self.info.map_or(0, |info| info.frames.saturating_sub(1))
    }
}

/// How the working area shows the tab's takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The shown take alone, at the current frame.
    #[default]
    Single,
    /// Every take, a row of eight frames each, in filmstrip order.
    Strips,
    /// The shown take on the left and another, by its uid, on the right, at
    /// the same time.
    Side { other: u64 },
}

/// One step of the history. A take taken out of the gallery is held by its
/// step until the step is put back.
#[derive(Debug)]
enum Change {
    /// A take put in at `at`; held here while the step is undone.
    Add { at: usize, take: Option<Box<Take>> },
    /// A take taken out from `at`; held here while the step is done.
    Remove { at: usize, take: Option<Box<Take>> },
    Rename { at: usize, before: String, after: String },
    /// `to` counted after the moved take is taken out.
    Move { from: usize, to: usize },
    /// The view and the take shown, before and after.
    View { before: (Mode, usize), after: (Mode, usize) },
    /// The tab's cells and the one the Prompt page shows, before and after.
    Cells { before: (Vec<Cell>, Option<String>), after: (Vec<Cell>, Option<String>) },
    /// One take's notes, by its file, before and after.
    Notes { path: String, sha256: String, before: Notes, after: Notes },
}

/// The notes marked on a take, kept by its file: its path, as the `.vgx`
/// names a take's file, and the sha256 of the bytes they were marked on,
/// so they come back only for the same take.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeNotes {
    pub path: String,
    pub sha256: String,
    pub notes: Notes,
}

#[derive(Debug)]
struct Step {
    change: Change,
    /// The take shown before the step, by position then.
    shown_before: usize,
}

#[derive(Debug, Default)]
pub struct Gallery {
    /// May be empty: a new video has no takes yet.
    pub takes: Vec<Take>,
    /// The take shown, by position; 0 when there are none.
    pub shown: usize,
    pub mode: Mode,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Rises with every change, done or undone.
    pub edits: u64,
    next_uid: u64,
    /// The tab's cells, each a pointer to a chain step or a draft before
    /// its first run, kept in its file, and the one the Prompt page shows.
    pub cells: Vec<Cell>,
    pub cell: Option<String>,
    /// The notes marked on takes, the gallery's and the rounds' alike,
    /// kept in its file as editing state.
    pub notes: Vec<TakeNotes>,
}

impl Gallery {
    /// A gallery of `takes`, as a file keeps them, with no history.
    pub fn new(takes: Vec<(String, Link, PathBuf, Option<media::Info>)>, shown: usize) -> Self {
        let mut gallery = Self { next_uid: 1, ..Self::default() };
        for (name, link, file, info) in takes {
            let take = gallery.take_of(&name, link, file, info);
            gallery.takes.push(take);
        }
        gallery.shown = shown.min(gallery.takes.len().saturating_sub(1));
        gallery
    }

    fn take_of(&mut self, name: &str, link: Link, file: PathBuf, info: Option<media::Info>) -> Take {
        let uid = self.next_uid;
        self.next_uid += 1;
        Take { uid, name: name.to_string(), link, file, info }
    }

    /// The take shown, if there is one.
    pub fn take(&self) -> Option<&Take> {
        self.takes.get(self.shown)
    }

    pub fn uid_at(&self, index: usize) -> Option<u64> {
        self.takes.get(index).map(|take| take.uid)
    }

    pub fn position(&self, uid: u64) -> Option<usize> {
        self.takes.iter().position(|take| take.uid == uid)
    }

    /// The view as it can be drawn now: side by side only while its other
    /// take is in the gallery and is not the one shown.
    pub fn view(&self) -> Mode {
        match self.mode {
            Mode::Side { other } if self.position(other).is_none() || self.uid_at(self.shown) == Some(other) => Mode::Single,
            mode => mode,
        }
    }

    /// The take on the right while side by side.
    pub fn other(&self) -> Option<usize> {
        match self.view() {
            Mode::Side { other } => self.position(other),
            _ => None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Whether the step an undo, or a redo, would take is a take's notes,
    /// which leaves the take shown as it is.
    pub fn steps_notes(&self, redo: bool) -> bool {
        let next = if redo { self.redo.last() } else { self.undo.last() };
        next.is_some_and(|step| matches!(step.change, Change::Notes { .. }))
    }

    fn check(&self, index: usize) -> Result<(), String> {
        if index < self.takes.len() {
            Ok(())
        } else {
            Err(format!("There is no take {index}; the gallery holds {}.", self.takes.len()))
        }
    }

    fn commit(&mut self, change: Change, shown_before: usize) {
        self.undo.push(Step { change, shown_before });
        self.redo.clear();
        self.edits += 1;
    }

    /// The notes marked on the take whose file is `path` with `sha256`.
    pub fn notes_of(&self, path: &str, sha256: &str) -> Notes {
        self.notes.iter().find(|kept| kept.path == path && kept.sha256 == sha256).map(|kept| kept.notes.clone()).unwrap_or_default()
    }

    /// Replaces a take's notes. One step; the same notes are none. Returns
    /// whether they changed.
    pub fn set_notes(&mut self, path: &str, sha256: &str, notes: Notes) -> bool {
        let before = self.notes_of(path, sha256);
        if before == notes {
            return false;
        }
        self.put_notes(path, sha256, notes.clone());
        let shown = self.shown;
        self.commit(Change::Notes { path: path.to_string(), sha256: sha256.to_string(), before, after: notes }, shown);
        true
    }

    fn put_notes(&mut self, path: &str, sha256: &str, notes: Notes) {
        self.notes.retain(|kept| kept.path != path || kept.sha256 != sha256);
        if !notes.is_empty() {
            self.notes.push(TakeNotes { path: path.to_string(), sha256: sha256.to_string(), notes });
        }
    }

    /// The cell the Prompt page shows.
    pub fn current_cell(&self) -> Option<&Cell> {
        let id = self.cell.as_deref()?;
        self.cells.iter().find(|cell| cell.id == id)
    }

    /// Makes the Prompt page's cell, an empty draft of `size`, when the tab
    /// has none. It is no step: nothing was asked for. Returns whether it
    /// made one.
    pub fn ensure_cell(&mut self, size: (u32, u32)) -> bool {
        if self.current_cell().is_some() {
            return false;
        }
        if let Some(last) = self.cells.last() {
            self.cell = Some(last.id.clone());
            return false;
        }
        let draft = empty_draft(size);
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

    /// The draft cell `id` is now that chain step: it keeps only the
    /// pointer, here and in every step of the history that holds it, so
    /// no undo brings its draft back. Not a step itself. Returns whether
    /// the tab has that cell.
    pub fn point_at_step(&mut self, id: &str, step: &StepRef) -> bool {
        let point = |cells: &mut Vec<Cell>| {
            for cell in cells.iter_mut().filter(|cell| cell.id == id) {
                cell.step = Some(step.clone());
                *cell = layers::cells::stored_cell(cell);
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

    /// Shows the take at `index`. Choosing a take is not a step.
    pub fn select(&mut self, index: usize) -> Result<(), String> {
        self.check(index)?;
        self.shown = index;
        Ok(())
    }

    /// Adds a take of `file`, named `name` or the first name after it that
    /// no take of the gallery has, at `at` or at the end, and shows it. One
    /// step. Returns its position.
    pub fn add(&mut self, name: &str, link: Link, file: PathBuf, info: Option<media::Info>, at: Option<usize>) -> usize {
        let name = protocol::free_name(name.trim(), |name| self.takes.iter().any(|take| take.name == name));
        let take = self.take_of(&name, link, file, info);
        let at = at.unwrap_or(self.takes.len()).min(self.takes.len());
        let before = self.shown;
        self.put(at, take);
        self.commit(Change::Add { at, take: None }, before);
        at
    }

    /// A second entry linking the same file, beside it and shown, named
    /// "… copy". One step.
    pub fn duplicate(&mut self, index: usize) -> Result<usize, String> {
        self.check(index)?;
        let source = &self.takes[index];
        let (name, link, file, info) = (format!("{} copy", source.name), source.link.clone(), source.file.clone(), source.info);
        Ok(self.add(&name, link, file, info, Some(index + 1)))
    }

    /// Takes a take out of the gallery; its file stays as it is. One step.
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        self.check(index)?;
        let before = self.shown;
        let take = self.take_out(index);
        self.commit(Change::Remove { at: index, take: Some(Box::new(take)) }, before);
        Ok(())
    }

    /// Renames a take. One step; the same name is none.
    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), String> {
        self.check(index)?;
        let name = name.trim();
        if name.is_empty() {
            return Err("A take's name cannot be empty.".into());
        }
        let before = std::mem::replace(&mut self.takes[index].name, name.to_string());
        if before != name {
            let shown = self.shown;
            self.commit(Change::Rename { at: index, before, after: name.to_string() }, shown);
        }
        Ok(())
    }

    /// Moves a take to `to`, counted after it is taken out, as the strip's
    /// drag sends it. The take shown stays shown. One step.
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<(), String> {
        self.check(from)?;
        let to = to.min(self.takes.len() - 1);
        if from == to {
            return Ok(());
        }
        let before = self.shown;
        self.shift(from, to);
        self.shown = follow(before, from, to);
        self.commit(Change::Move { from, to }, before);
        Ok(())
    }

    /// Shows `mode`, with the take at `shown`. One step when either changes
    /// what the working area shows; choosing another take alone is not.
    pub fn set_view(&mut self, mode: Mode, shown: usize) -> Result<(), String> {
        if !self.takes.is_empty() {
            self.check(shown)?;
        }
        if let Mode::Side { other } = mode {
            match self.position(other) {
                None => return Err("The other take is not in the gallery.".into()),
                Some(position) if position == shown => return Err("A take cannot be shown beside itself.".into()),
                Some(_) => {}
            }
        }
        let before = (self.view(), self.shown);
        if before.0 == mode {
            self.shown = shown;
            return Ok(());
        }
        self.mode = mode;
        self.shown = shown;
        let after = (mode, shown);
        self.commit(Change::View { before, after }, before.1);
        Ok(())
    }

    fn shift(&mut self, from: usize, to: usize) {
        let take = self.takes.remove(from);
        self.takes.insert(to, take);
    }

    /// Puts a take in at `at` and shows it.
    fn put(&mut self, at: usize, take: Take) {
        self.takes.insert(at, take);
        self.shown = at;
    }

    /// Takes the take at `at` out. The one shown stays shown; when it is the
    /// one taken, the next is shown, or the one before at the end.
    fn take_out(&mut self, at: usize) -> Take {
        let take = self.takes.remove(at);
        if self.shown > at {
            self.shown -= 1;
        } else if self.shown == at {
            self.shown = at.min(self.takes.len().saturating_sub(1));
        }
        take
    }

    /// Undoes the last step, showing the take it touched, or, for an added
    /// or removed one, the take shown before the step.
    pub fn undo(&mut self) -> bool {
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        match &mut step.change {
            Change::Add { at, take } => {
                *take = Some(Box::new(self.take_out(*at)));
                self.shown = step.shown_before.min(self.takes.len().saturating_sub(1));
            }
            Change::Remove { at, take } => {
                if let Some(take) = take.take() {
                    self.put(*at, *take);
                }
                // Side by side with the take put back holds again.
                self.shown = step.shown_before.min(self.takes.len() - 1);
            }
            Change::Rename { at, before, .. } => {
                self.takes[*at].name = before.clone();
                self.shown = *at;
            }
            Change::Move { from, to } => {
                self.shift(*to, *from);
                self.shown = *from;
            }
            Change::View { before, .. } => {
                (self.mode, self.shown) = *before;
            }
            Change::Cells { before, .. } => {
                self.cells = before.0.clone();
                self.cell = before.1.clone();
            }
            Change::Notes { path, sha256, before, .. } => {
                let (path, sha256, notes) = (path.clone(), sha256.clone(), before.clone());
                self.put_notes(&path, &sha256, notes);
            }
        }
        self.redo.push(step);
        self.edits += 1;
        true
    }

    /// Does the last undone step again.
    pub fn redo(&mut self) -> bool {
        let Some(mut step) = self.redo.pop() else {
            return false;
        };
        match &mut step.change {
            Change::Add { at, take } => {
                if let Some(take) = take.take() {
                    self.put(*at, *take);
                }
            }
            Change::Remove { at, take } => {
                *take = Some(Box::new(self.take_out(*at)));
            }
            Change::Rename { at, after, .. } => {
                self.takes[*at].name = after.clone();
                self.shown = *at;
            }
            Change::Move { from, to } => {
                self.shift(*from, *to);
                self.shown = *to;
            }
            Change::View { after, .. } => {
                (self.mode, self.shown) = *after;
            }
            Change::Cells { after, .. } => {
                self.cells = after.0.clone();
                self.cell = after.1.clone();
            }
            Change::Notes { path, sha256, after, .. } => {
                let (path, sha256, notes) = (path.clone(), sha256.clone(), after.clone());
                self.put_notes(&path, &sha256, notes);
            }
        }
        self.undo.push(step);
        self.edits += 1;
        true
    }
}

/// A take's length a new draft asks for, in seconds, as the Prompt page's
/// Length starts.
pub const LENGTH: f64 = 8.0;
/// A new draft's Size with no take to take it from.
pub const SIZE: (u32, u32) = (1280, 720);

/// A new video cell with no prompt, as the Prompt page starts: its Size
/// `size`, written "1280 × 720", and its Length 8 seconds.
pub fn empty_draft(size: (u32, u32)) -> Cell {
    let mut cell = Cell::text(protocol::cell_id(), "");
    cell.kind = String::from(protocol::VIDEO_KIND);
    cell.input = Vec::new();
    cell.state = CellState::Idle;
    cell.settings = Some(draft_settings(size));
    cell
}

/// A new draft's settings: its Size, as ig-request-anchor writes one, and
/// its Length.
pub fn draft_settings(size: (u32, u32)) -> serde_json::Value {
    serde_json::json!({ "size": format!("{} × {}", size.0, size.1), "length": LENGTH })
}

/// Where the take at `index` is after the one at `from` moves to `to`.
fn follow(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else {
        let out = if index > from { index - 1 } else { index };
        if out >= to { out + 1 } else { out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str) -> (Link, PathBuf) {
        (Link { path: format!("takes/{name}.mp4"), sha256: String::from("0") }, PathBuf::from(format!("/scratch/takes/{name}.mp4")))
    }

    fn add(gallery: &mut Gallery, name: &str) -> usize {
        let (link, file) = link(name);
        gallery.add(name, link, file, None, None)
    }

    fn names(gallery: &Gallery) -> Vec<&str> {
        gallery.takes.iter().map(|take| take.name.as_str()).collect()
    }

    /// Catches a step that undo does not reverse, or redo does not do
    /// again: add, duplicate, remove, rename, move, and the view, with the
    /// shown take following each.
    #[test]
    fn undo_reverses_each_change() {
        let mut gallery = Gallery::new(Vec::new(), 0);
        assert!(gallery.take().is_none());
        assert_eq!(add(&mut gallery, "Beach"), 0);
        assert_eq!(add(&mut gallery, "Beach"), 1);
        assert_eq!(names(&gallery), ["Beach", "Beach 2"]);
        assert_eq!(gallery.duplicate(0).unwrap(), 1);
        assert_eq!(names(&gallery), ["Beach", "Beach copy", "Beach 2"]);
        assert_eq!(gallery.takes[1].link, gallery.takes[0].link, "a copy links another file");
        gallery.rename(2, " Dunes ").unwrap();
        gallery.move_to(2, 0).unwrap();
        assert_eq!(names(&gallery), ["Dunes", "Beach", "Beach copy"]);
        let other = gallery.uid_at(0).unwrap();
        gallery.set_view(Mode::Side { other }, 1).unwrap();
        assert_eq!(gallery.other(), Some(0));
        gallery.remove(0).unwrap();
        assert_eq!(gallery.view(), Mode::Single, "side by side outlived its other take");

        assert!(gallery.undo());
        assert_eq!(gallery.view(), Mode::Side { other }, "the take came back without the view");
        assert!(gallery.undo());
        assert_eq!(gallery.view(), Mode::Single);
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Beach", "Beach copy", "Dunes"]);
        assert_eq!(gallery.shown, 2);
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Beach", "Beach copy", "Beach 2"]);
        assert!(gallery.undo());
        assert_eq!(names(&gallery), ["Beach", "Beach 2"]);
        assert!(gallery.undo());
        assert!(gallery.undo());
        assert!(gallery.takes.is_empty());
        assert!(!gallery.undo());
        for _ in 0..7 {
            assert!(gallery.redo());
        }
        assert_eq!(names(&gallery), ["Beach", "Beach copy"]);
        assert!(!gallery.can_redo());
    }

    /// Catches a move that loses track of the take shown, and a view that
    /// shows a take beside itself.
    #[test]
    fn a_move_keeps_the_shown_take_and_a_take_is_not_beside_itself() {
        let mut gallery = Gallery::new(Vec::new(), 0);
        for name in ["A", "B", "C", "D"] {
            add(&mut gallery, name);
        }
        gallery.select(1).unwrap();
        gallery.move_to(0, 3).unwrap();
        assert_eq!(names(&gallery), ["B", "C", "D", "A"]);
        assert_eq!(gallery.take().unwrap().name, "B");
        let shown = gallery.uid_at(0).unwrap();
        assert!(gallery.set_view(Mode::Side { other: shown }, 0).is_err());
        assert!(!gallery.can_redo());
        gallery.set_view(Mode::Strips, 0).unwrap();
        gallery.set_view(Mode::Strips, 2).unwrap();
        assert_eq!(gallery.shown, 2, "choosing a take in the same view was refused");
        assert!(gallery.undo());
        assert_eq!((gallery.view(), gallery.shown), (Mode::Single, 0));
    }

    /// Catches a cell edit that is no step, a new draft with no Size or
    /// Length, or a step's draft that an undo brings back.
    #[test]
    fn the_cells_change_as_steps_and_a_step_keeps_its_pointer() {
        let mut gallery = Gallery::new(Vec::new(), 0);
        assert!(gallery.ensure_cell(SIZE));
        assert!(!gallery.can_undo(), "making the first draft was a step");
        let draft = gallery.current_cell().unwrap().clone();
        assert_eq!(draft.kind, protocol::VIDEO_KIND);
        assert_eq!(draft.settings, Some(serde_json::json!({ "size": "1280 × 720", "length": 8.0 })));
        let mut edited = draft.clone();
        edited.input = vec![protocol::CellPart::Text { text: "The door opens.".into() }];
        assert!(gallery.set_cells(vec![edited], Some(draft.id.clone())));
        let step = StepRef { chain: "shot-12".into(), step: "take".into() };
        assert!(gallery.point_at_step(&draft.id, &step));
        assert!(gallery.undo());
        let cell = gallery.current_cell().unwrap();
        assert!(cell.step.is_some() && !cell.copies_its_step(), "an undo brought the draft back");
    }
}
