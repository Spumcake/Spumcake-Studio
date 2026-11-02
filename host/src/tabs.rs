//! Tab bar: the menu button, the tabs every running sub-application reports,
//! and the plus, from Simple Sketch 1.0.1's `tabs.rs`. A click sends the
//! command its sub-application declared for it.

use crate::toolbar::{self, BAR, ICON, icon_text};
use crate::widgets;
use eframe::egui::{self, Color32, RichText};

const INACTIVE: Color32 = Color32::from_rgb(243, 244, 247);
const TAB_EDGE: Color32 = Color32::from_rgb(229, 231, 235);
const TAB_HEIGHT: f32 = 34.0;

/// One tab in the strip: which running sub-application reported it, and how
/// it reported it.
pub(crate) struct StripTab {
    pub app: usize,
    pub id: String,
    pub label: String,
    /// The tab the window shows.
    pub active: bool,
    pub close_waits: bool,
    pub unsaved: bool,
}

/// What the person did on the tab bar.
pub(crate) enum TabEvent {
    /// A row of the selected tab's menu.
    Menu(protocol::Action),
    Select {
        app: usize,
        tab: String,
    },
    Close {
        app: usize,
        tab: String,
    },
    Plus,
}

/// Draws the tab bar: the menu button with the selected tab's declared menu,
/// every running sub-application's tabs, and the plus. While `locked`, the
/// plus is drawn waiting and does nothing.
pub(crate) fn show(
    ui: &mut egui::Ui,
    tabs: &[StripTab],
    menu_rows: &[protocol::MenuEntry],
    plus_tip: Option<&str>,
    locked: bool,
) -> Vec<TabEvent> {
    let mut events = Vec::new();
    let mut active_tab = None;
    let bar = egui::Frame::new()
        .fill(BAR)
        .inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 6,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.set_height(TAB_HEIGHT);
                menu(ui, menu_rows, &mut events);
                active_tab = tab_row(ui, tabs, &mut events);
                let plus = toolbar::glyph_button(
                    ui,
                    "plus",
                    16.0,
                    egui::vec2(32.0, TAB_HEIGHT),
                    plus_tip,
                    !locked,
                    false,
                );
                if plus.clicked() && !locked {
                    events.push(TabEvent::Plus);
                }
            });
        });
    // The hairline stops under the active tab, which joins the white toolbar.
    let rect = bar.response.rect;
    match active_tab {
        Some(tab) => {
            toolbar::hairline(ui, rect.left(), tab.left(), rect.bottom());
            toolbar::hairline(ui, tab.right(), rect.right(), rect.bottom());
        }
        None => toolbar::hairline(ui, rect.left(), rect.right(), rect.bottom()),
    }
    events
}

/// The menu button. Its rows are the selected tab's declared menu; with none
/// declared, it opens nothing.
fn menu(ui: &mut egui::Ui, rows: &[protocol::MenuEntry], events: &mut Vec<TabEvent>) {
    let response = toolbar::glyph_button(
        ui,
        "menu",
        18.0,
        egui::vec2(32.0, TAB_HEIGHT),
        Some("Menu"),
        true,
        false,
    );
    if rows.is_empty() {
        return;
    }
    egui::Popup::menu(&response).show(|ui| {
        if let Some(action) = widgets::entry_rows(ui, rows).and_then(|row| row.action.clone()) {
            events.push(TabEvent::Menu(action));
        }
    });
}

/// Returns the visible part of the active tab.
fn tab_row(ui: &mut egui::Ui, tabs: &[StripTab], events: &mut Vec<TabEvent>) -> Option<egui::Rect> {
    let width = (ui.available_width() - 36.0).max(48.0);
    let mut active_rect = None;
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(width, TAB_HEIGHT), row, |ui| {
        ui.set_max_width(width);
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                // A full-height row: `horizontal` would start short and grow
                // downward, which puts the tabs below the menu button.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_height(TAB_HEIGHT);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let visible = ui.clip_rect();
                    for strip in tabs {
                        let tab = Tab {
                            app: strip.app,
                            id: &strip.id,
                            name: &strip.label,
                            active: strip.active,
                            close_waits: strip.close_waits,
                            unsaved: strip.unsaved,
                        };
                        let rect = tab_chip(ui, tab, events);
                        if strip.active {
                            active_rect = Some(rect.intersect(visible));
                        }
                    }
                });
            });
    });
    active_rect.filter(|rect| rect.width() > 0.0)
}

