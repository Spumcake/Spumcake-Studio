//! The published versions of one canon file, as the approved view
//! `ig-canon-history` shows them: a small card under the toolbar item that
//! opened it, a gray "Published versions", then one row for each version,
//! newest first, with its image, "Version N", and when it was published.
//! The canon one carries a dark "Canon" tag; each other one has Open and
//! "Revert to this", which send the workspace `history_open` and
//! `history_revert` with the version's file.
//!
//! The host draws what the runner read and names no workspace.

// Drawn once the window opens these cards; until then only the tests do.
#![allow(dead_code)]

use crate::typescale as ts;
use crate::widgets::{self, BORDER, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use protocol::Action;
use serde_json::json;

/// A canon file's published versions, newest first, as the runner read
/// them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct History {
    pub artifact: String,
    pub versions: Vec<HistoryVersion>,
    /// The project's folder, against which a version's relative file is
    /// read.
    pub root: Option<std::path::PathBuf>,
}

/// One published version.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HistoryVersion {
    pub file: String,
    /// When it was published, in seconds since the Unix epoch; 0 when not
    /// known.
    pub published: u64,
    /// Shown when the pointer rests on its row.
    pub note: String,
    pub canon: bool,
}

const WIDTH: f32 = 400.0;
const PAD: f32 = 12.0;
const RADIUS: f32 = 14.0;
const GAP: f32 = 6.0;
const LABEL_H: f32 = 24.0;
const ROW_H: f32 = 64.0;
const THUMB: egui::Vec2 = vec2(84.0, 48.0);
const BUTTON_H: f32 = 28.0;
const BUTTON_PAD: f32 = 10.0;
const DARK: Color32 = Color32::from_rgb(24, 25, 28);
const THUMB_FILL: Color32 = Color32::from_rgb(243, 244, 246);
const SHADOW: egui::epaint::Shadow = egui::epaint::Shadow { offset: [0, 8], blur: 32, spread: 0, color: Color32::from_black_alpha(18) };
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn closed_id() -> egui::Id {
    egui::Id::new("history-card-closed")
}

/// Whether a press outside the card, or Escape, closed it since the last
/// call.
#[allow(dead_code)]
pub(crate) fn take_closed(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(closed_id()).unwrap_or(false))
}

/// The card's left edge: centred under `anchor`, kept inside `screen`.
fn left(anchor: Rect, screen: Rect, width: f32) -> f32 {
    (anchor.center().x - width / 2.0).min(screen.right() - width - 8.0).max(screen.left() + 8.0)
}

/// "Version N" for the row at `index`, newest first, of `count`.
fn version_name(index: usize, count: usize) -> String {
    format!("Version {}", count.saturating_sub(index))
}

/// The open and revert commands for a version's file.
fn open_action(file: &str) -> Action {
    Action::with_arg("history_open", json!({ "file": file }))
}

fn revert_action(file: &str) -> Action {
    Action::with_arg("history_revert", json!({ "file": file }))
}

/// Draws the card under `anchor`. Returns the commands chosen: Open or
/// "Revert to this" on a version.
pub(crate) fn show(ctx: &egui::Context, anchor: Rect, history: &History) -> Vec<Action> {
    let screen = ctx.content_rect();
    let width = WIDTH.min(screen.width() - 16.0);
    let inner = width - 2.0 * PAD;
    let mut chosen = Vec::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |now| now.as_secs());
    let shown = egui::Area::new(egui::Id::new("history-card"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(left(anchor, screen, width), anchor.bottom() + GAP))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::WHITE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(RADIUS)
                .inner_margin(PAD)
                .shadow(SHADOW)
                .show(ui, |ui| {
                    ui.set_width(inner);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let (label, _) = ui.allocate_exact_size(vec2(inner, LABEL_H), Sense::hover());
                    ui.painter().text(label.left_center(), Align2::LEFT_CENTER, "Published versions", ts::small(), MUTED);
                    let count = history.versions.len();
                    for (index, version) in history.versions.iter().enumerate() {
                        if index > 0 {
                            let (line, _) = ui.allocate_exact_size(vec2(inner, 1.0), Sense::hover());
                            ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, BORDER));
                        }
                        version_row(ui, history, version, &version_name(index, count), now, inner, &mut chosen);
                    }
                });
        });
    let pressed_away = ctx.input(|input| {
        input.pointer.any_pressed()
            && input.pointer.interact_pos().is_some_and(|at| !shown.response.rect.contains(at) && !anchor.contains(at))
    });
    if pressed_away || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        ctx.data_mut(|data| data.insert_temp(closed_id(), true));
    }
    chosen
}

