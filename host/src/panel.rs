//! The properties panel, from Simple Sketch 1.0.1's `panel.rs`. It draws each
//! declared row from its kind and sends a property or an action. Every row
//! sits on one grid — a label column, then the control, then a value field —
//! so rows line up from section to section, and every control comes from
//! `widgets`, shared with the toolbar. A page can hold a cell as a card, the
//! Prompt page's: its header, prompt, references, settings, and cost.

use crate::widgets::{self, BORDER, INK, MUTED, Scale, TEXT};
use draw::document::{Cap, CurvePoint, Dash, Sample};
use draw::ink;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Painter, PopupCloseBehavior, Rect, Sense,
    Stroke, StrokeKind, UiBuilder, pos2, vec2,
};
use protocol::{
    Action, Cell, CellPart, CellState, ChoicePreview, PanelCell, PresetSample, PropValue, Properties, ReferenceRole, Row,
    RowKind,
};
use serde_json::{Value, json};
use std::collections::HashMap;

/// What the panel and the outliner send.
#[derive(Clone)]
pub(crate) enum Sent {
    Run(Action),
    Property {
        prop: Value,
        value: PropValue,
        preview: bool,
    },
}

/// The panel draws its contents at this scale of the toolbar's size.
const S: Scale = Scale::PANEL;

/// The contents, 268 points wide at full size, at the panel's scale, plus
/// the side padding, rounded to a whole point so the edge stays crisp.
pub(crate) const WIDTH: f32 = 266.0;

const PAD: f32 = 16.0;
const ROW: f32 = S.of(30.0);
const ROW_GAP: f32 = S.of(8.0);
const LABEL_W: f32 = S.of(92.0);
const NUMBER_W: f32 = S.of(56.0);
const GAP: f32 = S.of(10.0);
const PRESET_SAMPLE_W: f32 = S.of(84.0);
const CURVE_H: f32 = S.of(140.0);
/// Space above the first heading. The panel has no title bar.
const TOP: f32 = S.of(22.0);
const HEADING: f32 = S.of(20.0);
/// How far below the panel's top a page's first line of text is centered:
/// the first section heading, or the outliner's header row.
pub(crate) const FIRST_LINE: f32 = TOP + HEADING / 2.0;
const TAB: egui::Vec2 = vec2(22.0, 40.0);
/// The tab's distance below the top of the canvas.
const TAB_TOP: f32 = 18.0;

const PLOT: Color32 = Color32::from_rgb(247, 248, 250);
const PLOT_GRID: Color32 = Color32::from_rgb(232, 234, 238);
const ERASER_SAMPLE: Color32 = Color32::from_rgb(160, 164, 170);

/// Draws the panel in `rect`: the declared properties, or an empty column
/// when the tab declares the panel but no properties.
pub(crate) fn show(ui: &mut egui::Ui, properties: Option<&Properties>, rect: Rect) -> Vec<Sent> {
    show_with(ui, properties, &CardState::default(), rect)
}

/// As `show`, with what a declared cell's card shows beyond its
/// declaration. The card's own requests wait for `take_card_events`.
pub(crate) fn show_with(ui: &mut egui::Ui, properties: Option<&Properties>, card: &CardState, rect: Rect) -> Vec<Sent> {
    let mut sent = Vec::new();
    let mut events = Vec::new();
    ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    ui.painter()
        .vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, BORDER));

    // Too narrow for the padding: nothing can be drawn there this frame.
    if rect.width() <= 2.0 * PAD {
        return sent;
    }
    let content = rect.shrink2(vec2(PAD, 0.0));
    let mut panel = ui.new_child(
        UiBuilder::new()
            .max_rect(content)
            .layout(Layout::top_down(Align::Min)),
    );
    panel.set_clip_rect(rect);
    panel.spacing_mut().item_spacing = vec2(0.0, 0.0);
    // No scroll bars: the page scrolls with the wheel or by dragging it.
    let scrolled = egui::ScrollArea::vertical()
        .id_salt("properties-panel")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: false,
            drag: egui::scroll_area::DragScroll::Always,
            mouse_wheel: true,
        })
        .show(&mut panel, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.set_width(content.width());
            let Some(properties) = properties else {
                ui.add_space(PAD);
                return;
            };
            // With page tabs, as the gallery mockup shows: the tabs,
            // then the chosen layer's settings, then the list of layers.
            if !properties.pages.is_empty() {
                page_tabs(ui, &properties.pages, &mut sent);
                // The Prompt page's cell, after the tabs and before the
                // sections.
                if let Some(cell) = &properties.cell {
                    prompt_card(ui, cell, card, &mut sent, &mut events);
                }
                for (index, section) in properties.sections.iter().enumerate() {
                    draw_section(ui, section, &properties.finish, index == 0 && properties.cell.is_none(), &mut sent);
                }
                if let Some(layers) = &properties.layers {
                    if !properties.sections.is_empty() {
                        ui.add_space(S.of(14.0));
                        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, BORDER));
                        ui.add_space(S.of(4.0));
                    }
                    crate::outliner::show(ui, layers, true, &mut sent);
                }
                ui.add_space(PAD);
                return;
            }
            if let Some(layers) = &properties.layers {
                crate::outliner::show(ui, layers, false, &mut sent);
            }
            // A page opened from another, such as a color from an object's
            // palette or the outliner, starts with the way back to it.
            if let Some(back) = &properties.back {
                back_row(ui, &back.label, &back.action, &mut sent);
            }
            if let Some(cell) = &properties.cell {
                prompt_card(ui, cell, card, &mut sent, &mut events);
            }
            for (index, section) in properties.sections.iter().enumerate() {
                draw_section(
                    ui,
                    section,
                    &properties.finish,
                    index == 0 && properties.back.is_none() && properties.cell.is_none(),
                    &mut sent,
                );
            }
            ui.add_space(PAD);
        });
    widgets::drag_scroll_cursor(ui, scrolled.id);
    if !events.is_empty() {
        ui.data_mut(|data| data.get_temp_mut_or_default::<Vec<CardEvent>>(card_events_id()).extend(events));
    }
    sent
}

/// The tab that shows and hides the panel. It sits at the panel's left edge,
/// or at the canvas's right edge while the panel is closed, on a layer above
/// the canvas so a click on it never draws. Returns whether it was clicked.
pub(crate) fn toggle_tab(
    ctx: &egui::Context,
    open: bool,
    tip: &str,
    edge_x: f32,
    top: f32,
) -> bool {
    let origin = pos2(edge_x - TAB.x, top + TAB_TOP);
    let mut clicked = false;
    egui::Area::new(egui::Id::new("panel-toggle"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            // One point wider than the tab, so it covers the panel's hairline
            // and joins the panel instead of sitting beside it.
            let (rect, response) = ui.allocate_exact_size(TAB + vec2(1.0, 0.0), Sense::click());
            let response = response.on_hover_text(tip);
            let radius = CornerRadius {
                nw: 10,
                sw: 10,
                ne: 0,
                se: 0,
            };
            let fill = if response.hovered() {
                widgets::SELECTED
            } else {
                Color32::WHITE
            };
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, BORDER),
                StrokeKind::Inside,
            );
            if open {
                let seam = Rect::from_min_max(
                    pos2(rect.right() - 1.5, rect.top() + 1.0),
                    pos2(rect.right(), rect.bottom() - 1.0),
                );
                ui.painter().rect_filled(seam, 0.0, fill);
            }
            // Centered on the tab's visible inside: from the inner edge of
            // its border to the panel edge, which the tab overlaps by one
            // point. The rounded left side is part of that inside.
            let inside = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top()), rect.max);
            let center = pos2(inside.center().x + 0.5, rect.center().y);
            widgets::chevron_side(ui.painter(), center, open, TEXT);
            clicked = response.clicked();
        });
    clicked
}

/// The page tabs' height, and the space above them.
pub(crate) const PAGE_H: f32 = S.of(40.0);
pub(crate) const PAGE_TOP: f32 = S.of(14.0);
/// A page tab's words, and the room on either side of them.
const PAGE_TEXT: f32 = S.of(15.5);
const PAGE_PAD: f32 = S.of(14.0);
/// The least room beside a tab's words while the tabs fit at their size,
/// and the least once they no longer do.
const PAGE_PAD_MIN: f32 = S.of(4.0);
const PAGE_PAD_TIGHT: f32 = S.of(2.5);
/// A tab's words one step smaller, for tabs too wide for the panel.
const PAGE_TEXT_SMALL: f32 = S.of(14.0);

/// How a row of page tabs fits its width: the words' size, the room on
/// either side of each, each tab's words' width, and the tab whose words
/// are cut with "…", if one is.
#[derive(Clone, Debug, PartialEq)]
struct TabRow {
    size: f32,
    pad: f32,
    widths: Vec<f32>,
    elided: Option<usize>,
}

/// Fits `count` tabs in `width`, `measure` giving tab `index`'s words'
/// width at a size. Tabs that fit are laid out as they always were. Too
/// wide, the room beside the words tightens; then the words are a step
/// smaller; then the longest is cut to what is left.
fn tab_row(width: f32, count: usize, measure: impl Fn(usize, f32) -> f32) -> TabRow {
    let sides = 2.0 * count.max(1) as f32;
    let at = |size: f32| -> (Vec<f32>, f32) {
        let widths: Vec<f32> = (0..count).map(|index| measure(index, size)).collect();
        let words = widths.iter().sum();
        (widths, words)
    };
    let (widths, words) = at(PAGE_TEXT);
    // As before: narrow panels squeeze the room around the words first.
    let pad = PAGE_PAD.min(((width - words) / sides).max(PAGE_PAD_MIN));
    if words + sides * pad <= width {
        return TabRow { size: PAGE_TEXT, pad, widths, elided: None };
    }
    if words + sides * PAGE_PAD_TIGHT <= width {
        let pad = ((width - words) / sides).min(PAGE_PAD_MIN);
        return TabRow { size: PAGE_TEXT, pad, widths, elided: None };
    }
    let (mut widths, words) = at(PAGE_TEXT_SMALL);
    let pad = ((width - words) / sides).clamp(PAGE_PAD_TIGHT, PAGE_PAD_MIN);
    if words + sides * pad <= width {
        return TabRow { size: PAGE_TEXT_SMALL, pad, widths, elided: None };
    }
    let longest = (0..count).max_by(|a, b| widths[*a].total_cmp(&widths[*b]));
    if let Some(longest) = longest {
        let others: f32 = widths.iter().enumerate().filter(|(index, _)| *index != longest).map(|(_, width)| width).sum();
        widths[longest] = (width - sides * pad - others).max(0.0);
    }
    TabRow { size: PAGE_TEXT_SMALL, pad, widths, elided: longest }
}

/// Page tabs across the panel's top, as the gallery mockup shows: the
/// chosen page underlined, the others in gray,
/// and a page that waits lighter still. A click on another page that can be
/// used sends its action.
fn page_tabs(ui: &mut egui::Ui, pages: &[protocol::PanelPage], sent: &mut Vec<Sent>) {
    ui.add_space(PAGE_TOP);
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, PAGE_H), Sense::hover());
    let fitted = tab_row(width, pages.len(), |index, size| {
        ui.painter().layout_no_wrap(pages[index].label.clone(), page_font(pages[index].active, size), TEXT).size().x
    });
    let galleys: Vec<_> = pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let mut job = egui::text::LayoutJob::simple_singleline(page.label.clone(), page_font(page.active, fitted.size), TEXT);
            if fitted.elided == Some(index) {
                job.wrap = egui::text::TextWrapping::truncate_at_width(fitted.widths[index].max(1.0));
            }
            ui.painter().layout_job(job)
        })
        .collect();
    let pad = fitted.pad;
    let mut x = row.left();
    for (index, (page, galley)) in pages.iter().zip(galleys).enumerate() {
        let tab = Rect::from_min_size(pos2(x, row.top()), vec2(galley.size().x + 2.0 * pad, PAGE_H));
        x = tab.right();
        let usable = page.enabled && !page.active && page.action.is_some();
        let sense = if usable { Sense::click() } else { Sense::hover() };
        let mut response = ui.interact(tab, ui.id().with(("panel-page", index)), sense);
        // A cut name shows whole under the pointer.
        if fitted.elided == Some(index) {
            response = response.on_hover_text(&page.label);
        }
        // The chosen page is marked by its underline alone, with no raised
        // tab or shadow, as the owner set on 2026-10-30.
        if page.active {
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(tab.left() + pad, tab.bottom() - 2.0), pos2(tab.right() - pad, tab.bottom())),
                0.0,
                INK,
            );
        }
        let color = match (page.active, page.enabled, usable && response.hovered()) {
            (true, _, _) => TEXT,
            (false, false, _) => widgets::quiet(false),
            (false, true, true) => TEXT,
            (false, true, false) => MUTED,
        };
        let at = pos2(tab.center().x - galley.size().x / 2.0, tab.center().y - galley.size().y / 2.0);
        ui.painter().galley_with_override_text_color(at, galley, color);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, page.enabled, page.active, &page.label)
        });
        if usable
            && response.clicked()
            && let Some(action) = &page.action
        {
            sent.push(Sent::Run(action.clone()));
        }
    }
}

