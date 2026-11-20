//! A request's report, read natively from the record its page embeds,
//! `<script type="application/json" id="request">` in `requests/<id>.html`,
//! never drawn as the page: what was asked and why, or the note written
//! afterwards, the prompt as sent, the references in the order sent, the
//! service, model, and settings, the cost, and the outputs, as the approved
//! report card (rp-anchor) shows them. `tell` says it in the words of
//! Spumcake-Skills' own `request.py tell`, which the checks hold it to.

use serde_json::Value;
use std::path::Path;

/// One reference, in the order it was sent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reference {
    /// Its path in the project; none when the report keeps it itself.
    pub file: Option<String>,
    pub role: String,
}

/// A request's report, as its record holds it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Report {
    /// `<time>-<name>`, such as "2026-11-14_13-30-keyframe-shot-12".
    pub id: String,
    pub name: String,
    pub skill: Option<String>,
    pub pipeline: Option<String>,
    pub asked: Option<String>,
    pub why: Option<String>,
    /// The note written for a report rebuilt afterwards, in place of what
    /// was asked and why.
    pub written_afterwards: Option<String>,
    pub service: Option<String>,
    pub model: Option<String>,
    /// The settings sent, each by its own name, such as ("seed", "1114").
    pub settings: Vec<(String, String)>,
    pub references: Vec<Reference>,
    pub outputs: Vec<String>,
    /// The cost as the record words it, such as "$0.07, as billed by
    /// OpenRouter".
    pub cost: Option<String>,
    /// Why the result was never filed, for a request closed as lost.
    pub lost: Option<String>,
    pub prompt: Option<String>,
    /// The report's path in the project, such as
    /// "requests/2026-11-14_13-30-keyframe-shot-12.html".
    pub path: String,
    /// The record itself, for what only `tell` says.
    record: Value,
}

impl Report {
    /// Reads the report at `path`, relative to the project.
    pub(crate) fn read(project: &Path, path: &str) -> Result<Report, String> {
        let text = std::fs::read_to_string(project.join(path)).map_err(|e| format!("{path} cannot be read: {e}"))?;
        let record = crate::skills_setup::record_of(&text).ok_or_else(|| format!("{path} holds no request record"))?;
        Ok(Report::of(record, path))
    }