/// One version's row: its image, its name and time, and the Canon tag or
/// Open and "Revert to this".
fn version_row(ui: &mut egui::Ui, history: &History, version: &HistoryVersion, name: &str, now: u64, width: f32, chosen: &mut Vec<Action>) {
    let (row, response) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    if !version.note.is_empty() {
        response.on_hover_text(&version.note);
    }
    let thumb = Rect::from_min_size(pos2(row.left(), row.center().y - THUMB.y / 2.0), THUMB);
    ui.painter().rect_filled(thumb, 4.0, THUMB_FILL);
    let file = match &history.root {
        Some(root) if std::path::Path::new(&version.file).is_relative() => root.join(&version.file),
        _ => std::path::PathBuf::from(&version.file),
    };
    if let Some(texture) = crate::chat::shown_texture(ui.ctx(), &file.to_string_lossy()) {
        // Filling its box, cut at the edges.
        let size = texture.size_vec2();
        let scale = (THUMB.x / size.x.max(1.0)).max(THUMB.y / size.y.max(1.0));
        let image = Rect::from_center_size(thumb.center(), size * scale);
        ui.painter().with_clip_rect(thumb.intersect(ui.clip_rect())).image(
            texture.id(),
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    ui.painter().rect_stroke(thumb, 4.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let text_left = thumb.right() + 14.0;
    ui.painter().text(pos2(text_left, row.center().y - 10.0), Align2::LEFT_CENTER, name, ts::body(), TEXT);
    let time = when(version.published, now);
    if !time.is_empty() {
        ui.painter().text(pos2(text_left, row.center().y + 11.0), Align2::LEFT_CENTER, time, ts::small(), MUTED);
    }
    if version.canon {
        let words = ui.painter().layout_no_wrap(String::from("Canon"), widgets::semibold(ts::small_size()), Color32::WHITE);
        let tag = Rect::from_min_size(pos2(row.right() - words.size().x - 2.0 * BUTTON_PAD, row.center().y - BUTTON_H / 2.0), vec2(words.size().x + 2.0 * BUTTON_PAD, BUTTON_H));
        ui.painter().rect_filled(tag, 7.0, DARK);
        ui.painter().galley(tag.center() - words.size() / 2.0, words, Color32::WHITE);
        return;
    }
    let mut right = row.right();
    let asks = [("Revert to this", revert_action(&version.file)), ("Open", open_action(&version.file))];
    for (word, action) in asks {
        let words = ui.painter().layout_no_wrap(word.to_string(), ts::small(), TEXT);
        let spot = Rect::from_min_size(pos2(right - words.size().x - 2.0 * BUTTON_PAD, row.center().y - BUTTON_H / 2.0), vec2(words.size().x + 2.0 * BUTTON_PAD, BUTTON_H));
        right = spot.left() - 6.0;
        let response = ui
            .interact(spot, ui.id().with(("history-version", version.file.as_str(), word)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        widgets::outlined(ui.painter(), spot, 7.0, if response.hovered() { widgets::SELECTED } else { Color32::WHITE }, BORDER);
        ui.painter().galley(spot.center() - words.size() / 2.0, words, TEXT);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, word));
        if response.clicked() {
            chosen.push(action);
        }
    }
}

/// When a version was published: its local time today, or its date and
/// time on another day, such as "Nov 12, 9:40 PM". Nothing when unknown.
fn when(published: u64, now: u64) -> String {
    if published == 0 {
        return String::new();
    }
    let clock = crate::chat::clock(published);
    let (year, month, day) = local_date(published);
    if (year, month, day) == local_date(now) {
        return clock;
    }
    format!("{} {day}, {clock}", MONTHS[month.min(11) as usize])
}

/// A time's local year, month from 0, and day of the month.
#[cfg(unix)]
fn local_date(seconds: u64) -> (i32, u32, u32) {
    let time = seconds as libc::time_t;
    // Safety: `localtime_r` writes only into `parts`, which lives on this
    // stack frame, and reads only `time`.
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    let done = unsafe { libc::localtime_r(&time, &mut parts) };
    if done.is_null() {
        return utc_date(seconds);
    }
    (parts.tm_year, parts.tm_mon as u32, parts.tm_mday as u32)
}

#[cfg(not(unix))]
fn local_date(seconds: u64) -> (i32, u32, u32) {
    utc_date(seconds)
}

/// A time's year, month from 0, and day in UTC, from the count of days
/// (Howard Hinnant's `civil_from_days`).
fn utc_date(seconds: u64) -> (i32, u32, u32) {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = (of_year - (153 * shifted + 2) / 5 + 1) as u32;
    let month = (if shifted < 10 { shifted + 2 } else { shifted - 10 }) as u32;
    let year = (year_of_era + era * 400 + i64::from(month < 2)) as i32;
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish_card::tests::{screen, texts};

    fn history() -> History {
        let version = |file: &str, published: u64, canon: bool| HistoryVersion { file: file.into(), published, note: String::new(), canon };
        History {
            artifact: "shot 12: keyframe".into(),
            versions: vec![
                version("canon/keyframe-shot-12.png", 1_763_302_800, true),
                version("history/keyframe-shot-12.3.png", 1_763_222_700, false),
                version("history/keyframe-shot-12.2.png", 1_762_980_000, false),
            ],
            root: None,
        }
    }

    /// Catches a date read a day or a month off: the days count from the
    /// epoch as the calendar does.
    #[test]
    fn a_time_reads_as_its_date() {
        assert_eq!(utc_date(0), (1970, 0, 1));
        assert_eq!(utc_date(951_782_400), (2000, 1, 29));
        assert_eq!(utc_date(1_763_294_400), (2025, 10, 16));
        let now = 1_763_294_400;
        assert_eq!(when(now, now), crate::chat::clock(now));
        assert_eq!(when(0, now), "");
        // Noon on 12 November, UTC, is the 12th or 13th everywhere.
        assert!(when(1_762_948_800, now).starts_with("Nov 1"), "{}", when(1_762_948_800, now));
        assert_eq!(version_name(0, 4), "Version 4");
        assert_eq!(version_name(3, 4), "Version 1");
    }

    /// Catches the card drawn away from its button or a row out of order:
    /// it sits under the anchor, newest first, the canon one tagged, and
    /// "Revert to this" sends the workspace that version's file.
    #[test]
    fn the_card_lists_versions_under_its_button_and_reverts() {
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let anchor = Rect::from_min_size(pos2(1000.0, 56.0), vec2(32.0, 32.0));
        let history = history();
        let mut chosen = Vec::new();
        let mut draw = |events: Vec<egui::Event>, chosen: &mut Vec<Action>| {
            let output = ctx.run_ui(screen(events), |ui| chosen.extend(show(ui.ctx(), anchor, &history)));
            texts(&output.shapes)
        };
        draw(Vec::new(), &mut chosen);
        let found = draw(Vec::new(), &mut chosen);
        let find = |wanted: &str| found.iter().filter(|(text, _)| text == wanted).map(|(_, rect)| *rect).collect::<Vec<_>>();
        let label = find("Published versions");
        assert_eq!(label.len(), 1, "the label was not drawn once");
        assert!(label[0].top() > anchor.bottom(), "the card is not under its button");
        let names: Vec<Rect> = ["Version 3", "Version 2", "Version 1"].iter().map(|name| find(name)[0]).collect();
        assert!(names[0].top() < names[1].top() && names[1].top() < names[2].top(), "the versions are not newest first");
        assert_eq!(find("Canon").len(), 1, "the canon tag is not on one row");
        assert!((find("Canon")[0].center().y - names[0].center().y).abs() < ROW_H / 2.0, "the canon tag is not on the newest row");
        assert_eq!(find("Open").len(), 2);
        let reverts = find("Revert to this");
        assert_eq!(reverts.len(), 2);
        let at = reverts.iter().copied().max_by(|a, b| a.top().total_cmp(&b.top())).map(|rect| rect.center()).unwrap_or_default();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        draw(vec![egui::Event::PointerMoved(at), press(true)], &mut chosen);
        draw(vec![press(false)], &mut chosen);
        assert_eq!(chosen, vec![revert_action("history/keyframe-shot-12.2.png")]);
        assert_eq!(chosen[0].arg, Some(json!({ "file": "history/keyframe-shot-12.2.png" })));
        assert!(!take_closed(&ctx), "a press on the card closed it");
    }

    /// Catches a card that runs off the window under a button at its edge.
    #[test]
    fn the_card_stays_inside_the_window() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let at_edge = Rect::from_min_size(pos2(1250.0, 10.0), vec2(30.0, 30.0));
        assert_eq!(left(at_edge, screen, WIDTH), 1280.0 - WIDTH - 8.0);
        let middle = Rect::from_min_size(pos2(620.0, 10.0), vec2(40.0, 30.0));
        assert_eq!(left(middle, screen, WIDTH), 640.0 - WIDTH / 2.0);
    }
}
