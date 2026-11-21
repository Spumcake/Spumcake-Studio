//! A raster tab's filmstrip: the images a sub-application lists, along the
//! bottom of the working area, as the owner's mockup and 0.11's gallery views
//! show. The host draws the thumbnails and the + tile, and sends what the
//! pointer does on them: `focus` at any press, `select` for a click, a
//! thumbnail's own rows for a right-click, the + tile's rows, and `reorder`
//! for a drag. What any of it means is the sub-application's. Items come in
//! runs, each under its label, such as a round of takes, and a drag stays
//! inside its run. A tile carries words on its corner, a mark, the chosen
//! take's star, or, for a take not made yet, a striped or failed tile; an
//! out-of-date take has a red outline, a stack of iterations its count on a
//! dark round badge that opens it, and any tile words under it.

use crate::widgets;
use eframe::egui::{
    self, Color32, CursorIcon, Event, Key, MouseWheelUnit, PointerButton, Popup, Rect, RectAlign, Sense,
    SetOpenCommand, pos2, vec2,
};
use protocol::{Action, Film, FilmItem, FilmPending, FilmStack};
use serde_json::{Value, json};

/// The strip's height, its hairline included, in points.
pub(crate) const HEIGHT: f32 = 68.0;
/// A thumbnail's height. Its width follows its image's proportions, within
/// these, so a very long or very tall image still makes a tile to click.
const THUMB_H: f32 = 44.0;
const THUMB_MIN_W: f32 = 24.0;
const THUMB_MAX_W: f32 = 3.0 * THUMB_H;
/// The + tile, as wide as a landscape thumbnail, as gallery.png shows.
const ADD_W: f32 = 68.0;
/// The space between tiles, and before the first.
const GAP: f32 = 10.0;
const INSET: f32 = 18.0;
const RADIUS: f32 = 4.0;
/// The shown image's outline, as an Image cell's filmstrip draws it.
const SHOWN: Color32 = Color32::from_rgb(37, 99, 235);
/// The + tile's sign, lighter than text, as gallery.png shows it.
const SIGN: Color32 = Color32::from_rgb(160, 164, 172);
/// The room between a menu and the tile it opens from.
const MENU_GAP: f32 = 8.0;
/// A drag this near an end of the strip scrolls it, this far each frame.
const EDGE: f32 = 28.0;
const EDGE_STEP: f32 = 8.0;
/// A wheel notch, as on the image.
const NOTCH_POINTS: f32 = 50.0;
/// What a run's label adds above the thumbnails, and the space between two
/// runs, with a hairline in its middle, as ig-filmstrip-rounds shows.
const LABEL_H: f32 = 22.0;
const RUN_GAP: f32 = 25.0;
/// A run's label, small and gray.
const LABEL_SIZE: f32 = 12.5;
/// The words on a tile's corner, white on a dark rounded badge.
const BADGE_SIZE: f32 = 11.5;
const BADGE_FILL: Color32 = Color32::from_rgba_premultiplied(17, 18, 20, 190);
/// A tile still being made: light gray, striped, as vg-take-running shows.
const PENDING_FILL: Color32 = Color32::from_rgb(243, 244, 246);
const STRIPE: Color32 = Color32::from_rgb(232, 234, 238);
const STRIPE_STEP: f32 = 8.0;
/// A tile that failed: pale, with a red mark, as ig-request-failed shows.
const FAILED_FILL: Color32 = Color32::from_rgb(248, 249, 250);
const FAILED: Color32 = Color32::from_rgb(220, 38, 38);
/// An out-of-date take's outline, as ig-out-of-date draws it.
const OUT_OF_DATE: Color32 = Color32::from_rgb(220, 38, 38);
/// The line of words under the tiles, such as an iteration's number, when
/// any tile has them.
const CAPTION_H: f32 = 16.0;
const CAPTION_SIZE: f32 = 11.5;
/// A stack's count badge, a dark disc at the tile's bottom right.
const STACK_RADIUS: f32 = 8.0;
const SPINNER: Color32 = Color32::from_rgb(120, 126, 136);

/// The strip's height for `film`: taller by a line when its runs have
/// labels, and by another when its tiles have words under them.
pub(crate) fn height(film: &Film) -> f32 {
    let label = if labelled(film) { LABEL_H } else { 0.0 };
    let caption = if captioned(film) { CAPTION_H } else { 0.0 };
    HEIGHT + label + caption
}

/// Whether any tile has words under it.
fn captioned(film: &Film) -> bool {
    film.items.iter().any(|item| !item.caption.is_empty())
}

/// Whether any run has a label to draw above it.
fn labelled(film: &Film) -> bool {
    film.groups.iter().any(|group| !group.label.is_empty())
}

/// Where a filmstrip was drawn this frame, if one was, so notices can sit
/// above it.
pub(crate) fn drawn(ctx: &egui::Context) -> Option<Rect> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<(u64, Rect)>(drawn_id()))
        .filter(|(at, _)| *at == pass)
        .map(|(_, rect)| rect)
}

