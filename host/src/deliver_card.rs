//! The Deliver card, as the approved view `vg-deliver` shows it: a white
//! card in the middle of the window over a faint overlay, titled Deliver,
//! with rows of a muted label and its value: the file it writes, in the
//! code face inside a light grey field, its size, frame rate, format, and
//! what it uses. Then Cancel and a dark Deliver.
//!
//! The values come whole from whoever fills the card; the card makes no
//! words of its own but its labels and buttons, and names no workspace.

use crate::typescale as ts;
use crate::widgets::{self, BORDER, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};

/// Row labels not yet in the conventions guide, pending the owner's yes.
const SIZE: &str = "Size";
const FORMAT: &str = "Format";
const USES: &str = "Uses";

/// What the card shows, each value whole, as written by whoever fills it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeliverCardView {
    /// The file it writes, such as
    /// `output/2026-11-15_08-25-012-the-last-signal.mp4`.
    pub file: String,
    /// Such as "3840 × 2160".
    pub size: String,
    /// Such as "60 fps".
    pub frame_rate: String,
    /// Such as "H.264 with AAC sound".
    pub format: String,
    /// What the film is made of, such as "Take 3 · 60 fps · 4K, with
    /// Voice, Bed and Door".
    pub uses: String,
}

/// What the person chose on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliverCardEvent {
    Deliver,
    /// Cancel, Escape, or a press on the overlay.
    Cancel,
}

const WIDTH: f32 = 640.0;
const PAD: f32 = 28.0;
const RADIUS: f32 = 14.0;
const TITLE: f32 = 24.0;
/// The label column, and the room between rows.
const LABEL_W: f32 = 120.0;
const ROW_GAP: f32 = 16.0;
/// The file's field: its height and the room inside it.
const FIELD_H: f32 = 40.0;
const FIELD_PAD: f32 = 12.0;
const FIELD_FILL: Color32 = Color32::from_rgb(243, 244, 246);
const BUTTON_H: f32 = 44.0;
const DARK: Color32 = Color32::from_rgb(24, 25, 28);
/// The overlay behind the card, faint, so the work stays readable.
const OVERLAY: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 40);
const SHADOW: egui::epaint::Shadow = egui::epaint::Shadow { offset: [0, 8], blur: 32, spread: 0, color: Color32::from_black_alpha(24) };

/// The card's place: the middle of the window.
fn modal(id: egui::Id) -> egui::Modal {
    egui::Modal::new(id)
        .backdrop_color(OVERLAY)
        .frame(egui::Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0, BORDER)).corner_radius(RADIUS).inner_margin(PAD).shadow(SHADOW))
}

