//! The window's side of publishing from a workspace, in a Skills project:
//! what a workspace asks to publish, revert, or see the history of waits in
//! `Publishing::asked` for the runner, which fills the publish card or the
//! history card. The window draws them, keeps what the person chose in
//! `Publishing::confirmed` for the runner to carry out, and sends the
//! runner's answer back to whoever asked: a workspace, or the assistant's
//! `publish_take`, through the same card. Outside a Skills project a
//! workspace's publish is refused, since there is no canon to publish into.

use super::HostWindow;
use crate::history_card::History;
use crate::publish_card::{PublishCardEvent, PublishCardView, RefusedEvent};
use eframe::egui;
use protocol::{HostMessage, ToolResult};
use serde_json::json;
use std::collections::VecDeque;

/// Who asked to publish, and so hears the answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asker {
    /// A workspace's tab, by the workspace's index.
    Workspace { app: usize, tab: String },
    /// The assistant's `publish_take`, by its call.
    Assistant { call: String },
}

/// The answer one asker hears.
#[derive(Debug, PartialEq)]
pub(crate) enum Reply {
    Workspace(usize, HostMessage),
    Assistant(ToolResult),
}

/// The answer to a publish or revert: the file published and its version,
/// or why not. `declined` is the user's Cancel, which is no failure.
pub(crate) fn published_reply(asker: Asker, artifact: String, file: String, version: Option<u32>, error: Option<String>, declined: bool) -> Reply {
    match asker {
        Asker::Workspace { app, tab } => Reply::Workspace(app, HostMessage::Published { tab, artifact, file, version, error }),
        Asker::Assistant { call } => {
            let output = match (&error, declined) {
                (_, true) => json!({ "error": "The user did not approve it, so nothing was published.", "declined": true }),
                (Some(error), false) => json!({ "error": error, "artifact": artifact }),
                (None, false) => json!({ "artifact": artifact, "file": file, "version": version }),
            };
            Reply::Assistant(ToolResult { call, ok: error.is_none() && !declined, output, images: Vec::new() })
        }
    }
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

impl HostWindow {
    /// Takes what workspaces asked to publish, revert, or see.
    pub(super) fn take_publishing(&mut self) {
        let mut asked = Vec::new();
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
        }
        if asked.is_empty() {
            return;
        }
        if self.skills_project().is_some() {
            self.publishing.asked.extend(asked);
            return;
        }
        for asked in asked {
            match asked {
                Asked::Publish { asker, artifact, source: file, .. } | Asked::Revert { asker, artifact, file } => {
                    let error = Some(String::from("Only a Skills project has canon to publish into."));
                    self.reply(published_reply(asker, artifact, file, None, error, false));
                }
                Asked::History { app, tab, artifact } => self.send(app, HostMessage::History { tab, artifact, versions: Vec::new() }),
            }
        }
    }

    /// Sends an answer to whoever asked.
    pub(crate) fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Workspace(app, message) => self.send(app, message),
            Reply::Assistant(answer) => self.assistant.send(assistant::Command::Result(answer)),
        }
    }

    /// The runner's answer to a confirmed card: the workspace hears it and
    /// says the publish with its version, and a refusal shows with "Ask the
    /// assistant".
    /// The assistant hears it on its call instead, and needs no refusal card.
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

    /// Catches a Cancel heard as a publish, or heard by the wrong asker: a
    /// workspace hears `published` with no file and no error, and the
    /// assistant hears on its own call that nothing was published.
    #[test]
    fn cancel_answers_whoever_asked() {
        let workspace = published_reply(Asker::Workspace { app: 2, tab: String::from("1") }, String::from("the diner: layout"), String::new(), None, None, true);
        assert_eq!(
            workspace,
            Reply::Workspace(2, HostMessage::Published { tab: String::from("1"), artifact: String::from("the diner: layout"), file: String::new(), version: None, error: None })
        );
        let Reply::Assistant(answer) = published_reply(Asker::Assistant { call: String::from("c7") }, String::from("the diner: layout"), String::new(), None, None, true) else {
            panic!("the assistant's Cancel went to a workspace");
        };
        assert_eq!(answer.call, "c7");
        assert!(!answer.ok && answer.output["declined"] == true);
        let Reply::Assistant(published) = published_reply(Asker::Assistant { call: String::from("c8") }, String::from("the diner: layout"), String::from("library/x.png"), Some(3), None, false) else {
            panic!("the assistant's publish went to a workspace");
        };
        assert!(published.ok && published.output["version"] == 3);
    }
}
