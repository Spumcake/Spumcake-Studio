//! A tab's notice cards, at the bottom right of its contents, or the bottom
//! left when the workspace asks: text, a detail line, progress, done or
//! failed, actions, and a close mark, drawn from what the workspace declared
//! with `notices`. In a raster tab they sit above its filmstrip, never over
//! it.

use eframe::egui::{self, Align2, Color32, Rect, Sense, pos2, vec2};
use protocol::{Action, Notice, NoticeCorner, NoticeState};

use crate::widgets::{self, Scale};

const S: Scale = Scale::PANEL;
const WIDTH: f32 = 320.0;
const MARGIN: f32 = 16.0;
const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
const DONE: Color32 = Color32::from_rgb(22, 163, 74);
const FAILED: Color32 = Color32::from_rgb(220, 38, 38);
const BAR: Color32 = Color32::from_rgb(37, 99, 235);

/// A card that fits its text, at the bottom left, as ig-layer-export
/// shows, is between these widths.
const FIT_MIN: f32 = 240.0;
const FIT_MAX: f32 = 480.0;

/// Draws `notices` stacked up from the bottom right of `area`, disabled
/// while `locked`. Returns the actions chosen.
pub(crate) fn show(ui: &mut egui::Ui, notices: &[Notice], area: Rect, locked: bool) -> Vec<Action> {
    show_in(ui, notices, NoticeCorner::BottomRight, area, locked)
}

/// Draws `notices` stacked up from `corner` of `area`, above a filmstrip
/// drawn in it this frame, disabled while `locked`. Returns the actions
/// chosen.
pub(crate) fn show_in(ui: &mut egui::Ui, notices: &[Notice], corner: NoticeCorner, area: Rect, locked: bool) -> Vec<Action> {
    let mut chosen = Vec::new();
    let area = above_strip(area, crate::film::drawn(ui.ctx()));
    let mut bottom = area.bottom() - MARGIN;
    for notice in notices.iter().rev() {
        let height = card_height(notice);
        let rect = match corner {
            NoticeCorner::BottomRight => Rect::from_min_size(pos2(area.right() - MARGIN - WIDTH, bottom - height), vec2(WIDTH, height)),
            NoticeCorner::BottomLeft => {
                let width = fitted_width(ui, notice).min(area.width() - 2.0 * MARGIN);
                Rect::from_min_size(pos2(area.left() + MARGIN, bottom - height), vec2(width, height))
            }
        };
        if rect.top() < area.top() {
            break;
        }
        card(ui, notice, rect, locked, &mut chosen);
        bottom = rect.top() - GAP;
    }
    chosen
}

/// `area` with a filmstrip drawn along its bottom left out, so notices sit
/// above the strip.
fn above_strip(area: Rect, strip: Option<Rect>) -> Rect {
    match strip {
        Some(strip) if strip.top() > area.top() && strip.top() < area.bottom() && strip.x_range().intersects(area.x_range()) => {
            Rect::from_min_max(area.min, pos2(area.right(), strip.top()))
        }
        _ => area,
    }
}

/// How wide a card is that fits its words: its text, detail, and buttons,
/// with its marks, between `FIT_MIN` and `FIT_MAX`.
fn fitted_width(ui: &egui::Ui, notice: &Notice) -> f32 {
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_string(), widgets::body(S), widgets::TEXT).size().x;
    let mark = match notice.state {
        NoticeState::Done => S.of(20.0),
        NoticeState::Failed => S.of(16.0),
        NoticeState::Working => 0.0,
    };
    let close = if notice.close.is_some() { 24.0 } else { 0.0 };
    let mut words = mark + measure(&notice.text) + close;
    if !notice.detail.is_empty() {
        words = words.max(measure(&notice.detail));
    }
    let buttons: f32 = notice.actions.iter().map(|button| action_width(&button.label) + GAP).sum();
    fit(words.max(buttons) + 2.0 * PAD)
}

/// A width held between `FIT_MIN` and `FIT_MAX`.
fn fit(width: f32) -> f32 {
    width.clamp(FIT_MIN, FIT_MAX)
}

/// An action button's width, from its label.
fn action_width(label: &str) -> f32 {
    S.of(16.0) + 7.5 * label.chars().count() as f32
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
            let width = action_width(&button.label);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a bottom-left card that grows past its limits, or notices
    /// drawn over a raster tab's filmstrip.
    #[test]
    fn notices_fit_their_words_and_sit_above_the_strip() {
        assert_eq!(fit(10.0), FIT_MIN);
        assert_eq!(fit(300.0), 300.0);
        assert_eq!(fit(900.0), FIT_MAX);
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        let strip = Rect::from_min_max(pos2(0.0, 532.0), pos2(800.0, 600.0));
        assert_eq!(above_strip(area, Some(strip)).bottom(), 532.0);
        assert_eq!(above_strip(area, None), area);
        // A strip from another place, outside the area, moves nothing.
        let elsewhere = Rect::from_min_max(pos2(900.0, 532.0), pos2(1200.0, 600.0));
        assert_eq!(above_strip(area, Some(elsewhere)), area);
    }
}
