//! A take as a reference asset, as the data model guide has it. Use this
//! take on a take's menu asks the host to use it, beside the takes in use,
//! replacing none, so it asks nothing first; a fresh `takes` brings its
//! seal, `badge-check` on its tile, from `canon`. An out-of-date take's
//! menu adds Ignore, which asks the host to clear its outline, and "Ask the
//! assistant to fix the dependency chain", which puts the fix in the
//! composer. Save on an image of the gallery made from a take writes the
//! image as a PNG in the project's `work/` and asks the host to save it as
//! the asset's next iteration; the host warns first when other work uses
//! it. A take's stack badge opens its iterations as a run. Each answer's
//! error is said at the working area's bottom left; a success says
//! nothing, and takes that error away.

use super::*;
use protocol::{Action, Notice, NoticeCorner, NoticeState};

impl Session {
    /// Use this take: `use_take` for the take at `path`, with the cell's
    /// item as `takes` names it, the tab's file and the cell. The tab's file
    /// is written first when it has none, so the item has a file.
    pub(super) fn use_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((tab, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::UseTake { tab, cell, path: path.to_string() });
        }
    }

    /// Ignore: `ignore_take` for the out-of-date take at `path`, which
    /// clears its outline and no other's.
    pub(super) fn ignore_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((tab, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::IgnoreTake { tab, cell, path: path.to_string() });
        }
    }

    /// "Ask the assistant to fix the dependency chain": the fix put in the
    /// composer with the take's cell, for the person to send.
    pub(super) fn ask_to_fix(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((_, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::AskAssistant { text: FIX.into(), items: vec![cell] });
        }
    }

    /// A take's stack badge: its iterations opened as a run after its
    /// round, or closed.
    pub(super) fn toggle_stack(&mut self, path: &str) {
        let cache = self.cache.clone();
        let Some(image) = self.images.get_mut(self.active) else {
            return;
        };
        let order: Vec<String> = image.gallery.cells.iter().map(|own| own.id.clone()).collect();
        for gone in image.rounds.toggle_stack(path, &order) {
            forget_files(&cache, &image.id, gone);
        }
        // The take itself and its earlier iterations are read once shown.
        let root = self.project.clone().unwrap_or_default();
        let mut next = self.next_revision;
        image.rounds.read_files(&root, &image.id, &cache, &mut next);
        self.next_revision = next;
    }

    /// The tab and the cell's item as `takes` names it, for the take at
    /// `path` or any iteration of its asset, the tab's file written first
    /// when it has none, so the item has a file.
    fn asked_of(&mut self, path: &str, out: &mut Vec<AppMessage>) -> Option<(String, ItemRef)> {
        let index = self.active;
        let image = self.images.get(index)?;
        let tile = image.rounds.tile_of(path).or_else(|| image.rounds.asset_of(path))?;
        let cell = image.rounds.tiles[tile].cell.clone();
        if self.images[index].path.is_none() {
            self.images[index].dirty = true;
        }
        let file = match self.write_into_project(index) {
            Ok(Some(file)) => file,
            Ok(None) => return None,
            Err(reason) => {
                out.push(notices(&self.images[index].id, Some(failed(&reason))));
                return None;
            }
        };
        let own = self.relative(&file)?;
        Some((self.images[index].id.clone(), ItemRef { path: own, item: Some(cell) }))
    }

    /// Whether the image shown is an image of the gallery made from a take
    /// of the strip, a linked layer naming the take or one of its
    /// iterations, so Save saves it as the take's next iteration.
    pub fn edits_a_take(&self) -> bool {
        self.active_image().is_some_and(|image| {
            !image.take_shown() && all_links(&image.gallery.frame().doc).iter().any(|link| image.rounds.asset_of(link).is_some())
        })
    }

    /// Save, in a project: when the image shown of the gallery was made
    /// from a take of the strip, a linked layer naming the take or one of
    /// its iterations, and differs from the asset's latest iteration, it is
    /// written as a PNG in `work/`, named after the image and free among
    /// the files there, and `save_iteration` asks the host to make it the
    /// asset's next iteration. The image is compared with the latest
    /// iteration's file, so an edit kept by itself and reopened is saved,
    /// and one saved already, or never changed, is not.
    pub(super) fn save_iteration(&mut self, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(image) = self.images.get(index) else {
            return;
        };
        if image.take_shown() {
            return;
        }
        let frame = image.gallery.frame();
        let (uid, edits, name) = (frame.uid, frame.doc.edits, frame.name.clone());
        let Some((path, tile)) = all_links(&frame.doc).into_iter().find_map(|link| image.rounds.asset_of(&link).map(|tile| (link, tile))) else {
            return;
        };
        let latest = image.rounds.tiles[tile].path().map(str::to_string);
        let tab = image.id.clone();
        // Saved as an iteration since its last change, while the program runs.
        if self.iterated.get(&(tab.clone(), uid)) == Some(&edits) {
            return;
        }
        let Some(folder) = self.work_folder() else {
            return;
        };
        let root = self.project.clone().unwrap_or_default();
        let doc = &mut self.images[index].gallery.frame_mut().doc;
        let (width, height) = (doc.image.width, doc.image.height);
        let pixels = doc.composite().to_vec();
        // The same pixels as the latest iteration: nothing new to save.
        let same = latest.and_then(|latest| files::import(&files::resolve(&root, &latest)).ok()).is_some_and(|file| file.width == width && file.height == height && file.data == pixels);
        if same {
            return;
        }
        let png = match files::encode_png(width, height, &pixels) {
            Ok(png) => png,
            Err(reason) => {
                out.push(notices(&tab, Some(failed(&reason))));
                return;
            }
        };
        let stem = match safe_name(&name) {
            stem if stem.is_empty() => String::from("iteration"),
            stem => stem,
        };
        let stem = protocol::free_name(&stem, |stem| folder.join(format!("{stem}.png")).exists());
        let written = folder.join(format!("{stem}.png"));
        if let Err(reason) = files::write_atomic(&written, &png) {
            out.push(notices(&tab, Some(failed(&reason))));
            return;
        }
        let (Some(source), Some((tab, cell))) = (self.relative(&written), self.asked_of(&path, out)) else {
            return;
        };
        self.iterating.insert(tab.clone(), (uid, edits, written));
        out.push(AppMessage::SaveIteration { tab, cell, path, source });
    }

    /// `iteration_saved`: the host's answer to a Save. A file means the
    /// image is that iteration now, so it is not saved as another until it
    /// changes; no file and no error means the warning was cancelled. The
    /// PNG written for it goes either way: the host files a copy.
    pub(super) fn iteration_saved(&mut self, tab: &str, file: Option<String>, error: Option<String>, out: &mut Vec<AppMessage>) {
        if let Some((uid, edits, written)) = self.iterating.remove(tab) {
            let _ = std::fs::remove_file(&written);
            if file.is_some_and(|file| !file.is_empty()) {
                self.iterated.insert((tab.to_string(), uid), edits);
            }
        }
        self.take_used(tab, error, out);
    }

    /// `take_used`, and `take_ignored`: the host's answer to Use this take,
    /// or to Ignore. An error is said as the failed notice; a success says
    /// nothing, as the seal or the outline comes with the next `takes`, and
    /// takes away a refusal still shown.
    pub(super) fn take_used(&mut self, tab: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        if let Some(error) = error.filter(|error| !error.is_empty()) {
            self.refused.insert(tab.to_string());
            out.push(notices(tab, Some(failed(&error))));
        } else if self.refused.remove(tab) {
            out.push(notices(tab, None));
        }
    }
}

/// What "Ask the assistant to fix the dependency chain" puts in the
/// composer.
pub(super) const FIX: &str = "Fix the dependency chain.";

/// A tab's notice cards, in the working area's bottom left as
/// ig-layer-export shows them: this one, or none.
pub(super) fn notices(tab: &str, notice: Option<Notice>) -> AppMessage {
    AppMessage::Notices { tab: tab.to_string(), notices: notice.into_iter().collect(), corner: NoticeCorner::BottomLeft }
}

/// A failed notice with its sentence, which can be closed.
fn failed(reason: &str) -> Notice {
    Notice {
        id: String::from("in-use"),
        text: reason.to_string(),
        detail: String::new(),
        progress: None,
        state: NoticeState::Failed,
        actions: Vec::new(),
        close: Some(Action::new("notice_close")),
    }
}

/// Every linked layer's file in an image, lowest first, each once.
pub(super) fn all_links(doc: &Doc) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for linked in doc.image.every_node().into_iter().filter_map(|node| node.link.as_ref()) {
        if !files.contains(&linked.link.path) {
            files.push(linked.link.path.clone());
        }
    }
    files
}