struct Tab<'a> {
    app: usize,
    id: &'a str,
    name: &'a str,
    active: bool,
    close_waits: bool,
    /// Unsaved changes: a dot stands in for the close mark until the tab is
    /// hovered.
    unsaved: bool,
}

/// One tab: its name, then its close control. Clicking the tab shows it.
/// Double-clicking does not edit the title. A middle click closes it.
fn tab_chip(ui: &mut egui::Ui, tab: Tab<'_>, events: &mut Vec<TabEvent>) -> egui::Rect {
    const PAD_LEFT: f32 = 14.0;
    const PAD_RIGHT: f32 = 8.0;
    const CLOSE: f32 = 20.0;
    const MIN_WIDTH: f32 = 120.0;
    const MAX_LABEL: f32 = 180.0;

    let galley = egui::WidgetText::from(RichText::new(tab.name).size(13.5).color(ICON))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            MAX_LABEL,
            egui::FontSelection::Default,
        );
    let label_width = galley.size().x;
    let width = (PAD_LEFT + label_width + 10.0 + CLOSE + PAD_RIGHT).max(MIN_WIDTH);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, TAB_HEIGHT), egui::Sense::click());
    paint_tab(ui.painter(), rect, tab.active);

    let label_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + PAD_LEFT, rect.top()),
        egui::vec2(label_width, TAB_HEIGHT),
    );
    let top = rect.center().y - galley.size().y / 2.0;
    ui.painter()
        .galley(egui::pos2(label_rect.left(), top), galley, ICON);

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PAD_RIGHT - CLOSE / 2.0, rect.center().y),
        egui::vec2(CLOSE, CLOSE),
    );
    let close = close_control(ui, close_rect, tab.app, tab.id, !tab.close_waits);
    let dot = tab.unsaved && !response.hovered() && !close.hovered();
    paint_close_mark(ui, close_rect, !tab.close_waits, dot);
    if close.clicked() || (response.middle_clicked() && !tab.close_waits) {
        events.push(TabEvent::Close {
            app: tab.app,
            tab: tab.id.to_string(),
        });
    } else if response.clicked() {
        events.push(TabEvent::Select {
            app: tab.app,
            tab: tab.id.to_string(),
        });
    }
    rect
}

/// Rounded top corners and an outline on the top and sides. The bottom edge
/// is the bar's hairline, or, for the active tab, the white toolbar.
fn paint_tab(painter: &egui::Painter, rect: egui::Rect, active: bool) {
    let fill = if active { Color32::WHITE } else { INACTIVE };
    let radius = egui::CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(rect, radius, fill);

    let edge = rect.shrink2(egui::vec2(0.5, 0.0));
    let r = 8.0;
    let mut outline = vec![egui::pos2(edge.left(), edge.bottom())];
    let corner = |center: egui::Pos2, from: f32, outline: &mut Vec<egui::Pos2>| {
        for step in 0..=6 {
            let angle = from + std::f32::consts::FRAC_PI_2 * step as f32 / 6.0;
            outline.push(center + r * egui::vec2(angle.cos(), angle.sin()));
        }
    };
    let top = edge.top() + 0.5;
    corner(
        egui::pos2(edge.left() + r, top + r),
        std::f32::consts::PI,
        &mut outline,
    );
    corner(
        egui::pos2(edge.right() - r, top + r),
        -std::f32::consts::FRAC_PI_2,
        &mut outline,
    );
    outline.push(egui::pos2(edge.right(), edge.bottom()));
    painter.add(egui::Shape::line(outline, egui::Stroke::new(1.0, TAB_EDGE)));
}

fn close_control(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    app: usize,
    id: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    ui.interact(rect, egui::Id::new(("tab-close", app, id)), sense)
}

/// The close mark, or a dot while the drawing has unsaved changes and the
/// pointer is away from its tab.
fn paint_close_mark(ui: &egui::Ui, rect: egui::Rect, enabled: bool, dot: bool) {
    let color = if enabled { ICON } else { toolbar::ICON_QUIET };
    if dot {
        ui.painter().circle_filled(rect.center(), 4.0, color);
        return;
    }
    let glyph = icon_text("xmark", 11.0, color);
    let galley = egui::WidgetText::from(glyph).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontSelection::Default,
    );
    let top_left = rect.center() - galley.size() / 2.0;
    ui.painter().galley(top_left, galley, color);
}
