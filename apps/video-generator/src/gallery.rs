//! A tab's gallery: its takes in filmstrip order, the one shown, how the
//! working area shows them, and the gallery's own history. A take is a
//! link to a video file, which is never copied or written. Adding,
//! duplicating, removing, renaming, and moving takes, and changing the
//! view, are steps of the history; choosing a take on the strip and
//! stepping its frames are not, as in the Image Generator.

use layers::link::Link;
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
    /// the same frame.
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
        }
        self.undo.push(step);
        self.edits += 1;
        true
    }
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
}
