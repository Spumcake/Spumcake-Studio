//! A tab's timeline, docked under its contents and drawn from its
//! declaration, whatever the contents' kind: rows, their items or frame
//! cells, the ruler, the playhead, and ranges. The host previews each
//! gesture and sends one `timeline_gesture` when it ends; dragging the
//! playhead sends `seek` while it is held. It knows no workspace.
//!
//! The lifecycle contract gives this file to the role session that owns
//! the timeline's approved views, which draws them here. Until then the host shows
//! the timeline's area and its header, empty.

use eframe::egui::{self, Rect};
use protocol::{Action, Timeline, TimelineGesture};

use crate::widgets;

/// The timeline's height under the contents, before the contents are too
/// short to share it.
pub(crate) const HEIGHT: f32 = 240.0;

/// What the timeline sends. The timeline's drawing, still to come, makes them.
#[allow(dead_code)]
pub(crate) enum Out {
    Run(Action),
    Gesture(TimelineGesture),
    Seek { frame: u32, live: bool },
}

/// Draws `timeline` in `rect`, disabled while `locked`. Returns what was
/// chosen this frame.
pub(crate) fn show(ui: &mut egui::Ui, timeline: &Timeline, rect: Rect, locked: bool) -> Vec<Out> {
    let _ = (timeline, locked);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
    painter.hline(rect.x_range(), rect.top(), (1.0, widgets::BORDER));
    Vec::new()
}
