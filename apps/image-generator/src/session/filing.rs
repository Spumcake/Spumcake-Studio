//! The marked image a next round sends with a take's notes, filed before
//! any model is sent it, as the contract's "References from a workspace"
//! says: a PNG written beside the tab's file in `work/`, named after the
//! image and " notes", free among the files there, then filed with
//! `file_resource`, from the tab's file and the take; `filed` names the
//! file as filed, which joins the next round's draft as a path. Nothing a
//! model is sent lives only in `work/`.

use super::*;
use protocol::{CellPart, NoticeState};

/// A file being filed for a draft: the marked image, with the role "Notes
/// on the last round", added to the draft that `made` was, as part of the
/// step that made it.
#[derive(Debug, Clone)]
pub(super) struct Filing {
    id: String,
    tab: String,
    cell: String,
    made: Vec<CellPart>,
}

impl Session {
    /// Writes `png` beside the tab's file in `work/`, named `stem`, or the
    /// first free name after it, and asks the runner to file it, from the
    /// tab's file and then `made_from`, for the draft `cell` that `made`
    /// was. The tab's file is written first, so it has a file to be from.
    pub(super) fn file_for_draft(
        &mut self,
        index: usize,
        stem: &str,
        png: &[u8],
        made_from: &str,
        cell: &str,
        made: Vec<CellPart>,
        out: &mut Vec<AppMessage>,
    ) -> Result<(), String> {
        if self.project.is_none() {
            return Err("No project is open.".into());
        }
        if self.images[index].path.is_none() {
            self.images[index].dirty = true;
        }
        let file = self.write_into_project(index)?.ok_or_else(|| String::from("The image could not be written."))?;
        let own = self.relative(&file).ok_or_else(|| String::from("The image is not in the project."))?;
        let Some(folder) = self.work_folder() else {
            return Err("No project is open.".into());
        };
        let stem = protocol::free_name(&safe_name(stem), |name| folder.join(format!("{name}.png")).exists());
        let written = folder.join(format!("{stem}.png"));
        files::write_atomic(&written, png)?;
        let path = self.relative(&written).ok_or_else(|| String::from("The image is not in the project."))?;
        let id = format!("filing-{}", self.next_file);
        self.next_file += 1;
        let tab = self.images[index].id.clone();
        let mut from = vec![own];
        if !made_from.is_empty() && !from.iter().any(|own| own == made_from) {
            from.push(made_from.to_string());
        }
        out.push(AppMessage::FileResource { tab: tab.clone(), path, from, id: Some(id.clone()) });
        self.filings.push(Filing { id, tab, cell: cell.to_string(), made });
        Ok(())
    }

    /// The runner's `filed` for the marked image: the filed file joins the
    /// draft as a path with the role "Notes on the last round", or why it
    /// could not be filed shows as the failed notice, at the working area's
    /// bottom left.
    pub(super) fn filed_for_draft(&mut self, id: &str, file: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        let Some(at) = self.filings.iter().position(|filing| filing.id == id) else {
            return;
        };
        let filing = self.filings.remove(at);
        if let Some(error) = error.filter(|error| !error.is_empty()) {
            let notice = protocol::Notice {
                id: String::from("filed"),
                text: error,
                detail: String::new(),
                progress: None,
                state: NoticeState::Failed,
                actions: Vec::new(),
                close: Some(protocol::Action::new("notice_close")),
            };
            out.push(AppMessage::Notices { tab: filing.tab, notices: vec![notice], corner: protocol::NoticeCorner::BottomLeft });
            return;
        }
        let Some(index) = self.images.iter().position(|image| image.id == filing.tab) else {
            return;
        };
        let part = CellPart::Path { path: file.to_string(), media_type: String::from("image/png"), role: Some(NOTES_WORDS.into()), field: None };
        let image = &mut self.images[index];
        if image.gallery.amend_draft(&filing.cell, &filing.made, part) {
            image.dirty = true;
        }
        out.extend(self.emit());
    }
}