/// A page tab's words: the chosen page's semibold.
fn page_font(active: bool, size: f32) -> egui::FontId {
    if active { widgets::semibold(size) } else { egui::FontId::proportional(size) }
}

/// A caption and a row of icons that each turn on and off, as the mockup
/// draws "Lock:" and its four locks. A click sends the icon's index.
fn icons_row(ui: &mut egui::Ui, prop: &Value, row: Rect, label: &str, icons: &[protocol::IconToggle], sent: &mut Vec<Sent>) {
    let mut x = row.left();
    if !label.is_empty() {
        let shown = ui.painter().text(row.left_center(), Align2::LEFT_CENTER, label, widgets::body(S), MUTED);
        x = shown.right() + S.of(14.0);
    }
    let cell = S.of(34.0);
    for (index, icon) in icons.iter().enumerate() {
        let spot = Rect::from_min_size(pos2(x + index as f32 * (cell + S.of(6.0)), row.top()), vec2(cell, row.height()));
        let response = ui
            .interact(spot, ui.id().with(("panel-icon", prop.to_string(), index)), Sense::click_and_drag())
            .on_hover_text(&icon.tip);
        let plate = Rect::from_center_size(spot.center(), vec2(cell, cell).min(spot.size()));
        if icon.on {
            ui.painter().rect_filled(plate, S.of(6.0), ICON_ON_PLATE);
        } else if response.hovered() {
            ui.painter().rect_filled(plate, S.of(6.0), widgets::SELECTED);
        }
        let color = if icon.on { crate::toolbar::ICON_ACTIVE } else { TEXT };
        crate::toolbar::paint_icon(ui.painter(), spot.center(), &icon.icon, S.of(19.0), color);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, icon.on, &icon.tip));
        if response.clicked() {
            set(sent, prop, PropValue::Index(index as u8));
        }
    }
}

/// The plate behind an icon that is on.
const ICON_ON_PLATE: Color32 = Color32::from_rgb(226, 236, 252);

/// A left chevron and the name of the page this one was opened from, on the
/// panel's first line. Clicking it returns there.
fn back_row(ui: &mut egui::Ui, label: &str, action: &Action, sent: &mut Vec<Sent>) {
    ui.add_space(FIRST_LINE - ROW / 2.0);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click_and_drag());
    if response.hovered() {
        ui.painter().rect_filled(rect, S.of(6.0), widgets::SELECTED);
    }
    widgets::chevron_side(
        ui.painter(),
        pos2(rect.left() + S.of(10.0), rect.center().y),
        false,
        TEXT,
    );
    ui.painter().text(
        pos2(rect.left() + S.of(22.0), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        widgets::body(S),
        TEXT,
    );
    let response = response.on_hover_text(format!("Back to {label}"));
    if response.clicked() {
        sent.push(Sent::Run(action.clone()));
    }
}

/// A section: a hairline above every section but the first, its heading,
/// with its icon before it when it has one, as the Properties views show,
/// then its rows.
fn draw_section(
    ui: &mut egui::Ui,
    section: &protocol::Section,
    finish: &Action,
    first: bool,
    sent: &mut Vec<Sent>,
) {
    let (title, rows) = (section.title.as_str(), &section.rows);
    let width = ui.available_width();
    if first {
        ui.add_space(TOP);
    } else {
        ui.add_space(S.of(14.0));
        let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
        ui.painter()
            .hline(line.x_range(), line.center().y, Stroke::new(1.0, BORDER));
        ui.add_space(S.of(14.0));
    }
    let heading = if title.is_empty() {
        ui.add_space(if first { S.of(4.0) } else { 0.0 });
        Rect::from_min_size(ui.cursor().min, vec2(width, 0.0))
    } else {
        let (heading, _) = ui.allocate_exact_size(vec2(width, HEADING), Sense::hover());
        let mut left = heading.left();
        if let Some(icon) = &section.icon {
            crate::toolbar::paint_icon(ui.painter(), pos2(left + S.of(9.0), heading.center().y), icon, S.of(18.0), TEXT);
            left += S.of(26.0);
        }
        ui.painter().text(
            pos2(left, heading.center().y),
            Align2::LEFT_CENTER,
            title,
            widgets::semibold(S.of(15.0)),
            TEXT,
        );
        heading
    };
    let _ = heading;
    ui.add_space(S.of(10.0));
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            ui.add_space(ROW_GAP);
        }
        draw_row(ui, row, finish, sent);
    }
}

/// The label column and the space to its right. A row without a label uses
/// the whole width for its control.
fn split_row(ui: &egui::Ui, row: Rect, label: &str) -> Rect {
    if label.is_empty() {
        return row;
    }
    ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        label,
        widgets::body(S),
        MUTED,
    );
    Rect::from_min_max(pos2(row.left() + LABEL_W, row.top()), row.max)
}

fn row_id(ui: &egui::Ui, prop: &Value) -> egui::Id {
    ui.id().with(("panel-row", prop.to_string()))
}

fn set(sent: &mut Vec<Sent>, prop: &Value, value: PropValue) {
    sent.push(Sent::Property {
        prop: prop.clone(),
        value,
        preview: false,
    });
}

fn preview(sent: &mut Vec<Sent>, prop: &Value, value: PropValue) {
    sent.push(Sent::Property {
        prop: prop.clone(),
        value,
        preview: true,
    });
}

/// One declared row, as the panel draws it; the toolbar's popovers draw
/// theirs with it too.
pub(crate) fn draw_row(ui: &mut egui::Ui, desc: &Row, finish: &Action, sent: &mut Vec<Sent>) {
    let value = desc.value.clone();
    let width = ui.available_width();
    if let RowKind::Palette {
        owner,
        colors,
        sort,
        flags,
    } = &desc.kind
    {
        crate::outliner::palette(ui, *owner, colors, sort.as_ref(), flags, sent);
        return;
    }
    match &desc.kind {
        RowKind::Item { icon, detail, action } => {
            item_row(ui, &desc.prop, &desc.label, icon, detail, action, sent);
            return;
        }
        RowKind::Button { action } => {
            button_row(ui, &desc.prop, &desc.label, action, sent);
            return;
        }
        RowKind::Note => {
            note_row(ui, &desc.label);
            return;
        }
        RowKind::Paragraph { lines } => {
            paragraph_row(ui, &desc.prop, &desc.label, *lines, &desc.value, sent);
            return;
        }
        _ => {}
    }
    let height = match desc.kind {
        // A square plot is as tall as it is wide.
        RowKind::Curve { square: true, .. } => width,
        RowKind::Curve { .. } => CURVE_H,
        _ => ROW,
    };
    let (row, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let id = row_id(ui, &desc.prop);
    let prop = &desc.prop;
    match (&desc.kind, value) {
        (RowKind::Icons { icons }, _) => icons_row(ui, prop, row, &desc.label, icons, sent),
        (RowKind::Toggle, PropValue::Bool(on)) => {
            ui.painter().text(
                row.left_center(),
                Align2::LEFT_CENTER,
                &desc.label,
                widgets::body(S),
                MUTED,
            );
            let spot = Rect::from_min_max(pos2(row.right() - S.of(40.0), row.top()), row.max);
            if widgets::toggle(ui, spot, id, on, S).clicked() {
                set(sent, prop, PropValue::Bool(!on));
            }
        }
        (
            RowKind::Number {
                min,
                max,
                percent,
                places,
                centered,
            },
            PropValue::Number(number),
        ) => {
            let control = split_row(ui, row, &desc.label);
            number_row(
                ui,
                prop,
                control,
                id,
                (*min, *max),
                (*percent, *places, *centered),
                number,
                false,
                finish,
                sent,
            );
        }
        (RowKind::Field { min, max, places }, PropValue::Number(number)) => {
            let control = split_row(ui, row, &desc.label);
            field_row(ui, prop, control, id, (*min, *max), *places, number, false, finish, sent);
        }
        (RowKind::Field { min, max, places }, PropValue::Mixed) => {
            let control = split_row(ui, row, &desc.label);
            field_row(ui, prop, control, id, (*min, *max), *places, *min, true, finish, sent);
        }
        (RowKind::Text, PropValue::Text(text)) => {
            let control = split_row(ui, row, &desc.label);
            if let Some(next) = widgets::text_field(ui, control, id, &text, "", S)
                && next != text
            {
                set(sent, prop, PropValue::Text(next));
            }
        }
        (RowKind::Text, PropValue::Mixed) => {
            let control = split_row(ui, row, &desc.label);
            if let Some(next) = widgets::text_field(ui, control, id, "", "Mixed", S)
                && !next.trim().is_empty()
            {
                set(sent, prop, PropValue::Text(next));
            }
        }
        (
            RowKind::Number {
                min,
                max,
                percent,
                places,
                centered,
            },
            PropValue::Mixed,
        ) => {
            let control = split_row(ui, row, &desc.label);
            let mid = (min + max) / 2.0;
            number_row(
                ui,
                prop,
                control,
                id,
                (*min, *max),
                (*percent, *places, *centered),
                mid,
                true,
                finish,
                sent,
            );
        }
        (
            RowKind::Choice {
                options,
                segmented,
                preview,
            },
            PropValue::Index(index),
        ) => {
            let control = split_row(ui, row, &desc.label);
            let options: Vec<&str> = options.iter().map(String::as_str).collect();
            let index = usize::from(index);
            let preview = *preview;
            let chosen = if *segmented {
                let paint = |painter: &Painter, cell: Rect, option: usize, selected: bool| {
                    paint_option(painter, cell, preview, options[option], selected);
                };
                widgets::segmented(ui, control, id, index, &options, &paint, S)
            } else {
                let sample = |painter: &Painter, area: Rect, option: usize| {
                    if preview == ChoicePreview::Dash {
                        widgets::dash_sample(painter, area, options[option] == "Dashed", S);
                    }
                };
                let text = options.get(index).copied().unwrap_or_default();
                let preview_fn: Option<widgets::OptionPreview> =
                    (preview == ChoicePreview::Dash).then_some(&sample);
                widgets::dropdown(ui, control, id, text, &options, Some(index), preview_fn, S)
            };
            if let Some(chosen) = chosen
                && chosen != index
            {
                set(sent, prop, PropValue::Index(chosen as u8));
            }
        }
        (
            RowKind::Preset {
                options,
                custom,
                sample,
            },
            PropValue::Index(index),
        ) => {
            preset_row(
                ui,
                prop,
                row,
                id,
                index,
                options,
                custom,
                sample.as_ref(),
                sent,
            );
        }
        (RowKind::Rgb { quick }, PropValue::Rgb(rgb)) => {
            let control = split_row(ui, row, &desc.label);
            color_well(ui, prop, control, id, Some(rgb), quick, finish, sent);
        }
        (RowKind::Rgb { quick }, PropValue::Mixed) => {
            let control = split_row(ui, row, &desc.label);
            color_well(ui, prop, control, id, None, quick, finish, sent);
        }
        (RowKind::Toggle, PropValue::Mixed) => {
            ui.painter().text(
                row.left_center(),
                Align2::LEFT_CENTER,
                &desc.label,
                widgets::body(S),
                MUTED,
            );
            let spot = Rect::from_min_max(pos2(row.right() - S.of(72.0), row.top()), row.max);
            let response = ui.interact(spot, id, Sense::click_and_drag());
            ui.painter().text(
                spot.right_center(),
                Align2::RIGHT_CENTER,
                "Mixed",
                widgets::body(S),
                MUTED,
            );
            if response.clicked() {
                set(sent, prop, PropValue::Bool(true));
            }
        }
        (
            RowKind::Choice {
                options,
                segmented: false,
                preview,
            },
            PropValue::Mixed,
        ) => {
            let control = split_row(ui, row, &desc.label);
            let options: Vec<&str> = options.iter().map(String::as_str).collect();
            let preview = *preview;
            let sample = |painter: &Painter, area: Rect, option: usize| {
                if preview == ChoicePreview::Dash {
                    widgets::dash_sample(painter, area, options[option] == "Dashed", S);
                }
            };
            let preview_fn: Option<widgets::OptionPreview> =
                (preview == ChoicePreview::Dash).then_some(&sample);
            if let Some(chosen) =
                widgets::dropdown(ui, control, id, "Mixed", &options, None, preview_fn, S)
            {
                set(sent, prop, PropValue::Index(chosen as u8));
            }
        }
        (RowKind::Curve { histogram, square, free }, PropValue::Curve(points)) => {
            let plot = Plot { histogram, square: *square, free: *free };
            curve_editor(ui, prop, row, id, &points, &plot, finish, sent);
        }
        _ => {}
    }
}

/// An item's height, its icon tile's side, and its words, as
/// adjustments.png shows them.
const ITEM_H: f32 = S.of(56.0);
const ITEM_TILE: f32 = S.of(34.0);
const ITEM_TEXT: f32 = S.of(16.0);
const ITEM_DETAIL: f32 = S.of(12.5);
const BUTTON_PAD: f32 = S.of(24.0);
const NOTE_TEXT: f32 = S.of(13.0);

/// A full-width row, as adjustments.png shows: an icon tile, the label, a
/// muted detail line under it, and a plus at the right. Pale while the
/// pointer is over it; a click sends `action`.
fn item_row(ui: &mut egui::Ui, prop: &Value, label: &str, icon: &str, detail: &str, action: &Action, sent: &mut Vec<Sent>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ITEM_H), Sense::hover());
    let response = ui.interact(rect, ui.id().with(("panel-item", prop.to_string(), label)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, S.of(8.0), widgets::SELECTED);
    }
    let tile = Rect::from_min_size(pos2(rect.left() + S.of(10.0), rect.center().y - ITEM_TILE / 2.0), vec2(ITEM_TILE, ITEM_TILE));
    widgets::outlined(ui.painter(), tile, S.of(6.0), Color32::WHITE, BORDER);
    crate::toolbar::paint_icon(ui.painter(), tile.center(), icon, S.of(18.0), TEXT);
    let x = tile.right() + S.of(14.0);
    let plus = S.of(22.0);
    let room = (rect.right() - plus - S.of(6.0) - x).max(0.0);
    let name = ui.painter().layout(label.to_string(), egui::FontId::proportional(ITEM_TEXT), TEXT, room);
    let more = ui.painter().layout(detail.to_string(), egui::FontId::proportional(ITEM_DETAIL), MUTED, room);
    let gap = S.of(3.0);
    let top = rect.center().y - (name.size().y + gap + more.size().y) / 2.0;
    let below = top + name.size().y + gap;
    ui.painter().galley(pos2(x, top), name, TEXT);
    ui.painter().galley(pos2(x, below), more, MUTED);
    crate::toolbar::paint_icon(ui.painter(), pos2(rect.right() - plus / 2.0 - S.of(4.0), rect.center().y), "plus", S.of(16.0), MUTED);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.clicked() {
        sent.push(Sent::Run(action.clone()));
    }
}

