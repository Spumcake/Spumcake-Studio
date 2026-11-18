//! The window's side of publishing from a workspace:
//! what a workspace asks to publish, revert, or see the history of waits in
//! `Publishing::asked` for the runner, which fills the publish card or the
//! history card. The window draws them, keeps what the person chose in
//! `Publishing::confirmed` for the runner to carry out, and sends the
//! runner's answer back to the workspace that asked. With no project open, or no
//! Spumcake-Skills found, a workspace's publish is refused with why.

use super::HostWindow;
use crate::history_card::History;
use crate::publish_card::{PublishCardEvent, PublishCardView, RefusedEvent};
use eframe::egui;
use protocol::HostMessage;
use std::collections::VecDeque;

/// Who asked to publish, and so hears the answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asker {
    /// A workspace's tab, by the workspace's index.
    Workspace { app: usize, tab: String },
}

/// The answer one asker hears.
#[derive(Debug, PartialEq)]
pub(crate) enum Reply {
    Workspace(usize, HostMessage),
}

/// The answer to a publish or revert: the file published and its version,
/// or why not. `declined` is the user's Cancel, which is no failure.
pub(crate) fn published_reply(asker: Asker, artifact: String, file: String, version: Option<u32>, error: Option<String>, _declined: bool) -> Reply {
    let Asker::Workspace { app, tab } = asker;
    Reply::Workspace(app, HostMessage::Published { tab, artifact, file, version, error })
}

/// A film a workspace asked to deliver: published as `artifact` and
/// copied into `output/`, on the user's yes on the Deliver card, which
/// shows `uses`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Delivery {
    pub app: usize,
    pub tab: String,
    pub artifact: String,
    pub file: String,
    pub uses: String,
    /// The Deliver card's lines of the film, as the workspace read it.
    pub size: String,
    pub frame_rate: String,
    pub format: String,
}

/// What was asked for, with who asked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asked {
    /// Publish a file as a canon artifact: a workspace's file with what it
    /// came from, as `resources` has it, or a take; and the artifacts
    /// published with it.
    /// `sources` are the canon artifacts it was made from, with their roles,
    /// as the asker names them, beside what its step and its record say.
    Publish { asker: Asker, artifact: String, source: String, from: Vec<String>, with: Vec<String>, sources: Vec<(String, String)> },
    /// Publish an earlier version of an artifact again.
    Revert { asker: Asker, artifact: String, file: String },
    /// Every version of an artifact, for the history card.
    History { app: usize, tab: String, artifact: String },
}

/// A publish or revert card, and what it answers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    pub asked: Asked,
    pub view: PublishCardView,
}

/// A refusal shown: whom it answers, the file and artifact, and why.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Refusal {
    pub app: usize,
    pub tab: String,
    pub artifact: String,
    pub file: String,
    pub sentence: String,
}

/// The history card shown, under the toolbar item that asked for it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub app: usize,
    pub tab: String,
    pub anchor: Option<String>,
    pub history: History,
}

#[derive(Default)]
pub(crate) struct Publishing {
    /// What workspaces asked for, oldest first, for the runner to take.
    pub asked: VecDeque<Asked>,
    /// Films workspaces asked to deliver, oldest first, for the runner to
    /// take. The Deliver card is drawn from what it reads.
    pub delivering: VecDeque<Delivery>,
    /// The publish or revert card the runner filled.
    pub card: Option<Card>,
    /// Cards the person confirmed, with whether Commit was on, for the
    /// runner to carry out.
    pub confirmed: VecDeque<(Card, bool)>,
    pub refused: Option<Refusal>,
    pub history: Option<Shown>,
    /// The toolbar action pressed last, by id, so a history card it led to
    /// sits under it.
    pub last_tool: Option<String>,
}

