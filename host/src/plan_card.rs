//! The plan card: what the runner asks the person before anything runs, as
//! the approved view `ig-request-confirm` shows it, over a faint overlay.
//! Before a request it shows how many and the cost, the model and its
//! settings, the prompt as sent, and every reference in the order sent with
//! its full path, then Cancel and Send request. Before any other command it
//! asks "Run this command?" with the command in full, then Cancel and Run.
//! Before research it shows the searches with their cost, then Search, or
//! a file to download with its page and licence, then Download.

use crate::panel::ReferencePreview;
use crate::runner::Asking;
use crate::typescale as ts;
use crate::widgets::{self, BORDER, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, pos2, vec2};
use serde_json::Value;
use std::collections::HashMap;

const WIDTH: f32 = 560.0;
const PAD: f32 = 28.0;
const RADIUS: f32 = 14.0;
const BOX_FILL: Color32 = Color32::from_rgb(243, 244, 246);
const THUMB_W: f32 = 104.0;
const THUMB_H: f32 = 60.0;
const BUTTON_H: f32 = 46.0;
const DARK: Color32 = Color32::from_rgb(24, 25, 28);
/// The overlay behind the card, faint, so the work stays readable.
const OVERLAY: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 40);
/// The fill behind a preview's corner words, as on the Prompt page.
const BADGE_FILL: Color32 = Color32::from_rgba_premultiplied(17, 18, 20, 190);
/// The tallest the prompt's box grows before it scrolls.
const PROMPT_MOST: f32 = 220.0;

/// One line of the card, painted later in order.
enum Line {
    Title(String),
    Muted(String),
    Label(String),
    Boxed(String, bool),
    Reference { number: usize, path: String, label: String },
}

/// The service a request kind goes to, as the user knows it.
pub(crate) fn service_name(kind: &str) -> String {
    match kind.split('-').next().unwrap_or(kind) {
        "openrouter" => String::from("OpenRouter"),
        "openai" => String::from("OpenAI"),
        "fal" => String::from("fal.ai"),
        _ => kind.to_string(),
    }
}

/// A plan's cost as the card says it: the estimate, or "Cost unknown",
/// never the script's own words for having no price.
fn cost_words(estimate: &str) -> String {
    if estimate.trim().is_empty() || estimate.starts_with("unknown") { String::from("Cost unknown") } else { estimate.to_string() }
}