fn drawn_id() -> egui::Id {
    egui::Id::new("film-drawn")
}

/// The strip's runs, in order: each group's items, then any items past
/// the groups as a run of their own, with no label. An empty run is left
/// out, unless the strip has no items at all.
fn runs(film: &Film) -> Vec<(usize, usize, Option<usize>)> {
    let len = film.items.len();
    let mut runs = Vec::new();
    let mut start = 0;
    for (index, group) in film.groups.iter().enumerate() {
        let end = (start + group.count).min(len);
        if end > start {
            runs.push((start, end, Some(index)));
        }
        start = end;
    }
    if start < len {
        runs.push((start, len, None));
    }
    if runs.is_empty() {
        runs.push((0, 0, None));
    }
    runs
}

/// The area a raster tab gives its image, and the strip under it when the
/// view has a filmstrip. An area too short for both keeps the image.
pub(crate) fn split(rect: Rect, film: Option<&Film>) -> (Rect, Option<Rect>) {
    match film {
        Some(film) if rect.height() > 2.0 * height(film) => {
            let top = rect.bottom() - height(film);
            (Rect::from_min_max(rect.min, pos2(rect.right(), top)), Some(Rect::from_min_max(pos2(rect.left(), top), rect.max)))
        }
        _ => (rect, None),
    }
}

/// The strip's state between frames: how far it is scrolled, and the
/// thumbnail being dragged along it.
#[derive(Default)]
pub(crate) struct Strip {
    scroll: f32,
    moving: Option<Moving>,
    /// A thumbnail being renamed from its menu: its index, the name typed so
    /// far, the row's action, and whether its field still wants the keyboard.
    renaming: Option<(usize, String, Action, bool)>,
    /// Each tab's labelled runs shown last frame, by label, with whether
    /// any of its places was still being made, so a run that appears or
    /// finishes after a tab's first frame is brought into view; opening a
    /// tab never scrolls.
    seen: std::collections::HashMap<String, Vec<(String, bool)>>,
}

/// Each labelled run of `film`, by its label and, after the first run of
/// that label, its count among them, as two cells' rounds can be labelled
/// alike, with whether any of its places is still being made: a failed
/// place is finished.
fn runs_seen(film: &Film, layout: &Layout) -> Vec<(String, bool)> {
    let mut alike: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut runs = Vec::new();
    for span in &layout.spans {
        let Some(group) = span.group.and_then(|group| film.groups.get(group)) else {
            continue;
        };
        let count = alike.entry(group.label.as_str()).or_default();
        let key = if *count == 0 { group.label.clone() } else { format!("{} ({count})", group.label) };
        *count += 1;
        runs.push((key, film.items[span.start..span.end].iter().any(|item| matches!(item.pending, Some(FilmPending::Running { .. })))));
    }
    runs
}

/// The labelled run to bring into view: the last one that is new since
/// `before`, or that was being made then and is finished now.
fn run_to_reveal(before: &[(String, bool)], now: &[(String, bool)]) -> Option<usize> {
    now.iter().rposition(|(label, making)| match before.iter().find(|(seen, _)| seen == label) {
        None => true,
        Some((_, was_making)) => *was_making && !making,
    })
}

/// The scroll that shows the run from `left` to `right` along the strip in
/// a strip `width` wide, moving `scroll` as little as it can.
fn scroll_to_show(scroll: f32, left: f32, right: f32, width: f32) -> f32 {
    if right - scroll > width {
        (right - width + INSET).min(left - INSET)
    } else if left < scroll {
        left - INSET
    } else {
        scroll
    }
}

struct Moving {
    from: usize,
    /// Where the pointer took the thumbnail, from its left edge.
    grab: f32,
}