    /// The report a record makes, read from `path` in the project.
    pub(crate) fn of(record: Value, path: &str) -> Report {
        let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let sent = &record["sent"];
        let settings = sent["settings"]
            .as_object()
            .map(|settings| settings.iter().map(|(name, value)| (name.clone(), py_str(value))).collect())
            .unwrap_or_default();
        let references = record["references"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|r| Reference { file: text(&r["file"]), role: r.get("role").map_or_else(String::new, py_str) })
                    .collect()
            })
            .unwrap_or_default();
        let outputs = record["outputs"].as_array().map(|list| list.iter().map(py_str).collect()).unwrap_or_default();
        let id = text(&record["id"]).unwrap_or_default();
        Report {
            name: text(&record["name"]).unwrap_or_else(|| id.get(17..).unwrap_or(&id).to_string()),
            id,
            skill: text(&record["skill"]),
            pipeline: text(&record["pipeline"]),
            asked: text(&record["asked"]),
            why: text(&record["why"]),
            written_afterwards: text(&record["written_afterwards"]),
            service: text(&sent["service"]),
            model: text(&sent["model"]),
            settings,
            references,
            outputs,
            cost: text(&record["result"]["cost"]),
            lost: text(&record["result"]["lost"]),
            prompt: text(&record["prompt"]),
            path: path.to_string(),
            record,
        }
    }

    /// The day and time it was made, from its id: "14 Nov, 13:30".
    pub(crate) fn made(&self) -> String {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let id = &self.id;
        let month = id.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
        let day = id.get(8..10).and_then(|d| d.parse::<u32>().ok());
        match (month, day, id.get(11..13), id.get(14..16)) {
            (Some(month), Some(day), Some(hour), Some(minute)) => format!("{day} {}, {hour}:{minute}", MONTHS[month - 1]),
            _ => String::new(),
        }
    }

    /// The cost as the card's title says it: "$0.24" as billed, "about
    /// $0.16" where only the estimate was recorded, or "cost unknown" when
    /// the record names no amount, as a lost request's does not.
    pub(crate) fn cost_short(&self) -> String {
        cost_words(self.cost.as_deref().unwrap_or_default(), self.lost.is_some())
    }

    /// The report as `request.py tell` says it, line for line, which the
    /// checks hold the card's model to.
    #[cfg(test)]
    pub(crate) fn tell(&self) -> String {
        let d = &self.record;
        let or = |value: &Value, fallback: &str| if truthy(value) { py_str(value) } else { fallback.to_string() };
        let sent = &d["sent"];
        let mut lines = vec![
            format!("**{}** ({})", py_str(&d["id"]), or(&d["skill"], "no skill named")),
            format!("- Asked: {}", or(&d["asked"], "not recorded")),
            format!("- Why: {}", or(&d["why"], "not recorded")),
            format!(
                "- Sent to: {}{}{}",
                sent.get("service").map_or_else(|| String::from("not recorded"), py_str),
                if truthy(&sent["model"]) { format!(", {}", py_str(&sent["model"])) } else { String::new() },
                if truthy(&sent["settings"]) { format!(", {}", py_dumps(&sent["settings"])) } else { String::new() },
            ),
        ];
        let references = d["references"].as_array().cloned().unwrap_or_default();
        lines.push(String::from(if references.is_empty() { "- References: none" } else { "- References, in the order sent:" }));
        for (i, r) in references.iter().enumerate() {
            let role = r.get("role").map_or_else(String::new, py_str);
            lines.push(format!("  {}. {}, {}", i + 1, or(&r["file"], "kept in the report"), role));
        }
        if let Some(canon) = d["canon"].as_array().filter(|c| !c.is_empty()) {
            lines.push(String::from("- Canon read:"));
            for c in canon {
                let at = |key: &str| c.get(key).map_or_else(String::new, py_str);
                lines.push(format!("  - {} = {} ({}, {})", py_str(&c["name"]), py_str(&c["value"]), at("from"), at("read_at")));
            }
        }
        if let Some(scripts) = d["scripts"].as_array().filter(|s| !s.is_empty()) {
            let said: Vec<String> =
                scripts.iter().map(|s| if truthy(&s["command"]) { py_str(&s["command"]) } else { s.get("file").map_or_else(String::new, py_str) }).collect();
            lines.push(format!("- Scripts: {}", said.join("; ")));
        }
        let outputs: Vec<String> = d["outputs"].as_array().map(|o| o.iter().map(py_str).collect()).unwrap_or_default();
        lines.push(format!("- Outputs: {}", if outputs.is_empty() { String::from("none") } else { outputs.join(", ") }));
        for s in d["shown"].as_array().into_iter().flatten() {
            lines.push(format!("- Shown in the report: {}", if truthy(&s["caption"]) { py_str(&s["caption"]) } else { py_str(&s["as"]) }));
        }
        lines.push(format!("- Cost: {}", or(&d["result"]["cost"], "not recorded")));
        lines.push(format!("- Report: {}", self.path));
        lines.push(String::from("- Prompt, as sent:"));
        lines.push(format!("```text\n{}\n```", or(&d["prompt"], "none")));
        lines.join("\n")
    }
}

/// A record's cost words as the card's title says them.
fn cost_words(cost: &str, lost: bool) -> String {
    let amount = |text: &str| text.split([',', ' ', ';']).next().and_then(|n| n.parse::<f64>().ok());
    if lost {
        return String::from("cost unknown");
    }
    if let Some(amount) = cost.strip_prefix('$').and_then(amount) {
        return format!("${amount:.2}");
    }
    match cost.strip_prefix("estimate $").and_then(amount) {
        Some(amount) => format!("about ${amount:.2}"),
        None => String::from("cost unknown"),
    }
}

/// What the card shows of an output beyond its record: its words, such as
/// "in use", and whether it is outlined as out of date.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OutputState {
    pub words: String,
    pub outlined: bool,
}

