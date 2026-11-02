//! The close confirmation, the replace question, recovery offers, and file
//! errors of `tabs.rs` in Simple Sketch 1.0.1, and the walk over unsaved
//! storyboards when the window closes. Each question the window drew as a
//! dialog is an `ask` message; the host draws it and sends the answer.

use crate::session::{CloseEffect, FileError, RecoveryOffer, SaveResult, Session};
use draw::document::DrawingId;
use protocol::{Ask, AskButton};
use std::path::PathBuf;

/// The one close confirmation, a replace question, and recovery offers.
/// Marks, cameras, and history stay in the session.
pub(crate) struct Chrome {
    confirm: Option<DrawingId>,
    /// Window close is walking dirty storyboards in tab order.
    quitting: bool,
    /// Dirty storyboards still to ask about, after the one in `confirm`.
    quit_rest: Vec<DrawingId>,
    /// Recovery files still to offer, one at a time.
    recoveries: Vec<RecoveryOffer>,
    replace: Option<ReplaceAsk>,
    /// A failed save or open being shown.
    error: Option<FileError>,
    /// The walk finished: the window may close.
    quit_ready: bool,
}

#[derive(Clone)]
struct ReplaceAsk {
    id: DrawingId,
    path: PathBuf,
    copy: bool,
    close_after: bool,
}

const KEEP: &str = "keep";
const REPLACE: &str = "replace";
const RECOVERY: &str = "recovery";
const ERROR: &str = "error";

impl Chrome {
    pub fn new() -> Self {
        Self {
            confirm: None,
            quitting: false,
            quit_rest: Vec::new(),
            recoveries: Vec::new(),
            replace: None,
            error: None,
            quit_ready: false,
        }
    }

    pub fn set_recoveries(&mut self, offers: Vec<RecoveryOffer>) {
        self.recoveries = offers;
    }

    /// The replace question is waiting for an answer.
    pub fn replacing(&self) -> bool {
        self.replace.is_some()
    }

    pub fn close(&mut self, session: &mut Session, id: DrawingId) -> CloseEffect {
        let effect = session.request_close(id);
        match effect {
            CloseEffect::NeedsConfirm => self.confirm = Some(id),
            CloseEffect::Unchanged => {}
            CloseEffect::Closed { .. } => {
                if self.confirm == Some(id) {
                    self.confirm = None;
                }
            }
        }
        effect
    }

    pub fn ask_replace(&mut self, id: DrawingId, path: PathBuf, copy: bool, close_after: bool) {
        self.replace = Some(ReplaceAsk {
            id,
            path,
            copy,
            close_after,
        });
    }

    /// Save as was cancelled, or the write failed. The storyboard stays open.
    /// A quit in progress stops.
    pub fn cancel_save_as(&mut self) {
        self.stop_confirm();
        self.replace = None;
    }

    /// The storyboard was saved. Close it, and continue a quit when one is walking
    /// the remaining dirty storyboards. Returns whether a new blank should be centered.
    pub fn complete_save(&mut self, session: &mut Session, id: DrawingId) -> bool {
        let effect = session.request_close(id);
        let replacement = matches!(effect, CloseEffect::Closed { replacement: true });
        if !matches!(effect, CloseEffect::Closed { .. }) {
            return false;
        }
        if self.quitting {
            self.advance_quit(session);
        } else {
            self.confirm = None;
        }
        replacement
    }

    /// The window's close was pressed: ask once per dirty storyboard, in tab
    /// order. With none, the window may close at once.
    pub fn note_quit(&mut self, session: &mut Session) {
        if self.quitting {
            return;
        }
        let _ = session.commit();
        let dirty = session.storyboards_to_keep();
        if dirty.is_empty() {
            self.quit_ready = true;
            return;
        }
        self.start_quit(session, dirty);
    }

    pub fn take_quit_ready(&mut self) -> bool {
        std::mem::take(&mut self.quit_ready)
    }

    fn start_quit(&mut self, session: &mut Session, mut dirty: Vec<DrawingId>) {
        let Some(first) = dirty.first().copied() else {
            return;
        };
        dirty.remove(0);
        self.quitting = true;
        self.quit_rest = dirty;
        self.confirm = Some(first);
        session.activate(first);
    }

    fn advance_quit(&mut self, session: &mut Session) {
        while let Some(next) = self.quit_rest.first().copied() {
            self.quit_rest.remove(0);
            if session.name_of(next).is_some() {
                self.confirm = Some(next);
                session.activate(next);
                return;
            }
        }
        self.quitting = false;
        self.quit_rest.clear();
        self.confirm = None;
        self.quit_ready = true;
    }

    fn stop_confirm(&mut self) {
        self.confirm = None;
        self.quitting = false;
        self.quit_rest.clear();
    }