/// A bordered button holding the label, such as Reset or Invert, as the
/// Properties views show. A click sends `action`.
fn button_row(ui: &mut egui::Ui, prop: &Value, label: &str, action: &Action, sent: &mut Vec<Sent>) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    let words = ui.painter().layout_no_wrap(label.to_string(), widgets::body(S), TEXT);
    let button = Rect::from_min_size(row.min, vec2((words.size().x + 2.0 * BUTTON_PAD).min(width), row.height()));
    let response = ui.interact(button, ui.id().with(("panel-button", prop.to_string(), label)), Sense::click());
    let fill = if response.hovered() { widgets::SELECTED } else { Color32::WHITE };
    widgets::outlined(ui.painter(), button, S.of(6.0), fill, BORDER);
    ui.painter().galley(button.center() - words.size() / 2.0, words, TEXT);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.clicked() {
        sent.push(Sent::Run(action.clone()));
    }
}

/// The label as muted words, wrapped to the panel, as properties-mask.png
/// shows its note.
fn note_row(ui: &mut egui::Ui, label: &str) {
    let width = ui.available_width();
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat { font_id: egui::FontId::proportional(NOTE_TEXT), color: MUTED, line_height: Some(S.of(20.0)), ..Default::default() },
    );
    job.wrap.max_width = width;
    let words = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(width, words.size().y), Sense::hover());
    ui.painter().galley(rect.min, words, MUTED);
}

/// A label above text of several lines, such as a clip's notes, sent when
/// the field lets go of the keyboard.
fn paragraph_row(ui: &mut egui::Ui, prop: &Value, label: &str, lines: u8, value: &PropValue, sent: &mut Vec<Sent>) {
    let width = ui.available_width();
    let (top, _) = ui.allocate_exact_size(vec2(width, S.of(20.0)), Sense::hover());
    ui.painter().text(top.left_center(), Align2::LEFT_CENTER, label, widgets::body(S), MUTED);
    let line = S.of(20.0);
    let height = line * f32::from(lines.max(1)) + 2.0 * S.of(8.0);
    let (field, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let current = match value {
        PropValue::Text(text) => text.as_str(),
        _ => "",
    };
    if let Some(next) = widgets::text_area(ui, field, row_id(ui, prop), current, S)
        && next != current
    {
        set(sent, prop, PropValue::Text(next));
    }
}

/// A choice cell's content: a cap sample or a line sample.
fn paint_option(
    painter: &Painter,
    cell: Rect,
    preview: ChoicePreview,
    option: &str,
    selected: bool,
) {
    let color = if selected { INK } else { TEXT };
    match preview {
        ChoicePreview::Cap => {
            widgets::cap_sample(painter, cell.center(), option == "Round", color, S)
        }
        ChoicePreview::Label => {
            painter.text(
                cell.center(),
                Align2::CENTER_CENTER,
                option,
                widgets::body(S),
                color,
            );
        }
        ChoicePreview::Dash => widgets::dash_sample(
            painter,
            cell.shrink2(vec2(S.of(12.0), 0.0)),
            option == "Dashed",
            S,
        ),
    }
}

/// A slider and a typed number. `centered` fills the slider from its
/// middle, for a value that goes either way.
#[allow(clippy::too_many_arguments)]
fn number_row(
    ui: &mut egui::Ui,
    prop: &Value,
    control: Rect,
    id: egui::Id,
    (min, max): (f64, f64),
    (percent, places, centered): (bool, u8, bool),
    value: f64,
    mixed: bool,
    finish: &Action,
    sent: &mut Vec<Sent>,
) {
    let scale = if percent {
        100.0
    } else {
        10f64.powi(i32::from(places))
    };
    let shown_min = (min * scale).round() as i32;
    let shown_max = (max * scale).round() as i32;
    let track = Rect::from_min_max(
        control.min,
        pos2(control.right() - NUMBER_W - GAP, control.bottom()),
    );
    let field = Rect::from_min_max(pos2(control.right() - NUMBER_W, control.top()), control.max);

    let span = (max - min).max(f64::EPSILON);
    // The knob shows what was asked for, not the lagging echo.
    let echoed = (value * scale).round() / scale;
    let t = ((widgets::held(ui, id.with("slider"), &echoed).unwrap_or(value) - min) / span) as f32;
    let (response, asked) = if centered {
        widgets::slider_from(ui, track, id.with("slider"), t, 0.5)
    } else {
        widgets::slider(ui, track, id.with("slider"), t)
    };
    // Only a change from what this hold last sent is sent: the value shown
    // is the application's, and lags while its replies queue.
    let asked = asked.map(|asked| ((min + f64::from(asked) * span) * scale).round() / scale);
    if let Some(next) = widgets::unsent(ui, id.with("slider"), asked, &echoed) {
        preview(sent, prop, PropValue::Number(next));
    }
    if response.drag_stopped() || response.clicked() {
        sent.push(Sent::Run(finish.clone()));
    }

    // While the slider is held, its field shows what it asked for.
    let value = widgets::held(ui, id.with("slider"), &echoed).unwrap_or(value);
    let shown = (value * scale).round() as i32;
    let suffix = if percent { "%" } else { "" };
    let edit = widgets::number_field(
        ui,
        field,
        id.with("field"),
        shown,
        (shown_min, shown_max),
        suffix,
        mixed,
        places,
        centered,
        S,
    );
    match (edit.value, edit.finished) {
        (Some(next), true) => {
            set(sent, prop, PropValue::Number(f64::from(next) / scale));
        }
        (Some(next), false) => {
            preview(sent, prop, PropValue::Number(f64::from(next) / scale));
        }
        (None, true) => sent.push(Sent::Run(finish.clone())),
        (None, false) => {}
    }
}

/// A number typed into a bordered field across the row's control, with no
/// slider: the field sets the value when its typing ends.
#[allow(clippy::too_many_arguments)]
fn field_row(
    ui: &mut egui::Ui,
    prop: &Value,
    control: Rect,
    id: egui::Id,
    (min, max): (f64, f64),
    places: u8,
    value: f64,
    mixed: bool,
    finish: &Action,
    sent: &mut Vec<Sent>,
) {
    let scale = 10f64.powi(i32::from(places));
    let edit = widgets::number_field(
        ui,
        control,
        id.with("field"),
        (value * scale).round() as i32,
        ((min * scale).round() as i32, (max * scale).round() as i32),
        "",
        mixed,
        places,
        false,
        S,
    );
    match (edit.value, edit.finished) {
        (Some(next), true) => set(sent, prop, PropValue::Number(f64::from(next) / scale)),
        (Some(next), false) => preview(sent, prop, PropValue::Number(f64::from(next) / scale)),
        (None, true) => sent.push(Sent::Run(finish.clone())),
        (None, false) => {}
    }
}

/// A stroke sample drawn with the tool's own dynamics, then the preset list.
#[allow(clippy::too_many_arguments)]
fn preset_row(
    ui: &mut egui::Ui,
    prop: &Value,
    row: Rect,
    id: egui::Id,
    index: u8,
    options: &[String],
    custom: &str,
    sample: Option<&PresetSample>,
    sent: &mut Vec<Sent>,
) {
    let area = Rect::from_min_size(row.min, vec2(PRESET_SAMPLE_W, row.height()));
    widgets::field(ui.painter(), area, false, S);
    if let Some(sample) = sample
        && let Some(dynamics) = draw::document::snapshot::dynamics_from_value(&sample.dynamics)
    {
        let color = if sample.eraser { ERASER_SAMPLE } else { INK };
        let diameter = f64::from(S.of(if sample.eraser { 9.0 } else { 6.0 }));
        paint_stroke_sample(
            ui.painter(),
            area.shrink2(vec2(S.of(12.0), S.of(8.0))),
            &dynamics,
            diameter,
            color,
        );
    }
    let list = Rect::from_min_max(pos2(area.right() + S.of(8.0), row.top()), row.max);
    let options: Vec<&str> = options.iter().map(String::as_str).collect();
    let selected = (usize::from(index) < options.len()).then_some(usize::from(index));
    let text = selected
        .and_then(|index| options.get(index).copied())
        .unwrap_or(custom);
    if let Some(chosen) = widgets::dropdown(ui, list, id, text, &options, selected, None, S) {
        set(sent, prop, PropValue::Index(chosen as u8));
    }
}

/// A gentle S-curve with pressure rising and falling, outlined exactly as a
/// stroke would be with these dynamics.
fn paint_stroke_sample(
    painter: &Painter,
    area: Rect,
    dynamics: &draw::document::Dynamics,
    diameter: f64,
    color: Color32,
) {
    let steps = 24;
    let samples: Vec<Sample> = (0..=steps)
        .map(|step| {
            let t = f64::from(step) / f64::from(steps);
            let x = f64::from(area.left()) + t * f64::from(area.width());
            let wave = (t * std::f64::consts::TAU).sin() * 0.35;
            let y = f64::from(area.center().y) - wave * f64::from(area.height());
            let pressure = 0.25 + 0.75 * (t * std::f64::consts::PI).sin();
            Sample {
                x,
                y,
                pressure: Some(pressure),
            }
        })
        .collect();
    let Some(shapes) =
        ink::stroke_visible(&samples, dynamics, &[], diameter, Cap::Round, Dash::Solid)
    else {
        return;
    };
    let Ok(mesh) = ink::mesh_from_shapes(shapes) else {
        return;
    };
    let mut paint = egui::Mesh::default();
    for triangle in &mesh.triangles {
        let base = paint.vertices.len() as u32;
        for point in triangle {
            paint.colored_vertex(pos2(point[0] as f32, point[1] as f32), color);
        }
        paint.add_triangle(base, base + 1, base + 2);
    }
    painter.add(egui::Shape::mesh(paint));
}

/// A swatch and chevron in a field. Clicking opens a color picker; a drag in
/// the picker becomes one undo step when it is released.
#[allow(clippy::too_many_arguments)]
fn color_well(
    ui: &mut egui::Ui,
    prop: &Value,
    control: Rect,
    id: egui::Id,
    rgb: Option<[u8; 3]>,
    swatches: &[[u8; 3]],
    finish: &Action,
    sent: &mut Vec<Sent>,
) {
    // A chosen color is a swatch at the field's end. Mixed uses the whole
    // field for the word, so a placeholder gray is not a selected color.
    let well = match rgb {
        Some(_) => Rect::from_min_max(
            pos2(control.right() - S.of(64.0), control.top()),
            control.max,
        ),
        None => control,
    };
    let response = ui.interact(well, id, Sense::click_and_drag());
    widgets::field(ui.painter(), well, response.hovered(), S);
    let color = if let Some(rgb) = rgb {
        let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let dot = pos2(well.left() + S.of(20.0), well.center().y);
        ui.painter().circle_filled(dot, S.of(9.0), color);
        ui.painter()
            .circle_stroke(dot, S.of(9.0), Stroke::new(1.0, BORDER));
        color
    } else {
        ui.painter().text(
            pos2(well.left() + S.of(10.0), well.center().y),
            Align2::LEFT_CENTER,
            "Mixed",
            widgets::body(S),
            MUTED,
        );
        Color32::WHITE
    };
    widgets::chevron(
        ui.painter(),
        pos2(well.right() - S.of(14.0), well.center().y),
        TEXT,
        S,
    );
    let open = egui::Popup::menu(&response)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .gap(S.of(4.0))
        .show(|ui| {
            if !swatches.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for rgb in swatches {
                        let (spot, response) =
                            ui.allocate_exact_size(vec2(S.of(22.0), S.of(22.0)), Sense::click());
                        let fill = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        ui.painter().circle_filled(spot.center(), S.of(8.0), fill);
                        ui.painter().circle_stroke(
                            spot.center(),
                            S.of(8.0),
                            Stroke::new(1.0, BORDER),
                        );
                        if response.clicked() {
                            set(sent, prop, PropValue::Rgb(*rgb));
                        }
                    }
                });
                ui.separator();
            }
            let mut picked = color;
            if egui::color_picker::color_picker_color32(
                ui,
                &mut picked,
                egui::color_picker::Alpha::Opaque,
            ) {
                preview(
                    sent,
                    prop,
                    PropValue::Rgb([picked.r(), picked.g(), picked.b()]),
                );
            }
        })
        .is_some();
    if open && ui.input(|input| input.pointer.any_released()) {
        sent.push(Sent::Run(finish.clone()));
    }
}