/// What the card knows of a report beyond its record, read from the
/// living pipeline's canon: the cell that made it, and each output's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    /// "the animatic · keyframe, shot 12": the recipe and its step.
    pub cell: Option<String>,
    pub outputs: std::collections::HashMap<String, OutputState>,
}

impl Placed {
    /// Reads where a report's request sits in the living pipeline, through
    /// the project's assets as the cells' rounds group them: the cell of its
    /// outputs' asset, and each output as an asset, "in use", "iteration n
    /// of m", or "not in use", outlined while it is in use and out of date.
    pub(crate) fn read(project: &Path, scripts: &Path, vars: &[(String, String)], report: &Report) -> Placed {
        let place = crate::project_panel::Scripts { scripts: scripts.to_path_buf(), vars: vars.to_vec() };
        let Some((assets, canon)) = crate::project_panel::grouped(project, Some(&place)) else { return Placed::default() };
        let of = |output: &String| assets.iter().find(|asset| asset.files().any(|file| file == output));
        let cell = report.outputs.iter().filter_map(of).find_map(|asset| crate::project_panel::cell_words(asset, &canon));
        let outputs = report
            .outputs
            .iter()
            .map(|output| {
                let state = match of(output) {
                    Some(asset) if asset.canonical() == output.as_str() && asset.in_use => OutputState { words: String::from("in use"), outlined: asset.outlined },
                    Some(asset) if asset.canonical() != output.as_str() => {
                        let at = asset.files().position(|file| file == output).unwrap_or(0);
                        OutputState { words: format!("iteration {} of {}", at + 1, asset.files().count()), outlined: false }
                    }
                    _ => OutputState { words: String::from("not in use"), outlined: false },
                };
                (output.clone(), state)
            })
            .collect();
        Placed { cell, outputs }
    }
}

/// What the user did on the card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CardAnswer {
    Close,
    /// Play a video output, by its path in the project.
    Play(String),
}

/// A setting's name as the card shows it: "Seed", "Images".
fn setting_label(name: &str) -> String {
    match name {
        "n" | "num_images" | "images" => String::from("Images"),
        "duration" => String::from("Length"),
        _ => {
            let words = name.replace('_', " ");
            let mut letters = words.chars();
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
        }
    }
}

/// A setting's value as the card shows it: a size "1536 × 864".
fn setting_value(name: &str, value: &str) -> String {
    match (name, value.split_once('x')) {
        ("size", Some((w, h))) if w.parse::<u32>().is_ok() && h.parse::<u32>().is_ok() => format!("{w} × {h}"),
        _ => value.to_string(),
    }
}

/// One part of the card, painted in order.
enum Part {
    Line(crate::plan_card::Line),
    /// Body text, dark.
    Text(String),
    /// The red mark, its words, and why, in grey.
    Warning(String, String),
    /// Model settings, a name and its value each.
    Rows(Vec<(String, String)>),
    Reference { number: usize, path: String, role: String },
    Outputs,
}

const THUMB: egui::Vec2 = egui::vec2(104.0, 60.0);
const RED: egui::Color32 = egui::Color32::from_rgb(220, 38, 38);
const BOX_FILL: egui::Color32 = egui::Color32::from_rgb(243, 244, 246);

use eframe::egui;

