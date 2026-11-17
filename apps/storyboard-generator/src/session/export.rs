//! Exporting the animatic, by hand or through a tool: one export at a time,
//! on a thread of its own, with a card saying how far it is, then that it
//! is done and where its file is, or why it failed.

use super::Session;
use crate::board::{ANIMATIC, CUT};
use crate::export::Polled;
use protocol::{Action, Notice, NoticeAction, NoticeState};

/// The id of the export's card, which each export replaces.
const CARD: &str = "export";

impl Session {
    /// Starts exporting the active storyboard's animatic into the project's
    /// `exports/` folder. Returns its file, relative to the project, or why
    /// it cannot start, which the card also says.
    pub fn start_export(&mut self) -> Result<String, String> {
        // The card under way already says so.
        if self.export.is_some() {
            return Err("An export is already under way.".to_string());
        }
        let refused = self.export_refusal();
        if let Some(reason) = refused {
            self.export_failed(&reason);
            return Err(reason);
        }
        let project = self.project.clone().expect("checked above");
        let film = self.film().clone();
        // Named from the title, or from the storyboard's file when it has none.
        let file = crate::export::file_for(&crate::board::title_of(self.active_storyboard()));
        let mut sources = vec![(CUT.to_string(), self.cut_fingerprint())];
        for clip in &film.clips {
            sources.push((crate::board::frames_id(clip.id), crate::board::frames_fingerprint(self.active_storyboard(), clip)));
        }
        let tracks = self.mix_tracks();
        let plan = crate::export::plan(self.active_storyboard(), tracks, Some(project.as_path()));
        self.kept_notices.retain(|notice| notice.id != CARD);
        self.export = Some(crate::export::start(plan, &project, file.clone(), self.active_id(), sources));
        Ok(file)
    }

    /// Why the animatic cannot be exported now, if it cannot.
    fn export_refusal(&mut self) -> Option<String> {
        if self.project.is_none() {
            return Some("The storyboard is not in a project.".to_string());
        }
        // A Skills project's animatic is its build-animatic step's to make.
        if self.skills {
            return Some("In a Skills project, the animatic is made by running its step, which Studio cannot do yet.".to_string());
        }
        if self.film().clips.is_empty() {
            return Some("The film has no clips to export.".to_string());
        }
        let absent = self.absent_sounds();
        let file = absent.first()?;
        let name = file.rsplit('/').next().unwrap_or(file);
        Some(format!("{name} is missing from the sounds folder."))
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

    /// Cancels the export under way. Returns whether one was.
    pub fn cancel_export(&mut self) -> bool {
        match &self.export {
            Some(job) => {
                job.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether an export is under way, and how far: frames written of all.
    pub fn exporting(&self) -> Option<(u32, u32)> {
        self.export.as_ref().map(|job| (job.written(), job.total))
    }

    /// Ends the export when it finished. Returns whether anything changed.
    pub fn poll_export(&mut self) -> bool {
        let Some(job) = &self.export else {
            return false;
        };
        match job.poll() {
            Polled::Running => job.written() > 0,
            Polled::Failed(reason) => {
                let cancelled = job.is_cancelled();
                self.export = None;
                if !cancelled {
                    self.export_failed(&reason);
                }
                true
            }
            Polled::Done(exported) => {
                let storyboard = job.storyboard;
                let file = exported.file.clone();
                self.export = None;
                if let Some(record) = self.storyboards.iter_mut().find(|record| record.id() == storyboard) {
                    record.set_exported(Some(exported));
                }
                self.keep_notice(Notice {
                    id: CARD.to_string(),
                    text: "Animatic video exported".to_string(),
                    detail: file,
                    progress: None,
                    state: NoticeState::Done,
                    actions: Vec::new(),
                    close: Some(Action::with_arg("close_notice", serde_json::json!(CARD))),
                });
                true
            }
        }
    }

    /// The export's card while it runs.
    pub(crate) fn export_notice(&self) -> Option<Notice> {
        let job = self.export.as_ref()?;
        let written = job.written();
        Some(Notice {
            id: CARD.to_string(),
            text: "Exporting animatic video".to_string(),
            detail: format!("{written} of {} frames", job.total),
            progress: Some(written as f32 / job.total.max(1) as f32),
            state: NoticeState::Working,
            actions: vec![NoticeAction { label: "Cancel".to_string(), action: Action::new("cancel_export") }],
            close: None,
        })
    }

    /// The animatic as a file for another workspace's cell.
    pub fn render_animatic(&self) -> Result<(String, String), String> {
        let project = self.project.as_deref().ok_or("The storyboard is not in a project.")?;
        let exported = self.active_storyboard().exported().ok_or("The animatic has not been exported yet.")?;
        crate::export::render(project, exported)
    }

    /// The animatic's item, with whether it is exporting, done, or failed.
    pub fn animatic_state(&self) -> serde_json::Value {
        let path = self.active_storyboard().path().and_then(|path| {
            let project = self.project.as_deref()?;
            Some(path.strip_prefix(project).ok()?.to_string_lossy().replace('\\', "/"))
        });
        let state = if self.export.is_some() {
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
            "written": self.exporting().map(|(written, _)| written),
            "frames": self.exporting().map(|(_, total)| total),
        })
    }
}