impl Strip {
    /// The name field over a thumbnail being renamed, on the menus' card
    /// above it. Enter sends the row's action with the name as `value`;
    /// Escape, or a press elsewhere, keeps the old name.
    fn rename_field(&mut self, ui: &mut egui::Ui, layout: &Layout, film: &Film, start: f32, rect: Rect, out: &mut Vec<Action>) {
        let Some((index, text, action, focus)) = self.renaming.as_mut() else {
            return;
        };
        if *index >= film.items.len() {
            self.renaming = None;
            return;
        }
        let left = start + layout.lefts[*index];
        let width = 200.0;
        let at = pos2(left.min(rect.right() - width - 8.0).max(rect.left() + 8.0), rect.top() - 46.0);
        let id = egui::Id::new(("film-rename", *index));
        let mut done = None;
        let shown = egui::Area::new(egui::Id::new("film-rename-card"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = ui.add(egui::TextEdit::singleline(text).id(id).desired_width(width - 24.0));
                    if *focus {
                        edit.request_focus();
                        *focus = false;
                    }
                    if ui.input(|input| input.key_pressed(Key::Enter)) {
                        done = Some(true);
                    } else if ui.input(|input| input.key_pressed(Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        let pressed_away = ui.input(|input| {
            input.pointer.any_pressed() && input.pointer.interact_pos().is_some_and(|pos| !shown.response.rect.contains(pos))
        });
        if done.is_none() && pressed_away {
            done = Some(false);
        }
        if let Some(keep) = done {
            if keep && !text.trim().is_empty() {
                out.push(action.with_value(json!(text.trim())));
            }
            self.renaming = None;
        }
    }
}

/// Where each tile lies along the strip, from its start: the thumbnails,
/// in their runs, then the + tile.
struct Layout {
    lefts: Vec<f32>,
    widths: Vec<f32>,
    spans: Vec<Span>,
    /// Where each hairline between two runs is.
    dividers: Vec<f32>,
    add: f32,
    total: f32,
}

/// One run along the strip: its items, `start` to before `end`, the group
/// it is, and where its first tile starts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    start: usize,
    end: usize,
    group: Option<usize>,
    left: f32,
}

impl Layout {
    /// The run holding item `index`.
    fn span_of(&self, index: usize) -> Span {
        self.spans
            .iter()
            .copied()
            .find(|span| span.start <= index && index < span.end)
            .unwrap_or(Span { start: 0, end: self.lefts.len(), group: None, left: INSET })
    }
}

fn layout(film: &Film) -> Layout {
    let mut x = INSET;
    let mut lefts = Vec::with_capacity(film.items.len());
    let mut widths = Vec::with_capacity(film.items.len());
    let mut spans = Vec::new();
    let mut dividers = Vec::new();
    for (index, (start, end, group)) in runs(film).into_iter().enumerate() {
        if index > 0 {
            let right = x - GAP;
            dividers.push(right + RUN_GAP / 2.0);
            x = right + RUN_GAP;
        }
        spans.push(Span { start, end, group, left: x });
        for item in &film.items[start..end] {
            let width = thumb_width(item);
            lefts.push(x);
            widths.push(width);
            x += width + GAP;
        }
    }
    Layout { lefts, widths, spans, dividers, add: x, total: x + ADD_W + INSET }
}

/// A thumbnail's width at the strip's height, at its image's proportions.
fn thumb_width(item: &FilmItem) -> f32 {
    let (width, height) = (item.thumbnail.width.max(1) as f32, item.thumbnail.height.max(1) as f32);
    (THUMB_H * width / height).clamp(THUMB_MIN_W, THUMB_MAX_W)
}

/// Where a dragged thumbnail lands with the pointer at `x` along the strip:
/// inside its own run, before every other thumbnail of the run whose
/// middle is past it, counted after the dragged one is taken out, as
/// `reorder` wants.
fn landing(layout: &Layout, from: usize, x: f32) -> usize {
    let span = layout.span_of(from);
    span.start
        + (span.start..span.end)
            .filter(|index| *index != from && layout.lefts[*index] + layout.widths[*index] / 2.0 < x)
            .count()
}

/// `action` with these fields added to its argument, as `Action::with_value`
/// adds a value.
fn with_fields(action: &Action, fields: &[(&str, Value)]) -> Action {
    let mut object = match action.arg.clone() {
        Some(Value::Object(object)) => object,
        Some(other) => {
            let mut object = serde_json::Map::new();
            object.insert(String::from("arg"), other);
            object
        }
        None => serde_json::Map::new(),
    };
    for (key, value) in fields {
        object.insert((*key).to_string(), value.clone());
    }
    Action { id: action.id.clone(), arg: Some(Value::Object(object)) }
}

impl Strip {
    /// Draws the strip in `rect` and handles the pointer on it. The actions
    /// it sends go to `out`, in order. While the assistant works the strip is
    /// shown and scrolls, but sends nothing and opens no menu.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        tab: &str,
        film: &Film,
        rect: Rect,
        locked: bool,
        dismissing: bool,
        out: &mut Vec<Action>,
    ) {
        let pass = ui.ctx().cumulative_pass_nr();
        ui.ctx().data_mut(|data| data.insert_temp(drawn_id(), (pass, rect)));
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::WHITE);
        painter.hline(rect.x_range(), rect.top() + 0.5, (1.0, widgets::BORDER));
        let layout = layout(film);
        if self.moving.as_ref().is_some_and(|moving| moving.from >= film.items.len()) {
            self.moving = None;
        }

        let over = ui.rect_contains_pointer(rect);
        let (pos, pressed, primary_down, escape) = ui.input(|input| {
            (input.pointer.latest_pos(), input.pointer.any_pressed(), input.pointer.primary_down(), input.key_pressed(Key::Escape))
        });
        // Any press on the strip tells the sub-application the strip has the
        // keyboard, before whatever else the press does. A press that closes
        // a menu does nothing else.
        if over && pressed {
            if dismissing {
                Popup::close_all(ui.ctx());
            } else if !locked {
                out.push(film.focus.clone());
            }
        }
        let acts = !dismissing && !locked;

        // The wheel scrolls the strip sideways, as does a drag held near
        // either end.
        if over || self.moving.is_some() {
            let events = ui.input(|input| input.events.clone());
            for event in events {
                if let Event::MouseWheel { unit, delta, .. } = event {
                    let scale = match unit {
                        MouseWheelUnit::Point => 1.0,
                        MouseWheelUnit::Line => NOTCH_POINTS,
                        MouseWheelUnit::Page => rect.width(),
                    };
                    self.scroll -= (delta.x + delta.y) * scale;
                }
            }
        }
        if self.moving.is_some()
            && let Some(pos) = pos
        {
            if pos.x < rect.left() + EDGE {
                self.scroll -= EDGE_STEP;
                ui.ctx().request_repaint();
            } else if pos.x > rect.right() - EDGE {
                self.scroll += EDGE_STEP;
                ui.ctx().request_repaint();
            }
        }
        // A round that appears, or finishes, past the strip's edge is
        // brought into view, as "Round 15 appears at the end" has it.
        let now = runs_seen(film, &layout);
        if let Some(before) = self.seen.get(tab)
            && let Some(run) = run_to_reveal(before, &now)
            && let Some(span) = layout.spans.iter().filter(|span| span.group.is_some()).nth(run)
            && span.end > span.start
        {
            let right = layout.lefts[span.end - 1] + layout.widths[span.end - 1];
            self.scroll = scroll_to_show(self.scroll, span.left, right, rect.width());
        }
        self.seen.insert(tab.to_string(), now);
        self.scroll = self.scroll.clamp(0.0, (layout.total - rect.width()).max(0.0));
        let start = rect.left() - self.scroll;
        // Under the runs' labels when they have them, the thumbnails are
        // centered on the rest of the strip.
        let label_h = if labelled(film) { LABEL_H } else { 0.0 };
        let caption_h = if captioned(film) { CAPTION_H } else { 0.0 };
        let top = (rect.top() + label_h + rect.bottom() - caption_h) / 2.0 + 0.5 - THUMB_H / 2.0;
        let tile = |left: f32, width: f32| Rect::from_min_size(pos2(start + left, top), vec2(width, THUMB_H));

        // The open drag lands or is let go.
        if let Some(moving) = &self.moving {
            let along = pos.map_or(0.0, |pos| pos.x - start);
            if escape || locked {
                self.moving = None;
            } else if !primary_down {
                let to = landing(&layout, moving.from, along);
                if to != moving.from {
                    out.push(with_fields(&film.reorder, &[("from", json!(moving.from)), ("to", json!(to))]));
                }
                self.moving = None;
            }
        }

        let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt("film"));
        strip.set_clip_rect(rect.intersect(ui.clip_rect()));
        let mut chosen: Option<Action> = None;

        // Each run's label above its first tile, cut before the next run,
        // and a hairline between runs.
        for x in &layout.dividers {
            painter.vline(start + x, (rect.top() + 8.0)..=(top + THUMB_H), (1.0, widgets::BORDER));
        }
        if label_h > 0.0 {
            for (index, span) in layout.spans.iter().enumerate() {
                let Some(group) = span.group.and_then(|group| film.groups.get(group)) else {
                    continue;
                };
                if group.label.is_empty() {
                    continue;
                }
                let right = layout.spans.get(index + 1).map_or(layout.add + ADD_W, |next| next.left - RUN_GAP / 2.0 - 4.0);
                let place = Rect::from_min_max(pos2(start + span.left, rect.top() + 4.0), pos2(start + right, rect.top() + label_h + 6.0));
                let mut job = egui::text::LayoutJob::simple_singleline(group.label.clone(), egui::FontId::proportional(LABEL_SIZE), widgets::MUTED);
                job.wrap = egui::text::TextWrapping::truncate_at_width(place.width().max(1.0));
                let galley = painter.layout_job(job);
                let at = pos2(place.left(), place.center().y - galley.size().y / 2.0);
                let label = Rect::from_min_size(at, galley.size());
                painter.galley(at, galley, widgets::MUTED);
                if acts && !group.menu.is_empty() {
                    let response = strip.interact(label, strip.id().with(("run", index)), Sense::click());
                    if let Some(row) = widgets::action_menu(&response, &group.menu) {
                        chosen = row.action.clone();
                    }
                }
            }
        }
        for (index, item) in film.items.iter().enumerate() {
            let place = tile(layout.lefts[index], layout.widths[index]);
            let mut response = strip.interact(place, strip.id().with(("thumb", index)), Sense::click_and_drag());
            if self.moving.is_none() {
                response = response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&item.name);
            }
            if acts && response.clicked() {
                // A Ctrl-, Cmd-, or Shift-click shows a second item beside
                // the one shown.
                let extend = strip.input(|input| input.modifiers.ctrl || input.modifiers.command || input.modifiers.shift);
                out.push(film.select_message(index, extend));
            }
            let span = layout.span_of(index);
            if acts
                && self.moving.is_none()
                && span.end - span.start > 1
                && response.drag_started_by(PointerButton::Primary)
            {
                let grab = response.interact_pointer_pos().map_or(place.width() / 2.0, |at| at.x - place.left());
                self.moving = Some(Moving { from: index, grab });
            }
            if self.moving.as_ref().is_some_and(|moving| moving.from == index) {
                continue;
            }
            paint_thumb(&strip, &painter, place, item, index == film.shown);
            if !item.caption.is_empty() {
                let mut job = egui::text::LayoutJob::simple_singleline(item.caption.clone(), egui::FontId::proportional(CAPTION_SIZE), widgets::MUTED);
                job.wrap = egui::text::TextWrapping::truncate_at_width((place.width() + GAP).max(1.0));
                let galley = painter.layout_job(job);
                painter.galley(pos2(place.center().x - galley.size().x / 2.0, place.bottom() + 4.0), galley, widgets::MUTED);
            }
            // A stack's badge opens it, over the tile's own click.
            if let Some(stack) = &item.stack {
                let spot = Rect::from_center_size(stack_center(place), vec2(2.0 * STACK_RADIUS + 4.0, 2.0 * STACK_RADIUS + 4.0));
                let badge = strip.interact(spot, strip.id().with(("stack", index)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                if acts && badge.clicked() {
                    out.push(stack.open.clone());
                }
            }
            if !item.menu.is_empty() {
                let open = (acts && response.secondary_clicked()).then_some(SetOpenCommand::Bool(true));
                let mut renaming = None;
                Popup::menu(&response).open_memory(open).align(RectAlign::TOP_START).gap(MENU_GAP).show(|ui| {
                    match widgets::entry_rows(ui, &item.menu) {
                        // A rename row asks for the new name in place, as a
                        // layer's row does, and sends it as `value`.
                        Some(row) if row.rename => {
                            renaming = row.action.clone().map(|action| (index, item.name.clone(), action, true));
                        }
                        Some(row) => chosen = row.action.clone(),
                        None => {}
                    }
                });
                if renaming.is_some() {
                    self.renaming = renaming;
                }
            }
        }

        // The + tile, with its rows above it.
        let add = tile(layout.add, ADD_W);
        let response = strip.interact(add, strip.id().with("add"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let edge = if response.hovered() { widgets::RAIL } else { widgets::BORDER };
        widgets::outlined(&painter, add, RADIUS + 2.0, Color32::WHITE, edge);
        crate::toolbar::paint_icon(&painter, add.center(), "plus", 18.0, SIGN);
        if !film.add.is_empty() {
            let open = (acts && response.clicked()).then_some(SetOpenCommand::Toggle);
            Popup::menu(&response).open_memory(open).align(RectAlign::TOP_START).gap(MENU_GAP).show(|ui| {
                if let Some(action) = widgets::entry_rows(ui, &film.add).and_then(|row| row.action.clone()) {
                    chosen = Some(action);
                }
            });
        }
        out.extend(chosen);
        self.rename_field(ui, &layout, film, start, rect, out);

        // The dragged thumbnail follows the pointer, over a line where it
        // will land.
        if let Some(moving) = &self.moving
            && let Some(pos) = pos
        {
            let to = landing(&layout, moving.from, pos.x - start);
            let span = layout.span_of(moving.from);
            let others: Vec<usize> = (span.start..span.end).filter(|index| *index != moving.from).collect();
            let at = to - span.start;
            let before = if at == 0 { span.left - GAP } else { layout.lefts[others[at - 1]] + layout.widths[others[at - 1]] };
            let after = others.get(at).map_or(before + GAP, |index| layout.lefts[*index]);
            let x = start + (before + after) / 2.0;
            painter.vline(x, (top - 4.0)..=(top + THUMB_H + 4.0), (2.0, SHOWN));
            let width = layout.widths[moving.from];
            let place = Rect::from_min_size(pos2(pos.x - moving.grab, top), vec2(width, THUMB_H));
            let floating = painter.with_layer_id(egui::LayerId::new(egui::Order::Tooltip, strip.id().with("moving")));
            widgets::soft_shadow(&floating, place, RADIUS);
            paint_thumb(&strip, &floating, place, &film.items[moving.from], moving.from == film.shown);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }
}

/// One thumbnail: its image at its proportions, with rounded corners, over
/// squares where it is clear, outlined in blue when it is the one shown or
/// the take chosen and with a hairline otherwise. A take not made yet is a
/// striped tile with a spinner, or a pale one with a red mark. Over it go
/// its badge at the bottom right, its mark at the top left, and the chosen
/// take's star at the top right.
fn paint_thumb(ui: &egui::Ui, painter: &egui::Painter, place: Rect, item: &FilmItem, shown: bool) {
    // A stack's iterations show as edges behind it, up and to the right.
    if item.stack.is_some() {
        for step in [4.0, 2.0] {
            widgets::outlined(painter, place.translate(vec2(step, -step)), RADIUS, Color32::WHITE, widgets::BORDER);
        }
    }
    match &item.pending {
        Some(FilmPending::Running { since }) => paint_running(ui, painter, place, *since),
        Some(FilmPending::Failed) => {
            painter.rect_filled(place, RADIUS, FAILED_FILL);
            widgets::disc(painter, place.center(), 8.0, FAILED);
            painter.text(place.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(12.0), Color32::WHITE);
        }
        None => {
            let thumbnail = &item.thumbnail;
            match crate::raster::raw_image(ui.ctx(), &thumbnail.path, thumbnail.width, thumbnail.height, thumbnail.revision, &[], false) {
                Some((texture, opaque)) => {
                    if !opaque {
                        crate::raster::checker(ui.ctx(), painter, place);
                    }
                    let options = egui::ImageOptions { corner_radius: RADIUS.into(), ..Default::default() };
                    egui::widgets::paint_texture_at(painter, place, &options, &egui::load::SizedTexture::new(texture, place.size()));
                }
                None => crate::raster::checker(ui.ctx(), painter, place),
            }
        }
    }
    if shown || item.chosen {
        widgets::outlined(painter, place.expand(3.0), RADIUS + 3.0, Color32::TRANSPARENT, SHOWN);
        widgets::outlined(painter, place.expand(2.0), RADIUS + 2.0, Color32::TRANSPARENT, SHOWN);
    } else if !item.outlined {
        widgets::outlined(painter, place, RADIUS, Color32::TRANSPARENT, widgets::BORDER);
    }
    // Out of date: the red outline, inside the chosen one's blue.
    if item.outlined {
        let at = if shown || item.chosen { place.expand(0.5) } else { place.expand(1.5) };
        widgets::outlined(painter, at, RADIUS + 1.0, Color32::TRANSPARENT, OUT_OF_DATE);
        widgets::outlined(painter, at.shrink(1.0), RADIUS, Color32::TRANSPARENT, OUT_OF_DATE);
    }
    if let Some(stack) = &item.stack {
        stack_badge(painter, place, stack);
    }
    if !item.badge.is_empty() {
        // Beside a stack's badge when it has one.
        let room = if item.stack.is_some() { 2.0 * STACK_RADIUS + 4.0 } else { 0.0 };
        badge(painter, Rect::from_min_max(place.min, place.max - vec2(room, 0.0)), &item.badge);
    }
    if let Some(mark) = &item.mark {
        // The canon seal, as vg-filmstrip-rounds shows: a white icon in a
        // ring, on a dark disc so it reads on any image.
        let center = pos2(place.left() + 11.0, place.top() + 11.0);
        widgets::disc(painter, center, 8.0, BADGE_FILL);
        widgets::ring(painter, center, 8.0, 1.2, Color32::WHITE);
        crate::toolbar::paint_icon(painter, center, mark, 11.0, Color32::WHITE);
    }
    if item.chosen {
        // The chosen take's star, as ig-candidate-chosen shows.
        let center = pos2(place.right() - 10.0, place.top() + 10.0);
        widgets::disc(painter, center, 7.5, SHOWN);
        crate::toolbar::paint_icon(painter, center, "star-solid", 9.0, Color32::WHITE);
    }
}

/// Where a stack's count badge sits on its tile.
fn stack_center(place: Rect) -> egui::Pos2 {
    pos2(place.right() - STACK_RADIUS - 3.0, place.bottom() - STACK_RADIUS - 3.0)
}

/// A stack's count, white on a dark disc in a white ring, as
/// ig-iteration-stack shows it.
fn stack_badge(painter: &egui::Painter, place: Rect, stack: &FilmStack) {
    let center = stack_center(place);
    widgets::disc(painter, center, STACK_RADIUS, BADGE_FILL);
    widgets::ring(painter, center, STACK_RADIUS, 1.2, Color32::WHITE);
    painter.text(center, egui::Align2::CENTER_CENTER, stack.count.to_string(), egui::FontId::proportional(BADGE_SIZE - 1.0), Color32::WHITE);
}

/// Words on a dark rounded badge at the tile's bottom right, such as a
/// take's length or the minutes a take has been running.
fn badge(painter: &egui::Painter, place: Rect, words: &str) {
    let galley = painter.layout_no_wrap(words.to_string(), egui::FontId::proportional(BADGE_SIZE), Color32::WHITE);
    let size = galley.size() + vec2(8.0, 3.0);
    let spot = Rect::from_min_size(place.right_bottom() - size - vec2(4.0, 4.0), size);
    painter.rect_filled(spot, 4.0, BADGE_FILL);
    painter.galley(spot.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// A take being made: a light-gray tile with diagonal stripes, a spinner
/// in its middle, and, from `since`, the minutes it has been running.
fn paint_running(ui: &egui::Ui, painter: &egui::Painter, place: Rect, since: Option<u64>) {
    painter.rect_filled(place, RADIUS, PENDING_FILL);
    let stripes = painter.with_clip_rect(place.shrink(1.0).intersect(painter.clip_rect()));
    let mut x = place.left() - place.height();
    while x < place.right() {
        stripes.line_segment([pos2(x, place.bottom()), pos2(x + place.height(), place.top())], (1.0, STRIPE));
        x += STRIPE_STEP;
    }
    let time = ui.input(|input| input.time);
    spinner(painter, place.center(), 8.0, time);
    if let Some(since) = since {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |now| now.as_secs());
        badge(painter, place, &clock(now.saturating_sub(since)));
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

/// A spinner of twelve short spokes, the one leading darkest, turning once
/// a second.
pub(crate) fn spinner(painter: &egui::Painter, center: egui::Pos2, radius: f32, time: f64) {
    const SPOKES: usize = 12;
    let lead = ((time.fract() * SPOKES as f64) as usize) % SPOKES;
    for spoke in 0..SPOKES {
        let angle = spoke as f32 / SPOKES as f32 * std::f32::consts::TAU;
        let along = vec2(angle.sin(), -angle.cos());
        let behind = (lead + SPOKES - spoke) % SPOKES;
        let alpha = 1.0 - behind as f32 / SPOKES as f32 * 0.85;
        let color = SPINNER.gamma_multiply(alpha);
        painter.line_segment([center + along * radius * 0.45, center + along * radius], (1.6, color));
    }
}

/// Seconds as minutes and seconds, such as "2:08".
fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a round that appears, or finishes, past the strip's edge,
    /// left where the person must scroll to find it; a strip that jumps
    /// when nothing new appears; and one that jumps when a tab is opened.
    #[test]
    fn a_new_or_finished_round_is_brought_into_view() {
        let run = |label: &str, making: bool| (label.to_string(), making);
        let before = [run("Round 1", false), run("Round 2", false)];
        assert_eq!(run_to_reveal(&before, &before), None, "nothing new moved the strip");
        assert_eq!(run_to_reveal(&before, &[run("Round 1", false), run("Round 2", false), run("Round 3", true)]), Some(2));
        let making = [run("Round 1", false), run("Round 2", true)];
        assert_eq!(run_to_reveal(&making, &[run("Round 1", false), run("Round 2", false)]), Some(1), "a finished round was not shown");
        // Past the right edge: scrolled so its end shows; already in view:
        // not moved; past the left edge: scrolled back to its start.
        assert_eq!(scroll_to_show(0.0, 900.0, 1100.0, 800.0), 1100.0 - 800.0 + INSET);
        assert_eq!(scroll_to_show(100.0, 200.0, 400.0, 800.0), 100.0);
        assert_eq!(scroll_to_show(500.0, 200.0, 400.0, 800.0), 200.0 - INSET);
        // A tab's first frame records its runs and moves nothing: only what
        // appears or finishes later brings the strip to it.
        let mut strip = Strip::default();
        let rounds = |labels: &[&str]| Film {
            groups: labels.iter().map(|label| protocol::FilmGroup { label: label.to_string(), count: 1, menu: Vec::new() }).collect(),
            ..film(&vec![(160, 90); labels.len()])
        };
        let (ctx, rect) = (egui::Context::default(), Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0, 80.0)));
        crate::toolbar::install_fonts(&ctx);
        let draw = |strip: &mut Strip, tab: &str, film: &Film| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| strip.show(ui, tab, film, rect, false, false, &mut Vec::new()));
            output.textures_delta.clear();
            strip.scroll
        };
        let many = rounds(&["Round 1", "Round 2", "Round 3", "Round 4", "Round 5"]);
        assert_eq!(draw(&mut strip, "1", &many), 0.0, "opening a tab scrolled its strip");
        assert_eq!(draw(&mut strip, "2", &rounds(&["Round 1"])), 0.0);
        let more = rounds(&["Round 1", "Round 2", "Round 3", "Round 4", "Round 5", "Round 6"]);
        assert!(draw(&mut strip, "1", &more) > 0.0, "a new round past the edge was not brought into view");
        // A place that failed is finished, so a round that ends that way is
        // brought into view as one that ends with its takes.
        let mut failed = rounds(&["Round 1"]);
        failed.items[0].pending = Some(FilmPending::Failed);
        assert_eq!(runs_seen(&failed, &layout(&failed)), [(String::from("Round 1"), false)]);
        // Two cells' rounds labelled alike: the second cell's new round is
        // its own, not the first cell's seen again.
        let two = |second: &[&str]| {
            let mut labels = vec!["Round 1", "Round 2"];
            labels.extend_from_slice(second);
            rounds(&labels)
        };
        let mut strip = Strip::default();
        assert_eq!(draw(&mut strip, "1", &two(&["Round 1"])), 0.0);
        assert!(draw(&mut strip, "1", &two(&["Round 1", "Round 2"])) > 0.0, "a second cell's new round was taken for the first cell's");
    }
    /// Catches a stack whose badge does nothing or selects its tile, and
    /// words under the tiles drawn into a strip with no room for them.
    #[test]
    fn a_stacks_badge_opens_it_and_words_go_under_the_tiles() {
        let mut film = film(&[(160, 90), (160, 90)]);
        film.items[0].stack = Some(FilmStack { count: 3, open: Action::new("take_stack") });
        film.items[0].outlined = true;
        film.items[1].caption = "Unverified".into();
        assert_eq!(height(&film), HEIGHT + CAPTION_H);
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, height(&film)));
        let mut strip = Strip::default();
        let mut draw = |strip: &mut Strip, events: Vec<Event>| {
            let mut out = Vec::new();
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| strip.show(ui, "1", &film, rect, false, false, &mut out));
            output.textures_delta.clear();
            out
        };
        draw(&mut strip, Vec::new());
        let laid = layout(&film);
        let top = (rect.top() + rect.bottom() - CAPTION_H) / 2.0 + 0.5 - THUMB_H / 2.0;
        let at = stack_center(Rect::from_min_size(pos2(laid.lefts[0], top), vec2(laid.widths[0], THUMB_H)));
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut out = draw(&mut strip, vec![Event::PointerMoved(at), press(true)]);
        out.extend(draw(&mut strip, vec![press(false)]));
        let ids: Vec<&str> = out.iter().map(|action| action.id.as_str()).collect();
        assert!(ids.contains(&"take_stack") && !ids.contains(&"select"), "{ids:?}");
    }

    use protocol::Thumbnail;

    fn film(sizes: &[(u32, u32)]) -> Film {
        Film {
            items: sizes
                .iter()
                .map(|(width, height)| FilmItem {
                    name: String::new(),
                    thumbnail: Thumbnail { path: String::new(), width: *width, height: *height, revision: 1 },
                    menu: Vec::new(),
                    badge: String::new(),
                    mark: None,
                    chosen: false,
                    pending: None,
                    outlined: false,
                    stack: None,
                    caption: String::new(),
                })
                .collect(),
            shown: 0,
            select: Action::new("select"),
            reorder: Action::new("reorder"),
            add: Vec::new(),
            focus: Action::new("focus"),
            groups: Vec::new(),
        }
    }

    /// Catches a drag that lands one place off: `to` is counted after the
    /// dragged thumbnail is taken out, so a drag past every other one lands
    /// last, and one held over its own place lands where it was.
    #[test]
    fn a_dragged_thumbnail_lands_between_the_others() {
        let layout = layout(&film(&[(16, 9), (9, 16), (1, 1)]));
        assert_eq!(layout.widths, vec![THUMB_H * 16.0 / 9.0, THUMB_MIN_W.max(THUMB_H * 9.0 / 16.0), THUMB_H]);
        let middle = |index: usize| layout.lefts[index] + layout.widths[index] / 2.0;
        assert_eq!(landing(&layout, 0, middle(0)), 0);
        assert_eq!(landing(&layout, 0, middle(2) + 1.0), 2);
        assert_eq!(landing(&layout, 2, 0.0), 0);
        assert_eq!(landing(&layout, 2, middle(0) + 1.0), 1);
        assert_eq!(landing(&layout, 1, middle(1)), 1);
        let reorder = with_fields(&Action::with_arg("reorder", json!({ "tab": "1" })), &[("from", json!(0)), ("to", json!(2))]);
        assert_eq!(reorder.arg, Some(json!({ "tab": "1", "from": 0, "to": 2 })));
    }

    /// Catches runs laid out as one strip: each group starts after a wider
    /// gap with a hairline in it, the strip grows a line for the labels,
    /// and a drag lands only inside its own run, its index still counted
    /// along the whole strip.
    #[test]
    fn runs_keep_their_takes_and_their_drags() {
        let mut strip = film(&[(16, 9); 5]);
        strip.groups = vec![
            protocol::FilmGroup { label: "Round 12".into(), count: 2, menu: Vec::new() },
            protocol::FilmGroup { label: "Round 13".into(), count: 3, menu: Vec::new() },
        ];
        assert_eq!(height(&strip), HEIGHT + LABEL_H);
        let layout = layout(&strip);
        assert_eq!(layout.spans.len(), 2);
        assert_eq!((layout.spans[1].start, layout.spans[1].end), (2, 5));
        let first_right = layout.lefts[1] + layout.widths[1];
        assert_eq!(layout.lefts[2], first_right + RUN_GAP);
        assert_eq!(layout.dividers, vec![first_right + RUN_GAP / 2.0]);
        // Dragged far left, the second run's last take lands first in its
        // run, not first on the strip.
        assert_eq!(landing(&layout, 4, 0.0), 2);
        // Dragged far right, the first run's first take lands last in its run.
        assert_eq!(landing(&layout, 0, layout.add), 1);
        // A strip with no groups has no labels and one run.
        let plain = film(&[(16, 9); 2]);
        assert_eq!(height(&plain), HEIGHT);
        assert_eq!(layout_spans(&plain), vec![(0, 2, None)]);
        // Items past the groups make a run of their own.
        let mut over = film(&[(16, 9); 3]);
        over.groups = vec![protocol::FilmGroup { label: String::new(), count: 1, menu: Vec::new() }];
        assert_eq!(height(&over), HEIGHT);
        assert_eq!(layout_spans(&over), vec![(0, 1, Some(0)), (1, 3, None)]);
    }

    fn layout_spans(film: &Film) -> Vec<(usize, usize, Option<usize>)> {
        layout(film).spans.iter().map(|span| (span.start, span.end, span.group)).collect()
    }

    /// Catches a running take's minutes read as seconds or unpadded.
    #[test]
    fn running_minutes_count_up_as_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(128), "2:08");
        assert_eq!(clock(130), "2:10");
        assert_eq!(clock(3600), "60:00");
    }
}