/// How a curve row is drawn and edited, as its kind declares.
struct Plot<'a> {
    /// Drawn faintly behind the curve, left to right, each 0 to 1.
    histogram: &'a [f32],
    /// As tall as it is wide, with a faint diagonal, as
    /// properties-curves.png shows.
    square: bool,
    /// Up to sixteen points, end points that move inward, and any point to
    /// any height.
    free: bool,
}

/// The most points a curve holds: the pressure curve's five, or sixteen
/// for a free one.
fn most_points(free: bool) -> usize {
    if free { 16 } else { 5 }
}

/// The faint bars behind a curve.
const HISTOGRAM: Color32 = Color32::from_rgb(226, 228, 232);
const DIAGONAL: Color32 = Color32::from_rgb(206, 210, 216);

/// A point held on a curve, and the curve as the drag has made it. The
/// drag moves the point on this, and the plot shows it, rather than the
/// application's echo, which lags while its replies queue.
#[derive(Clone)]
struct CurveHold {
    index: usize,
    points: Vec<(f64, f64)>,
}

#[allow(clippy::too_many_arguments)]
fn curve_editor(
    ui: &mut egui::Ui,
    prop: &Value,
    rect: Rect,
    id: egui::Id,
    points: &[(f64, f64)],
    look: &Plot,
    finish: &Action,
    sent: &mut Vec<Sent>,
) {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let hold = if response.dragged() {
        ui.data(|data| data.get_temp::<CurveHold>(response.id))
    } else {
        ui.data_mut(|data| data.remove::<CurveHold>(response.id));
        None
    };
    let points = hold.as_ref().map_or(points, |hold| hold.points.as_slice());
    let plot = if look.square {
        // The plot fills the row, with room around it for the end points.
        let plot = rect.shrink(S.of(6.0));
        ui.painter().rect(plot, 0.0, Color32::WHITE, Stroke::new(1.0, BORDER), egui::StrokeKind::Outside);
        for step in 1..4 {
            let x = plot.left() + plot.width() * (step as f32 / 4.0);
            ui.painter().vline(x, plot.top()..=plot.bottom(), Stroke::new(1.0, PLOT_GRID));
        }
        plot
    } else {
        ui.painter().rect(
            rect,
            S.of(8.0),
            PLOT,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        rect.shrink(S.of(16.0))
    };
    let bins = look.histogram.len();
    for (index, height) in look.histogram.iter().enumerate() {
        let height = height.clamp(0.0, 1.0);
        if height <= 0.0 {
            continue;
        }
        let left = plot.left() + plot.width() * index as f32 / bins as f32;
        let right = plot.left() + plot.width() * (index + 1) as f32 / bins as f32;
        let bar = Rect::from_min_max(pos2(left, plot.bottom() - height * plot.height()), pos2(right, plot.bottom()));
        ui.painter().rect_filled(bar, 0.0, HISTOGRAM);
    }
    for step in 1..4 {
        let y = plot.bottom() - plot.height() * (step as f32 / 4.0);
        ui.painter()
            .hline(plot.left()..=plot.right(), y, Stroke::new(1.0, PLOT_GRID));
    }
    if look.square {
        ui.painter().line_segment([plot.left_bottom(), plot.right_top()], Stroke::new(1.0, DIAGONAL));
    }
    let curve: Vec<CurvePoint> = points
        .iter()
        .map(|(x, y)| CurvePoint { x: *x, y: *y })
        .collect();
    let line: Vec<egui::Pos2> = (0..=48)
        .map(|step| {
            let x = f64::from(step) / 48.0;
            plot_pos(plot, x, ink::curve_y(&curve, x))
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(line, Stroke::new(S.of(2.0), INK)));
    for point in points {
        let center = plot_pos(plot, point.0, point.1);
        ui.painter().circle_filled(center, S.of(5.0), INK);
        ui.painter()
            .circle_stroke(center, S.of(5.0), Stroke::new(1.5, Color32::WHITE));
    }

    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let hit = pointer.and_then(|pos| hit_point(plot, points, pos));
    if response.double_clicked()
        && let Some(index) = hit
        && points.len() > 2
    {
        let mut next = points.to_vec();
        next.remove(index);
        if !look.free {
            pin_curve_ends(&mut next);
        }
        set(sent, prop, PropValue::Curve(next));
        return;
    }
    if response.drag_started()
        && let Some(index) = hit
    {
        ui.data_mut(|data| data.insert_temp(response.id, CurveHold { index, points: points.to_vec() }));
    } else if response.drag_started()
        && points.len() < most_points(look.free)
        && let Some(pos) = pointer
    {
        // A drag that starts away from a point adds one where it starts,
        // and carries it, as a click would add it.
        let (x, y) = plot_value(plot, pos);
        let mut next = points.to_vec();
        if insert_curve_point(&mut next, x, y, look.free)
            && let Some(index) = next.iter().position(|point| (point.0 - x).abs() < 1e-9)
        {
            ui.data_mut(|data| data.insert_temp(response.id, CurveHold { index, points: next.clone() }));
            preview(sent, prop, PropValue::Curve(next));
        }
    }
    // The point held moves on the curve the drag has made, and only a
    // change is sent.
    if response.dragged()
        && let Some(mut hold) = ui.data(|data| data.get_temp::<CurveHold>(response.id))
        && let Some(pos) = pointer
        && hold.index < hold.points.len()
    {
        let (x, y) = plot_value(plot, pos);
        let mut next = hold.points.clone();
        move_curve_point(&mut next, hold.index, x, y, look.free);
        if next != hold.points {
            hold.points = next.clone();
            ui.data_mut(|data| data.insert_temp(response.id, hold));
            preview(sent, prop, PropValue::Curve(next));
        }
    }
    if response.drag_stopped() {
        sent.push(Sent::Run(finish.clone()));
    } else if response.clicked()
        && hit.is_none()
        && points.len() < most_points(look.free)
        && let Some(pos) = pointer
    {
        let (x, y) = plot_value(plot, pos);
        let mut next = points.to_vec();
        if insert_curve_point(&mut next, x, y, look.free) {
            set(sent, prop, PropValue::Curve(next));
        }
    }
}

fn plot_pos(plot: Rect, x: f64, y: f64) -> egui::Pos2 {
    pos2(
        plot.left() + (x as f32) * plot.width(),
        plot.bottom() - (y as f32) * plot.height(),
    )
}

fn plot_value(plot: Rect, pos: egui::Pos2) -> (f64, f64) {
    let x = f64::from((pos.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let y = f64::from((plot.bottom() - pos.y) / plot.height()).clamp(0.0, 1.0);
    (x, y)
}

fn hit_point(plot: Rect, points: &[(f64, f64)], pos: egui::Pos2) -> Option<usize> {
    points.iter().enumerate().rev().find_map(|(index, point)| {
        let center = plot_pos(plot, point.0, point.1);
        ((center.x - pos.x).powi(2) + (center.y - pos.y).powi(2) < 12.0 * 12.0).then_some(index)
    })
}

/// Moves a point, kept between its neighbours. The pressure curve keeps its
/// ends at 0 and 1 and every point between its neighbours' heights; a free
/// one lets its ends move inward and any point to any height.
fn move_curve_point(points: &mut [(f64, f64)], index: usize, x: f64, y: f64, free: bool) {
    let last = points.len() - 1;
    let eps = 0.01;
    if free {
        let min_x = if index == 0 { 0.0 } else { points[index - 1].0 + eps };
        let max_x = if index == last { 1.0 } else { points[index + 1].0 - eps };
        points[index].0 = x.clamp(min_x, max_x.max(min_x));
        points[index].1 = y.clamp(0.0, 1.0);
        return;
    }
    if index == 0 {
        points[0].0 = 0.0;
    } else if index == last {
        points[last].0 = 1.0;
    } else {
        let min_x = points[index - 1].0 + eps;
        let max_x = points[index + 1].0 - eps;
        points[index].0 = x.clamp(min_x, max_x.max(min_x));
    }
    let min_y = if index == 0 { 0.0 } else { points[index - 1].1 };
    let max_y = if index == last {
        1.0
    } else {
        points[index + 1].1
    };
    points[index].1 = y.clamp(min_y, max_y.max(min_y));
}

/// Adds a point where it falls by x, so the points stay in order. A free
/// curve takes it at any height, and before or after its ends when they
/// have moved inward.
fn insert_curve_point(points: &mut Vec<(f64, f64)>, x: f64, y: f64, free: bool) -> bool {
    if points.len() >= most_points(free) || points.len() < 2 {
        return false;
    }
    if free {
        let index = points.iter().position(|point| point.0 > x).unwrap_or(points.len());
        let near = |neighbour: Option<&(f64, f64)>| neighbour.is_some_and(|point| (point.0 - x).abs() < 0.02);
        if near(index.checked_sub(1).and_then(|before| points.get(before))) || near(points.get(index)) {
            return false;
        }
        points.insert(index, (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
        return true;
    }
    let mut index = 1;
    while index < points.len() && points[index].0 < x {
        index += 1;
    }
    let left = points[index - 1].0;
    let right = points[index].0;
    if right - left < 0.04 {
        return false;
    }
    let x = x.clamp(left + 0.01, right - 0.01);
    let y = y.clamp(points[index - 1].1, points[index].1);
    points.insert(index, (x, y));
    true
}

fn pin_curve_ends(points: &mut [(f64, f64)]) {
    if points.is_empty() {
        return;
    }
    let last = points.len() - 1;
    points[0].0 = 0.0;
    points[last].0 = 1.0;
}

/// What a cell's card shows beyond its declaration, from the window and
/// the runner: the cell, whether it runs, and what only the runner knows.
#[derive(Default)]
pub(crate) struct CardState<'a> {
    /// The cell as shown: for a draft, the cell the tab declared with
    /// `cells`; for a step, the runner's read of it, with its prompt,
    /// references, settings, and model filled in.
    pub cell: Option<&'a Cell>,
    /// A run of the cell is in progress: Generate is Stop.
    pub running: bool,
    /// The line under the header while it runs, such as "Making 4 images…".
    pub status: Option<&'a str>,
    /// Why the newest round failed, when the runner knows. Without it, a
    /// failed cell's own error is shown.
    pub error: Option<&'a str>,
    /// The model's name, such as "GPT Image 2.5". Without it, the last
    /// part of the cell's model id.
    pub model: Option<&'a str>,
    /// The gray cost line, such as "About $0.24 · OpenRouter".
    pub cost: Option<&'a str>,
    /// Small images of references, by `reference_key`, as `previewed`
    /// answered.
    pub previews: Option<&'a HashMap<String, ReferencePreview>>,
    /// The project's folder, against which a reference's relative path is
    /// read.
    pub root: Option<&'a std::path::Path>,
}

/// A reference's small image, with words for its corner, such as a
/// motion reference's "8.0 s".
#[derive(Clone, Debug, Default)]
pub(crate) struct ReferencePreview {
    pub thumbnail: Option<protocol::Thumbnail>,
    pub badge: String,
    /// A sound's own cue marks, when its file holds them.
    #[allow(dead_code)]
    pub cues: Vec<protocol::Cue>,
}

/// What a cell's card asks of the window and the runner. Each names the
/// cell by its id.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CardEvent {
    /// Generate, or Try again: the runner opens the plan card.
    Run { cell: String },
    Stop { cell: String },
    /// The prompt, a reference, the references' order, or a setting
    /// changed: the whole input and settings as edited. The runner saves a
    /// draft's as `edit_cell` and a step's to the step.
    Edit {
        cell: String,
        input: Vec<CellPart>,
        settings: Option<Value>,
    },
    /// "Add reference": the picker, offering these media types.
    AddReference { cell: String, media_types: Vec<String> },
    /// Choose… on a reference, by index, to replace it, or on an empty
    /// slot, by its role, to fill it.
    Choose {
        cell: String,
        reference: Option<usize>,
        role: Option<ReferenceRole>,
        media_types: Vec<String>,
    },
    /// Open report, on a failed round.
    OpenReport { cell: String },
}

fn card_events_id() -> egui::Id {
    egui::Id::new("panel-card-events")
}

/// The card's requests since the last call.
#[allow(dead_code)]
pub(crate) fn take_card_events(ctx: &egui::Context) -> Vec<CardEvent> {
    ctx.data_mut(|data| data.remove_temp::<Vec<CardEvent>>(card_events_id()).unwrap_or_default())
}

/// The card's sizes, as ig-request-anchor draws them at the panel's scale.
const CARD_TOP: f32 = S.of(14.0);
const CARD_HEADER: f32 = S.of(44.0);
const KIND_TILE: f32 = S.of(30.0);
const CARD_BUTTON_H: f32 = S.of(32.0);
const CARD_BUTTON_PAD: f32 = S.of(14.0);
const MORE: f32 = S.of(24.0);
const CARD_GAP: f32 = S.of(12.0);
const LABEL_ROW: f32 = S.of(24.0);
const REFERENCE_ROW: f32 = S.of(52.0);
const REFERENCE_GAP: f32 = S.of(6.0);
const HANDLE_W: f32 = S.of(20.0);
const REFERENCE_NUMBER_W: f32 = S.of(24.0);
const REFERENCE_THUMB: egui::Vec2 = vec2(S.of(66.0), S.of(42.0));
/// The status and failure lines' marks.
const MARK: f32 = S.of(24.0);
/// Lines a new prompt leaves room for, as ig-request-new shows.
const EMPTY_LINES: f32 = 3.0;
/// How many frames an edited prompt stays shown while its echo is awaited.
const ECHO_PASSES: u64 = 60;

const KIND_FILL: Color32 = Color32::from_rgb(243, 244, 246);
const WAITS: Color32 = Color32::from_rgb(170, 174, 180);
const FAILED_RED: Color32 = Color32::from_rgb(220, 38, 38);
const LANDING: Color32 = Color32::from_rgb(37, 99, 235);
const BADGE_FILL: Color32 = Color32::from_rgba_premultiplied(17, 18, 20, 190);

fn card_title() -> egui::FontId {
    widgets::semibold(S.of(16.0))
}

fn card_small() -> egui::FontId {
    egui::FontId::proportional(S.of(13.5))
}

/// One row of the references list: a reference, by its index among the
/// references, or a slot's role not yet filled, by its index in `roles`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Reference(usize),
    Slot(usize),
}

/// The Prompt page's card, drawn from `declared` and the cell `state`
/// holds: the failure when the newest round failed, the header with the
/// kind, the model, Generate or Stop, and "…", the status while it runs,
/// the prompt, the references, the settings, and the cost.
fn prompt_card(ui: &mut egui::Ui, declared: &PanelCell, state: &CardState, sent: &mut Vec<Sent>, events: &mut Vec<CardEvent>) {
    let empty;
    let cell = match state.cell {
        Some(cell) => cell,
        None => {
            let mut blank = Cell::text(declared.cell.clone(), "");
            blank.kind = String::new();
            empty = blank;
            &empty
        }
    };
    let id = declared.cell.as_str();
    let width = ui.available_width();
    ui.add_space(CARD_TOP);

    let failure = if state.running {
        None
    } else {
        state.error.or_else(|| (cell.state == CellState::Failed).then(|| cell.error.as_deref().unwrap_or_default()))
    };
    if let Some(reason) = failure {
        failure_block(ui, id, reason, events);
    }

    // The header: the kind's tile and word, the model in gray, Generate or
    // Stop, and "…".
    let (header, _) = ui.allocate_exact_size(vec2(width, CARD_HEADER), Sense::hover());
    let painter = ui.painter().clone();
    let mid = header.center().y;
    let tile = Rect::from_min_size(pos2(header.left(), mid - KIND_TILE / 2.0), vec2(KIND_TILE, KIND_TILE));
    painter.rect_filled(tile, S.of(7.0), KIND_FILL);
    let tile_icon = match cell.kind.as_str() {
        protocol::IMAGE_KIND => "media-image",
        protocol::VIDEO_KIND => "media-video",
        protocol::SOUND_KIND => "sound-high",
        _ => "align-left",
    };
    crate::toolbar::paint_icon(&painter, tile.center(), tile_icon, S.of(17.0), TEXT);
    let title = painter.text(pos2(tile.right() + S.of(12.0), mid), Align2::LEFT_CENTER, crate::cells::kind_label(&cell.kind), card_title(), TEXT);
    let more = Rect::from_center_size(pos2(header.right() - MORE / 2.0, mid), vec2(MORE, MORE));
    let more_response = ui.interact(more, ui.id().with(("card-more", id)), Sense::click());
    let more_color = if declared.menu.is_empty() { WAITS } else if more_response.hovered() { INK } else { TEXT };
    crate::toolbar::paint_icon(&painter, more.center(), "more-horiz", S.of(18.0), more_color);
    if !declared.menu.is_empty() {
        egui::Popup::menu(&more_response).show(|ui| {
            if let Some(action) = widgets::entry_rows(ui, &declared.menu).and_then(|row| row.action.clone()) {
                sent.push(Sent::Run(action));
            }
        });
    }
    let prompt_now = cell.prompt();
    let can_run = state.running || !prompt_now.trim().is_empty();
    let word = if state.running { "Stop" } else { "Generate" };
    let words = painter.layout_no_wrap(word.to_string(), widgets::body(S), TEXT);
    let button_w = CARD_BUTTON_PAD * 2.0 + S.of(14.0) + S.of(8.0) + words.size().x;
    let button = Rect::from_min_size(pos2(more.left() - S.of(10.0) - button_w, mid - CARD_BUTTON_H / 2.0), vec2(button_w, CARD_BUTTON_H));
    let sense = if can_run { Sense::click() } else { Sense::hover() };
    let pressed = ui.interact(button, ui.id().with(("card-generate", id)), sense);
    let ink = if can_run { TEXT } else { WAITS };
    let fill = if can_run && pressed.hovered() { widgets::SELECTED } else { Color32::WHITE };
    widgets::outlined(&painter, button, S.of(8.0), fill, BORDER);
    let icon_at = pos2(button.left() + CARD_BUTTON_PAD + S.of(7.0), mid);
    if state.running {
        painter.rect_filled(Rect::from_center_size(icon_at, vec2(S.of(11.0), S.of(11.0))), 1.5, ink);
    } else {
        crate::toolbar::paint_icon(&painter, icon_at, "flash-solid", S.of(15.0), ink);
    }
    painter.galley_with_override_text_color(pos2(icon_at.x + S.of(7.0) + S.of(8.0), mid - words.size().y / 2.0), words, ink);
    pressed.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, can_run, word));
    if can_run && pressed.clicked() {
        events.push(if state.running { CardEvent::Stop { cell: id.to_string() } } else { CardEvent::Run { cell: id.to_string() } });
    }
    let model = state
        .model
        .map(str::to_string)
        .or_else(|| cell.model.as_deref().filter(|model| !model.is_empty()).map(|model| model.rsplit('/').next().unwrap_or(model).to_string()));
    // The model sits beside the kind when it fits there, and otherwise on
    // a line of its own under the header, so a long name is never cut to
    // a letter.
    if let Some(model) = model {
        let room = Rect::from_min_max(pos2(title.right() + S.of(10.0), header.top()), pos2(button.left() - S.of(8.0), header.bottom()));
        let whole = painter.layout_no_wrap(model.clone(), card_small(), MUTED);
        if whole.size().x <= room.width() {
            painter.galley(pos2(room.left(), mid - whole.size().y / 2.0), whole, MUTED);
        } else {
            let (line, _) = ui.allocate_exact_size(vec2(width, S.of(20.0)), Sense::hover());
            let mut job = egui::text::LayoutJob::simple_singleline(model, card_small(), MUTED);
            job.wrap = egui::text::TextWrapping::truncate_at_width((line.right() - title.left()).max(1.0));
            let galley = painter.layout_job(job);
            painter.galley(pos2(title.left(), line.center().y - galley.size().y / 2.0), galley, MUTED);
        }
    }

    // While it runs, the status under the header, as ig-request-running
    // shows.
    if state.running {
        let (line, _) = ui.allocate_exact_size(vec2(width, S.of(26.0)), Sense::hover());
        let time = ui.input(|input| input.time);
        crate::film::spinner(&painter, pos2(line.left() + MARK / 2.0, line.center().y), S.of(8.0), time);
        if let Some(status) = state.status {
            painter.text(pos2(line.left() + MARK + S.of(10.0), line.center().y), Align2::LEFT_CENTER, status, card_small(), MUTED);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        ui.add_space(S.of(6.0));
        hairline(ui, width);
    }

    // The prompt, plain wrapped text, edited in place.
    ui.add_space(CARD_GAP);
    if let Some(text) = prompt_field(ui, id, cell, &declared.hint, width) {
        events.push(CardEvent::Edit { cell: id.to_string(), input: with_prompt(&cell.input, &text), settings: cell.settings.clone() });
    }
    ui.add_space(CARD_GAP);
    hairline(ui, width);
    ui.add_space(S.of(8.0));

    references(ui, declared, cell, state, width, sent, events);

    // The settings, drawn as the panel's rows.
    if !declared.settings.is_empty() {
        ui.add_space(S.of(8.0));
        hairline(ui, width);
        ui.add_space(S.of(8.0));
        let (label, _) = ui.allocate_exact_size(vec2(width, LABEL_ROW), Sense::hover());
        painter.text(label.left_center(), Align2::LEFT_CENTER, "Settings", card_small(), MUTED);
        ui.add_space(S.of(4.0));
        let none = Action::new("");
        let mut edits = Vec::new();
        for (index, setting) in declared.settings.iter().enumerate() {
            if index > 0 {
                ui.add_space(ROW_GAP);
            }
            let value = cell.settings.as_ref().and_then(|settings| settings.get(&setting.key));
            let row = Row {
                prop: json!({ "cell": id, "setting": setting.key }),
                label: setting.label.clone(),
                kind: setting.kind.clone(),
                value: setting_value(&setting.kind, value),
            };
            draw_row(ui, &row, &none, &mut edits);
        }
        let mut settings = cell.settings.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
        let mut changed = false;
        for edit in edits {
            let Sent::Property { prop, value, preview: false } = edit else {
                continue;
            };
            let Some(setting) = prop.get("setting").and_then(Value::as_str).and_then(|key| declared.settings.iter().find(|setting| setting.key == key)) else {
                continue;
            };
            if let Some(value) = setting_json(&setting.kind, &value) {
                settings[setting.key.as_str()] = value;
                changed = true;
            }
        }
        if changed {
            events.push(CardEvent::Edit { cell: id.to_string(), input: cell.input.clone(), settings: Some(settings) });
        }
    }

    // The cost, in gray, last.
    if let Some(cost) = state.cost {
        ui.add_space(S.of(14.0));
        let (line, _) = ui.allocate_exact_size(vec2(width, S.of(20.0)), Sense::hover());
        painter.text(line.left_center(), Align2::LEFT_CENTER, cost, card_small(), MUTED);
    }
    ui.add_space(S.of(4.0));
}

/// A hairline across the panel, as between the card's parts.
fn hairline(ui: &mut egui::Ui, width: f32) {
    let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, BORDER));
}

