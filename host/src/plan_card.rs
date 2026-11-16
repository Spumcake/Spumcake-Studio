//! The plan card: what the runner asks the person before anything runs, as
//! the approved view `ig-request-confirm` shows it, over a faint overlay.
//! Before a request it shows how many and the cost, the model and its
//! settings, the prompt as sent, and every reference in the order sent with
//! its full path, then Cancel and Send request. Before any other command it
//! asks "Run this command?" with the command in full, then Cancel and Run.

use crate::runner::Asking;
use crate::typescale as ts;
use crate::widgets::{self, BORDER, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, pos2, vec2};
use serde_json::Value;

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

fn lines(asking: &Asking) -> (Vec<Line>, String) {
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
            let cost = if *stand_in {
                String::from("nothing; a stand-in answers")
            } else {
                plans.iter().filter_map(|plan| plan.get("estimate").and_then(Value::as_str)).collect::<Vec<_>>().join(", ")
            };
            let mut lines = vec![Line::Title(format!("{count} {noun} · {cost}"))];
            for plan in plans {
                let endpoint = plan.get("endpoint").cloned().unwrap_or(Value::Null);
                if !endpoint.is_null() {
                    let model = endpoint.get("model").and_then(Value::as_str).unwrap_or("its default model");
                    let kind = endpoint.get("kind").and_then(Value::as_str).unwrap_or("");
                    lines.push(Line::Muted(format!("{model} on {kind}")));
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
                let references = plan.get("references").and_then(Value::as_array).cloned().unwrap_or_default();
                if !references.is_empty() {
                    lines.push(Line::Label(String::from("References, in the order sent")));
                    for (index, reference) in references.iter().enumerate() {
                        let text = |key: &str| reference.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                        let path = if text("path").is_empty() { text("file") } else { text("path") };
                        lines.push(Line::Reference { number: index + 1, path, label: text("role") });
                    }
                }
            }
            let send = match (script_only, count) {
                (true, _) => String::from("Run"),
                (false, 1) => String::from("Send request"),
                (false, _) => format!("Send {count} requests"),
            };
            (lines, send)
        }
    }
}

/// Draws the card. Returns the person's answer: true for Send request or
/// Run, false for Cancel or Escape, None while they decide.
pub(crate) fn show(ctx: &egui::Context, asking: &Asking) -> Option<bool> {
    let (lines, send) = lines(asking);
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
                for line in &lines {
                    paint_line(ui, line, inner);
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

fn paint_line(ui: &mut egui::Ui, line: &Line, width: f32) {
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
            match crate::chat::shown_texture(ui.ctx(), path) {
                Some(texture) => {
                    let size = texture.size_vec2();
                    let scale = (THUMB_W / size.x).min(THUMB_H / size.y);
                    let image = Rect::from_center_size(thumb.center(), size * scale);
                    ui.painter().image(texture.id(), image, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    widgets::field(ui.painter(), thumb, false, widgets::Scale::PANEL);
                }
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
