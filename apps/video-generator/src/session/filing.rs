//! The files a tab makes for a draft, filed before any model is sent them,
//! as the contract's "References from a workspace" says: the marked frame
//! a next round sends with a take's notes, and a Start frame taken from the
//! animatic. Each is a PNG written beside the tab's file in `work/`, named
//! after the tab and what it is, free among the files there, then filed
//! with `file_resource`, from the tab's file and what it was made from;
//! `filed` names the file as filed, which goes into the draft as a path.
//! Nothing a model is sent lives only in `work/`.

use super::*;
use protocol::{CellPart, NoticeState};

/// What a filed file becomes in its draft.
#[derive(Debug, Clone)]
pub(super) enum Becomes {
    /// The marked frame, with the role "Notes on the last round", added to
    /// the next round's draft that `made` was, as part of the step that
    /// made it.
    Notes { made: Vec<CellPart> },
    /// The draft's Start frame, in place of any: one step when it is filed.
    StartFrame,
}

/// A file being filed for a draft.
#[derive(Debug, Clone)]
pub(super) struct Filing {
    id: String,
    tab: String,
    cell: String,
    becomes: Becomes,
}

impl Session {
    /// Writes `png` beside the tab's file in `work/`, named `stem`, or the
    /// first free name after it, and asks the runner to file it, from the
    /// tab's file and then `made_from`, for the draft `cell`. The tab's file
    /// is written first, so it has a file to be from.
    pub(super) fn file_for_draft(
        &mut self,
        index: usize,
        stem: &str,
        png: &[u8],
        made_from: &str,
        cell: &str,
        becomes: Becomes,
        out: &mut Vec<AppMessage>,
    ) -> Result<(), String> {
        if self.project.is_none() {
            return Err("No project is open.".into());
        }
        if self.tabs[index].path.is_none() {
            self.tabs[index].dirty = true;
        }
        let file = self.write_into_project(index)?.ok_or_else(|| String::from("The video could not be written."))?;
        let own = self.relative(&file).ok_or_else(|| String::from("The video is not in the project."))?;
        let Some(folder) = self.work_folder() else {
            return Err("No project is open.".into());
        };
        let stem = protocol::free_name(&safe_name(stem), |name| folder.join(format!("{name}.png")).exists());
        let written = folder.join(format!("{stem}.png"));
        files::write_atomic(&written, png)?;
        let path = self.relative(&written).ok_or_else(|| String::from("The video is not in the project."))?;
        let id = format!("filing-{}", self.next_file);
        self.next_file += 1;
        let tab = self.tabs[index].id.clone();
        let mut from = vec![own];
        if !made_from.is_empty() && !from.iter().any(|own| own == made_from) {
            from.push(made_from.to_string());
        }
        out.push(AppMessage::FileResource { tab: tab.clone(), path, from, id: Some(id.clone()) });
        self.filings.push(Filing { id, tab, cell: cell.to_string(), becomes });
        Ok(())
    }

    /// The runner's `filed` for a draft's file: the filed file goes into the
    /// draft as a path, or why it could not be filed shows as the failed
    /// notice. Returns whether `id` was one of these.
    pub(super) fn filed_for_draft(&mut self, id: &str, file: &str, error: Option<String>, out: &mut Vec<AppMessage>) -> bool {
        let Some(at) = self.filings.iter().position(|filing| filing.id == id) else {
            return false;
        };
        let filing = self.filings.remove(at);
        if let Some(error) = error.filter(|error| !error.is_empty()) {
            out.push(super::deliver::notices(&filing.tab, Some((error, NoticeState::Failed))));
            return true;
        }
        let Some(index) = self.tab_index(&filing.tab) else {
            return true;
        };
        match filing.becomes {
            Becomes::Notes { made } => {
                let part = CellPart::Path { path: file.to_string(), media_type: String::from("image/png"), role: Some(NOTES_WORDS.into()), field: None };
                let tab = &mut self.tabs[index];
                if tab.gallery.amend_draft(&filing.cell, &made, part) {
                    tab.dirty = true;
                }
            }
            Becomes::StartFrame => self.put_start_frame(index, &filing.cell, file),
        }
        out.extend(self.emit());
        true
    }
}
