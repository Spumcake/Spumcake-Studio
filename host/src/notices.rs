//! A tab's notice cards, at the bottom right of its contents: text, a
//! detail line, progress, done or failed, actions, and a close mark, drawn
//! from what the workspace declared with `notices`.

use eframe::egui::{self, Align2, Color32, Rect, Sense, pos2, vec2};
use protocol::{Action, Notice, NoticeState};

use crate::widgets::{self, Scale};

const S: Scale = Scale::PANEL;
const WIDTH: f32 = 320.0;
const MARGIN: f32 = 16.0;
const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
const DONE: Color32 = Color32::from_rgb(22, 163, 74);
const FAILED: Color32 = Color32::from_rgb(220, 38, 38);
const BAR: Color32 = Color32::from_rgb(37, 99, 235);

/// Draws `notices` stacked up from the bottom right of `area`, disabled
/// while `locked`. Returns the actions chosen.
pub(crate) fn show(ui: &mut egui::Ui, notices: &[Notice], area: Rect, locked: bool) -> Vec<Action> {
    let mut chosen = Vec::new();
    let mut bottom = area.bottom() - MARGIN;
    for notice in notices.iter().rev() {
        let height = card_height(notice);
        let rect = Rect::from_min_size(pos2(area.right() - MARGIN - WIDTH, bottom - height), vec2(WIDTH, height));
        if rect.top() < area.top() {
            break;
        }
        card(ui, notice, rect, locked, &mut chosen);
        bottom = rect.top() - GAP;
    }
    chosen
}

fn card_height(notice: &Notice) -> f32 {
    let mut height = PAD * 2.0 + S.of(20.0);
    if !notice.detail.is_empty() {
        height += S.of(18.0);
    }
    if notice.progress.is_some() {
        height += GAP + 4.0;
    }
    if !notice.actions.is_empty() {
        height += GAP + S.of(28.0);
    }
    height
}

fn card(ui: &mut egui::Ui, notice: &Notice, rect: Rect, locked: bool, chosen: &mut Vec<Action>) {
    let painter = ui.painter().clone();
    widgets::soft_shadow(&painter, rect, 10.0);
    let edge = match notice.state {
        NoticeState::Failed => FAILED,
        _ => widgets::BORDER,
    };
    widgets::outlined(&painter, rect, 10.0, Color32::WHITE, edge);
    let inner = rect.shrink(PAD);
    let mut x = inner.left();
    match notice.state {
        NoticeState::Done => {
            painter.text(pos2(x, inner.top() + S.of(10.0)), Align2::LEFT_CENTER, "✓", widgets::body(S), DONE);
            x += S.of(20.0);
        }
        NoticeState::Failed => {
            painter.text(pos2(x, inner.top() + S.of(10.0)), Align2::LEFT_CENTER, "!", widgets::body(S), FAILED);
            x += S.of(16.0);
        }
        NoticeState::Working => {}
    }
    painter.text(pos2(x, inner.top() + S.of(10.0)), Align2::LEFT_CENTER, &notice.text, widgets::body(S), widgets::TEXT);
    let mut y = inner.top() + S.of(20.0);
    if !notice.detail.is_empty() {
        painter.text(pos2(inner.left(), y + S.of(9.0)), Align2::LEFT_CENTER, &notice.detail, widgets::body(S), widgets::MUTED);
        y += S.of(18.0);
    }
    if let Some(progress) = notice.progress {
        y += GAP;
        let track = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 4.0));
        painter.rect_filled(track, 2.0, widgets::RAIL);
        let done = Rect::from_min_size(track.min, vec2(track.width() * progress.clamp(0.0, 1.0), 4.0));
        painter.rect_filled(done, 2.0, BAR);
        y += 4.0;
        ui.ctx().request_repaint();
    }
    if let Some(close) = &notice.close {
        let mark = Rect::from_center_size(pos2(inner.right() - 6.0, inner.top() + S.of(10.0)), vec2(16.0, 16.0));
        let response = ui.interact(mark, ui.id().with(("notice-close", &notice.id)), Sense::click());
        let color = if response.hovered() { widgets::TEXT } else { widgets::MUTED };
        painter.text(mark.center(), Align2::CENTER_CENTER, "×", widgets::body(S), color);
        if response.clicked() && !locked {
            chosen.push(close.clone());
        }
    }
    if !notice.actions.is_empty() {
        y += GAP;
        let mut right = inner.right();
        for button in notice.actions.iter().rev() {
            let width = S.of(16.0) + 7.5 * button.label.chars().count() as f32;
            let spot = Rect::from_min_size(pos2(right - width, y), vec2(width, S.of(28.0)));
            let response = ui.interact(spot, ui.id().with(("notice-action", &notice.id, &button.label)), Sense::click());
            let fill = if response.hovered() { widgets::SELECTED } else { Color32::WHITE };
            widgets::outlined(&painter, spot, 6.0, fill, widgets::BORDER);
            painter.text(spot.center(), Align2::CENTER_CENTER, &button.label, widgets::body(S), widgets::TEXT);
            if response.clicked() && !locked {
                chosen.push(button.action.clone());
            }
            right = spot.left() - GAP;
        }
    }
}
