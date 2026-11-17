//! Publishing an edited image, as milestone 3 of the Image and Video build
//! plan has it: the artifact an image publishes as, which edit may be
//! published, the refusal while a take beneath it is not canon, the
//! `publish` the host's runner carries out with the person's yes, and the
//! canon seal, whose versions the host's history card shows. A take enters the chain only through the
//! assistant: a raw take is never published, and an edit of one only once
//! every take beneath it is canon. Nothing of this is kept in the file but
//! the artifact's name.

use super::*;
use protocol::{Action, Notice, NoticeAction, NoticeCorner, NoticeState};

/// The words of publishing's notices, as the guide has them.
const NOT_CANON: &str = "Its inputs aren't canon.";
const ASK: &str = "Ask the assistant";

impl Session {
    /// What the gallery's image shown publishes as: the artifact of the
    /// step of the cell whose take a linked layer of it was made from, as
    /// the take's round or the cell says it, and whether the image is an
    /// edit. None while a take is shown, or no artifact is known.
    pub fn publishing_as(&self, index: usize) -> Option<(String, bool)> {
        let image = self.images.get(index)?;
        if image.take_shown() {
            return None;
        }
        let doc = &image.gallery.frame().doc;
        let artifact = all_links(doc).iter().find_map(|path| {
            let tile = &image.rounds.tiles[image.rounds.tile_of(path)?];
            image.rounds.round_of(tile).and_then(|round| round.artifact.clone()).or_else(|| image.gallery.artifacts.get(&tile.cell).cloned())
        })?;
        Some((artifact, edited(doc)))
    }

    /// Whether the gallery's image shown has been published while the
    /// program runs.
    pub fn is_published(&self, index: usize) -> bool {
        self.images.get(index).is_some_and(|image| image.published.contains_key(&image.gallery.frame().uid))
    }

