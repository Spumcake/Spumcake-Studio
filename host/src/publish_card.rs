//! The publish card, as the approved view `ig-publish` shows it: a white
//! card at the top centre over a faint overlay, naming the file and the
//! version it writes, with Commit, on by default, and the earlier
//! uncommitted lines that ride along with it, then Cancel and Publish. A
//! revert is the same card, titled for the version it brings back. When
//! the runner refuses, the refusal shows in the failed request's look: the
//! red mark, its sentence, and "Ask the assistant".
//!
//! The host draws what the runner gives it and names no workspace: the
//! file, the canon name, the next version, and the lines that ride along.

use crate::typescale as ts;
use crate::widgets::{self, BORDER, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};

/// What the card shows, from the runner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PublishCardView {
    /// The file to publish, or for a revert the earlier version's file.
    pub file: String,
    /// The canon name it is published as, such as "keeper: sheet".
    pub artifact: String,
    /// The version it writes. The first is 1.
    pub version: u32,
    /// Earlier uncommitted lines of the run log that a commit takes along.
    pub ride_along: Vec<String>,
    /// Whether Commit starts on.
    pub commit: bool,
    /// The card brings an earlier version back.
    pub revert: bool,
}

/// What the person chose on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublishCardEvent {
    Confirm { commit: bool },
    Cancel,
}

/// What the person chose on a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefusedEvent {
    /// "Ask the assistant", for the window to put the refusal in the
    /// conversation's composer.
    AskAssistant,
    /// Escape, or a press outside the card.
    Dismiss,
}

const WIDTH: f32 = 500.0;
const PAD: f32 = 24.0;
const RADIUS: f32 = 14.0;
/// The card's distance below the window's top, as `ig-publish` sets it
/// over the toolbar.
const TOP: f32 = 48.0;
const TITLE: f32 = 20.0;
const BUTTON_H: f32 = 40.0;
const CHECK: f32 = 18.0;
/// Where the lines under Commit start, past its box.
const CHECK_INDENT: f32 = CHECK + 10.0;
const DARK: Color32 = Color32::from_rgb(24, 25, 28);
const CHECKED: Color32 = Color32::from_rgb(37, 99, 235);
const FAILED_RED: Color32 = Color32::from_rgb(220, 38, 38);
/// The overlay behind the card, faint, so the work stays readable.
const OVERLAY: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 40);
const SHADOW: egui::epaint::Shadow = egui::epaint::Shadow { offset: [0, 8], blur: 32, spread: 0, color: Color32::from_black_alpha(24) };

/// The card's title: "Publish <file name>", or for a revert "Revert
/// <artifact> to <file name>".
pub(crate) fn title(view: &PublishCardView) -> String {
    if view.revert {
        format!("Revert {} to {}", view.artifact, file_name(&view.file))
    } else {
        format!("Publish {}", file_name(&view.file))
    }
}

/// The gray line under the title.
pub(crate) fn version_line(version: u32) -> String {
    if version <= 1 {
        String::from("Writes the first version.")
    } else {
        format!("Writes version {version}. The previous version moves to history.")
    }
}

/// The dark button's word.
fn confirm_word(view: &PublishCardView) -> &'static str {
    if view.revert { "Revert" } else { "Publish" }
}

/// A path's last part.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The card's place: centred under the window's top.
fn modal(id: egui::Id) -> egui::Modal {
    egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, TOP)))
        .backdrop_color(OVERLAY)
        .frame(egui::Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0, BORDER)).corner_radius(RADIUS).inner_margin(PAD).shadow(SHADOW))
}