    /// The question to show now: a file error over everything, then the
    /// replace question, the close confirmation, and a recovery offer.
    pub fn current(&mut self, session: &mut Session) -> Option<Ask> {
        if self.error.is_none() {
            self.error = session.take_file_error();
        }
        if let Some(error) = &self.error {
            let title = if error.saving {
                format!("Could not save “{}”", error.file)
            } else {
                format!("Could not open “{}”", error.file)
            };
            let mut lines = vec![error.reason.clone()];
            if error.saving {
                lines.push("Your changes are still open.".to_string());
            }
            return Some(Ask {
                id: ERROR.to_string(),
                title,
                lines,
                buttons: vec![button("ok", "OK")],
                dismiss: Some("ok".to_string()),
            });
        }
        if let Some(ask) = &self.replace {
            return Some(Ask {
                id: REPLACE.to_string(),
                title: "Replace this file?".to_string(),
                lines: vec![crate::store::file_title(&ask.path)],
                buttons: vec![button("replace", "Replace"), button("cancel", "Cancel")],
                dismiss: Some("cancel".to_string()),
            });
        }
        while let Some(id) = self.confirm {
            let Some(name) = session.name_of(id).map(str::to_string) else {
                if self.quitting {
                    self.advance_quit(session);
                } else {
                    self.confirm = None;
                }
                continue;
            };
            return Some(Ask {
                id: format!("{KEEP}-{}", id.get()),
                title: "Keep this storyboard?".to_string(),
                lines: vec![name],
                buttons: vec![
                    button("save", "Save"),
                    button("discard", "Discard"),
                    button("cancel", "Cancel"),
                ],
                dismiss: Some("cancel".to_string()),
            });
        }
        let offer = self.recoveries.first()?;
        Some(Ask {
            id: format!("{RECOVERY}-{}", offer.id.get()),
            title: "Recovery".to_string(),
            lines: vec![offer.name.clone()],
            buttons: vec![button("restore", "Restore"), button("discard", "Discard")],
            dismiss: None,
        })
    }

    /// The person chose `button` on the question `id`. An answer to a
    /// question no longer shown is ignored. Returns whether a replacement
    /// blank should be centered.
    pub fn answer(&mut self, session: &mut Session, id: &str, button: &str) -> bool {
        if id == ERROR {
            self.error = None;
            return false;
        }
        if id == REPLACE {
            return self.answer_replace(session, button);
        }
        if let Some(storyboard) = id.strip_prefix(&format!("{KEEP}-")) {
            let Some(storyboard) = self
                .confirm
                .filter(|confirm| confirm.get().to_string() == storyboard)
            else {
                return false;
            };
            return self.answer_confirm(session, storyboard, button);
        }
        if let Some(file) = id.strip_prefix(&format!("{RECOVERY}-")) {
            let Some(offer) = self
                .recoveries
                .first()
                .filter(|offer| offer.id.get().to_string() == file)
                .cloned()
            else {
                return false;
            };
            match button {
                "restore" => {
                    session.restore_recovery(offer.id);
                    self.recoveries.remove(0);
                }
                "discard" => {
                    session.discard_recovery_file(offer.id);
                    self.recoveries.remove(0);
                }
                _ => {}
            }
        }
        false
    }

    fn answer_confirm(&mut self, session: &mut Session, id: DrawingId, button: &str) -> bool {
        match button {
            "save" => {
                if let Some(path) = session.file_path(id) {
                    match session.save_path(id, path, false, true) {
                        SaveResult::Wrote => return self.complete_save(session, id),
                        SaveResult::NeedsReplace(path) => {
                            self.ask_replace(id, path, false, true);
                            return false;
                        }
                        SaveResult::Failed => return false,
                    }
                }
                session.request_save_dialog(id, true);
                false
            }
            "discard" => {
                let effect = session.discard(id);
                let replacement = matches!(effect, CloseEffect::Closed { replacement: true });
                if !matches!(effect, CloseEffect::Closed { .. }) {
                    return false;
                }
                if self.quitting {
                    self.advance_quit(session);
                } else {
                    self.confirm = None;
                }
                replacement
            }
            _ => {
                self.stop_confirm();
                false
            }
        }
    }

    fn answer_replace(&mut self, session: &mut Session, button: &str) -> bool {
        let Some(ask) = self.replace.clone() else {
            return false;
        };
        if button == "replace" {
            self.replace = None;
            match session.save_path(ask.id, ask.path, ask.copy, true) {
                SaveResult::Wrote if ask.close_after => {
                    return self.complete_save(session, ask.id);
                }
                SaveResult::Wrote => return false,
                SaveResult::Failed | SaveResult::NeedsReplace(_) => {
                    if ask.close_after {
                        self.cancel_save_as();
                    }
                    return false;
                }
            }
        }
        self.replace = None;
        if ask.close_after {
            self.cancel_save_as();
        }
        false
    }
}

fn button(id: &str, label: &str) -> AskButton {
    AskButton {
        id: id.to_string(),
        label: label.to_string(),
    }
}