    /// Keeps the artifact a cell's step publishes, as the host says it,
    /// while the program runs.
    pub(super) fn keep_artifact(&mut self, cell: &str, artifact: String, out: &mut Vec<AppMessage>) {
        let Some(index) = self.images.iter().position(|image| image.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let image = &mut self.images[index];
        if image.gallery.artifacts.get(cell) != Some(&artifact) {
            image.gallery.artifacts.insert(cell.to_string(), artifact);
            out.extend(self.declared());
        }
    }

    /// Publish: refused, with a notice naming them and Ask the assistant,
    /// while any take beneath the image is not canon, or any linked file is
    /// outside the project, however little was painted. Otherwise the image
    /// as it is now is written out as a PNG under `protocol::render_path`,
    /// and `publish` asks the host's runner to file it as the artifact,
    /// from the tab's file and every linked file beneath, as
    /// `file_resource` names them, and publish it on the person's yes. A
    /// linked file in the project that is no take of the tab's rounds is
    /// the runner's to find canon or not.
    pub(super) fn publish(&mut self, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some((artifact, true)) = self.publishing_as(index) else {
            return;
        };
        let image = &self.images[index];
        let tab = image.id.clone();
        let links = all_links(&image.gallery.frame().doc);
        let outside = |path: &String| Path::new(path).is_absolute() && self.relative(Path::new(path)).is_none();
        let refused: Vec<String> = links
            .iter()
            .filter(|path| {
                outside(path) || image.rounds.tile_of(path).and_then(|tile| image.rounds.tiles[tile].file.as_ref()).is_some_and(|file| !file.canon)
            })
            .cloned()
            .collect();
        if !refused.is_empty() {
            let detail = refused.iter().map(|path| path.rsplit('/').next().unwrap_or(path)).collect::<Vec<_>>().join(", ");
            // Only a take can be put into the chain by asking.
            let takes = refused.iter().any(|path| image.rounds.tile_of(path).is_some());
            let notice = Notice {
                id: String::from("publish"),
                text: NOT_CANON.into(),
                detail,
                progress: None,
                state: NoticeState::Failed,
                actions: if takes { vec![NoticeAction { label: ASK.into(), action: Action::new("publish_ask") }] } else { Vec::new() },
                close: Some(Action::new("notice_close")),
            };
            self.images[index].refused = refused;
            out.push(notices(&tab, Some(notice)));
            return;
        }
        // The tab's file is written first, with the edit, so the runner
        // files the source from it as it is.
        if self.images[index].path.is_none() {
            self.images[index].dirty = true;
        }
        let file = match self.write_into_project(index) {
            Ok(Some(file)) => file,
            Ok(None) => return,
            Err(reason) => {
                out.push(notices(&tab, Some(failed(&reason))));
                return;
            }
        };
        let (Some(project), Some(own)) = (self.project.clone(), self.relative(&file)) else {
            return;
        };
        let frame = self.images[index].gallery.frame_mut();
        let (uid, name) = (frame.uid, frame.name.clone());
        let (width, height) = (frame.doc.image.width, frame.doc.image.height);
        let png = match files::encode_png(width, height, frame.doc.composite()) {
            Ok(png) => png,
            Err(reason) => {
                out.push(notices(&tab, Some(failed(&reason))));
                return;
            }
        };
        // Named by the image and its pixels, so publishing the same edit
        // again writes nothing new, and `work/` keeps only editable work.
        let fingerprint = protocol::fingerprint(&png);
        let item = ItemRef { path: own.clone(), item: Some(format!("publish {name}")) };
        let source = protocol::render_path(OWNER, &item, None, &fingerprint, "png");
        let written = project.join(&source);
        if !written.exists()
            && let Err(reason) = files::write_atomic(&written, &png)
        {
            out.push(notices(&tab, Some(failed(&reason))));
            return;
        }
        let mut from = vec![own];
        // Every link is in the project by now: one outside it was refused.
        for link in links {
            let link = if Path::new(&link).is_absolute() { self.relative(Path::new(&link)) } else { Some(link) };
            if let Some(link) = link
                && !from.contains(&link)
            {
                from.push(link);
            }
        }
        self.images[index].publishing = Some(uid);
        out.push(AppMessage::Publish { tab, artifact, source, from, with: Vec::new() });
    }

    /// The refusal's Ask the assistant: the request to put the first take
    /// that is not canon into the chain, as Ask the assistant to use this
    /// take words it, with every such take named.
    pub(super) fn ask_about_inputs(&mut self, out: &mut Vec<AppMessage>) {
        let Some(image) = self.active_image() else {
            return;
        };
        let Some(first) = image.refused.iter().find(|path| image.rounds.tile_of(path).is_some()) else {
            return;
        };
        let Some(tile) = image.rounds.tile_of(first).map(|tile| &image.rounds.tiles[tile]) else {
            return;
        };
        let text = format!("Put take {} of round {}, {first}, into the chain.", tile.take + 1, tile.number);
        let items = image.refused.iter().map(|path| ItemRef { path: path.clone(), item: None }).collect();
        out.push(AppMessage::AskAssistant { text, items });
    }

    /// `published`: the host's runner published the edit, as that version
    /// when it says: the image published is marked with the seal as canon,
    /// and a notice says so. An empty file is nothing published, after the
    /// person's Cancel or with the host's refusal card, which says why
    /// itself: the workspace only stops waiting.
    pub(super) fn published(&mut self, tab: &str, file: String, version: Option<u32>, out: &mut Vec<AppMessage>) {
        let Some(index) = self.image_index(tab) else {
            return;
        };
        let image = &mut self.images[index];
        let uid = image.publishing.take();
        if file.is_empty() {
            return;
        }
        if let Some(uid) = uid {
            image.published.insert(uid, file.clone());
        }
        let notice = Notice {
            id: String::from("publish"),
            text: match version {
                Some(version) => format!("Published {file}, version {version}."),
                None => format!("Published {file}."),
            },
            detail: String::new(),
            progress: None,
            state: NoticeState::Done,
            actions: Vec::new(),
            close: Some(Action::new("notice_close")),
        };
        out.push(notices(tab, Some(notice)));
        out.extend(self.declared());
    }

    /// The canon seal: asks the host for the artifact's versions.
    pub(super) fn canon_history(&mut self, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some((artifact, _)) = self.publishing_as(index) else {
            return;
        };
        out.push(AppMessage::CanonHistory { tab: self.images[index].id.clone(), artifact });
    }

    /// Revert to this: asks the host to make that version of the artifact
    /// canon again.
    pub(super) fn revert(&mut self, file: &str, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some((artifact, _)) = self.publishing_as(index) else {
            return;
        };
        out.push(AppMessage::Revert { tab: self.images[index].id.clone(), artifact, file: file.to_string() });
    }
}

/// A tab's notice cards, in the working area's bottom left as
/// ig-layer-export shows them: this one, or none.
pub(super) fn notices(tab: &str, notice: Option<Notice>) -> AppMessage {
    AppMessage::Notices { tab: tab.to_string(), notices: notice.into_iter().collect(), corner: NoticeCorner::BottomLeft }
}

fn failed(reason: &str) -> Notice {
    Notice {
        id: String::from("publish"),
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

/// Whether an image is an edit, its pixels other than every linked take's
/// file: it has a linked layer, and a linked layer painted on, a mask, or
/// any layer of its own besides.
pub fn edited(doc: &Doc) -> bool {
    let nodes = doc.image.every_node();
    nodes.iter().any(|node| node.link.is_some())
        && nodes.iter().any(|node| node.link.as_ref().is_some_and(|linked| linked.edited) || node.mask.is_some() || (node.link.is_none() && !node.is_group()))
}
