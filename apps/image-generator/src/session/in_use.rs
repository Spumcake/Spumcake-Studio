//! A take in use, as the data model guide has it: Use this take on a
//! take's menu asks the host to use it, beside the takes in use, replacing
//! none, so it asks nothing first; the host answers `take_used`, and a
//! fresh `takes` brings its seal, `badge-check` on its tile, from `canon`.
//! An error is said at the working area's bottom left; a success says
//! nothing, and takes that error away. Saving an image as a new iteration
//! of an asset, and the outline on what was made from one, come later.

use super::*;
use protocol::{Action, Notice, NoticeCorner, NoticeState};

impl Session {
    /// Use this take: `use_take` for the take at `path`, with the cell's
    /// item as `takes` names it, the tab's file and the cell. The tab's file
    /// is written first when it has none, so the item has a file.
    pub(super) fn use_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(cell) = self.images.get(index).and_then(|image| image.rounds.tile_of(path).map(|tile| image.rounds.tiles[tile].cell.clone())) else {
            return;
        };
        if self.images[index].path.is_none() {
            self.images[index].dirty = true;
        }
        let file = match self.write_into_project(index) {
            Ok(Some(file)) => file,
            Ok(None) => return,
            Err(reason) => {
                out.push(notices(&self.images[index].id, Some(failed(&reason))));
                return;
            }
        };
        let Some(own) = self.relative(&file) else {
            return;
        };
        let tab = self.images[index].id.clone();
        out.push(AppMessage::UseTake { tab, cell: ItemRef { path: own, item: Some(cell) }, path: path.to_string() });
    }

    /// `take_used`: the host's answer to Use this take. An error is said as
    /// the failed notice; a success says nothing, as the seal comes with
    /// the next `takes`, and takes away a refusal still shown.
    pub(super) fn take_used(&mut self, tab: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        if let Some(error) = error.filter(|error| !error.is_empty()) {
            self.refused.insert(tab.to_string());
            out.push(notices(tab, Some(failed(&error))));
        } else if self.refused.remove(tab) {
            out.push(notices(tab, None));
        }
    }
}

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