/// The file `pipeline.py deliver` writes, as it names it: the minute, the
/// pipeline's number, and the project folder's name, with the film's own
/// extension, in `output/`.
pub(crate) fn output_name(stamp: &str, pipeline: &str, project: &str, film: &str) -> String {
    let extension = std::path::Path::new(film).extension().map(|ext| format!(".{}", ext.to_string_lossy())).unwrap_or_default();
    let number = pipeline.parse::<u32>().unwrap_or(0);
    format!("output/{stamp}-{number:03}-{project}{extension}")
}

impl HostWindow {
    /// Takes what workspaces asked to publish, revert, or see.
    pub(super) fn take_publishing(&mut self) {
        let mut asked = Vec::new();
        let mut deliveries = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            let declared = &mut started.running.declared;
            for (tab, artifact, source, from, with) in declared.publishes.drain(..) {
                asked.push(Asked::Publish { asker: Asker::Workspace { app, tab }, artifact, source, from, with, sources: Vec::new() });
            }
            for (tab, artifact, file) in declared.reverts.drain(..) {
                asked.push(Asked::Revert { asker: Asker::Workspace { app, tab }, artifact, file });
            }
            for (tab, artifact) in declared.canon_histories.drain(..) {
                asked.push(Asked::History { app, tab, artifact });
            }
            for (tab, artifact, file, uses, size, frame_rate, format) in declared.delivers.drain(..) {
                deliveries.push(Delivery { app, tab, artifact, file, uses, size, frame_rate, format });
            }
        }
        // A delivery waits for the runner, which fills the Deliver card,
        // once the project's scripts are found.
        if self.project_scripts().is_some() {
            self.publishing.delivering.extend(deliveries);
        } else {
            for delivery in deliveries {
                let error = Some(String::from(self.scripts_missing()));
                self.send(delivery.app, HostMessage::Delivered { tab: delivery.tab, file: String::new(), error });
            }
        }
        if asked.is_empty() {
            return;
        }
        if self.project_scripts().is_some() {
            self.publishing.asked.extend(asked);
            return;
        }
        for asked in asked {
            match asked {
                Asked::Publish { asker, artifact, source: file, .. } | Asked::Revert { asker, artifact, file } => {
                    let error = Some(String::from(self.scripts_missing()));
                    self.answer_published(asker, artifact, file, None, error);
                }
                Asked::History { app, tab, artifact } => self.send(app, HostMessage::History { tab, artifact, versions: Vec::new() }),
            }
        }
    }

    /// Sends an answer to whoever asked.
    pub(crate) fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Workspace(app, message) => self.send(app, message),
        }
    }

    /// The runner's answer to a confirmed card: the workspace hears it and
    /// says the publish with its version, and a refusal shows with "Ask the
    /// assistant".
    pub(crate) fn answer_published(&mut self, asker: Asker, artifact: String, file: String, version: Option<u32>, error: Option<String>) {
        if let (Some(sentence), Asker::Workspace { app, tab }) = (&error, &asker) {
            self.publishing.refused = Some(Refusal { app: *app, tab: tab.clone(), artifact: artifact.clone(), file: file.clone(), sentence: sentence.clone() });
        }
        self.reply(published_reply(asker, artifact, file, version, error, false));
    }

    /// The runner's history for a workspace's `canon_history`: the
    /// workspace hears it, and the card opens under what was pressed.
    pub(crate) fn answer_history(&mut self, app: usize, tab: String, artifact: String, versions: Vec<protocol::CanonVersion>) {
        let history = History {
            artifact: artifact.clone(),
            versions: versions
                .iter()
                .map(|version| crate::history_card::HistoryVersion {
                    file: version.file.clone(),
                    published: version.published,
                    note: version.note.clone(),
                    canon: version.canon,
                })
                .collect(),
            root: self.project.as_ref().map(|project| project.folder.clone()),
        };
        let anchor = self.publishing.last_tool.clone();
        self.publishing.history = Some(Shown { app, tab: tab.clone(), anchor, history });
        self.send(app, HostMessage::History { tab, artifact, versions });
    }

    /// Draws the publish card, a refusal, and the history card, and acts on
    /// what the person chose.
    pub(super) fn show_publishing(&mut self, ctx: &egui::Context) {
        // The Deliver card, over a delivery the runner checked: its yes
        // publishes and delivers, its Cancel answers with nothing.
        let folder = self.project.as_ref().map(|project| project.folder.clone());
        if let Some(delivery) = &self.publish_runs.deliver_card {
            let pipeline = folder.as_deref().and_then(crate::prompt_page::living_pipeline).unwrap_or_default();
            let project = folder.as_deref().and_then(|folder| folder.file_name()).map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
            let view = crate::deliver_card::DeliverCardView {
                file: output_name(&crate::skills_setup::local_stamp(), &pipeline, &project, &delivery.file),
                size: delivery.size.clone(),
                frame_rate: delivery.frame_rate.clone(),
                format: delivery.format.clone(),
                uses: delivery.uses.clone(),
            };
            if let Some(event) = crate::deliver_card::show(ctx, &view) {
                self.answer_delivery(event == crate::deliver_card::DeliverCardEvent::Deliver);
            }
        }
        if let Some(card) = self.publishing.card.clone()
            && let Some(event) = crate::publish_card::show(ctx, &card.view)
        {
            self.publishing.card = None;
            match event {
                PublishCardEvent::Confirm { commit } => self.publishing.confirmed.push_back((card, commit)),
                // Cancel publishes nothing, and the workspace stops waiting.
                PublishCardEvent::Cancel => {
                    if let Asked::Publish { asker, artifact, .. } | Asked::Revert { asker, artifact, .. } = card.asked {
                        self.reply(published_reply(asker, artifact, String::new(), None, None, true));
                    }
                }
            }
        }
        if let Some(refusal) = self.publishing.refused.clone()
            && let Some(event) = crate::publish_card::refused(ctx, &refusal.sentence)
        {
            self.publishing.refused = None;
            if event == RefusedEvent::AskAssistant {
                let text = format!("Publishing {} as {} was refused: {}", refusal.file, refusal.artifact, refusal.sentence);
                let project = self.project.as_ref().map(|project| project.folder.clone());
                let items = [protocol::ItemRef { path: refusal.file.clone(), item: None }];
                crate::chat::prepare(&mut self.chat, &text, &items, project.as_deref());
            }
        }
        if let Some(shown) = self.publishing.history.clone() {
            let anchor = shown
                .anchor
                .as_deref()
                .and_then(|id| crate::toolbar::item_rect(ctx, id))
                .unwrap_or_else(|| {
                    let screen = ctx.content_rect();
                    egui::Rect::from_min_size(egui::pos2(screen.right() - 48.0, screen.top() + super::WORKSPACE_CHROME - 8.0), egui::vec2(32.0, 8.0))
                });
            let actions = crate::history_card::show(ctx, anchor, &shown.history);
            if crate::history_card::take_closed(ctx) {
                self.publishing.history = None;
            }
            for action in actions {
                self.publishing.history = None;
                let viewport = self.viewport;
                self.send(shown.app, HostMessage::Command { id: action.id, tab: Some(shown.tab.clone()), arg: action.arg, node: None, viewport });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a Deliver card that names a file other than the one
    /// `pipeline.py deliver` writes.
    #[test]
    fn the_deliver_card_names_the_file_deliver_writes() {
        assert_eq!(output_name("2026-11-15_08-25", "002", "last-order", "library/2026-11-15_08-20-final-shot-mix.mp4"), "output/2026-11-15_08-25-002-last-order.mp4");
    }

    /// Catches a Cancel heard as a publish: the workspace hears
    /// `published` with no file and no error.
    #[test]
    fn cancel_answers_whoever_asked() {
        let workspace = published_reply(Asker::Workspace { app: 2, tab: String::from("1") }, String::from("the diner: layout"), String::new(), None, None, true);
        assert_eq!(
            workspace,
            Reply::Workspace(2, HostMessage::Published { tab: String::from("1"), artifact: String::from("the diner: layout"), file: String::new(), version: None, error: None })
        );
    }
}