/// `text` cut in the middle with "…" until `fits` says it fits, keeping
/// as much of its start and its end as it can, as a long path keeps its
/// folder and its file's name.
fn middle_ellipsis(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let cut = |kept: usize| -> String {
        let head = kept.div_ceil(2);
        let tail = kept - head;
        let mut shown: String = chars[..head].iter().collect();
        shown.push('…');
        shown.extend(&chars[chars.len() - tail..]);
        shown
    };
    // The most characters kept that still fit, found by halving.
    let (mut low, mut high) = (0, chars.len() - 1);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&cut(middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    cut(low)
}

/// Draws the card for `view`. Returns the person's choice, or None while
/// they decide.
pub(crate) fn show(ctx: &egui::Context, view: &DeliverCardView) -> Option<DeliverCardEvent> {
    let id = egui::Id::new(("deliver-card", view.file.as_str()));
    let inner = WIDTH.min(ctx.content_rect().width() - 32.0).max(2.0 * PAD + 2.0 * LABEL_W) - 2.0 * PAD;
    let value_w = inner - LABEL_W;
    let mut chosen = None;
    let shown = modal(id).show(ctx, |ui| {
        ui.set_width(inner);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let title = ui.painter().layout_no_wrap(String::from("Deliver"), widgets::semibold(TITLE), TEXT);
        let (rect, _) = ui.allocate_exact_size(vec2(inner, title.size().y), Sense::hover());
        ui.painter().galley(rect.min, title, TEXT);
        ui.add_space(20.0);

        // The file, in the code face, in its field, cut in the middle when
        // it is too long for it.
        let (row, _) = ui.allocate_exact_size(vec2(inner, FIELD_H), Sense::hover());
        label(ui, row, "File");
        let field = Rect::from_min_max(pos2(row.left() + LABEL_W, row.top()), row.max);
        ui.painter().rect_filled(field, 8.0, FIELD_FILL);
        let room = field.width() - 2.0 * FIELD_PAD;
        let fits = |text: &str| ui.painter().layout_no_wrap(text.to_string(), ts::code(), TEXT).size().x <= room;
        let file = middle_ellipsis(&view.file, fits);
        let galley = ui.painter().layout_no_wrap(file, ts::code(), TEXT);
        ui.painter().galley(pos2(field.left() + FIELD_PAD, field.center().y - galley.size().y / 2.0), galley, TEXT);

        // Size, frame rate, format, and what it uses, which wraps.
        for (words, value) in [(SIZE, &view.size), ("Frame rate", &view.frame_rate), (FORMAT, &view.format), (USES, &view.uses)] {
            ui.add_space(ROW_GAP);
            let galley = ui.painter().layout(value.clone(), ts::body(), TEXT, value_w);
            let line = ui.painter().layout_no_wrap(String::from("A"), ts::body(), TEXT).size().y;
            let (row, _) = ui.allocate_exact_size(vec2(inner, galley.size().y.max(line)), Sense::hover());
            label(ui, Rect::from_min_size(row.min, vec2(LABEL_W, line)), words);
            ui.painter().galley(pos2(row.left() + LABEL_W, row.top()), galley, TEXT);
        }
        ui.add_space(28.0);

        // Cancel, then the dark Deliver at the right.
        let (bar, _) = ui.allocate_exact_size(vec2(inner, BUTTON_H), Sense::hover());
        let deliver_w = ui.painter().layout_no_wrap(String::from("Deliver"), widgets::semibold(ts::body_size()), Color32::WHITE).size().x + 40.0;
        let deliver = Rect::from_min_size(pos2(bar.right() - deliver_w, bar.top()), vec2(deliver_w, BUTTON_H));
        let cancel = Rect::from_min_size(pos2(deliver.left() - 12.0 - 88.0, bar.top()), vec2(88.0, BUTTON_H));
        let deliver_click = ui.interact(deliver, id.with("deliver"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let cancel_click = ui.interact(cancel, id.with("cancel"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(deliver, 9.0, DARK);
        ui.painter().text(deliver.center(), Align2::CENTER_CENTER, "Deliver", widgets::semibold(ts::body_size()), Color32::WHITE);
        ui.painter().text(cancel.center(), Align2::CENTER_CENTER, "Cancel", ts::body(), if cancel_click.hovered() { TEXT } else { MUTED });
        deliver_click.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Deliver"));
        cancel_click.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel"));
        if deliver_click.clicked() {
            chosen = Some(DeliverCardEvent::Deliver);
        } else if cancel_click.clicked() {
            chosen = Some(DeliverCardEvent::Cancel);
        }
    });
    if chosen.is_none() && shown.should_close() {
        chosen = Some(DeliverCardEvent::Cancel);
    }
    chosen
}

/// A row's muted label, at the left of `row`, centred on it.
fn label(ui: &egui::Ui, row: Rect, words: &str) {
    ui.painter().text(pos2(row.left(), row.center().y), Align2::LEFT_CENTER, words, ts::body(), MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish_card::tests::{screen, texts};

    fn view() -> DeliverCardView {
        DeliverCardView {
            file: "output/2026-11-15_08-25-012-the-last-signal.mp4".into(),
            size: "3840 × 2160".into(),
            frame_rate: "60 fps".into(),
            format: "H.264 with AAC sound".into(),
            uses: "Take 3 · 60 fps · 4K, with Voice, Bed and Door".into(),
        }
    }

    /// Draws the card for one pass with `events`, and returns its texts and
    /// the choice.
    fn draw(ctx: &egui::Context, view: &DeliverCardView, events: Vec<egui::Event>) -> (Vec<(String, Rect)>, Option<DeliverCardEvent>) {
        let mut chosen = None;
        let mut output = ctx.run_ui(screen(events), |ui| chosen = show(ui.ctx(), view));
        let found = texts(&output.shapes);
        output.textures_delta.clear();
        (found, chosen)
    }

    /// The card drawn twice, so its first, unseen sizing pass is behind it.
    fn drawn(view: &DeliverCardView) -> (egui::Context, Vec<(String, Rect)>) {
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        draw(&ctx, view, Vec::new());
        let (found, chosen) = draw(&ctx, view, Vec::new());
        assert_eq!(chosen, None);
        (ctx, found)
    }

    fn find(found: &[(String, Rect)], wanted: &str) -> Vec<Rect> {
        found.iter().filter(|(text, _)| text == wanted).map(|(_, rect)| *rect).collect()
    }

    /// Where the card's insides run, from its title's left edge.
    fn inside(found: &[(String, Rect)]) -> (f32, f32) {
        let title = find(found, "Deliver").into_iter().min_by(|a, b| a.top().total_cmp(&b.top())).expect("the title was not drawn");
        let left = title.left() - 2.0;
        (left, left + (WIDTH - 2.0 * PAD) + 4.0)
    }

    fn click(ctx: &egui::Context, view: &DeliverCardView, at: egui::Pos2) -> Option<DeliverCardEvent> {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let first = draw(ctx, view, vec![egui::Event::PointerMoved(at), press(true)]).1;
        first.or(draw(ctx, view, vec![press(false)]).1)
    }

    /// Catches a row left out: every label and value is drawn, and Deliver
    /// both titles the card and confirms it.
    #[test]
    fn every_label_and_value_is_drawn() {
        let card = view();
        let (_, found) = drawn(&card);
        for wanted in [
            "File",
            card.file.as_str(),
            "Size",
            "3840 × 2160",
            "Frame rate",
            "60 fps",
            "Format",
            "H.264 with AAC sound",
            "Uses",
            card.uses.as_str(),
            "Cancel",
        ] {
            assert_eq!(find(&found, wanted).len(), 1, "{wanted:?} was not drawn once");
        }
        assert_eq!(find(&found, "Deliver").len(), 2, "the title and the button are not both drawn");
        let size = find(&found, "Size")[0];
        let value = find(&found, "3840 × 2160")[0];
        assert!((size.center().y - value.center().y).abs() < 4.0 && value.left() > size.right(), "a value is not beside its label");
    }

    /// Catches a button that does nothing: Deliver and Cancel return their
    /// choices, and so do Escape and a press on the overlay.
    #[test]
    fn deliver_and_cancel_return_their_choices() {
        let card = view();
        let (ctx, found) = drawn(&card);
        let button = find(&found, "Deliver").into_iter().max_by(|a, b| a.top().total_cmp(&b.top())).expect("Deliver was not drawn");
        assert_eq!(click(&ctx, &card, button.center()), Some(DeliverCardEvent::Deliver));
        let (ctx, found) = drawn(&card);
        assert_eq!(click(&ctx, &card, find(&found, "Cancel")[0].center()), Some(DeliverCardEvent::Cancel));
        let (ctx, _) = drawn(&card);
        assert_eq!(click(&ctx, &card, pos2(20.0, 780.0)), Some(DeliverCardEvent::Cancel));
        let (ctx, _) = drawn(&card);
        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        assert_eq!(draw(&ctx, &card, vec![escape]).1, Some(DeliverCardEvent::Cancel));
    }

    /// Catches a long path running out of its card: it is cut in the
    /// middle, keeping its start and its file's end, inside the card.
    #[test]
    fn a_long_path_stays_inside_the_card() {
        let mut card = view();
        card.file = format!("output/{}/the-last-signal.mp4", "deliveries-of-the-last-signal".repeat(4));
        card.file.truncate(120);
        assert_eq!(card.file.chars().count(), 120);
        let (_, found) = drawn(&card);
        let (left, right) = inside(&found);
        let (path, rect) = found.iter().find(|(text, _)| text.contains('…')).expect("the long path was not cut");
        assert!(path.starts_with("output/") && card.file.ends_with(&path[path.rfind('…').map_or(0, |at| at + '…'.len_utf8())..]), "the cut lost the path's ends: {path}");
        assert!(rect.left() >= left && rect.right() <= right, "the path runs out of the card: {rect:?} in {left}..{right}");
    }

    /// Catches a long Uses on one line: it wraps under its label, inside
    /// the card.
    #[test]
    fn uses_wraps_inside_the_card() {
        let mut card = view();
        card.uses = "Take 3 · 60 fps · 4K, with Voice, Bed, Door, Rain, Wind, Radio, Thunder, Footsteps, Sea, and the lighthouse's horn".into();
        let (_, found) = drawn(&card);
        let (left, right) = inside(&found);
        let rect = find(&found, &card.uses)[0];
        let one_line = find(&found, "Size")[0].height();
        assert!(rect.height() > 1.5 * one_line, "Uses did not wrap: {rect:?}");
        assert!(rect.left() >= left && rect.right() <= right, "Uses runs out of the card: {rect:?} in {left}..{right}");
    }

    /// Catches a cut that keeps the wrong ends, or cuts a path that fits.
    #[test]
    fn a_cut_keeps_both_ends() {
        let fits = |text: &str| text.chars().count() <= 9;
        assert_eq!(middle_ellipsis("short", fits), "short");
        assert_eq!(middle_ellipsis("abcdefghijklmnop", fits), "abcd…mnop");
        assert_eq!(middle_ellipsis("abcdefghijklmnop", |text: &str| text.chars().count() <= 1), "…");
    }
}