/// A failed round, above the header, as ig-request-failed shows: the red
/// mark, why, and Try again and Open report.
fn failure_block(ui: &mut egui::Ui, id: &str, reason: &str, events: &mut Vec<CardEvent>) {
    let width = ui.available_width();
    let text_left = MARK + S.of(12.0);
    let galley = ui.painter().layout(reason.to_string(), widgets::body(S), TEXT, (width - text_left).max(1.0));
    let (block, _) = ui.allocate_exact_size(vec2(width, galley.size().y.max(MARK)), Sense::hover());
    let center = pos2(block.left() + MARK / 2.0, block.top() + MARK / 2.0);
    widgets::disc(ui.painter(), center, MARK / 2.0, FAILED_RED);
    ui.painter().text(center, Align2::CENTER_CENTER, "!", widgets::semibold(S.of(15.0)), Color32::WHITE);
    ui.painter().galley(pos2(block.left() + text_left, block.top()), galley, TEXT);
    ui.add_space(S.of(10.0));
    let (row, _) = ui.allocate_exact_size(vec2(width, CARD_BUTTON_H), Sense::hover());
    let mut x = row.left() + text_left;
    let asks = [("Try again", CardEvent::Run { cell: id.to_string() }), ("Open report", CardEvent::OpenReport { cell: id.to_string() })];
    for (word, event) in asks {
        let galley = ui.painter().layout_no_wrap(word.to_string(), widgets::body(S), TEXT);
        let spot = Rect::from_min_size(pos2(x, row.top()), vec2(galley.size().x + 2.0 * CARD_BUTTON_PAD, CARD_BUTTON_H));
        let response = ui.interact(spot, ui.id().with(("card-failed", id, word)), Sense::click());
        let fill = if response.hovered() { widgets::SELECTED } else { Color32::WHITE };
        widgets::outlined(ui.painter(), spot, S.of(7.0), fill, BORDER);
        ui.painter().galley(spot.center() - galley.size() / 2.0, galley, TEXT);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, word));
        if response.clicked() {
            events.push(event);
        }
        x = spot.right() + S.of(8.0);
    }
    ui.add_space(S.of(14.0));
    hairline(ui, width);
    ui.add_space(S.of(4.0));
}