fn lines(asking: &Asking, model_name: &dyn Fn(&str) -> String) -> (Vec<Line>, String) {
    match asking {
        Asking::Command { argv, why, .. } => {
            let mut lines = vec![Line::Title(String::from("Run this command?"))];
            if !why.trim().is_empty() {
                lines.push(Line::Muted(why.clone()));
            }
            lines.push(Line::Boxed(argv.join(" "), true));
            (lines, String::from("Run"))
        }
        Asking::Plans { plans, stand_in, .. } => {
            let count: u64 = plans.iter().map(|plan| plan.get("requests").and_then(Value::as_u64).unwrap_or(1)).sum();
            // A pipeline step that runs a script, such as a builder, sends
            // nothing to a model: it is shown by the command it runs.
            let script_only = plans.iter().all(|plan| plan.get("endpoint").is_none_or(Value::is_null));
            let noun = match (script_only, count) {
                (true, 1) => "step",
                (true, _) => "steps",
                (false, 1) => "request",
                (false, _) => "requests",
            };
            // Research is filed, not sent: it spends nothing here.
            let research = plans.iter().all(|plan| plan.pointer("/endpoint/kind").and_then(Value::as_str) == Some("research"));
            let cost = if *stand_in {
                String::from("nothing; a stand-in answers")
            } else if research {
                String::from("nothing; research is filed, not sent")
            } else {
                plans.iter().filter_map(|plan| plan.get("estimate").and_then(Value::as_str)).map(cost_words).collect::<Vec<_>>().join(", ")
            };
            let mut lines = vec![Line::Title(format!("{count} {noun} · {cost}"))];
            // A fix of the dependency chain names what it leaves out of date.
            if let Asking::Plans { fix: Some(fix), .. } = asking
                && !fix.not_fixed().is_empty()
            {
                lines.push(Line::Muted(crate::runner::not_fixed_line(fix.not_fixed())));
            }
            if let Asking::Plans { fix: Some(fix), .. } = asking
                && !fix.waiting().is_empty()
            {
                lines.push(Line::Muted(crate::runner::waiting_line(fix.waiting())));
            }
            // What a listed price leaves out, such as charges per token.
            for note in plans.iter().filter_map(|plan| plan.get("price_note").and_then(Value::as_str)) {
                lines.push(Line::Muted(format!("The price {note}.")));
            }
            // Where the prices of a model priced by its tokens are set.
            for hint in plans.iter().filter_map(|plan| plan.get("price_hint").and_then(Value::as_str)) {
                lines.push(Line::Muted(hint.to_string()));
            }
            for plan in plans {
                let endpoint = plan.get("endpoint").cloned().unwrap_or(Value::Null);
                if !endpoint.is_null() {
                    // The model and the service as the user knows them, as
                    // the approved view reads: "GPT Image 2.5 on OpenRouter".
                    let model = endpoint.get("model").and_then(Value::as_str).map(model_name).unwrap_or_else(|| String::from("its default model"));
                    let kind = endpoint.get("kind").and_then(Value::as_str).unwrap_or("");
                    lines.push(Line::Muted(format!("{model} on {}", service_name(kind))));
                }
                if let Some(script) = plan.get("script").and_then(Value::as_str) {
                    lines.push(Line::Label(String::from("Runs")));
                    lines.push(Line::Boxed(script.to_string(), true));
                }
                if let Some(Value::Object(settings)) = endpoint.get("settings")
                    && !settings.is_empty()
                {
                    let shown: Vec<String> = settings.iter().map(|(name, value)| format!("{name} {}", value.as_str().map(String::from).unwrap_or_else(|| value.to_string()))).collect();
                    lines.push(Line::Muted(format!("Settings: {}", shown.join(", "))));
                }
                if let Some(scripts) = plan.get("scripts").and_then(Value::as_array)
                    && !scripts.is_empty()
                {
                    lines.push(Line::Label(String::from("Runs")));
                    lines.push(Line::Boxed(scripts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"), true));
                }
                if let Some(prompt) = plan.get("prompt").and_then(Value::as_str) {
                    lines.push(Line::Label(String::from("Prompt, as sent")));
                    lines.push(Line::Boxed(prompt.to_string(), false));
                }
                let filed = plan.get("filed").and_then(Value::as_array).cloned().unwrap_or_default();
                if !filed.is_empty() {
                    lines.push(Line::Label(String::from("Files, into sources/")));
                    for (index, file) in filed.iter().enumerate() {
                        let text = |key: &str| file.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                        let found = [text("page"), text("address")].into_iter().find(|at| !at.is_empty()).unwrap_or_default();
                        let label = if text("use").is_empty() { found } else { format!("{} · {found}", text("use")) };
                        lines.push(Line::Reference { number: index + 1, path: text("file"), label });
                    }
                }
                let references = plan.get("references").and_then(Value::as_array).cloned().unwrap_or_default();
                if !references.is_empty() {
                    lines.push(Line::Label(String::from("References, in the order sent")));
                    for (index, reference) in references.iter().enumerate() {
                        let text = |key: &str| reference.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                        let path = if text("path").is_empty() { text("file") } else { text("path") };
                        // A reference an earlier fix in the same plan replaces.
                        let label = match reference.get(crate::runner::FIXED_BY).and_then(Value::as_str) {
                            Some(artifact) => format!("{} · made by the fix of {artifact}", text("role")),
                            None => text("role"),
                        };
                        lines.push(Line::Reference { number: index + 1, path, label });
                    }
                }
            }
            let send = match (script_only, count) {
                _ if research => String::from("File"),
                (true, _) => String::from("Run"),
                (false, 1) => String::from("Send request"),
                (false, _) => format!("Send {count} requests"),
            };
            (lines, send)
        }
        Asking::Searches { searches, why, .. } => {
            let noun = if searches.len() == 1 { "search" } else { "searches" };
            let mut lines = vec![Line::Title(format!("{} {noun} · {} · Brave Search", searches.len(), crate::runner::searches_cost(searches.len())))];
            if !why.trim().is_empty() {
                lines.push(Line::Muted(why.clone()));
            }
            lines.push(Line::Label(String::from("Searches, in the order sent")));
            let shown: Vec<String> = searches.iter().enumerate().map(|(index, (query, images))| format!("{}. {query}{}", index + 1, if *images { "  (images)" } else { "" })).collect();
            lines.push(Line::Boxed(shown.join("\n"), false));
            (lines, String::from("Search"))
        }
        Asking::Download { url, page, licence, to, .. } => {
            let mut lines = vec![Line::Title(String::from("Download this file?"))];
            lines.push(Line::Label(String::from("File")));
            lines.push(Line::Boxed(url.clone(), true));
            if !page.is_empty() {
                lines.push(Line::Label(String::from("On the page")));
                lines.push(Line::Boxed(page.clone(), true));
            }
            lines.push(Line::Label(String::from("Licence")));
            lines.push(Line::Muted(licence.clone()));
            lines.push(Line::Label(String::from("Waits in")));
            lines.push(Line::Muted(to.clone()));
            (lines, String::from("Download"))
        }
    }
}

/// Draws the card. Returns the person's answer: true for Send request or
/// Run, false for Cancel or Escape, None while they decide.
/// The path of every reference and file the card shows, so the window can
/// ask for previews of those that are not images.
pub(crate) fn reference_paths(asking: &Asking) -> Vec<String> {
    lines(asking, &|model| model.to_string())
        .0
        .into_iter()
        .filter_map(|line| match line {
            Line::Reference { path, .. } if !path.is_empty() => Some(path),
            _ => None,
        })
        .collect()
}

/// `previews` holds what the previewing workspace made of each reference
/// that is not an image, by its path.
pub(crate) fn show(ctx: &egui::Context, asking: &Asking, model_name: &dyn Fn(&str) -> String, previews: &HashMap<String, ReferencePreview>) -> Option<bool> {
    let (lines, send) = lines(asking, model_name);
    card(ctx, &lines, &send, previews)
}

/// A question on the same card, as `ig-dependency-warning` shows one: its
/// title, its lines in gray, then Cancel and a dark `yes`. Some(true) on
/// `yes`, Some(false) on Cancel or Escape.
pub(crate) fn ask(ctx: &egui::Context, title: &str, said: &[String], yes: &str) -> Option<bool> {
    let lines: Vec<Line> = std::iter::once(Line::Title(title.to_string())).chain(said.iter().map(|line| Line::Muted(line.clone()))).collect();
    card(ctx, &lines, yes, &HashMap::new())
}

fn card(ctx: &egui::Context, lines: &[Line], send: &str, previews: &HashMap<String, ReferencePreview>) -> Option<bool> {
    let send = send.to_string();
    let screen = ctx.content_rect();
    let mut answer = None;
    egui::Area::new(egui::Id::new("plan-card-overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (overlay, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(overlay, 0.0, OVERLAY);
    });
    let width = WIDTH.min(screen.width() - 32.0);
    egui::Area::new(egui::Id::new("plan-card")).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        let inner = width - 2.0 * PAD;
        egui::Frame::new().fill(Color32::WHITE).stroke(egui::Stroke::new(1.0, BORDER)).corner_radius(RADIUS).inner_margin(PAD).show(ui, |ui| {
            ui.set_width(inner);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let max_height = (screen.height() - 220.0).max(200.0);
            egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                for line in lines {
                    paint_line(ui, line, inner, previews);
                }
            });
            ui.add_space(18.0);
            let (bar, _) = ui.allocate_exact_size(vec2(inner, BUTTON_H), Sense::hover());
            let send_w = ui.painter().layout_no_wrap(send.clone(), widgets::semibold(ts::body_size()), Color32::WHITE).size().x + 52.0;
            let send_rect = Rect::from_min_size(pos2(bar.right() - send_w, bar.top()), vec2(send_w, BUTTON_H));
            let cancel_w = 96.0;
            let cancel_rect = Rect::from_min_size(pos2(send_rect.left() - 12.0 - cancel_w, bar.top()), vec2(cancel_w, BUTTON_H));
            let send_click = ui.interact(send_rect, egui::Id::new("plan-card-send"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let cancel_click = ui.interact(cancel_rect, egui::Id::new("plan-card-cancel"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(send_rect, 10.0, DARK);
            ui.painter().text(send_rect.center(), Align2::CENTER_CENTER, &send, widgets::semibold(ts::body_size()), Color32::WHITE);
            ui.painter().text(cancel_rect.center(), Align2::CENTER_CENTER, "Cancel", ts::body(), if cancel_click.hovered() { TEXT } else { MUTED });
            if send_click.clicked() {
                answer = Some(true);
            }
            if cancel_click.clicked() {
                answer = Some(false);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}

fn paint_line(ui: &mut egui::Ui, line: &Line, width: f32, previews: &HashMap<String, ReferencePreview>) {
    match line {
        Line::Title(text) => {
            let galley = ui.painter().layout(text.clone(), widgets::semibold(ts::heading(2).0), TEXT, width);
            let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 4.0), Sense::hover());
            ui.painter().galley(rect.min, galley, TEXT);
        }
        Line::Muted(text) => {
            let galley = ui.painter().layout(text.clone(), ts::body(), MUTED, width);
            let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 4.0), Sense::hover());
            ui.painter().galley(rect.min, galley, MUTED);
        }
        Line::Label(text) => {
            ui.add_space(16.0);
            let (rect, _) = ui.allocate_exact_size(vec2(width, ts::small_line() + 8.0), Sense::hover());
            ui.painter().text(pos2(rect.left(), rect.top()), Align2::LEFT_TOP, text, ts::small(), MUTED);
        }
        Line::Boxed(text, code) => {
            ui.add_space(4.0);
            let font = if *code { ts::code() } else { ts::body() };
            let galley = ui.painter().layout(text.clone(), font, TEXT, width - 32.0);
            let height = (galley.size().y + 28.0).min(PROMPT_MOST.max(galley.size().y.min(PROMPT_MOST)));
            let full = galley.size().y + 28.0;
            if full <= PROMPT_MOST {
                let (rect, _) = ui.allocate_exact_size(vec2(width, full), Sense::hover());
                ui.painter().rect_filled(rect, 10.0, BOX_FILL);
                ui.painter().galley(pos2(rect.left() + 16.0, rect.top() + 14.0), galley, TEXT);
            } else {
                // The whole prompt is always shown: a long one scrolls in its box.
                egui::Frame::new().fill(BOX_FILL).corner_radius(10.0).inner_margin(egui::Margin::symmetric(16, 14)).show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    egui::ScrollArea::vertical().id_salt(("plan-card-box", text.len())).max_height(height).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(text).font(if *code { ts::code() } else { ts::body() }).color(TEXT)).wrap());
                    });
                });
            }
        }
        Line::Reference { number, path, label } => {
            ui.add_space(8.0);
            let (row, _) = ui.allocate_exact_size(vec2(width, THUMB_H), Sense::hover());
            ui.painter().text(pos2(row.left() + 14.0, row.center().y), Align2::CENTER_CENTER, number.to_string(), ts::body(), MUTED);
            let thumb = Rect::from_min_size(pos2(row.left() + 36.0, row.top()), vec2(THUMB_W, THUMB_H));
            // A video's or a sound's preview, as its workspace made it, or
            // else the image itself.
            let preview = previews.get(path);
            let shown = match preview.and_then(|preview| preview.thumbnail.as_ref()) {
                Some(thumbnail) => crate::raster::raw_image(ui.ctx(), &thumbnail.path, thumbnail.width, thumbnail.height, thumbnail.revision, &[], false).map(|(texture, _)| (texture, vec2(thumbnail.width.max(1) as f32, thumbnail.height.max(1) as f32))),
                None => crate::chat::shown_texture(ui.ctx(), path).map(|texture| (texture.id(), texture.size_vec2())),
            };
            match shown {
                Some((texture, size)) => {
                    let scale = (THUMB_W / size.x).min(THUMB_H / size.y);
                    let image = Rect::from_center_size(thumb.center(), size * scale);
                    ui.painter().image(texture, image, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    widgets::field(ui.painter(), thumb, false, widgets::Scale::PANEL);
                }
            }
            if let Some(badge) = preview.map(|preview| preview.badge.as_str()).filter(|badge| !badge.is_empty()) {
                let galley = ui.painter().layout_no_wrap(badge.to_string(), ts::small(), Color32::WHITE);
                let size = galley.size() + vec2(8.0, 3.0);
                let spot = Rect::from_min_size(thumb.right_bottom() - size - vec2(3.0, 3.0), size);
                ui.painter().rect_filled(spot, 4.0, BADGE_FILL);
                ui.painter().galley(spot.center() - galley.size() / 2.0, galley, Color32::WHITE);
            }
            let left = thumb.right() + 16.0;
            let text_w = row.right() - left;
            let name = if label.is_empty() { String::from("Reference") } else { label.clone() };
            ui.painter().with_clip_rect(row).text(pos2(left, row.center().y - 10.0), Align2::LEFT_CENTER, name, ts::body(), TEXT);
            // The full path, wrapped under the label: a person finds files by it.
            let galley = ui.painter().layout(path.clone(), ts::small(), MUTED, text_w);
            let extra = (galley.size().y - ts::small_line()).max(0.0);
            ui.painter().galley(pos2(left, row.center().y + 2.0), galley, MUTED);
            if extra > 0.0 {
                ui.add_space(extra);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches the card reading a script's own words for having no price,
    /// or a request kind where the user knows a service.
    #[test]
    fn the_card_names_its_cost_and_service_as_the_user_reads_them() {
        assert_eq!(cost_words("unknown: no price for this endpoint"), "Cost unknown");
        assert_eq!(cost_words("about $0.24 a request"), "about $0.24 a request");
        assert_eq!(service_name("openai-image-edit"), "OpenAI");
        assert_eq!(service_name("openrouter-image"), "OpenRouter");
        assert_eq!(service_name("fal"), "fal.ai");
    }
}