/// Draws the report card over a faint overlay, as rp-anchor shows it, and
/// returns what the user did. `previews` holds what a workspace made of an
/// output or a reference that is not an image, by its path.
pub(crate) fn show_card(
    ctx: &egui::Context,
    project: &Path,
    report: &Report,
    placed: &Placed,
    model_name: &dyn Fn(&str) -> String,
    previews: &std::collections::HashMap<String, crate::panel::ReferencePreview>,
) -> Option<CardAnswer> {
    use crate::plan_card::{BUTTON_H, Line, OVERLAY, PAD, RADIUS, WIDTH};
    use crate::typescale as ts;
    use crate::widgets::{BORDER, MUTED, TEXT};
    use egui::{Align2, Color32, Sense, pos2, vec2};
    let mut parts = vec![Part::Line(Line::Title(format!("{} · {}", report.name, report.cost_short())))];
    let mut about = Vec::new();
    if let Some(model) = &report.model {
        let service = report.service.clone().unwrap_or_default();
        let known = crate::plan_card::service_name(&service.to_lowercase());
        let service = if known == service.to_lowercase() { service } else { known };
        about.push(if service.is_empty() { model_name(model) } else { format!("{} on {service}", model_name(model)) });
    }
    about.push(report.made());
    if let Some(cell) = placed.cell.clone().or_else(|| report.skill.clone()) {
        about.push(cell);
    }
    parts.push(Part::Line(Line::Muted(about.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · "))));
    if let Some(why) = &report.lost {
        parts.push(Part::Warning(String::from("Likely spent, never filed"), why.clone()));
    }
    if report.asked.is_some() || report.why.is_some() {
        parts.push(Part::Line(Line::Label(String::from("Asked"))));
        parts.push(Part::Text(report.asked.clone().unwrap_or_else(|| String::from("not recorded"))));
        parts.push(Part::Line(Line::Label(String::from("Why"))));
        parts.push(Part::Text(report.why.clone().unwrap_or_else(|| String::from("not recorded"))));
    } else if let Some(note) = &report.written_afterwards {
        parts.push(Part::Line(Line::Muted(note.clone())));
    }
    parts.push(Part::Line(Line::Label(String::from("Prompt, as sent"))));
    parts.push(Part::Line(Line::Boxed(report.prompt.clone().unwrap_or_else(|| String::from("none")), false)));
    parts.push(Part::Line(Line::Label(String::from("References, in the order sent"))));
    if report.references.is_empty() {
        parts.push(Part::Line(Line::Muted(String::from("None."))));
    }
    for (index, reference) in report.references.iter().enumerate() {
        parts.push(Part::Reference { number: index + 1, path: reference.file.clone().unwrap_or_default(), role: reference.role.clone() });
    }
    if !report.settings.is_empty() {
        parts.push(Part::Line(Line::Label(String::from("Model"))));
        parts.push(Part::Rows(report.settings.iter().map(|(name, value)| (setting_label(name), setting_value(name, value))).collect()));
    }
    parts.push(Part::Line(Line::Label(String::from("Outputs"))));
    parts.push(if report.outputs.is_empty() { Part::Line(Line::Muted(String::from("None."))) } else { Part::Outputs });

    let screen = ctx.content_rect();
    let mut answer = None;
    egui::Area::new(egui::Id::new("report-card-overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (overlay, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(overlay, 0.0, OVERLAY);
    });
    let width = WIDTH.min(screen.width() - 32.0);
    let no_previews = std::collections::HashMap::new();
    egui::Area::new(egui::Id::new("report-card")).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        let inner = width - 2.0 * PAD;
        egui::Frame::new().fill(Color32::WHITE).stroke(egui::Stroke::new(1.0, BORDER)).corner_radius(RADIUS).inner_margin(PAD).show(ui, |ui| {
            ui.set_width(inner);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let max_height = (screen.height() - 180.0).max(200.0);
            egui::ScrollArea::vertical()
                .id_salt(("report-card", &report.id))
                .max_height(max_height)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                for part in &parts {
                    match part {
                        Part::Line(line) => crate::plan_card::paint_line(ui, line, inner, &no_previews),
                        Part::Text(text) => {
                            let galley = ui.painter().layout(text.clone(), ts::body(), TEXT, inner);
                            let (rect, _) = ui.allocate_exact_size(vec2(inner, galley.size().y + 4.0), Sense::hover());
                            ui.painter().galley(rect.min, galley, TEXT);
                        }
                        Part::Warning(words, why) => {
                            ui.add_space(10.0);
                            let galley = ui.painter().layout(why.clone(), ts::body(), MUTED, inner - 30.0);
                            let (rect, _) = ui.allocate_exact_size(vec2(inner, 26.0 + galley.size().y), Sense::hover());
                            crate::widgets::disc(ui.painter(), pos2(rect.left() + 11.0, rect.top() + 11.0), 11.0, RED);
                            ui.painter().text(pos2(rect.left() + 11.0, rect.top() + 11.0), Align2::CENTER_CENTER, "!", crate::widgets::semibold(ts::body_size()), Color32::WHITE);
                            ui.painter().text(pos2(rect.left() + 30.0, rect.top() + 11.0), Align2::LEFT_CENTER, words, ts::body(), RED);
                            ui.painter().galley(pos2(rect.left() + 30.0, rect.top() + 24.0), galley, MUTED);
                        }
                        Part::Rows(rows) => {
                            for (name, value) in rows {
                                let (row, _) = ui.allocate_exact_size(vec2(inner, ts::body_size() + 8.0), Sense::hover());
                                ui.painter().text(pos2(row.left(), row.center().y), Align2::LEFT_CENTER, name, ts::body(), TEXT);
                                // A long value, such as an address, is cut to the card.
                                let left = row.left() + inner * 0.55;
                                let galley = cut(ui, value, ts::body(), TEXT, row.right() - left);
                                ui.painter().galley(pos2(left, row.center().y - galley.size().y / 2.0), galley, TEXT);
                            }
                        }
                        Part::Reference { number, path, role } => {
                            ui.add_space(8.0);
                            let (row, _) = ui.allocate_exact_size(vec2(inner, THUMB.y), Sense::hover());
                            ui.painter().text(pos2(row.left() + 14.0, row.center().y), Align2::CENTER_CENTER, number.to_string(), ts::body(), MUTED);
                            let thumb = egui::Rect::from_min_size(pos2(row.left() + 36.0, row.top()), THUMB);
                            shown_file(ui, project, path, previews, thumb, false);
                            let left = thumb.right() + 16.0;
                            let name = if role.is_empty() { String::from("Reference") } else { role.clone() };
                            let painter = ui.painter().with_clip_rect(row);
                            let named = cut(ui, &name, ts::body(), TEXT, row.right() - left);
                            painter.galley(pos2(left, row.center().y - 10.0 - named.size().y / 2.0), named, TEXT);
                            let file = if path.is_empty() { String::from("kept in the report") } else { path.rsplit('/').next().unwrap_or(path).to_string() };
                            painter.text(pos2(left, row.center().y + 10.0), Align2::LEFT_CENTER, file, ts::small(), MUTED);
                        }
                        Part::Outputs => {
                            ui.add_space(6.0);
                            let gap = 12.0;
                            let video = report.outputs.iter().any(|output| crate::project_panel::Kind::of(output) == Some(crate::project_panel::Kind::Video));
                            let sound = report.outputs.iter().all(|output| crate::project_panel::Kind::of(output) == Some(crate::project_panel::Kind::Sound));
                            let across = if sound { 1 } else if video { 2 } else { 4 };
                            let tile_w = (inner - gap * (across - 1) as f32) / across as f32;
                            let tile_h = if sound { 80.0 } else { tile_w * 9.0 / 16.0 };
                            for chunk in report.outputs.chunks(across) {
                                let (band, _) = ui.allocate_exact_size(vec2(inner, tile_h + 26.0), Sense::hover());
                                for (i, output) in chunk.iter().enumerate() {
                                    let tile = egui::Rect::from_min_size(pos2(band.left() + i as f32 * (tile_w + gap), band.top()), vec2(tile_w, tile_h));
                                    let state = placed.outputs.get(output).cloned().unwrap_or_default();
                                    if shown_file(ui, project, output, previews, tile, true) {
                                        answer = Some(CardAnswer::Play(output.clone()));
                                    }
                                    if state.outlined {
                                        ui.painter().rect_stroke(tile, 2.0, egui::Stroke::new(2.0, RED), egui::StrokeKind::Inside);
                                    }
                                    ui.painter().text(pos2(tile.center().x, tile.bottom() + 13.0), Align2::CENTER_CENTER, &state.words, ts::small(), MUTED);
                                }
                                ui.add_space(6.0);
                            }
                        }
                    }
                }
            });
            ui.add_space(18.0);
            let (bar, _) = ui.allocate_exact_size(vec2(inner, BUTTON_H), Sense::hover());
            let close = egui::Rect::from_min_size(pos2(bar.right() - 96.0, bar.top()), vec2(96.0, BUTTON_H));
            let click = ui.interact(close, egui::Id::new("report-card-close"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().rect_stroke(close, 10.0, egui::Stroke::new(1.0, if click.hovered() { MUTED } else { BORDER }), egui::StrokeKind::Inside);
            ui.painter().text(close.center(), Align2::CENTER_CENTER, "Close", ts::body(), TEXT);
            if click.clicked() {
                answer = Some(CardAnswer::Close);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        answer = Some(CardAnswer::Close);
    }
    answer
}

/// One line of text cut to `width` with an ellipsis.
pub(crate) fn cut(ui: &egui::Ui, text: &str, font: egui::FontId, color: egui::Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

/// A file shown in `rect`: an image itself, or what its workspace made
/// of a video or a sound, with its length; a video carries Play, and
/// returns true when Play was clicked.
pub(crate) fn shown_file(
    ui: &mut egui::Ui,
    project: &Path,
    path: &str,
    previews: &std::collections::HashMap<String, crate::panel::ReferencePreview>,
    rect: egui::Rect,
    playable: bool,
) -> bool {
    use egui::{Color32, Pos2, Sense, pos2, vec2};
    let preview = previews.get(path);
    let shown = match preview.and_then(|preview| preview.thumbnail.as_ref()) {
        Some(thumbnail) => crate::raster::raw_image(ui.ctx(), &thumbnail.path, thumbnail.width, thumbnail.height, thumbnail.revision, &[], false)
            .map(|(texture, _)| (texture, vec2(thumbnail.width.max(1) as f32, thumbnail.height.max(1) as f32))),
        None if !path.is_empty() => crate::chat::shown_texture(ui.ctx(), &project.join(path).display().to_string()).map(|texture| (texture.id(), texture.size_vec2())),
        None => None,
    };
    match shown {
        Some((texture, size)) => {
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let image = egui::Rect::from_center_size(rect.center(), size * scale);
            ui.painter().image(texture, image, egui::Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(rect, 4.0, BOX_FILL);
        }
    }
    if let Some(badge) = preview.map(|preview| preview.badge.as_str()).filter(|badge| !badge.is_empty()) {
        let galley = ui.painter().layout_no_wrap(badge.to_string(), crate::typescale::small(), Color32::WHITE);
        let size = galley.size() + vec2(8.0, 3.0);
        let spot = egui::Rect::from_min_size(rect.right_bottom() - size - vec2(4.0, 4.0), size);
        ui.painter().rect_filled(spot, 4.0, crate::plan_card::BADGE_FILL);
        ui.painter().galley(spot.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
    if playable && crate::project_panel::Kind::of(path) == Some(crate::project_panel::Kind::Video) {
        let play = egui::Rect::from_center_size(rect.center(), vec2(40.0, 40.0));
        let response = ui.interact(play, egui::Id::new(("report-card-play", path)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        crate::widgets::disc(ui.painter(), play.center(), 20.0, crate::plan_card::BADGE_FILL);
        crate::toolbar::paint_icon(ui.painter(), play.center() + vec2(2.0, 0.0), "play-solid", 18.0, Color32::WHITE);
        return response.clicked();
    }
    false
}

/// Whether Python reads a value as true, as `request.py`'s `or` does.
#[cfg(test)]
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A value as Python's `%s` prints it.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => String::from("None"),
        Value::Bool(true) => String::from("True"),
        Value::Bool(false) => String::from("False"),
        Value::String(s) => s.clone(),
        other => py_dumps(other),
    }
}

/// A value as Python's `json.dumps` writes it: ", " and ": " between items,
/// and every character past ASCII escaped. Keys keep the record's order
/// only as far as `serde_json` keeps it, so the checks compare settings as
/// values.
fn py_dumps(value: &Value) -> String {
    match value {
        Value::Array(items) => format!("[{}]", items.iter().map(py_dumps).collect::<Vec<_>>().join(", ")),
        Value::Object(fields) => {
            let items: Vec<String> = fields.iter().map(|(k, v)| format!("{}: {}", py_dumps(&Value::String(k.clone())), py_dumps(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
        Value::String(s) => {
            let mut out = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                        let mut units = [0u16; 2];
                        for unit in c.encode_utf16(&mut units) {
                            out.push_str(&format!("\\u{unit:04x}"));
                        }
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keyframe() -> Report {
        Report::of(
            json!({
                "record": 2,
                "id": "2026-11-14_13-30-keyframe-shot-12",
                "name": "keyframe-shot-12",
                "skill": "generate-keyframe",
                "pipeline": "002",
                "asked": "Make the keeper turning from the window as the radio crackles.",
                "why": "The keyframe for shot 12, from its layout.",
                "sent": { "service": "OpenRouter", "model": "openai/gpt-image-2.5-sunburst", "settings": { "seed": 1114, "n": 4 } },
                "references": [
                    { "file": "library/previs-shot-12-grey.png", "role": "Layout, the room" },
                    { "file": null, "role": "image 2" },
                ],
                "outputs": ["library/2026-11-14_13-30-keyframe-shot-12-1.png", "library/2026-11-14_13-30-keyframe-shot-12-2.png"],
                "result": { "cost": "$0.2391, as OpenRouter reported it" },
                "prompt": "Shot 12. The keeper turns — from the window.",
            }),
            "requests/2026-11-14_13-30-keyframe-shot-12.html",
        )
    }

    #[test]
    fn a_report_reads_as_its_card_shows_it() {
        let report = keyframe();
        assert_eq!(report.made(), "14 Nov, 13:30");
        assert_eq!(report.cost_short(), "$0.24");
        assert_eq!(report.references[1], Reference { file: None, role: String::from("image 2") });
        assert_eq!(report.settings, vec![(String::from("n"), String::from("4")), (String::from("seed"), String::from("1114"))]);
        let tell = report.tell();
        assert!(tell.starts_with("**2026-11-14_13-30-keyframe-shot-12** (generate-keyframe)\n- Asked: Make the keeper"), "{tell}");
        assert!(tell.contains("\n  2. kept in the report, image 2\n"), "{tell}");
        assert!(tell.ends_with("- Prompt, as sent:\n```text\nShot 12. The keeper turns — from the window.\n```"), "{tell}");
        assert_eq!(py_dumps(&json!({ "quality": "high", "note": "— é" })), r#"{"note": "\u2014 \u00e9", "quality": "high"}"#);
    }

    #[test]
    fn a_cost_reads_as_billed_estimated_or_unknown() {
        assert_eq!(cost_words("$0.0436, as OpenRouter reported it", false), "$0.04");
        assert_eq!(cost_words("estimate $0.16; billed not returned", false), "about $0.16");
        assert_eq!(cost_words("not returned by the endpoint", false), "cost unknown");
        assert_eq!(cost_words("see response.json", false), "cost unknown");
        assert_eq!(cost_words("$0.24", true), "cost unknown");
    }

    #[test]
    fn a_lost_request_has_no_amount() {
        let lost = Report::of(
            json!({ "id": "2026-11-15_09-00-storm-plate-3", "result": { "cost": "likely spent, never filed: about $0.24", "lost": "The disk was full." } }),
            "requests/2026-11-15_09-00-storm-plate-3.html",
        );
        assert_eq!(lost.name, "storm-plate-3");
        assert_eq!(lost.cost_short(), "cost unknown");
        assert_eq!(lost.lost.as_deref(), Some("The disk was full."));
        assert!(lost.tell().contains("\n- Asked: not recorded\n- Why: not recorded\n- Sent to: not recorded\n- References: none\n- Outputs: none\n"));
    }

    /// Every one of Last order's reports, read by the card's model, says what
    /// Skills' own `request.py tell` says, line for line; the settings are
    /// compared as values, since their key order is the record's. Runs where
    /// `LAST_ORDER` and `SPUMCAKE_SKILLS_SCRIPTS` are set, as check.sh sets
    /// them; reads Last order and writes nothing.
    #[test]
    fn every_report_of_last_order_tells_as_skills_does() {
        let (Some(project), Some(scripts)) = (std::env::var_os("LAST_ORDER"), std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS")) else {
            eprintln!("LAST_ORDER or SPUMCAKE_SKILLS_SCRIPTS is not set, so the reports are not compared with request.py tell");
            return;
        };
        let project = Path::new(&project);
        // One run of Skills' own tell for every report, rather than one a
        // report.
        let told = std::process::Command::new("python3")
            .args(["-c", TELL_ALL])
            .arg(&scripts)
            .current_dir(project)
            .output()
            .unwrap();
        assert!(told.status.success(), "{}", String::from_utf8_lossy(&told.stderr));
        let told: serde_json::Map<String, Value> = serde_json::from_slice(&told.stdout).unwrap();
        let reports: Vec<String> = std::fs::read_dir(project.join("requests"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".html"))
            .collect();
        assert_eq!(told.len(), reports.len(), "request.py told a different number of reports");
        assert!(reports.len() >= 439, "Last order has {} reports", reports.len());
        for name in reports {
            let path = format!("requests/{name}");
            let report = Report::read(project, &path).unwrap();
            let skills = told[&path].as_str().unwrap();
            // What the card draws, field by field, against what tell says:
            // the prompt, the references in order, the model, and the cost.
            let prompt = skills.split("- Prompt, as sent:\n```text\n").nth(1).and_then(|rest| rest.strip_suffix("\n```")).unwrap();
            assert_eq!(report.prompt.as_deref().unwrap_or("none"), prompt, "{path}: the prompt");
            let sent: Vec<String> = skills.lines().skip_while(|line| !line.starts_with("- References")).skip(1).take_while(|line| line.starts_with("  ")).map(str::to_string).collect();
            let drawn: Vec<String> = report
                .references
                .iter()
                .enumerate()
                .map(|(i, r)| format!("  {}. {}, {}", i + 1, r.file.as_deref().unwrap_or("kept in the report"), r.role))
                .collect();
            assert_eq!(drawn, sent, "{path}: the references in order");
            let to = skills.lines().find_map(|line| line.strip_prefix("- Sent to: ")).unwrap();
            if let Some(model) = &report.model {
                assert!(to.contains(&format!(", {model}")), "{path}: the model {model} is not tell's {to}");
            }
            let cost = skills.lines().find_map(|line| line.strip_prefix("- Cost: ")).unwrap();
            assert_eq!(report.cost_short(), cost_words(cost, report.lost.is_some()), "{path}: the cost");
            let ours = report.tell();
            let (skills_lines, our_lines): (Vec<&str>, Vec<&str>) = (skills.lines().collect(), ours.lines().collect());
            assert_eq!(skills_lines.len(), our_lines.len(), "{path}:\n{skills}\n---\n{ours}");
            for (theirs, mine) in skills_lines.iter().zip(&our_lines) {
                if theirs.starts_with("- Sent to: ") {
                    let settings = |line: &str| line.find(", {").map(|at| (line[..at].to_string(), serde_json::from_str::<Value>(&line[at + 2..]).ok()));
                    assert_eq!(settings(theirs), settings(mine), "{path}: {theirs} | {mine}");
                    assert_eq!(theirs.find(", {").is_none(), mine.find(", {").is_none(), "{path}");
                    if theirs.find(", {").is_none() {
                        assert_eq!(theirs, mine, "{path}");
                    }
                } else {
                    assert_eq!(theirs, mine, "{path}");
                }
            }
        }
    }

    /// Prints `{"requests/<id>.html": <tell>}` for every report in
    /// `requests/`, through request.py's own `_tell_to`.
    const TELL_ALL: &str = r#"
import io, json, os, sys
sys.path.insert(0, sys.argv[1])
import request
root = os.getcwd()
told = {}
for name in sorted(os.listdir("requests")):
    if name.endswith(".html"):
        out = io.StringIO()
        request._tell_to(root, os.path.join(root, "requests", name), out)
        told["requests/" + name] = out.getvalue().rstrip("\n")
print(json.dumps(told))
"#;
}