/// The prompt as plain wrapped text that edits in place, with the hint
/// in gray while it is empty. Returns the new text when an edit is let go
/// and differs from the cell's. What was let go stays shown until the cell
/// echoes it.
fn prompt_field(ui: &mut egui::Ui, id: &str, cell: &Cell, hint: &str, width: f32) -> Option<String> {
    let field_id = ui.id().with(("card-prompt", id));
    let stored = cell.prompt();
    let pass = ui.ctx().cumulative_pass_nr();
    let sent_key = field_id.with("sent");
    let echo: Option<(u64, String)> = ui.data(|data| data.get_temp(sent_key));
    let echoed = match echo {
        Some((at, text)) if text != stored && pass.saturating_sub(at) < ECHO_PASSES => Some(text),
        Some(_) => {
            ui.data_mut(|data| data.remove::<(u64, String)>(sent_key));
            None
        }
        None => None,
    };
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let mut text = if focused {
        ui.data(|data| data.get_temp::<String>(field_id.with("buffer"))).unwrap_or_else(|| echoed.clone().unwrap_or_else(|| stored.clone()))
    } else {
        echoed.unwrap_or_else(|| stored.clone())
    };
    let font = widgets::body(S);
    let line = ui.painter().layout_no_wrap("A".to_string(), font.clone(), TEXT).size().y;
    let galley = ui.painter().layout(text.clone(), font.clone(), TEXT, width);
    let height = if text.is_empty() { line * EMPTY_LINES } else { galley.size().y.max(line) };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    // In a child of its own, so the field never moves the page's cursor.
    let mut field = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    let response = field.add(
        egui::TextEdit::multiline(&mut text)
            .id(field_id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_width(width)
            .desired_rows(1)
            .layouter(&mut |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                let job = egui::text::LayoutJob::simple(text.as_str().to_string(), font.clone(), TEXT, wrap);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            }),
    );
    if text.is_empty() && !response.has_focus() {
        ui.painter().text(rect.left_top(), Align2::LEFT_TOP, hint, widgets::body(S), MUTED);
    }
    if response.changed() {
        ui.data_mut(|data| data.insert_temp(field_id.with("buffer"), text.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|data| data.remove::<String>(field_id.with("buffer")));
        if text != stored {
            ui.data_mut(|data| data.insert_temp(sent_key, (pass, text.clone())));
            return Some(text);
        }
    }
    None
}

/// The references: the gray label with "Add reference", then a row for
/// each reference and each empty slot, dragged by their handles to reorder.
#[allow(clippy::too_many_arguments)]
fn references(
    ui: &mut egui::Ui,
    declared: &PanelCell,
    cell: &Cell,
    state: &CardState,
    width: f32,
    sent: &mut Vec<Sent>,
    events: &mut Vec<CardEvent>,
) {
    let id = declared.cell.as_str();
    let (label, _) = ui.allocate_exact_size(vec2(width, LABEL_ROW), Sense::hover());
    ui.painter().text(label.left_center(), Align2::LEFT_CENTER, "References", card_small(), MUTED);
    let add_words = ui.painter().layout_no_wrap("Add reference".to_string(), card_small(), MUTED);
    let add = Rect::from_min_max(pos2(label.right() - add_words.size().x - S.of(18.0), label.top()), label.max);
    let add_response = ui.interact(add, ui.id().with(("card-add-reference", id)), Sense::click());
    let add_color = if add_response.hovered() { TEXT } else { MUTED };
    crate::toolbar::paint_icon(ui.painter(), pos2(add.left() + S.of(6.0), add.center().y), "plus", S.of(13.0), add_color);
    ui.painter().galley_with_override_text_color(pos2(add.right() - add_words.size().x, add.center().y - add_words.size().y / 2.0), add_words, add_color);
    add_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add reference"));
    if add_response.clicked() {
        events.push(CardEvent::AddReference { cell: id.to_string(), media_types: declared.reference_types.clone() });
    }
    ui.add_space(S.of(4.0));

    let parts: Vec<&CellPart> = cell.input.iter().filter(|part| is_reference(part)).collect();
    let matched: Vec<Option<usize>> = parts.iter().map(|part| split_role(part_role(part), &declared.roles).0).collect();
    let entries = entries(&matched, &declared.roles);
    if entries.is_empty() {
        let (line, _) = ui.allocate_exact_size(vec2(width, S.of(36.0)), Sense::hover());
        ui.painter().text(line.left_center(), Align2::LEFT_CENTER, "No references yet.", widgets::body(S), MUTED);
        return;
    }

    let drag_id = ui.id().with(("card-reference-drag", id));
    let mut drag: Option<(usize, f32)> = ui.data(|data| data.get_temp(drag_id));
    if drag.is_some_and(|(from, _)| from >= parts.len()) {
        drag = None;
    }
    let mut placed: Vec<(usize, Rect)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            ui.add_space(REFERENCE_GAP);
        }
        let (row, _) = ui.allocate_exact_size(vec2(width, REFERENCE_ROW), Sense::hover());
        match *entry {
            Entry::Reference(reference) => {
                placed.push((reference, row));
                let lifted = drag.is_some_and(|(from, _)| from == reference);
                if lifted {
                    // The row's place waits, empty, while it is dragged.
                    handle_dots(ui.painter(), row, WAITS);
                    let thumb = thumb_rect(row);
                    dashed_box(ui.painter(), Rect::from_min_max(thumb.min, pos2(row.right(), thumb.bottom())));
                    continue;
                }
                let handle = Rect::from_min_size(row.min, vec2(HANDLE_W, row.height()));
                let grip = ui
                    .interact(handle, ui.id().with(("card-reference-handle", id, reference)), Sense::drag())
                    .on_hover_cursor(egui::CursorIcon::Grab);
                if grip.drag_started()
                    && parts.len() > 1
                    && let Some(at) = grip.interact_pointer_pos()
                {
                    drag = Some((reference, at.y - row.top()));
                }
                reference_row(ui, row, index + 1, reference, parts[reference], declared, cell, state, sent, events);
            }
            Entry::Slot(role) => slot_row(ui, row, index + 1, &declared.roles[role], declared, events),
        }
    }

    // The dragged row follows the pointer, over a blue line where it will
    // land, as ig-reference-reorder shows. The release reorders.
    if let Some((from, grab)) = drag {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let others: Vec<Rect> = placed.iter().filter(|(reference, _)| *reference != from).map(|(_, rect)| *rect).collect();
        if let Some(pointer) = pointer {
            let to = others.iter().filter(|rect| rect.center().y < pointer.y).count();
            let y = match others.get(to) {
                Some(below) => below.top() - REFERENCE_GAP / 2.0,
                None => others.last().map_or(pointer.y, |above| above.bottom() + REFERENCE_GAP / 2.0),
            };
            ui.painter().hline(placed[0].1.left()..=placed[0].1.right(), y, Stroke::new(2.0, LANDING));
            let floating = Rect::from_min_size(pos2(placed[0].1.left() + S.of(14.0), pointer.y - grab), vec2(width - S.of(14.0), REFERENCE_ROW));
            let layer = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, drag_id));
            widgets::soft_shadow(&layer, floating, S.of(8.0));
            layer.rect_filled(floating, S.of(8.0), Color32::WHITE);
            let number = entries.iter().position(|entry| *entry == Entry::Reference(from)).unwrap_or(from) + 1;
            paint_reference(&layer, ui, floating, number, parts[from], state);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if !ui.input(|input| input.pointer.primary_down()) {
                if to != from {
                    events.push(CardEvent::Edit { cell: id.to_string(), input: reorder_references(&cell.input, from, to), settings: cell.settings.clone() });
                }
                drag = None;
            }
        } else {
            drag = None;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            drag = None;
        }
    }
    ui.data_mut(|data| match drag {
        Some(drag) => {
            data.insert_temp(drag_id, drag);
        }
        None => {
            data.remove::<(usize, f32)>(drag_id);
        }
    });
}

/// Where a reference row's image sits.
fn thumb_rect(row: Rect) -> Rect {
    Rect::from_min_size(pos2(row.left() + HANDLE_W + REFERENCE_NUMBER_W, row.center().y - REFERENCE_THUMB.y / 2.0), REFERENCE_THUMB)
}

/// The six dots a row is dragged by.
fn handle_dots(painter: &Painter, row: Rect, color: Color32) {
    let center = pos2(row.left() + HANDLE_W / 2.0, row.center().y);
    for column in [-1.0, 1.0] {
        for line in [-1.0, 0.0, 1.0] {
            painter.circle_filled(center + vec2(column * S.of(3.0), line * S.of(5.0)), S.of(1.5), color);
        }
    }
}

/// A rounded box drawn in dashes, as an empty slot or a dragged row's place.
fn dashed_box(painter: &Painter, rect: Rect) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    painter.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.0, widgets::RAIL), 4.0, 3.0));
}

