//! Exporting the animatic, by hand or through a tool: the project's
//! animatic step runs again, after the timing is saved when it differs, as
//! `build-animatic` makes it, with a card saying it is under way, then that
//! it is done and where its take is, or why it failed.

use super::Session;
use crate::board::{ANIMATIC, CUT};
use protocol::{Action, Notice, NoticeAction, NoticeState};

/// The id of the export's card, which each export replaces.
const CARD: &str = "export";

impl Session {
    /// Starts making the active storyboard's animatic with the project's
    /// animatic step. Returns the step, or why it cannot start, which the
    /// card also says.
    pub fn start_export(&mut self) -> Result<String, String> {
        // The card under way already says so.
        if self.exporting_by_step() {
            return Err("An export is already under way.".to_string());
        }
        self.kept_notices.retain(|notice| notice.id != CARD);
        if let Some(reason) = self.export_refusal() {
            self.export_failed(&reason);
            return Err(reason);
        }
        self.export_by_step().inspect_err(|reason| self.export_failed(reason))
    }

    /// The items the animatic is made from, with their fingerprints now.
    pub(crate) fn export_sources(&mut self) -> Vec<(String, String)> {
        let mut sources = vec![(CUT.to_string(), self.cut_fingerprint())];
        for clip in &self.film().clips {
            sources.push((crate::board::frames_id(clip.id), crate::board::frames_fingerprint(self.active_storyboard(), clip)));
        }
        sources
    }

    /// Why the animatic cannot be exported now, if it cannot.
    fn export_refusal(&mut self) -> Option<String> {
        if self.project.is_none() {
            return Some("No project is open.".to_string());
        }
        if self.film().clips.is_empty() {
            return Some("The film has no clips to export.".to_string());
        }
        None
    }

    fn export_failed(&mut self, reason: &str) {
        self.keep_notice(Notice {
            id: CARD.to_string(),
            text: "Animatic video not exported".to_string(),
            detail: reason.to_string(),
            progress: None,
            state: NoticeState::Failed,
            actions: vec![NoticeAction { label: "Try again".to_string(), action: Action::new("export_animatic") }],
            close: Some(Action::with_arg("close_notice", serde_json::json!(CARD))),
        });
    }

    /// The animatic's step could not make the animatic.
    pub(crate) fn step_export_failed(&mut self, reason: &str) {
        self.export_failed(reason);
    }

    /// The animatic's step made `storyboard`'s animatic as `take`, a file
    /// in the project, which the publish card now offers: it is the
    /// animatic's file from now on, as an export's is.
    pub(crate) fn step_exported(&mut self, storyboard: draw::document::DrawingId, take: &str, sources: Vec<(String, String)>) {
        let bytes = self.project.as_ref().and_then(|project| std::fs::read(project.join(take)).ok()).unwrap_or_default();
        if let Some(record) = self.storyboards.iter_mut().find(|record| record.id() == storyboard) {
            record.set_exported(Some(crate::export::Exported { file: take.to_string(), bytes: protocol::fingerprint(&bytes), sources }));
        }
        self.keep_notice(Notice {
            id: CARD.to_string(),
            text: "Animatic video exported".to_string(),
            detail: take.to_string(),
            progress: None,
            state: NoticeState::Done,
            actions: Vec::new(),
            close: Some(Action::with_arg("close_notice", serde_json::json!(CARD))),
        });
    }

    /// The export's card while the step runs, or while the timing is saved
    /// before it.
    pub(crate) fn export_notice(&self) -> Option<Notice> {
        if !self.exporting_by_step() {
            return None;
        }
        Some(Notice {
            id: CARD.to_string(),
            text: "Exporting animatic video".to_string(),
            detail: String::new(),
            progress: None,
            state: NoticeState::Working,
            actions: Vec::new(),
            close: None,
        })
    }

    /// The animatic as a file for another workspace's cell: the step's take
    /// itself, where it is in the project.
    pub fn render_animatic(&self) -> Result<(String, String), String> {
        let project = self.project.as_deref().ok_or("No project is open.")?;
        let exported = self.active_storyboard().exported().ok_or("The animatic has not been exported yet.")?;
        crate::export::render(project, exported)
    }

    /// The animatic's item, with whether it is exporting, done, or failed.
    pub fn animatic_state(&self) -> serde_json::Value {
        let path = self.active_storyboard().path().and_then(|path| {
            let project = self.project.as_deref()?;
            Some(path.strip_prefix(project).ok()?.to_string_lossy().replace('\\', "/"))
        });
        let state = if self.exporting_by_step() {
            "exporting"
        } else if self.kept_notices.iter().any(|notice| notice.id == CARD && notice.state == NoticeState::Failed) {
            "failed"
        } else if self.active_storyboard().exported().is_some() {
            "done"
        } else {
            "not exported"
        };
        serde_json::json!({
            "item": path.map(|path| serde_json::json!({ "path": path, "item": ANIMATIC })),
            "file": self.active_storyboard().exported().map(|exported| exported.file.clone()),
            "state": state,
        })
    }
}