/// Draws the card for `view`. Returns the person's choice: Confirm, with
/// whether to commit, or Cancel, from its button, Escape, or a press on
/// the overlay; None while they decide.
pub(crate) fn show(ctx: &egui::Context, view: &PublishCardView) -> Option<PublishCardEvent> {
    let id = egui::Id::new(("publish-card", view.file.as_str(), view.artifact.as_str(), view.revert));
    let commit_id = id.with("commit");
    let mut commit = ctx.data(|data| data.get_temp::<bool>(commit_id)).unwrap_or(view.commit);
    let inner = WIDTH.min(ctx.content_rect().width() - 32.0).max(2.0 * PAD + 120.0) - 2.0 * PAD;
    let mut chosen = None;
    let shown = modal(id).show(ctx, |ui| {
        ui.set_width(inner);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        text_block(ui, &title(view), widgets::semibold(TITLE), TEXT, inner);
        ui.add_space(6.0);
        text_block(ui, &version_line(view.version), ts::body(), MUTED, inner);
        ui.add_space(18.0);

        // Commit, a box to tick, and under it the lines that ride along.
        let (row, _) = ui.allocate_exact_size(vec2(inner, CHECK + 8.0), Sense::hover());
        let words = ui.painter().layout_no_wrap(String::from("Commit"), ts::body(), TEXT);
        let spot = Rect::from_min_max(row.min, pos2(row.left() + CHECK_INDENT + words.size().x, row.bottom()));
        let response = ui.interact(spot, id.with("commit-box"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let check = Rect::from_min_size(pos2(row.left(), row.center().y - CHECK / 2.0), vec2(CHECK, CHECK));
        if commit {
            ui.painter().rect_filled(check, 4.0, CHECKED);
            crate::toolbar::paint_icon(ui.painter(), check.center(), "check", 14.0, Color32::WHITE);
        } else {
            widgets::outlined(ui.painter(), check, 4.0, Color32::WHITE, if response.hovered() { MUTED } else { widgets::RAIL });
        }
        ui.painter().galley(pos2(row.left() + CHECK_INDENT, row.center().y - words.size().y / 2.0), words, TEXT);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, commit, "Commit"));
        if response.clicked() {
            commit = !commit;
        }
        if commit && !view.ride_along.is_empty() {
            ui.add_space(4.0);
            for line in &view.ride_along {
                ui.horizontal(|ui| {
                    ui.add_space(CHECK_INDENT);
                    text_block(ui, line, ts::small(), MUTED, inner - CHECK_INDENT);
                });
            }
        }
        ui.add_space(22.0);

        // Cancel, then the dark button at the right.
        let (bar, _) = ui.allocate_exact_size(vec2(inner, BUTTON_H), Sense::hover());
        let word = confirm_word(view);
        let confirm_w = ui.painter().layout_no_wrap(word.to_string(), widgets::semibold(ts::body_size()), Color32::WHITE).size().x + 36.0;
        let confirm = Rect::from_min_size(pos2(bar.right() - confirm_w, bar.top()), vec2(confirm_w, BUTTON_H));
        let cancel = Rect::from_min_size(pos2(confirm.left() - 8.0 - 80.0, bar.top()), vec2(80.0, BUTTON_H));
        let confirm_click = ui.interact(confirm, id.with("confirm"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let cancel_click = ui.interact(cancel, id.with("cancel"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(confirm, 9.0, DARK);
        ui.painter().text(confirm.center(), Align2::CENTER_CENTER, word, widgets::semibold(ts::body_size()), Color32::WHITE);
        ui.painter().text(cancel.center(), Align2::CENTER_CENTER, "Cancel", ts::body(), if cancel_click.hovered() { TEXT } else { MUTED });
        confirm_click.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, word));
        cancel_click.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel"));
        if confirm_click.clicked() {
            chosen = Some(PublishCardEvent::Confirm { commit });
        } else if cancel_click.clicked() {
            chosen = Some(PublishCardEvent::Cancel);
        }
    });
    if chosen.is_none() && shown.should_close() {
        chosen = Some(PublishCardEvent::Cancel);
    }
    ctx.data_mut(|data| {
        if chosen.is_some() {
            data.remove::<bool>(commit_id);
        } else {
            data.insert_temp(commit_id, commit);
        }
    });
    chosen
}

/// Draws a refusal of a publish: the red mark, `sentence`, and "Ask the
/// assistant". Returns the person's choice, or None while they decide.
pub(crate) fn refused(ctx: &egui::Context, sentence: &str) -> Option<RefusedEvent> {
    let id = egui::Id::new("publish-refused");
    let inner = WIDTH.min(ctx.content_rect().width() - 32.0).max(2.0 * PAD + 120.0) - 2.0 * PAD;
    let mut chosen = None;
    let shown = modal(id).show(ctx, |ui| {
        ui.set_width(inner);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mark = 24.0;
        let text_left = mark + 12.0;
        let galley = ui.painter().layout(sentence.to_string(), ts::body(), TEXT, inner - text_left);
        let (block, _) = ui.allocate_exact_size(vec2(inner, galley.size().y.max(mark)), Sense::hover());
        let center = pos2(block.left() + mark / 2.0, block.top() + mark / 2.0);
        widgets::disc(ui.painter(), center, mark / 2.0, FAILED_RED);
        ui.painter().text(center, Align2::CENTER_CENTER, "!", widgets::semibold(15.0), Color32::WHITE);
        ui.painter().galley(pos2(block.left() + text_left, block.top()), galley, TEXT);
        ui.add_space(14.0);
        let (row, _) = ui.allocate_exact_size(vec2(inner, 34.0), Sense::hover());
        let word = "Ask the assistant";
        let words = ui.painter().layout_no_wrap(word.to_string(), ts::body(), TEXT);
        let spot = Rect::from_min_size(pos2(row.left() + text_left, row.top()), vec2(words.size().x + 28.0, row.height()));
        let response = ui.interact(spot, id.with("ask"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        widgets::outlined(ui.painter(), spot, 8.0, if response.hovered() { widgets::SELECTED } else { Color32::WHITE }, BORDER);
        ui.painter().galley(spot.center() - words.size() / 2.0, words, TEXT);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, word));
        if response.clicked() {
            chosen = Some(RefusedEvent::AskAssistant);
        }
    });
    if chosen.is_none() && shown.should_close() {
        chosen = Some(RefusedEvent::Dismiss);
    }
    chosen
}

/// Words wrapped to `width`, taking their height.
fn text_block(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32, width: f32) {
    let galley = ui.painter().layout(text.to_string(), font, color, width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every text drawn, with where it was drawn.
    pub(crate) fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
        fn walk(shape: &egui::Shape, found: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => found.push((text.galley.text().to_string(), text.visual_bounding_rect())),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    pub(crate) fn screen(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))), events, ..Default::default() }
    }

    fn view(revert: bool, ride_along: Vec<String>) -> PublishCardView {
        PublishCardView {
            file: "work/keyframe-shot-12.png".into(),
            artifact: "shot 12: keyframe".into(),
            version: 4,
            ride_along,
            commit: true,
            revert,
        }
    }

    /// Draws the card for a few passes, the last with `events`, and returns
    /// its texts and the choice.
    fn draw(ctx: &egui::Context, view: &PublishCardView, events: Vec<egui::Event>) -> (Vec<(String, Rect)>, Option<PublishCardEvent>) {
        let mut chosen = None;
        let mut output = ctx.run_ui(screen(events), |ui| chosen = show(ui.ctx(), view));
        output.textures_delta.clear();
        (texts(&output.shapes), chosen)
    }

    fn click(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![vec![egui::Event::PointerMoved(at), press(true)], vec![press(false)]]
    }

    /// Catches a card that says the wrong thing: the title names the file,
    /// the line names the version, the first version says so, and a revert
    /// is titled and confirmed as one.
    #[test]
    fn the_card_names_what_it_writes() {
        assert_eq!(title(&view(false, Vec::new())), "Publish keyframe-shot-12.png");
        assert_eq!(title(&view(true, Vec::new())), "Revert shot 12: keyframe to keyframe-shot-12.png");
        assert_eq!(version_line(4), "Writes version 4. The previous version moves to history.");
        assert_eq!(version_line(1), "Writes the first version.");
        assert_eq!(confirm_word(&view(true, Vec::new())), "Revert");
    }

    /// Catches a card drawn off its place or missing a part: it sits at the
    /// top centre with its title, line, Commit, the lines that ride along,
    /// Cancel, and Publish; unticking Commit hides those lines and is what
    /// Publish sends.
    #[test]
    fn the_card_sits_at_the_top_and_publishes_with_or_without_commit() {
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let card = view(false, vec!["Ran request 14 for shot 12".into()]);
        draw(&ctx, &card, Vec::new());
        let (texts, chosen) = draw(&ctx, &card, Vec::new());
        assert_eq!(chosen, None);
        let find = |texts: &[(String, Rect)], wanted: &str| texts.iter().find(|(text, _)| text == wanted).map(|(_, rect)| *rect);
        let heading = find(&texts, "Publish keyframe-shot-12.png").expect("the title was not drawn");
        assert!(heading.top() >= TOP && heading.top() < TOP + 2.0 * PAD + 10.0, "the card is not at the top: {heading:?}");
        let middle = 640.0;
        assert!(heading.left() < middle && heading.left() > middle - WIDTH / 2.0 - 1.0, "the card is not centred: {heading:?}");
        for wanted in ["Writes version 4. The previous version moves to history.", "Commit", "Ran request 14 for shot 12", "Cancel", "Publish"] {
            assert!(find(&texts, wanted).is_some(), "{wanted:?} was not drawn");
        }
        let commit = find(&texts, "Commit").expect("Commit was not drawn");
        for events in click(commit.center()) {
            draw(&ctx, &card, events);
        }
        let (texts, _) = draw(&ctx, &card, Vec::new());
        assert!(find(&texts, "Ran request 14 for shot 12").is_none(), "the lines ride along without a commit");
        let publish = find(&texts, "Publish").expect("Publish was not drawn");
        let mut chosen = None;
        for events in click(publish.center()) {
            chosen = chosen.or(draw(&ctx, &card, events).1);
        }
        assert_eq!(chosen, Some(PublishCardEvent::Confirm { commit: false }));
    }

    /// Catches a press outside the card that does nothing: it cancels.
    #[test]
    fn a_press_outside_cancels() {
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let card = view(true, Vec::new());
        draw(&ctx, &card, Vec::new());
        let (texts, _) = draw(&ctx, &card, Vec::new());
        assert!(texts.iter().any(|(text, _)| text == "Revert"), "a revert's button was not drawn");
        let mut chosen = None;
        for events in click(pos2(40.0, 760.0)) {
            chosen = chosen.or(draw(&ctx, &card, events).1);
        }
        assert_eq!(chosen, Some(PublishCardEvent::Cancel));
    }
}