/// A reference's row: its handle, number, image, role, and file, with its
/// menu at the right.
#[allow(clippy::too_many_arguments)]
fn reference_row(
    ui: &mut egui::Ui,
    row: Rect,
    number: usize,
    reference: usize,
    part: &CellPart,
    declared: &PanelCell,
    cell: &Cell,
    state: &CardState,
    sent: &mut Vec<Sent>,
    events: &mut Vec<CardEvent>,
) {
    let id = declared.cell.as_str();
    let hovered = ui.rect_contains_pointer(row);
    let painter = ui.painter().clone();
    // The whole row first, so the role and "…" over it take their own
    // clicks; a right-click elsewhere on it opens its menu.
    let whole = ui.interact(row, ui.id().with(("card-reference-row", id, reference)), Sense::click());
    let more = Rect::from_center_size(pos2(row.right() - MORE / 2.0, row.center().y), vec2(MORE, MORE));
    let more_response = ui.interact(more, ui.id().with(("card-reference-more", id, reference)), Sense::click());
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more_response));
    if hovered || menu_open {
        painter.rect_filled(row.expand2(vec2(S.of(4.0), 0.0)), S.of(6.0), widgets::SELECTED);
    }
    let text = paint_reference(&painter, ui, row, number, part, state);

    // The role opens a menu of the declared roles and "Name what it's for…";
    // the latter names it in place.
    let (current, words) = split_role(part_role(part), &declared.roles);
    let rename_id = ui.id().with(("card-reference-rename", id));
    let renaming: Option<(usize, String, bool)> = ui.data(|data| data.get_temp(rename_id));
    match renaming {
        Some((at, mut typed, focus)) if at == reference => {
            let field_id = rename_id.with("field");
            let mut field = ui.new_child(UiBuilder::new().max_rect(text.role).layout(Layout::left_to_right(Align::Center)));
            let edit = field.add(
                egui::TextEdit::singleline(&mut typed)
                    .id(field_id)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .font(widgets::body(S))
                    .desired_width(text.role.width()),
            );
            if focus {
                edit.request_focus();
            }
            let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
            if cancel || edit.lost_focus() {
                ui.data_mut(|data| data.remove::<(usize, String, bool)>(rename_id));
                if !cancel && typed.trim() != words {
                    let role = current.map(|index| &declared.roles[index]);
                    let field = part_field(part).map(str::to_string);
                    events.push(CardEvent::Edit {
                        cell: id.to_string(),
                        input: with_role(&cell.input, reference, role_words(role, typed.trim()), role.map_or(field, |role| role.field.clone())),
                        settings: cell.settings.clone(),
                    });
                }
            } else {
                ui.data_mut(|data| data.insert_temp(rename_id, (reference, typed, false)));
            }
        }
        _ => {
            let role_response = ui.interact(text.role, ui.id().with(("card-reference-role", id, reference)), Sense::click());
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&role_response));
            if role_response.hovered() || open {
                widgets::chevron(&painter, pos2(text.role_end + S.of(10.0), text.role.center().y), TEXT, S);
            }
            egui::Popup::menu(&role_response).show(|ui| {
                let mut items: Vec<widgets::MenuItem> = declared
                    .roles
                    .iter()
                    .enumerate()
                    .map(|(index, role)| widgets::MenuItem {
                        tip: None,
                        label: &role.label,
                        icon: (current == Some(index)).then_some("check"),
                        enabled: true,
                        submenu: false,
                        divider: false,
                    })
                    .collect();
                items.push(widgets::MenuItem { tip: None, label: "Name what it's for…", icon: None, enabled: true, submenu: false, divider: !declared.roles.is_empty() });
                match widgets::menu_rows(ui, &items) {
                    Some(index) if index < declared.roles.len() => {
                        let role = &declared.roles[index];
                        events.push(CardEvent::Edit {
                            cell: id.to_string(),
                            input: with_role(&cell.input, reference, role_words(Some(role), words), role.field.clone()),
                            settings: cell.settings.clone(),
                        });
                    }
                    Some(_) => {
                        ui.data_mut(|data| data.insert_temp(rename_id, (reference, words.to_string(), true)));
                    }
                    None => {}
                }
            });
        }
    }

    // The row's menu: Choose…, the declared rows, and Clear, from "…" or
    // a right-click.
    if hovered || menu_open {
        crate::toolbar::paint_icon(&painter, more.center(), "more-horiz", S.of(18.0), TEXT);
    }
    let mut chosen = None;
    egui::Popup::menu(&more_response).show(|ui| chosen = reference_menu(ui, declared));
    egui::Popup::context_menu(&whole).show(|ui| chosen = chosen.or(reference_menu(ui, declared)));
    match chosen {
        Some(0) => events.push(CardEvent::Choose {
            cell: id.to_string(),
            reference: Some(reference),
            role: None,
            media_types: declared.reference_types.clone(),
        }),
        Some(index) if index <= declared.reference_menu.len() => {
            if let Some(action) = &declared.reference_menu[index - 1].action {
                sent.push(Sent::Run(action.with_value(json!(reference))));
            }
        }
        Some(_) => events.push(CardEvent::Edit { cell: id.to_string(), input: without_reference(&cell.input, reference), settings: cell.settings.clone() }),
        None => {}
    }
}

/// A reference row's menu: Choose…, the declared rows, and Clear. Returns
/// the row chosen, by its place in that order.
fn reference_menu(ui: &mut egui::Ui, declared: &PanelCell) -> Option<usize> {
    let mut items = vec![widgets::MenuItem { tip: None, label: "Choose…", icon: None, enabled: true, submenu: false, divider: false }];
    items.extend(declared.reference_menu.iter().map(|row| widgets::MenuItem {
        tip: row.tip.as_deref(),
        label: &row.label,
        icon: row.icon.as_deref(),
        enabled: row.enabled,
        submenu: false,
        divider: row.divider,
    }));
    items.push(widgets::MenuItem { tip: None, label: "Clear", icon: None, enabled: true, submenu: false, divider: true });
    widgets::menu_rows(ui, &items)
}

/// Where a reference row's words went: the role's line, and where its
/// words end.
struct ReferenceText {
    role: Rect,
    role_end: f32,
}

/// A reference's row, without its pointer: the handle, the number, the
/// image with its badge, the role, and the file in gray. Drawn in place
/// and, while dragged, under the pointer.
#[allow(clippy::too_many_arguments)]
fn paint_reference(
    painter: &Painter,
    ui: &egui::Ui,
    row: Rect,
    number: usize,
    part: &CellPart,
    state: &CardState,
) -> ReferenceText {
    handle_dots(painter, row, MUTED);
    painter.text(
        pos2(row.left() + HANDLE_W + REFERENCE_NUMBER_W / 2.0, row.center().y),
        Align2::CENTER_CENTER,
        number.to_string(),
        widgets::body(S),
        MUTED,
    );
    let thumb = thumb_rect(row);
    reference_thumb(ui, painter, thumb, part, state);
    let left = thumb.right() + S.of(12.0);
    let right = row.right() - MORE - S.of(4.0);
    let file = part_file(part);
    let role = part_role(part);
    let (main, second) = if role.is_empty() { (file.as_str(), "") } else { (role, file.as_str()) };
    let role_y = if second.is_empty() { row.center().y } else { row.center().y - S.of(9.0) };
    let mut job = egui::text::LayoutJob::simple_singleline(main.to_string(), widgets::body(S), TEXT);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(1.0));
    let galley = painter.layout_job(job);
    let role_end = left + galley.size().x;
    painter.galley(pos2(left, role_y - galley.size().y / 2.0), galley, TEXT);
    if !second.is_empty() {
        let file_y = row.center().y + S.of(10.0);
        let mut job = egui::text::LayoutJob::simple_singleline(second.to_string(), card_small(), MUTED);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(1.0));
        let galley = painter.layout_job(job);
        painter.galley(pos2(left, file_y - galley.size().y / 2.0), galley, MUTED);
    }
    ReferenceText {
        role: Rect::from_min_max(pos2(left, role_y - S.of(10.0)), pos2(right, role_y + S.of(10.0))),
        role_end,
    }
}

