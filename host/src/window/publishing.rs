//! The window's side of publishing from a workspace, in a Skills project:
//! what a workspace asks to publish, revert, or see the history of waits in
//! `Publishing::asked` for the runner, which fills the publish card or the
//! history card. The window draws them, keeps what the person chose in
//! `Publishing::confirmed` for the runner to carry out, and sends the
//! runner's answer back to the workspace. Outside a Skills project a
//! publish is refused, since there is no canon to publish into.

use super::HostWindow;
use crate::history_card::History;
use crate::publish_card::{PublishCardEvent, PublishCardView, RefusedEvent};
use eframe::egui;
use protocol::HostMessage;
use std::collections::VecDeque;

/// What a workspace asked for, with the workspace's index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asked {
    /// Publish a workspace file as a canon artifact: what it came from, as
    /// `resources` has it, and the artifacts published with it.
    Publish { app: usize, tab: String, artifact: String, source: String, from: Vec<String>, with: Vec<String> },
    /// Publish an earlier version of an artifact again.
    Revert { app: usize, tab: String, artifact: String, file: String },
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
                asked.push(Asked::Publish { app, tab, artifact, source, from, with });
            }
            for (tab, artifact, file) in declared.reverts.drain(..) {
                asked.push(Asked::Revert { app, tab, artifact, file });
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
                Asked::Publish { app, tab, artifact, source: file, .. } | Asked::Revert { app, tab, artifact, file } => {
                    let error = Some(String::from("Only a Skills project has canon to publish into."));
                    self.send(app, HostMessage::Published { tab, artifact, file, version: None, error });
                }
                Asked::History { app, tab, artifact } => self.send(app, HostMessage::History { tab, artifact, versions: Vec::new() }),
            }
        }
    }

    /// The runner's answer to a confirmed card: the workspace hears it and
    /// says the publish with its version, and a refusal shows with "Ask the
    /// assistant".
    // Called by the runner's publish, which comes with Chat Assistant Work's
    // publishing slice.
    #[allow(dead_code)]
    pub(crate) fn answer_published(&mut self, app: usize, tab: String, artifact: String, file: String, version: Option<u32>, error: Option<String>) {
        if let Some(sentence) = &error {
            self.publishing.refused = Some(Refusal { app, tab: tab.clone(), artifact: artifact.clone(), file: file.clone(), sentence: sentence.clone() });
        }
        self.send(app, HostMessage::Published { tab, artifact, file, version, error });
    }

    /// The runner's history for a workspace's `canon_history`: the
    /// workspace hears it, and the card opens under what was pressed.
    // Called by the runner's publish, which comes with Chat Assistant Work's
    // publishing slice.
    #[allow(dead_code)]
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
                    if let Asked::Publish { app, tab, artifact, .. } | Asked::Revert { app, tab, artifact, .. } = card.asked {
                        self.send(app, HostMessage::Published { tab, artifact, file: String::new(), version: None, error: None });
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