/// A slot's role not yet filled, such as an End frame: a dashed image
/// and "Choose…", which fills it.
fn slot_row(ui: &mut egui::Ui, row: Rect, number: usize, role: &ReferenceRole, declared: &PanelCell, events: &mut Vec<CardEvent>) {
    let painter = ui.painter().clone();
    handle_dots(&painter, row, WAITS);
    painter.text(pos2(row.left() + HANDLE_W + REFERENCE_NUMBER_W / 2.0, row.center().y), Align2::CENTER_CENTER, number.to_string(), widgets::body(S), MUTED);
    let thumb = thumb_rect(row);
    dashed_box(&painter, thumb);
    let left = thumb.right() + S.of(12.0);
    painter.text(pos2(left, row.center().y - S.of(9.0)), Align2::LEFT_CENTER, &role.label, widgets::body(S), TEXT);
    let area = Rect::from_min_max(thumb.min, pos2(row.right(), thumb.bottom()));
    let response = ui
        .interact(area, ui.id().with(("card-slot", declared.cell.as_str(), &role.label)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if response.hovered() { TEXT } else { MUTED };
    painter.text(pos2(left, row.center().y + S.of(10.0)), Align2::LEFT_CENTER, "Choose…", card_small(), color);
    if response.clicked() {
        events.push(CardEvent::Choose {
            cell: declared.cell.clone(),
            reference: None,
            role: Some(role.clone()),
            media_types: declared.reference_types.clone(),
        });
    }
}

/// A reference's image: its preview, as `previewed` answered, with its
/// badge; an image file read from the project; or an icon for its kind.
fn reference_thumb(ui: &egui::Ui, painter: &Painter, rect: Rect, part: &CellPart, state: &CardState) {
    painter.rect_filled(rect, S.of(4.0), KIND_FILL);
    let inside = rect.shrink(1.0);
    let clipped = painter.with_clip_rect(inside.intersect(painter.clip_rect()));
    let preview = state.previews.and_then(|previews| previews.get(&reference_key(part)));
    let mut drawn = false;
    if let Some(thumbnail) = preview.and_then(|preview| preview.thumbnail.as_ref())
        && let Some((texture, _)) = crate::raster::raw_image(ui.ctx(), &thumbnail.path, thumbnail.width, thumbnail.height, thumbnail.revision, &[], false)
    {
        let size = vec2(thumbnail.width.max(1) as f32, thumbnail.height.max(1) as f32);
        clipped.image(texture, cover(inside, size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        drawn = true;
    }
    if !drawn
        && let CellPart::Path { path, media_type, .. } = part
        && media_type.starts_with("image/")
        && let Some(texture) = reference_image(ui.ctx(), &resolve(state.root, path))
    {
        clipped.image(texture.id(), cover(inside, texture.size_vec2()), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        drawn = true;
    }
    if !drawn {
        let icon = match part_media_type(part) {
            media if media.starts_with("video/") => "media-video",
            media if media.starts_with("audio/") => "sound-high",
            media if media.starts_with("image/") => "media-image",
            _ => "page",
        };
        crate::toolbar::paint_icon(painter, rect.center(), icon, S.of(17.0), MUTED);
    }
    if let Some(badge) = preview.map(|preview| preview.badge.as_str()).filter(|badge| !badge.is_empty()) {
        let galley = painter.layout_no_wrap(badge.to_string(), egui::FontId::proportional(S.of(12.0)), Color32::WHITE);
        let size = galley.size() + vec2(S.of(8.0), S.of(3.0));
        let spot = Rect::from_min_size(rect.right_bottom() - size - vec2(S.of(3.0), S.of(3.0)), size);
        painter.rect_filled(spot, S.of(4.0), BADGE_FILL);
        painter.galley(spot.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
    painter.rect_stroke(rect, S.of(4.0), Stroke::new(1.0, BORDER), StrokeKind::Inside);
}

/// `size` scaled to cover `area`, centered on it, so the image fills its
/// box and is cut at its edges.
fn cover(area: Rect, size: egui::Vec2) -> Rect {
    let scale = (area.width() / size.x.max(1.0)).max(area.height() / size.y.max(1.0));
    Rect::from_center_size(area.center(), size * scale)
}

/// A path as read: as it is when absolute, or else inside the project.
fn resolve(root: Option<&std::path::Path>, path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    root.map_or_else(|| path.to_path_buf(), |root| root.join(path))
}

/// An image file as a texture, read once for each time the file changes.
/// None when it cannot be read.
fn reference_image(ctx: &egui::Context, file: &std::path::Path) -> Option<egui::TextureHandle> {
    type Cache = HashMap<(std::path::PathBuf, Option<std::time::SystemTime>), Option<egui::TextureHandle>>;
    let changed = std::fs::metadata(file).and_then(|meta| meta.modified()).ok();
    let key = (file.to_path_buf(), changed);
    let id = egui::Id::new("panel-reference-images");
    if let Some(known) = ctx.data(|data| data.get_temp::<Cache>(id).and_then(|cache| cache.get(&key).cloned())) {
        return known;
    }
    let texture = std::fs::read(file).ok().and_then(|bytes| eframe::icon_data::from_png_bytes(&bytes).ok()).map(|icon| {
        let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
        ctx.load_texture(file.display().to_string(), image, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(id).insert(key, texture.clone());
    });
    texture
}

/// The key a reference's preview is kept under: its path, and for an item
/// of a file, the item's id after a `#`.
pub(crate) fn reference_key(part: &CellPart) -> String {
    match part {
        CellPart::Path { path, .. } => path.clone(),
        CellPart::Item { item, .. } => match &item.item {
            Some(id) => format!("{}#{id}", item.path),
            None => item.path.clone(),
        },
        CellPart::Text { .. } | CellPart::Other(_) => String::new(),
    }
}

fn is_reference(part: &CellPart) -> bool {
    matches!(part, CellPart::Path { .. } | CellPart::Item { .. })
}

fn part_role(part: &CellPart) -> &str {
    match part {
        CellPart::Path { role, .. } | CellPart::Item { role, .. } => role.as_deref().unwrap_or_default(),
        CellPart::Text { .. } | CellPart::Other(_) => "",
    }
}

fn part_field(part: &CellPart) -> Option<&str> {
    match part {
        CellPart::Path { field, .. } | CellPart::Item { field, .. } => field.as_deref(),
        CellPart::Text { .. } | CellPart::Other(_) => None,
    }
}

fn part_media_type(part: &CellPart) -> &str {
    match part {
        CellPart::Path { media_type, .. } | CellPart::Item { media_type, .. } => media_type,
        CellPart::Text { .. } | CellPart::Other(_) => "",
    }
}

/// A reference's file name, without its folders.
fn part_file(part: &CellPart) -> String {
    let path = match part {
        CellPart::Path { path, .. } => path.as_str(),
        CellPart::Item { item, .. } => item.path.as_str(),
        CellPart::Text { .. } | CellPart::Other(_) => "",
    };
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// A stored role, "<menu word>, <the user's words>", as the declared role
/// whose label is its menu word, by index, and the user's words. A role
/// with no declared menu word is all the user's words.
fn split_role<'a>(role: &'a str, roles: &[ReferenceRole]) -> (Option<usize>, &'a str) {
    if let Some(index) = roles.iter().position(|known| known.label == role) {
        return (Some(index), "");
    }
    if let Some((word, words)) = role.split_once(", ")
        && let Some(index) = roles.iter().position(|known| known.label == word)
    {
        return (Some(index), words);
    }
    (None, role)
}

/// A role as stored, from its menu word and the user's words.
fn role_words(role: Option<&ReferenceRole>, words: &str) -> String {
    match (role, words.trim()) {
        (Some(role), "") => role.label.clone(),
        (Some(role), words) => format!("{}, {words}", role.label),
        (None, words) => words.to_string(),
    }
}

/// The list's rows: the references in order, and each slot role no
/// reference fills, after the rows of the slot roles before it, or first.
fn entries(matched: &[Option<usize>], roles: &[ReferenceRole]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = (0..matched.len()).map(Entry::Reference).collect();
    for (index, role) in roles.iter().enumerate() {
        if !role.slot || matched.contains(&Some(index)) {
            continue;
        }
        let slot_before = |entry: &Entry| match *entry {
            Entry::Reference(reference) => matched[reference].is_some_and(|at| at < index && roles[at].slot),
            Entry::Slot(at) => at < index,
        };
        let at = entries.iter().rposition(slot_before).map_or(0, |at| at + 1);
        entries.insert(at, Entry::Slot(index));
    }
    entries
}

/// The prompt with its words replaced by `text`, in the place of its
/// first text part, its references kept where they are.
fn with_prompt(input: &[CellPart], text: &str) -> Vec<CellPart> {
    let mut parts = Vec::with_capacity(input.len());
    let mut placed = false;
    for part in input {
        match part {
            CellPart::Text { .. } => {
                if !placed {
                    parts.push(CellPart::Text { text: text.to_string() });
                    placed = true;
                }
            }
            other => parts.push(other.clone()),
        }
    }
    if !placed {
        parts.insert(0, CellPart::Text { text: text.to_string() });
    }
    parts
}

/// The input with reference `from` moved to `to`, both counted among the
/// references, `to` after it is taken out. Every other part stays.
fn reorder_references(input: &[CellPart], from: usize, to: usize) -> Vec<CellPart> {
    let places: Vec<usize> = input.iter().enumerate().filter(|(_, part)| is_reference(part)).map(|(index, _)| index).collect();
    if from >= places.len() {
        return input.to_vec();
    }
    let mut references: Vec<CellPart> = places.iter().map(|index| input[*index].clone()).collect();
    let moved = references.remove(from);
    references.insert(to.min(references.len()), moved);
    let mut parts = input.to_vec();
    for (place, part) in places.into_iter().zip(references) {
        parts[place] = part;
    }
    parts
}

/// The input with reference `reference` given `role` and `field`.
fn with_role(input: &[CellPart], reference: usize, role: String, field: Option<String>) -> Vec<CellPart> {
    let role = (!role.is_empty()).then_some(role);
    let mut seen = 0;
    input
        .iter()
        .map(|part| {
            if !is_reference(part) {
                return part.clone();
            }
            seen += 1;
            if seen - 1 != reference {
                return part.clone();
            }
            let mut part = part.clone();
            match &mut part {
                CellPart::Path { role: stored, field: filled, .. } | CellPart::Item { role: stored, field: filled, .. } => {
                    *stored = role.clone();
                    *filled = field.clone();
                }
                CellPart::Text { .. } | CellPart::Other(_) => {}
            }
            part
        })
        .collect()
}

/// The input without reference `reference`, as Clear leaves it.
fn without_reference(input: &[CellPart], reference: usize) -> Vec<CellPart> {
    let mut seen = 0;
    input
        .iter()
        .filter(|part| {
            if !is_reference(part) {
                return true;
            }
            seen += 1;
            seen - 1 != reference
        })
        .cloned()
        .collect()
}

/// A setting's stored value as its row shows it: a choice's words as
/// their index, and a missing value as the row's empty one.
fn setting_value(kind: &RowKind, value: Option<&Value>) -> PropValue {
    let words = |value: Option<&Value>| match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    match kind {
        RowKind::Toggle => PropValue::Bool(value.and_then(Value::as_bool).unwrap_or(false)),
        RowKind::Number { min, .. } | RowKind::Field { min, .. } => PropValue::Number(value.and_then(Value::as_f64).unwrap_or(*min)),
        RowKind::Text | RowKind::Paragraph { .. } => PropValue::Text(words(value)),
        RowKind::Choice { options, .. } | RowKind::Preset { options, .. } => {
            let index = match value {
                Some(Value::String(text)) => options.iter().position(|option| option == text).unwrap_or(0),
                Some(other) => other.as_u64().unwrap_or(0) as usize,
                None => 0,
            };
            PropValue::Index(index.min(u8::MAX as usize) as u8)
        }
        RowKind::Rgb { .. } => match value.and_then(|value| serde_json::from_value::<[u8; 3]>(value.clone()).ok()) {
            Some(rgb) => PropValue::Rgb(rgb),
            None => PropValue::Rgb([0, 0, 0]),
        },
        _ => PropValue::Idle,
    }
}

/// A row's edit as the setting stores it: a choice as its words, a whole
/// number with no places as an integer.
fn setting_json(kind: &RowKind, value: &PropValue) -> Option<Value> {
    match (kind, value) {
        (_, PropValue::Bool(on)) => Some(json!(on)),
        (RowKind::Number { places: 0, .. } | RowKind::Field { places: 0, .. }, PropValue::Number(number)) if number.fract() == 0.0 => {
            Some(json!(*number as i64))
        }
        (_, PropValue::Number(number)) => Some(json!(number)),
        (_, PropValue::Text(text)) => Some(json!(text)),
        (RowKind::Choice { options, .. } | RowKind::Preset { options, .. }, PropValue::Index(index)) => {
            Some(options.get(usize::from(*index)).map_or_else(|| json!(index), |option| json!(option)))
        }
        (_, PropValue::Index(index)) => Some(json!(index)),
        (_, PropValue::Rgb(rgb)) => Some(json!(rgb)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches page tabs cut at the panel's edge, as "Prompt" was at the
    /// panel's own width, the narrowest it is drawn: the Image Generator's
    /// four tabs fit whole, and tabs that already fit keep their look.
    #[test]
    fn the_page_tabs_fit_the_panel() {
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let width = WIDTH - 2.0 * PAD;
        let labels = ["Layers", "Properties", "Adjustments", "Prompt"];
        let fitting = ["Layers", "Properties"];
        let mut four = None;
        let mut two = None;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 900.0))), ..Default::default() },
            |ui| {
                let painter = ui.painter().clone();
                // The last tab is the chosen one, in semibold.
                let measure = |words: &[&str], index: usize, size: f32| {
                    painter.layout_no_wrap(words[index].to_string(), page_font(index == words.len() - 1, size), TEXT).size().x
                };
                four = Some(tab_row(width, labels.len(), |index, size| measure(&labels, index, size)));
                two = Some(tab_row(width, fitting.len(), |index, size| measure(&fitting, index, size)));
            },
        );
        // The fonts' textures are never drawn here.
        output.textures_delta.clear();
        let four = four.expect("the tabs were not laid out");
        let total: f32 = four.widths.iter().sum::<f32>() + 2.0 * labels.len() as f32 * four.pad;
        assert!(total <= width + 0.01, "the tabs run past the panel: {four:?}, {total} of {width}");
        assert_eq!(four.elided, None, "a tab was cut: {four:?}");
        assert!(four.pad >= PAGE_PAD_TIGHT, "the tabs touch: {four:?}");
        let two = two.expect("the tabs were not laid out");
        assert_eq!((two.size, two.pad, two.elided), (PAGE_TEXT, PAGE_PAD, None), "tabs that fit changed their look");
        // Far too narrow, the longest is cut to what is left.
        let cut = tab_row(100.0, 3, |index, size| [40.0, 80.0, 30.0][index] * size / PAGE_TEXT);
        assert_eq!(cut.elided, Some(1));
        let total: f32 = cut.widths.iter().sum::<f32>() + 6.0 * cut.pad;
        assert!((total - 100.0).abs() < 0.01, "the cut tab does not fill the row: {cut:?}");
    }

    fn role(label: &str, slot: bool) -> ReferenceRole {
        ReferenceRole { label: label.into(), field: Some(label.to_lowercase()), slot }
    }

    fn file(path: &str, role: Option<&str>) -> CellPart {
        CellPart::Path { path: path.into(), media_type: "image/png".into(), role: role.map(String::from), field: None }
    }

    /// Catches a role read without its menu word, or a menu word read as
    /// the user's words.
    #[test]
    fn a_role_is_its_menu_word_and_the_users_words() {
        let roles = [role("Reference", false), role("Start frame", true)];
        assert_eq!(split_role("Start frame, the keeper", &roles), (Some(1), "the keeper"));
        assert_eq!(split_role("Start frame", &roles), (Some(1), ""));
        assert_eq!(split_role("the light, at night", &roles), (None, "the light, at night"));
        assert_eq!(role_words(Some(&roles[0]), " the keeper "), "Reference, the keeper");
        assert_eq!(role_words(Some(&roles[0]), ""), "Reference");
        assert_eq!(role_words(None, "the keeper"), "the keeper");
    }

    /// Catches an empty slot drawn out of place: it follows the slots
    /// before it, or comes first, and a filled slot shows no empty row.
    #[test]
    fn empty_slots_sit_after_the_slots_before_them() {
        let roles = [role("Start frame", true), role("End frame", true), role("Reference", false)];
        assert_eq!(
            entries(&[Some(0), Some(2)], &roles),
            vec![Entry::Reference(0), Entry::Slot(1), Entry::Reference(1)]
        );
        assert_eq!(entries(&[Some(2)], &roles), vec![Entry::Slot(0), Entry::Slot(1), Entry::Reference(0)]);
        assert_eq!(entries(&[Some(0), Some(1)], &roles), vec![Entry::Reference(0), Entry::Reference(1)]);
        assert!(entries(&[], &[role("Reference", false)]).is_empty());
    }

    /// Catches an edit that loses a part: the prompt, a reorder, a role,
    /// and Clear each change only what they say.
    #[test]
    fn edits_change_only_their_own_parts() {
        let input = vec![CellPart::Text { text: "old".into() }, file("a.png", None), file("b.png", Some("Reference")), file("c.png", None)];
        let prompt = with_prompt(&input, "new");
        assert_eq!(prompt[0], CellPart::Text { text: "new".into() });
        assert_eq!(prompt[1..], input[1..]);
        assert_eq!(with_prompt(&input[1..2], "words")[0], CellPart::Text { text: "words".into() });
        let moved = reorder_references(&input, 2, 0);
        assert_eq!(moved, vec![input[0].clone(), input[3].clone(), input[1].clone(), input[2].clone()]);
        assert_eq!(reorder_references(&input, 0, 2), vec![input[0].clone(), input[2].clone(), input[3].clone(), input[1].clone()]);
        let named = with_role(&input, 1, "Start frame, the keeper".into(), Some("first_frame".into()));
        assert_eq!(
            named[2],
            CellPart::Path { path: "b.png".into(), media_type: "image/png".into(), role: Some("Start frame, the keeper".into()), field: Some("first_frame".into()) }
        );
        assert_eq!(without_reference(&input, 0), vec![input[0].clone(), input[2].clone(), input[3].clone()]);
    }

    /// Catches a setting shown or stored in the wrong shape: a choice's
    /// words round-trip through its index, and a whole number stays whole.
    #[test]
    fn settings_keep_their_stored_shape() {
        let choice = RowKind::Choice { options: vec!["1024 × 1024".into(), "1536 × 864".into()], segmented: false, preview: ChoicePreview::Label };
        assert_eq!(setting_value(&choice, Some(&json!("1536 × 864"))), PropValue::Index(1));
        assert_eq!(setting_json(&choice, &PropValue::Index(1)), Some(json!("1536 × 864")));
        let count = RowKind::Number { min: 1.0, max: 8.0, percent: false, places: 0, centered: false };
        assert_eq!(setting_value(&count, None), PropValue::Number(1.0));
        assert_eq!(setting_json(&count, &PropValue::Number(4.0)), Some(json!(4)));
        assert_eq!(setting_value(&RowKind::Text, Some(&json!(1114))), PropValue::Text("1114".into()));
        let seed = RowKind::Field { min: 0.0, max: 4_294_967_295.0, places: 0 };
        assert_eq!(setting_value(&seed, Some(&json!(1114))), PropValue::Number(1114.0));
        assert_eq!(setting_json(&seed, &PropValue::Number(1114.0)), Some(json!(1114)));
        assert_eq!(reference_key(&file("library/a.png", None)), "library/a.png");
    }
}
