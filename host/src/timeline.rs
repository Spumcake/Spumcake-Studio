//! A tab's timeline, docked under its contents and drawn from its
//! declaration, whatever the contents' kind: the header, page tabs, the way
//! back, the ruler, rows with their items or frame cells, the playhead, and
//! ranges. The host previews each gesture and sends one `timeline_gesture`
//! when it ends; dragging the playhead sends `seek` while it is held. It
//! knows no workspace: rows, items, spans, cells, and frames are all it
//! draws, as the approved timeline views show them.

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Mesh, Painter, PointerButton, Pos2, Rect, Sense,
    UiBuilder, Vec2, pos2, vec2,
};
use protocol::{
    Action, CellMark, CellSpan, DropRule, Edge, ItemStyle, MarkKind, RangeKind, RowContent, RulerUnit, SpanKind,
    Timeline, TimelineBack, TimelineGesture, TimelineItem, TimelinePick, TimelineRow, Toolbar,
};

use crate::toolbar::paint_icon;
use crate::widgets::{self, Scale};

/// The timeline's height under the contents, before the contents are too
/// short to share it: room for the way back, the header, the ruler, and
/// four rows, as the views give it.
pub(crate) const HEIGHT: f32 = 320.0;

/// What the timeline sends.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    Run(Action),
    Gesture(TimelineGesture),
    Seek { frame: u32, live: bool },
}

/// The way back's bar, the header's row, the ruler, and the loop's strip.
const BACK_H: f32 = 40.0;
const HEAD_H: f32 = crate::toolbar::HEIGHT;
const RULER_H: f32 = 36.0;
const LOOP_H: f32 = 18.0;
/// The scroll bar under the rows.
const BAR_H: f32 = 8.0;
/// The rows' names, and their narrower width in a narrow window.
const NAMES_W: f32 = 220.0;
const NAMES_NARROW_W: f32 = 160.0;
/// Room before the first frame.
const PAD: f32 = 12.0;
/// A row of items, a row of cells, and an indented row of cells with no
/// toggles of its own, such as a property under a layer.
const ITEMS_ROW_H: f32 = 40.0;
const CELLS_ROW_H: f32 = 30.0;
const SUB_ROW_H: f32 = 24.0;
const INDENT: f32 = 16.0;
/// How near an edge the pointer takes the edge rather than the item.
const EDGE: f32 = 6.0;
/// How far the pointer moves before a press is a drag.
const DRAG_AFTER: f32 = 4.0;
/// How near an edge or the playhead a dragged edge snaps to it.
const SNAP: f32 = 8.0;
/// Points per frame, at the farthest and nearest.
const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 60.0;
const HANDLE_REACH: f32 = 7.0;
/// The lighter look while the assistant works.
const LOCKED_ALPHA: f32 = 0.45;

const PLAYHEAD: Color32 = Color32::from_rgb(124, 58, 237);
/// A selected item's border, the snap line, and a chosen page's underline.
const BLUE: Color32 = Color32::from_rgb(37, 99, 235);
/// Selected cells and marks.
const CELL_BLUE: Color32 = Color32::from_rgb(59, 130, 246);
const ROW_SELECTED: Color32 = Color32::from_rgb(235, 242, 254);
const GRID: Color32 = Color32::from_rgb(236, 238, 241);
const FIFTH: Color32 = Color32::from_rgb(243, 244, 246);
const TICK: Color32 = Color32::from_rgb(176, 180, 186);
const TOGGLE_OFF: Color32 = Color32::from_rgb(168, 172, 178);
const REPLACE: Color32 = Color32::from_rgb(220, 38, 38);
const TWEEN_FILL: Color32 = Color32::from_rgb(240, 232, 253);
const TWEEN_EDGE: Color32 = Color32::from_rgb(221, 204, 250);
const HOLD_BAR: Color32 = Color32::from_rgb(176, 179, 184);
const LOOP: Color32 = Color32::from_rgb(249, 115, 22);
const LOOP_FILL: Color32 = Color32::from_rgb(254, 215, 170);
const BAND: Color32 = Color32::from_rgba_premultiplied(37, 45, 60, 40);
const EARLIER: Color32 = Color32::from_rgb(220, 38, 38);
const LATER: Color32 = Color32::from_rgb(22, 163, 74);
const FLAG: Color32 = Color32::from_rgb(220, 38, 38);
const COMMENT: Color32 = Color32::from_rgb(22, 163, 74);
const GAP_FILL: Color32 = Color32::from_rgb(247, 248, 250);
const HATCH: Color32 = Color32::from_rgb(222, 225, 229);

/// A timeline's own state between frames, kept per tab: its zoom, its
/// scroll, the press under way, and the last block of cells chosen here.
#[derive(Clone, Default)]
struct View {
    /// Points per frame; none fits the whole length in the width.
    zoom: Option<f32>,
    scroll: f32,
    scroll_y: f32,
    press: Option<Press>,
    /// Cells chosen by a drag, shown chosen until the next press: a row's
    /// declaration marks its chosen marks, not its chosen empty cells.
    block: Option<(Vec<String>, u32, u32)>,
}

#[derive(Clone)]
struct Press {
    at: Pos2,
    target: Target,
    dragging: bool,
    last_seek: Option<u32>,
}

/// What a press landed on.
#[derive(Clone, Debug)]
enum Target {
    Seek,
    Handle { index: usize, edge: Edge },
    Item { row: usize, item: usize },
    ItemEdge { row: usize, item: usize, edge: Edge },
    Cell { row: usize, frame: u32, moves: bool },
    SpanEdge { row: usize, span: usize },
    Empty { row: usize, frame: u32 },
    Bar { grab: f32 },
}

/// A gesture under way, as it would end if let go now.
#[derive(Clone, Debug, Default)]
enum Preview {
    #[default]
    None,
    Seek(u32),
    Range { index: usize, start: u32, end: u32 },
    Items(ItemMove),
    Cells(CellMove),
    Resize(Resize),
    Block { rows: Vec<usize>, start: u32, end: u32 },
}

#[derive(Clone, Debug)]
struct ItemMove {
    /// Each moved item, as row and item.
    moved: Vec<(usize, usize)>,
    delta: i64,
    /// The row the items land on, when it is not their own.
    row: Option<usize>,
    landing: Landing,
}

#[derive(Clone, Debug)]
struct CellMove {
    /// Each moved run of cells: its row, first frame, and length.
    runs: Vec<(usize, u32, u32)>,
    picks: Vec<TimelinePick>,
    delta: i64,
    landing: Landing,
}

/// Where moved frames land: the frames they replace, or how far later
/// frames move, the frame snapped to, and the pointer's row.
#[derive(Clone, Debug, Default)]
struct Landing {
    /// Row, first frame, and the frame after, of each run replaced.
    replaced: Vec<(usize, u32, u32)>,
    count: u32,
    push: bool,
    shift: bool,
    snap: Option<f32>,
    /// The row the pointer is over, for the tooltip.
    over: usize,
}

#[derive(Clone, Debug)]
struct Resize {
    row: usize,
    item: Option<usize>,
    span: Option<usize>,
    edge: Edge,
    start: u32,
    length: u32,
    delta: i64,
    snap: Option<f32>,
}

/// Where everything lies this frame.
struct Geo {
    whole: Rect,
    /// The ruler over the rows' cells, and the strip of names beside it.
    ruler: Rect,
    ruler_names: Rect,
    loop_strip: Option<Rect>,
    /// The rows' cells, and their names.
    rows: Rect,
    names: Rect,
    bar: Rect,
    zoom: f32,
    x0: f32,
    /// Each row's top and bottom, scrolled.
    tops: Vec<(f32, f32)>,
    max_scroll: f32,
    max_scroll_y: f32,
    content_w: f32,
    unit: RulerUnit,
    fps: f64,
}

impl Geo {
    fn x(&self, frame: f32) -> f32 {
        self.x0 + frame * self.zoom
    }

    fn frame_at(&self, x: f32) -> f32 {
        (x - self.x0) / self.zoom
    }

    fn cell_at(&self, x: f32) -> u32 {
        self.frame_at(x).floor().max(0.0) as u32
    }

    /// The middle of a frame's cell.
    fn mid(&self, frame: u32) -> f32 {
        self.x(frame as f32 + 0.5)
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        self.tops.iter().position(|(top, bottom)| y >= *top && y < *bottom)
    }

    /// The row at `y`, or the nearest one above or below the rows.
    fn row_near(&self, y: f32) -> Option<usize> {
        if self.tops.is_empty() {
            return None;
        }
        self.row_at(y).or_else(|| Some(if y < self.tops[0].0 { 0 } else { self.tops.len() - 1 }))
    }

    /// Where the playhead stands for a frame: a cell's middle when the
    /// ruler counts frames, its start when it reads time.
    fn playhead_x(&self, frame: u32) -> f32 {
        match self.unit {
            RulerUnit::Frames => self.mid(frame),
            RulerUnit::Timecode => self.x(frame as f32),
        }
    }

    fn row_rect(&self, row: usize) -> Rect {
        let (top, bottom) = self.tops[row];
        Rect::from_min_max(pos2(self.rows.left(), top), pos2(self.rows.right(), bottom))
    }

    /// The scroll bar's thumb, when the content is wider than the rows.
    fn thumb(&self, scroll: f32) -> Option<Rect> {
        if self.max_scroll <= 0.0 {
            return None;
        }
        let track = self.bar;
        let width = (track.width() * self.rows.width() / self.content_w).max(24.0).min(track.width());
        let left = track.left() + (track.width() - width) * (scroll / self.max_scroll).clamp(0.0, 1.0);
        Some(Rect::from_min_size(pos2(left, track.top() + 1.0), vec2(width, track.height() - 2.0)))
    }
}

fn row_height(row: &TimelineRow) -> f32 {
    match row.content {
        RowContent::Items { .. } => ITEMS_ROW_H,
        RowContent::Cells { .. } if row.indent > 0 && row.toggles.is_empty() => SUB_ROW_H,
        RowContent::Cells { .. } => CELLS_ROW_H,
    }
}

/// The frame after the last thing declared.
fn extent(timeline: &Timeline) -> u32 {
    let mut end = timeline.ruler.length.max(1);
    for row in &timeline.rows {
        match &row.content {
            RowContent::Items { items } => {
                for item in items {
                    end = end.max(item.start + item.length);
                }
            }
            RowContent::Cells { marks, spans } => {
                for mark in marks {
                    end = end.max(mark.frame + 1);
                }
                for span in spans {
                    end = end.max(span.start + span.length);
                }
            }
        }
    }
    for range in &timeline.ranges {
        end = end.max(range.end + 1);
    }
    end
}

/// Lays the timeline out under `top`, holding `view`'s zoom and scroll to
/// what the content allows.
fn geo(timeline: &Timeline, rect: Rect, top: f32, view: &mut View) -> Geo {
    let names_w = if rect.width() < 640.0 { NAMES_NARROW_W } else { NAMES_W };
    let split = rect.left() + names_w;
    let ruler = Rect::from_min_max(pos2(split, top), pos2(rect.right(), top + RULER_H));
    let ruler_names = Rect::from_min_max(pos2(rect.left(), top), pos2(split, top + RULER_H));
    let mut top = ruler.bottom();
    let loop_strip = timeline.ranges.iter().any(|range| range.kind == RangeKind::Loop).then(|| {
        top += LOOP_H;
        Rect::from_min_max(pos2(split, top - LOOP_H), pos2(rect.right(), top))
    });
    let bottom = (rect.bottom() - BAR_H).max(top);
    let rows = Rect::from_min_max(pos2(split, top), pos2(rect.right(), bottom));
    let names = Rect::from_min_max(pos2(rect.left(), top), pos2(split, bottom));
    let bar = Rect::from_min_max(pos2(split, bottom), pos2(rect.right(), rect.bottom()));

    let extent = extent(timeline) as f32;
    let room = (rows.width() - 2.0 * PAD).max(1.0);
    let zoom = view.zoom.unwrap_or(room / extent).clamp(MIN_ZOOM, MAX_ZOOM);
    let content_w = 2.0 * PAD + extent * zoom;
    let max_scroll = (content_w - rows.width()).max(0.0);
    view.scroll = view.scroll.clamp(0.0, max_scroll);
    let content_h: f32 = timeline.rows.iter().map(row_height).sum();
    let max_scroll_y = (content_h - rows.height()).max(0.0);
    view.scroll_y = view.scroll_y.clamp(0.0, max_scroll_y);

    let mut y = rows.top() - view.scroll_y;
    let tops = timeline
        .rows
        .iter()
        .map(|row| {
            let height = row_height(row);
            y += height;
            (y - height, y)
        })
        .collect();
    Geo {
        whole: rect,
        ruler,
        ruler_names,
        loop_strip,
        rows,
        names,
        bar,
        zoom,
        x0: rows.left() + PAD - view.scroll,
        tops,
        max_scroll,
        max_scroll_y,
        content_w,
        unit: timeline.ruler.unit,
        fps: timeline.ruler.fps,
    }
}

/// Draws `timeline` in `rect`, in its lighter look and sending nothing
/// while `locked`; it still scrolls and zooms. Returns what was chosen
/// this frame.
pub(crate) fn show(ui: &mut egui::Ui, timeline: &Timeline, rect: Rect, locked: bool) -> Vec<Out> {
    let mut out = Vec::new();
    let base = Id::new(("timeline", timeline.tab.as_str()));
    let mut view: View = ui.data(|data| data.get_temp(base)).unwrap_or_default();
    let mut child = ui.new_child(UiBuilder::new().id_salt(base).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    if locked {
        child.style_mut().visuals.disabled_alpha = LOCKED_ALPHA;
        child.disable();
    }
    let ui = &mut child;
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect, 0.0, Color32::WHITE);

    let mut top = rect.top();
    if let Some(back) = &timeline.back {
        let bar = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), BACK_H));
        back_bar(ui, &painter, back, bar, base, &mut out);
        top = bar.bottom();
    }
    if !timeline.pages.is_empty() || !timeline.header.is_empty() {
        let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), HEAD_H));
        header(ui, &painter, timeline, row, base, &mut out);
        top = row.bottom();
    }
    painter.hline(rect.x_range(), rect.top() + 0.5, (1.0, widgets::BORDER));

    let mut g = geo(timeline, rect, top, &mut view);
    if wheel(ui, &g, &mut view) {
        g = geo(timeline, rect, top, &mut view);
    }
    let preview = pointer(ui, timeline, &g, &mut view, locked, &mut out);
    if matches!(view.press.as_ref().map(|press| &press.target), Some(Target::Bar { .. })) {
        g = geo(timeline, rect, top, &mut view);
    }
    paint(ui, &painter, timeline, &g, &view, &preview, base, &mut out);
    ui.data_mut(|data| data.insert_temp(base, view));
    if locked {
        out.clear();
    }
    out
}

/// The way back: an arrow, then the path, its earlier parts as links and
/// the last in semibold. The arrow and every earlier part send its action.
fn back_bar(ui: &mut egui::Ui, painter: &Painter, back: &TimelineBack, bar: Rect, base: Id, out: &mut Vec<Out>) {
    let font = widgets::body(Scale(1.0));
    let arrow = Rect::from_center_size(pos2(bar.left() + 22.0, bar.center().y), vec2(28.0, 28.0));
    let response = ui.interact(arrow, base.with("back"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        painter.rect_filled(arrow, 6.0, widgets::SELECTED);
    }
    paint_icon(painter, arrow.center(), "arrow-left", 18.0, widgets::TEXT);
    if response.clicked() {
        out.push(Out::Run(back.action.clone()));
    }
    let mut x = arrow.right() + 12.0;
    for (index, part) in back.path.iter().enumerate() {
        let last = index + 1 == back.path.len();
        let face = if last { widgets::semibold(15.0) } else { font.clone() };
        let galley = painter.layout_no_wrap(part.clone(), face, widgets::TEXT);
        let place = Rect::from_min_size(pos2(x, bar.center().y - galley.size().y / 2.0), galley.size());
        if last {
            painter.galley(place.min, galley, widgets::TEXT);
            break;
        }
        let response = ui
            .interact(place, base.with(("back-part", index)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let color = if response.hovered() { widgets::TEXT } else { widgets::MUTED };
        painter.galley(place.min, galley, color);
        if response.hovered() {
            painter.hline(place.x_range(), place.bottom() + 1.0, (1.0, color));
        }
        if response.clicked() {
            out.push(Out::Run(back.action.clone()));
        }
        x = place.right() + 10.0;
        widgets::chevron_side(painter, pos2(x + 2.0, bar.center().y), true, widgets::MUTED);
        x += 14.0;
    }
    painter.hline(bar.x_range(), bar.bottom() - 0.5, (1.0, widgets::BORDER));
}

/// Page tabs, underlined when chosen, then the header's items drawn as the
/// toolbar draws them.
fn header(ui: &mut egui::Ui, painter: &Painter, timeline: &Timeline, row: Rect, base: Id, out: &mut Vec<Out>) {
    let mut x = row.left() + 16.0;
    for (index, page) in timeline.pages.iter().enumerate() {
        let face = if page.selected { widgets::semibold(14.0) } else { widgets::body(Scale(1.0)) };
        let color = if page.selected { widgets::TEXT } else { widgets::MUTED };
        let galley = painter.layout_no_wrap(page.label.clone(), face, color);
        let place = Rect::from_min_max(pos2(x - 8.0, row.top()), pos2(x + galley.size().x + 8.0, row.bottom()));
        let response = ui
            .interact(place, base.with(("page", index)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let shown = if response.hovered() && !page.selected { widgets::TEXT } else { color };
        painter.galley(pos2(x, row.center().y - galley.size().y / 2.0), galley, shown);
        if page.selected {
            painter.rect_filled(
                Rect::from_min_max(pos2(place.left(), row.bottom() - 4.0), pos2(place.right(), row.bottom() - 2.0)),
                1.0,
                BLUE,
            );
        }
        if response.clicked() {
            out.push(Out::Run(page.action.clone()));
        }
        x = place.right() + 16.0;
    }
    if !timeline.pages.is_empty() {
        widgets::vertical_rule(painter, x - 4.0, row.top() + 12.0, row.bottom() - 12.0, widgets::BORDER);
    }
    let items = Rect::from_min_max(pos2(if timeline.pages.is_empty() { row.left() } else { x }, row.top()), row.max);
    if !timeline.header.is_empty() && items.width() > 0.0 {
        let mut bar = ui.new_child(
            UiBuilder::new()
                .id_salt(base.with("header"))
                .max_rect(items)
                .layout(Layout::top_down(Align::Min)),
        );
        bar.set_clip_rect(items.intersect(ui.clip_rect()));
        let declared = Toolbar {
            items: timeline.header.clone(),
            trailing: Vec::new(),
            options: Vec::new(),
            options_trailing: Vec::new(),
        };
        out.extend(crate::toolbar::show(&mut bar, Some(&declared), 0.0).into_iter().map(Out::Run));
    }
    painter.hline(row.x_range(), row.bottom() - 0.5, (1.0, widgets::BORDER));
}

/// The wheel scrolls; with Ctrl it zooms about the pointer. Returns whether
/// the view changed.
fn wheel(ui: &mut egui::Ui, g: &Geo, view: &mut View) -> bool {
    let area = Rect::from_min_max(pos2(g.whole.left(), g.ruler.top()), g.whole.max);
    if !ui.rect_contains_pointer(area) {
        return false;
    }
    let (delta, zoom, pos) = ui.input(|input| (input.smooth_scroll_delta, input.zoom_delta(), input.pointer.latest_pos()));
    let mut changed = false;
    if zoom != 1.0 {
        let anchor = pos.map_or(g.rows.center().x, |pos| pos.x.clamp(g.rows.left(), g.rows.right()));
        let frame = g.frame_at(anchor);
        let next = (g.zoom * zoom).clamp(MIN_ZOOM, MAX_ZOOM);
        view.zoom = Some(next);
        view.scroll = frame * next + PAD - (anchor - g.rows.left());
        changed = true;
    }
    if delta != Vec2::ZERO {
        let (mut across, mut down) = (delta.x, delta.y);
        if g.max_scroll_y <= 0.0 {
            across += down;
            down = 0.0;
        }
        view.scroll -= across;
        view.scroll_y -= down;
        ui.input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
        changed = true;
    }
    changed
}

fn items(row: &TimelineRow) -> &[TimelineItem] {
    match &row.content {
        RowContent::Items { items } => items,
        RowContent::Cells { .. } => &[],
    }
}

fn cells(row: &TimelineRow) -> (&[CellMark], &[CellSpan]) {
    match &row.content {
        RowContent::Cells { marks, spans } => (marks, spans),
        RowContent::Items { .. } => (&[], &[]),
    }
}

fn item_rect(g: &Geo, row: usize, start: u32, length: u32) -> Rect {
    let (top, bottom) = g.tops[row];
    Rect::from_min_max(
        pos2(g.x(start as f32), top + 4.0),
        pos2(g.x((start + length) as f32), bottom - 4.0),
    )
}

/// Whether the cell at `frame` on `row` is chosen: its mark, a span its
/// chosen mark starts, or the block chosen here.
fn cell_chosen(timeline: &Timeline, view: &View, row: usize, frame: u32) -> bool {
    let (marks, spans) = cells(&timeline.rows[row]);
    let marked = |at: u32| marks.iter().any(|mark| mark.frame == at && mark.selected);
    marked(frame)
        || spans.iter().any(|span| frame >= span.start && frame < span.start + span.length && marked(span.start))
        || in_block(view, &timeline.rows[row].id, frame)
}

fn in_block(view: &View, row: &str, frame: u32) -> bool {
    view.block.as_ref().is_some_and(|(rows, start, end)| rows.iter().any(|id| id == row) && frame >= *start && frame <= *end)
}

/// What is under `pos`. While locked only the scroll bar answers.
fn hit(timeline: &Timeline, g: &Geo, view: &View, pos: Pos2, locked: bool) -> Option<Target> {
    if g.bar.contains(pos) {
        let thumb = g.thumb(view.scroll)?;
        let grab = if thumb.x_range().contains(pos.x) { pos.x - thumb.left() } else { thumb.width() / 2.0 };
        return Some(Target::Bar { grab });
    }
    if locked {
        return None;
    }
    for (index, range) in timeline.ranges.iter().enumerate() {
        for (edge, at) in handles(g, range.kind, range.start, range.end) {
            if at.distance(pos) <= HANDLE_REACH {
                return Some(Target::Handle { index, edge });
            }
        }
    }
    if g.ruler.contains(pos) {
        return Some(Target::Seek);
    }
    if !g.rows.contains(pos) {
        return None;
    }
    let row = g.row_at(pos.y)?;
    let declared = &timeline.rows[row];
    if declared.read_only {
        return None;
    }
    let frame = g.cell_at(pos.x);
    match &declared.content {
        RowContent::Items { items } => {
            for (index, item) in items.iter().enumerate().rev() {
                let place = item_rect(g, row, item.start, item.length);
                if !place.contains(pos) {
                    continue;
                }
                if item.movable && place.width() > 3.0 * EDGE {
                    if pos.x - place.left() <= EDGE {
                        return Some(Target::ItemEdge { row, item: index, edge: Edge::Start });
                    }
                    if place.right() - pos.x <= EDGE {
                        return Some(Target::ItemEdge { row, item: index, edge: Edge::End });
                    }
                }
                return Some(Target::Item { row, item: index });
            }
            Some(Target::Empty { row, frame })
        }
        RowContent::Cells { spans, .. } => {
            let reach = (g.zoom / 2.0).clamp(3.0, EDGE);
            for (index, span) in spans.iter().enumerate() {
                if span.length > 0 && (pos.x - g.x((span.start + span.length) as f32)).abs() <= reach {
                    return Some(Target::SpanEdge { row, span: index });
                }
            }
            Some(Target::Cell { row, frame, moves: cell_chosen(timeline, view, row, frame) })
        }
    }
}

/// A range's handles, with the edge each moves.
fn handles(g: &Geo, kind: RangeKind, start: u32, end: u32) -> [(Edge, Pos2); 2] {
    let (left, right) = (g.x(start as f32), g.x((end + 1) as f32));
    let y = match (kind, g.loop_strip) {
        (RangeKind::Loop, Some(strip)) => strip.center().y,
        _ => g.ruler.bottom() - 10.0,
    };
    [(Edge::Start, pos2(left, y)), (Edge::End, pos2(right, y))]
}

/// Reads the pointer: starts, previews, and ends a press. Returns the
/// gesture under way.
fn pointer(ui: &mut egui::Ui, timeline: &Timeline, g: &Geo, view: &mut View, locked: bool, out: &mut Vec<Out>) -> Preview {
    let (pressed, down, origin, pos, modifiers, double, escape) = ui.input(|input| {
        (
            input.pointer.primary_pressed(),
            input.pointer.primary_down(),
            input.pointer.press_origin(),
            input.pointer.latest_pos(),
            input.modifiers,
            input.pointer.button_double_clicked(PointerButton::Primary),
            input.key_pressed(egui::Key::Escape),
        )
    });
    let area = Rect::from_min_max(pos2(g.rows.left(), g.ruler.top()), g.whole.max);
    if pressed
        && let Some(at) = origin
        && area.contains(at)
        && ui.rect_contains_pointer(area)
        && !egui::Popup::is_any_open(ui.ctx())
        && let Some(target) = hit(timeline, g, view, at, locked)
    {
        if !matches!(target, Target::Cell { moves: true, .. } | Target::Bar { .. }) {
            view.block = None;
        }
        view.press = Some(Press { at, target, dragging: false, last_seek: None });
    }
    let Some(mut press) = view.press.take() else {
        return Preview::None;
    };
    let pos = pos.unwrap_or(press.at);
    if !press.dragging && pos.distance(press.at) > DRAG_AFTER {
        press.dragging = true;
    }
    if locked && !matches!(press.target, Target::Bar { .. }) {
        return Preview::None;
    }
    if escape && !matches!(press.target, Target::Seek | Target::Bar { .. }) {
        return Preview::None;
    }
    let preview = preview(timeline, g, view, &press, pos, modifiers.shift);
    match &press.target {
        Target::Seek => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
        Target::Item { .. } | Target::Cell { moves: true, .. } if press.dragging => {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing)
        }
        Target::ItemEdge { .. } | Target::SpanEdge { .. } | Target::Handle { .. } => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal)
        }
        _ => {}
    }
    if let Target::Bar { grab } = press.target {
        if let Some(thumb) = g.thumb(view.scroll) {
            let room = (g.bar.width() - thumb.width()).max(1.0);
            view.scroll = ((pos.x - grab - g.bar.left()) / room * g.max_scroll).clamp(0.0, g.max_scroll);
        }
        if down {
            view.press = Some(press);
        }
        return Preview::None;
    }
    if let Preview::Seek(frame) = preview
        && press.last_seek != Some(frame)
    {
        out.push(Out::Seek { frame, live: true });
        press.last_seek = Some(frame);
    }
    if down {
        view.press = Some(press);
        return preview;
    }

    // Let go.
    let add = modifiers.shift;
    let toggle = modifiers.ctrl || modifiers.command;
    let row_id = |row: usize| timeline.rows[row].id.clone();
    if !press.dragging {
        match press.target {
            Target::Item { row, item } | Target::ItemEdge { row, item, .. } => {
                let declared = &items(&timeline.rows[row])[item];
                let pick = TimelinePick { row: row_id(row), item: Some(declared.id.clone()), frame: None };
                out.push(Out::Gesture(TimelineGesture::Select { picks: vec![pick], add, toggle }));
                if double && let Some(open) = &declared.open {
                    out.push(Out::Run(open.clone()));
                }
            }
            Target::Cell { row, frame, .. } => click_cell(timeline, view, row, frame, add, toggle, out),
            Target::SpanEdge { row, span } => {
                let (_, spans) = cells(&timeline.rows[row]);
                let frame = (spans[span].start + spans[span].length).saturating_sub(1);
                click_cell(timeline, view, row, frame, add, toggle, out);
            }
            _ => {}
        }
    }
    if let Preview::Seek(frame) = preview {
        out.push(Out::Seek { frame, live: false });
        return Preview::None;
    }
    if !press.dragging {
        return Preview::None;
    }
    match preview {
        Preview::Range { index, start, end } => {
            let range = &timeline.ranges[index];
            if (start, end) != (range.start, range.end) {
                out.push(Out::Gesture(TimelineGesture::Range { id: range.id.clone(), start, end }));
            }
        }
        Preview::Items(moving) => {
            if moving.delta != 0 || moving.row.is_some() {
                let picks = moving
                    .moved
                    .iter()
                    .map(|(row, item)| TimelinePick {
                        row: row_id(*row),
                        item: Some(items(&timeline.rows[*row])[*item].id.clone()),
                        frame: None,
                    })
                    .collect();
                out.push(Out::Gesture(TimelineGesture::Move {
                    picks,
                    frames: moving.delta,
                    row: moving.row.map(row_id),
                    push: moving.landing.shift,
                }));
            }
        }
        Preview::Cells(moving) => {
            if moving.delta != 0 {
                out.push(Out::Gesture(TimelineGesture::Move {
                    picks: moving.picks,
                    frames: moving.delta,
                    row: None,
                    push: moving.landing.shift,
                }));
            }
        }
        Preview::Resize(resize) => {
            if resize.delta != 0 {
                let declared = &timeline.rows[resize.row];
                let pick = match resize.item {
                    Some(item) => TimelinePick { row: declared.id.clone(), item: Some(items(declared)[item].id.clone()), frame: None },
                    None => TimelinePick { row: declared.id.clone(), item: None, frame: Some(resize.start) },
                };
                out.push(Out::Gesture(TimelineGesture::Resize { pick, edge: resize.edge, frames: resize.delta }));
            }
        }
        Preview::Block { rows, start, end } => {
            let ids: Vec<String> = rows.into_iter().map(row_id).collect();
            view.block = Some((ids.clone(), start, end));
            out.push(Out::Gesture(TimelineGesture::SelectCells { rows: ids, start, end }));
        }
        Preview::None | Preview::Seek(_) => {}
    }
    Preview::None
}

/// A click on a cell: its mark is picked, or the cell alone is chosen.
fn click_cell(timeline: &Timeline, view: &mut View, row: usize, frame: u32, add: bool, toggle: bool, out: &mut Vec<Out>) {
    let declared = &timeline.rows[row];
    let (marks, _) = cells(declared);
    if marks.iter().any(|mark| mark.frame == frame) {
        let pick = TimelinePick { row: declared.id.clone(), item: None, frame: Some(frame) };
        out.push(Out::Gesture(TimelineGesture::Select { picks: vec![pick], add, toggle }));
    } else {
        view.block = Some((vec![declared.id.clone()], frame, frame));
        out.push(Out::Gesture(TimelineGesture::SelectCells { rows: vec![declared.id.clone()], start: frame, end: frame }));
    }
}

/// The nearest of `candidates` to any of `edges`, within the snap's reach:
/// how far to move, and the frame snapped to.
fn snap(edges: &[f32], candidates: &[f32], zoom: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for edge in edges {
        for candidate in candidates {
            let gap = candidate - edge;
            if gap.abs() * zoom <= SNAP && best.is_none_or(|(shift, _)| gap.abs() < shift.abs()) {
                best = Some((gap, *candidate));
            }
        }
    }
    best
}

/// Every item edge outside `skip`, and the playhead.
fn item_edges(timeline: &Timeline, skip: &[(usize, usize)]) -> Vec<f32> {
    let mut edges = vec![timeline.playhead as f32];
    for (row, declared) in timeline.rows.iter().enumerate() {
        for (index, item) in items(declared).iter().enumerate() {
            if !skip.contains(&(row, index)) {
                edges.push(item.start as f32);
                edges.push((item.start + item.length) as f32);
            }
        }
    }
    edges
}

fn preview(timeline: &Timeline, g: &Geo, view: &View, press: &Press, pos: Pos2, shift: bool) -> Preview {
    let last = timeline.ruler.length.max(1) - 1;
    let raw = ((pos.x - press.at.x) / g.zoom).round() as i64;
    match press.target {
        Target::Seek => {
            let frame = match g.unit {
                RulerUnit::Frames => g.frame_at(pos.x).floor(),
                RulerUnit::Timecode => g.frame_at(pos.x).round(),
            };
            Preview::Seek((frame.max(0.0) as u32).min(last))
        }
        Target::Handle { index, edge } => {
            let Some(range) = timeline.ranges.get(index) else {
                return Preview::None;
            };
            let at = g.frame_at(pos.x).round().max(0.0) as u32;
            let (start, end) = match edge {
                Edge::Start => (at.min(range.end), range.end),
                Edge::End => (range.start, at.saturating_sub(1).max(range.start)),
            };
            Preview::Range { index, start, end }
        }
        Target::Item { row, item } if press.dragging => {
            let Some(pressed) = items(&timeline.rows[row]).get(item) else {
                return Preview::None;
            };
            if !pressed.movable {
                return Preview::None;
            }
            let moved: Vec<(usize, usize)> = if pressed.selected {
                timeline
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, declared)| !declared.read_only)
                    .flat_map(|(r, declared)| {
                        items(declared)
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| item.selected && item.movable)
                            .map(move |(i, _)| (r, i))
                    })
                    .collect()
            } else {
                vec![(row, item)]
            };
            let lowest = moved.iter().map(|(r, i)| items(&timeline.rows[*r])[*i].start).min().unwrap_or(0);
            let mut delta = raw.max(-i64::from(lowest));
            let candidates = item_edges(timeline, &moved);
            let edges: Vec<f32> = moved
                .iter()
                .flat_map(|(r, i)| {
                    let item = &items(&timeline.rows[*r])[*i];
                    [(item.start as i64 + delta) as f32, (item.start as i64 + item.length as i64 + delta) as f32]
                })
                .collect();
            let mut snapped = None;
            if let Some((shift_by, at)) = snap(&edges, &candidates, g.zoom) {
                delta = (delta + shift_by.round() as i64).max(-i64::from(lowest));
                snapped = Some(at);
            }
            let one_row = moved.iter().all(|(r, _)| *r == row);
            let over = g.row_near(pos.y).unwrap_or(row);
            let target = (one_row
                && over != row
                && !timeline.rows[over].read_only
                && matches!(timeline.rows[over].content, RowContent::Items { .. }))
            .then_some(over);
            let mut landing = Landing { shift, snap: snapped, over: target.unwrap_or(row), ..Default::default() };
            for (r, i) in &moved {
                let dest = target.unwrap_or(*r);
                let item = &items(&timeline.rows[*r])[*i];
                let start = (item.start as i64 + delta).max(0) as u32;
                let end = start + item.length;
                for (j, other) in items(&timeline.rows[dest]).iter().enumerate() {
                    if moved.contains(&(dest, j)) || other.style == ItemStyle::Gap {
                        continue;
                    }
                    let (a, b) = (start.max(other.start), end.min(other.start + other.length));
                    if a < b {
                        landing.replaced.push((dest, a, b));
                        landing.count += b - a;
                    }
                }
            }
            landing.push = (timeline.rows[landing.over].drop == DropRule::Push) != shift;
            Preview::Items(ItemMove { moved, delta, row: target, landing })
        }
        Target::ItemEdge { row, item, edge } if press.dragging => {
            let Some(declared) = items(&timeline.rows[row]).get(item) else {
                return Preview::None;
            };
            let (start, length) = (declared.start as i64, declared.length as i64);
            let clamp = |delta: i64| match edge {
                Edge::Start => delta.clamp(-start, length - 1),
                Edge::End => delta.max(1 - length),
            };
            let mut delta = clamp(raw);
            let moving_edge = match edge {
                Edge::Start => start + delta,
                Edge::End => start + length + delta,
            };
            let mut snapped = None;
            if let Some((shift_by, at)) = snap(&[moving_edge as f32], &item_edges(timeline, &[(row, item)]), g.zoom) {
                delta = clamp(delta + shift_by.round() as i64);
                snapped = Some(at);
            }
            Preview::Resize(Resize {
                row,
                item: Some(item),
                span: None,
                edge,
                start: declared.start,
                length: declared.length,
                delta,
                snap: snapped,
            })
        }
        Target::SpanEdge { row, span } if press.dragging => {
            let (_, spans) = cells(&timeline.rows[row]);
            let Some(declared) = spans.get(span) else {
                return Preview::None;
            };
            Preview::Resize(Resize {
                row,
                item: None,
                span: Some(span),
                edge: Edge::End,
                start: declared.start,
                length: declared.length,
                delta: raw.max(1 - declared.length as i64),
                snap: None,
            })
        }
        Target::Cell { row, frame, moves: true } if press.dragging => cell_move(timeline, g, view, row, frame, raw, pos, shift),
        Target::Cell { row, frame, .. } | Target::Empty { row, frame } if press.dragging => {
            let over = g.row_near(pos.y).unwrap_or(row);
            let rows = (row.min(over)..=row.max(over)).filter(|r| !timeline.rows[*r].read_only).collect();
            let to = g.cell_at(pos.x);
            Preview::Block { rows, start: frame.min(to), end: frame.max(to) }
        }
        _ => Preview::None,
    }
}

/// Moving the chosen cells: the block chosen here, or every chosen mark
/// with the span it starts.
#[allow(clippy::too_many_arguments)]
fn cell_move(timeline: &Timeline, g: &Geo, view: &View, row: usize, frame: u32, raw: i64, pos: Pos2, shift: bool) -> Preview {
    let mut runs: Vec<(usize, u32, u32)> = Vec::new();
    let mut picks = Vec::new();
    if in_block(view, &timeline.rows[row].id, frame)
        && let Some((ids, start, end)) = &view.block
    {
        for (r, declared) in timeline.rows.iter().enumerate() {
            if ids.contains(&declared.id) {
                runs.push((r, *start, end - start + 1));
                picks.extend((*start..=*end).map(|f| TimelinePick { row: declared.id.clone(), item: None, frame: Some(f) }));
            }
        }
    } else {
        for (r, declared) in timeline.rows.iter().enumerate() {
            if declared.read_only {
                continue;
            }
            let (marks, spans) = cells(declared);
            for mark in marks.iter().filter(|mark| mark.selected) {
                let length = spans.iter().find(|span| span.start == mark.frame).map_or(1, |span| span.length.max(1));
                runs.push((r, mark.frame, length));
                picks.push(TimelinePick { row: declared.id.clone(), item: None, frame: Some(mark.frame) });
            }
        }
    }
    let lowest = runs.iter().map(|(_, start, _)| *start).min().unwrap_or(0);
    let delta = raw.max(-i64::from(lowest));
    let over = g.row_near(pos.y).unwrap_or(row);
    let mut landing = Landing { shift, over, ..Default::default() };
    for (r, start, length) in &runs {
        let (marks, spans) = cells(&timeline.rows[*r]);
        let moved = |f: u32| runs.iter().any(|(rr, s, l)| rr == r && f >= *s && f < s + l);
        let taken = |f: u32| {
            marks.iter().any(|mark| mark.frame == f) || spans.iter().any(|span| f >= span.start && f < span.start + span.length)
        };
        let from = (*start as i64 + delta).max(0) as u32;
        let mut run: Option<u32> = None;
        for f in from..=from + length {
            let hits = f < from + length && taken(f) && !moved(f);
            match (hits, run) {
                (true, None) => run = Some(f),
                (false, Some(a)) => {
                    landing.replaced.push((*r, a, f));
                    landing.count += f - a;
                    run = None;
                }
                _ => {}
            }
        }
    }
    landing.push = (timeline.rows[row].drop == DropRule::Push) != shift;
    Preview::Cells(CellMove { runs, picks, delta, landing })
}

fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

/// `a` taken `t` of the way to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

/// A filled convex polygon.
fn fill_polygon(painter: &Painter, points: &[Pos2], color: Color32) {
    let mut mesh = Mesh::default();
    for point in points {
        mesh.colored_vertex(*point, color);
    }
    for index in 1..points.len().saturating_sub(1) as u32 {
        mesh.add_triangle(0, index, index + 1);
    }
    painter.add(mesh);
}

/// An edge `width` points wide inside `rect`, one point at a time.
fn thick_edge(painter: &Painter, rect: Rect, radius: f32, color: Color32, width: f32) {
    let mut inset = 0.0;
    while inset < width.max(1.0) {
        widgets::outlined(painter, rect.shrink(inset), (radius - inset).max(0.0), Color32::TRANSPARENT, color);
        inset += 1.0;
    }
}

fn dashed_line(painter: &Painter, from: Pos2, to: Pos2, color: Color32, width: f32) {
    let length = from.distance(to);
    if length <= 0.0 {
        return;
    }
    let step = (to - from) / length;
    let mut at = 0.0;
    while at < length {
        let end = (at + 5.0).min(length);
        painter.line_segment([from + step * at, from + step * end], (width, color));
        at += 9.0;
    }
}

fn dashed_rect(painter: &Painter, rect: Rect, color: Color32) {
    let [a, b, c, d] = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for (from, to) in [(a, b), (b, c), (d, c), (a, d)] {
        dashed_line(painter, from, to, color, 1.2);
    }
}

/// Diagonal lines over `rect`, as a gap or frames about to be replaced.
fn hatch(painter: &Painter, rect: Rect, color: Color32) {
    let inner = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let height = rect.height();
    let mut x = rect.left() - height;
    while x < rect.right() {
        inner.line_segment([pos2(x, rect.bottom()), pos2(x + height, rect.top())], (1.0, color));
        x += 7.0;
    }
}

/// A dark rounded tooltip with its bottom middle at `anchor`, kept inside
/// `bounds`, with an icon before the words when one is given.
fn dark_tip(painter: &Painter, anchor: Pos2, text: &str, icon: Option<&str>, bounds: Rect) {
    let font = FontId::proportional(12.5);
    let galley = painter.layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let lead = if icon.is_some() { 18.0 } else { 0.0 };
    let size = vec2(galley.size().x + lead + 16.0, galley.size().y + 8.0);
    let left = (anchor.x - size.x / 2.0).clamp(bounds.left() + 2.0, (bounds.right() - size.x - 2.0).max(bounds.left()));
    let place = Rect::from_min_size(pos2(left, anchor.y - size.y), size);
    painter.rect_filled(place, 6.0, widgets::INK);
    if let Some(icon) = icon {
        paint_icon(painter, pos2(place.left() + 8.0 + 7.0, place.center().y), icon, 13.0, Color32::WHITE);
    }
    painter.galley(pos2(place.left() + 8.0 + lead, place.top() + 4.0), galley, Color32::WHITE);
}

fn signed(frames: i64) -> String {
    if frames < 0 { format!("\u{2212}{}", -frames) } else { format!("+{frames}") }
}

fn rate(fps: f64) -> u32 {
    fps.round().max(1.0) as u32
}

fn timecode(frame: u32, fps: f64) -> String {
    let rate = rate(fps);
    let seconds = frame / rate;
    format!("{:02}:{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60, frame % rate)
}

/// A resize's tooltip: its change in frames, with the new length in
/// seconds and frames when the ruler reads time, or the multiple when a
/// span is stretched to a whole multiple of itself.
fn resize_text(unit: RulerUnit, fps: f64, length: u32, delta: i64, stretch: bool) -> String {
    let next = (length as i64 + delta).max(1) as u32;
    if stretch && length > 0 && next % length == 0 && next / length >= 2 {
        return format!("\u{d7}{}", next / length);
    }
    match unit {
        RulerUnit::Frames => format!("{} f", signed(delta)),
        RulerUnit::Timecode => {
            let rate = rate(fps);
            format!("{} s {} f ({} f)", next / rate, next % rate, signed(delta))
        }
    }
}

/// A move's tooltip and its icon: the frames it replaces, how far later
/// frames move, or else how far it moves.
fn move_text(landing: &Landing, delta: i64) -> (String, Option<&'static str>) {
    if landing.count == 0 {
        return (format!("{} f", signed(delta)), None);
    }
    if landing.push {
        let icon = landing.shift.then_some("arrow-up");
        return (format!("Moves later frames +{}", landing.count), icon);
    }
    let noun = if landing.count == 1 { "frame" } else { "frames" };
    (format!("Replaces {} {noun}", landing.count), None)
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut egui::Ui,
    painter: &Painter,
    timeline: &Timeline,
    g: &Geo,
    view: &View,
    preview: &Preview,
    base: Id,
    out: &mut Vec<Out>,
) {
    let rows_painter = painter.with_clip_rect(g.rows.intersect(painter.clip_rect()));
    let names_painter = painter.with_clip_rect(g.names.intersect(painter.clip_rect()));
    let tips = painter.clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, base.with("tips")));

    paint_ruler(painter, g, timeline);
    // The add button sits above the names.
    if let Some(add) = &timeline.add_row {
        let spot = Rect::from_center_size(pos2(g.ruler_names.left() + 22.0, g.ruler_names.center().y), vec2(26.0, 26.0));
        let response = ui
            .interact(spot, base.with("add-row"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            painter.rect_filled(spot, 6.0, widgets::SELECTED);
        }
        paint_icon(painter, spot.center(), "plus", 18.0, widgets::TEXT);
        if response.clicked() {
            out.push(Out::Run(add.clone()));
        }
    }

    // The frame grid, under rows of cells.
    if g.unit == RulerUnit::Frames {
        let first = g.cell_at(g.rows.left()).saturating_sub(1);
        let last = g.cell_at(g.rows.right()) + 1;
        for (row, declared) in timeline.rows.iter().enumerate() {
            if !matches!(declared.content, RowContent::Cells { .. }) {
                continue;
            }
            let (top, bottom) = g.tops[row];
            if 5.0 * g.zoom >= 6.0 {
                for f in (first..=last).filter(|f| (f + 1) % 5 == 0) {
                    let cell = Rect::from_min_max(pos2(g.x(f as f32), top), pos2(g.x((f + 1) as f32), bottom));
                    rows_painter.rect_filled(cell, 0.0, FIFTH);
                }
            }
            if g.zoom >= 5.0 {
                for f in first..=last {
                    rows_painter.vline(g.x(f as f32).round() + 0.5, top..=bottom, (1.0, GRID));
                }
            }
        }
    }

    let moving_items = match preview {
        Preview::Items(moving) => Some(moving),
        _ => None,
    };
    let moving_cells = match preview {
        Preview::Cells(moving) => Some(moving),
        _ => None,
    };
    let resizing = match preview {
        Preview::Resize(resize) => Some(resize),
        _ => None,
    };

    for (row, declared) in timeline.rows.iter().enumerate() {
        let (top, bottom) = g.tops[row];
        if bottom < g.rows.top() || top > g.rows.bottom() {
            continue;
        }
        let names = Rect::from_min_max(pos2(g.names.left(), top), pos2(g.names.right(), bottom));
        let strip = g.row_rect(row);
        if declared.selected {
            names_painter.rect_filled(names, 0.0, ROW_SELECTED);
            rows_painter.rect_filled(strip, 0.0, ROW_SELECTED.gamma_multiply(0.6));
        }
        if let Some((ids, start, end)) = &view.block
            && ids.contains(&declared.id)
        {
            let block = Rect::from_min_max(pos2(g.x(*start as f32), top + 3.0), pos2(g.x((end + 1) as f32), bottom - 3.0));
            rows_painter.rect_filled(block, 0.0, CELL_BLUE);
        }
        if let Preview::Block { rows, start, end } = preview
            && rows.contains(&row)
        {
            let block = Rect::from_min_max(pos2(g.x(*start as f32), top + 3.0), pos2(g.x((end + 1) as f32), bottom - 3.0));
            rows_painter.rect_filled(block, 0.0, CELL_BLUE.gamma_multiply(0.8));
        }
        let above = if declared.read_only { widgets::RAIL } else { widgets::BORDER };
        if declared.read_only {
            painter.hline(g.whole.x_range(), top + 0.5, (1.5, above));
        }
        let line = painter.with_clip_rect(Rect::from_min_max(pos2(g.whole.left(), g.rows.top()), g.rows.max));
        line.hline(g.whole.x_range(), bottom - 0.5, (1.0, widgets::BORDER));

        // The row's empty space opens its menu.
        let strip_response = ui.interact(strip.intersect(g.rows), base.with(("strip", row)), Sense::click());
        if let Some(action) = widgets::action_menu(&strip_response, &declared.menu).and_then(|entry| entry.action.clone()) {
            out.push(Out::Run(action));
        }
        row_names(ui, &names_painter, declared, names.intersect(g.names), base.with(("names", row)), out);

        match &declared.content {
            RowContent::Items { items } => {
                for (index, item) in items.iter().enumerate() {
                    let lifted = moving_items.is_some_and(|moving| moving.moved.contains(&(row, index)));
                    let mut start = item.start as i64;
                    let mut length = item.length as i64;
                    if let Some(moving) = moving_items
                        && moving.landing.push
                        && moving.landing.count > 0
                        && moving.landing.over == row
                        && !lifted
                        && moving.moved.iter().any(|(r, i)| {
                            let moved = &timeline.rows[*r];
                            item.start as i64 >= self::items(moved)[*i].start as i64 + moving.delta
                        })
                    {
                        start += i64::from(moving.landing.count);
                    }
                    if let Some(resize) = resizing
                        && resize.row == row
                        && resize.item == Some(index)
                    {
                        match resize.edge {
                            Edge::Start => {
                                start += resize.delta;
                                length -= resize.delta;
                            }
                            Edge::End => length += resize.delta,
                        }
                    }
                    let place = item_rect(g, row, start.max(0) as u32, length.max(1) as u32);
                    if place.right() < g.rows.left() || place.left() > g.rows.right() {
                        continue;
                    }
                    if lifted {
                        dashed_rect(&rows_painter, place, BLUE);
                    } else {
                        paint_item(ui, &rows_painter, item, place);
                    }
                    item_response(ui, &rows_painter, g, item, place, base.with(("item", row, index)), view.press.is_none(), out);
                }
            }
            RowContent::Cells { marks, spans } => {
                let mut marks = marks.clone();
                let mut spans = spans.clone();
                let mut stretched = None;
                if let Some(resize) = resizing
                    && resize.row == row
                    && let Some(index) = resize.span
                    && let Some(span) = spans.get_mut(index)
                {
                    let old_end = span.start + span.length;
                    span.length = (span.length as i64 + resize.delta).max(1) as u32;
                    for mark in &mut marks {
                        if mark.frame + 1 == old_end && mark.kind == MarkKind::HoldEnd {
                            mark.frame = span.start + span.length - 1;
                        }
                    }
                    stretched = Some(index);
                }
                let runs: Vec<(u32, u32)> = moving_cells
                    .map(|moving| moving.runs.iter().filter(|(r, ..)| *r == row).map(|(_, s, l)| (*s, *l)).collect())
                    .unwrap_or_default();
                paint_cells(&rows_painter, g, declared, &marks, &spans, &runs, moving_cells.map_or(0, |m| m.delta), stretched, top, bottom);
            }
        }
    }
    painter.vline(g.names.right() - 0.5, g.ruler.top()..=g.whole.bottom(), (1.0, widgets::BORDER));

    // Frames about to be replaced, and items lifted over the rows.
    let landing = moving_items.map(|moving| &moving.landing).or(moving_cells.map(|moving| &moving.landing));
    if let Some(landing) = landing
        && !landing.push
    {
        for (row, start, end) in &landing.replaced {
            let (top, bottom) = g.tops[*row];
            let place = Rect::from_min_max(pos2(g.x(*start as f32), top + 3.0), pos2(g.x(*end as f32), bottom - 3.0));
            rows_painter.rect_filled(place, 0.0, REPLACE.gamma_multiply(0.08));
            hatch(&rows_painter, place, REPLACE.gamma_multiply(0.7));
        }
    }
    let lifted = rows_painter.clone().with_layer_id(egui::LayerId::new(egui::Order::Foreground, base.with("lifted")));
    if let Some(moving) = moving_items {
        for (row, index) in &moving.moved {
            let item = &items(&timeline.rows[*row])[*index];
            let dest = moving.row.unwrap_or(*row);
            let start = (item.start as i64 + moving.delta).max(0) as u32;
            let place = item_rect(g, dest, start, item.length).translate(vec2(0.0, -3.0));
            widgets::soft_shadow(&lifted, place, 6.0);
            paint_item(ui, &lifted, item, place);
        }
    }

    paint_empty_hint(painter, timeline, g);
    paint_ranges(painter, g, timeline, preview);
    let playhead = match preview {
        Preview::Seek(frame) => *frame,
        _ => timeline.playhead,
    };
    paint_playhead(painter, g, playhead);

    if let Some(at) = landing.and_then(|landing| landing.snap).or(resizing.and_then(|resize| resize.snap)) {
        let x = g.x(at);
        rows_painter.vline(x, g.rows.y_range(), (1.5, BLUE));
    }

    // The tooltips of the gesture under way.
    let bounds = g.whole;
    let pointer = ui.input(|input| input.pointer.latest_pos());
    match preview {
        Preview::Seek(frame) => {
            let text = match g.unit {
                RulerUnit::Frames => (frame + 1).to_string(),
                RulerUnit::Timecode => timecode(*frame, g.fps),
            };
            dark_tip(&tips, pos2(g.playhead_x(*frame), g.ruler.bottom() - 16.0), &text, None, bounds);
        }
        Preview::Items(ItemMove { delta, landing, .. }) | Preview::Cells(CellMove { delta, landing, .. }) => {
            let (text, icon) = move_text(landing, *delta);
            let top = g.tops.get(landing.over).map_or(g.rows.top(), |(top, _)| *top);
            let x = pointer.map_or(g.rows.center().x, |pos| pos.x);
            dark_tip(&tips, pos2(x, top.max(g.rows.top()) - 2.0), &text, icon, bounds);
        }
        Preview::Resize(resize) => {
            let text = resize_text(g.unit, g.fps, resize.length, resize.delta, resize.span.is_some());
            let edge = match resize.edge {
                Edge::Start => resize.start as i64 + resize.delta,
                Edge::End => resize.start as i64 + resize.length as i64 + resize.delta,
            };
            let top = g.tops[resize.row].0.max(g.rows.top());
            dark_tip(&tips, pos2(g.x(edge as f32), top - 2.0), &text, None, bounds);
        }
        _ => {}
    }

    // The scroll bar, when the rows are wider than their room.
    if let Some(thumb) = g.thumb(view.scroll) {
        let held = matches!(view.press.as_ref().map(|press| &press.target), Some(Target::Bar { .. }));
        let hovered = ui.rect_contains_pointer(g.bar);
        let color = if held || hovered { TICK } else { widgets::RAIL };
        painter.rect_filled(thumb, thumb.height() / 2.0, color);
    }
}

/// A row's names: its disclosure, colour, icon, name, and toggles. A click
/// picks the row; a right-click opens its menu.
fn row_names(ui: &mut egui::Ui, painter: &Painter, row: &TimelineRow, place: Rect, id: Id, out: &mut Vec<Out>) {
    if place.height() <= 0.0 {
        return;
    }
    let response = ui.interact(place, id, Sense::click());
    if response.clicked() && !row.read_only {
        let (add, toggle) = ui.input(|input| (input.modifiers.shift, input.modifiers.ctrl || input.modifiers.command));
        let pick = TimelinePick { row: row.id.clone(), item: None, frame: None };
        out.push(Out::Gesture(TimelineGesture::Select { picks: vec![pick], add, toggle }));
    }
    if let Some(action) = widgets::action_menu(&response, &row.menu).and_then(|entry| entry.action.clone()) {
        out.push(Out::Run(action));
    }
    let cy = place.center().y;
    let mut x = place.left() + 10.0 + f32::from(row.indent) * INDENT;
    if let Some(open) = row.open {
        let spot = Rect::from_center_size(pos2(x + 7.0, cy), vec2(18.0, place.height().min(24.0)));
        let response = ui.interact(spot, id.with("disclose"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let color = if response.hovered() { widgets::TEXT } else { widgets::MUTED };
        if open {
            widgets::chevron(painter, spot.center(), color, Scale(1.0));
        } else {
            widgets::chevron_side(painter, spot.center(), true, color);
        }
        if response.clicked()
            && let Some(action) = &row.disclose
        {
            out.push(Out::Run(action.clone()));
        }
        x += 22.0;
    }
    let cells_row = matches!(row.content, RowContent::Cells { .. });
    let toggle_ui = |ui: &mut egui::Ui, x: f32, index: usize, out: &mut Vec<Out>| {
        let toggle = &row.toggles[index];
        let spot = Rect::from_center_size(pos2(x + 12.0, cy), vec2(24.0, 24.0));
        let response = ui
            .interact(spot, id.with(("toggle", index)), Sense::click())
            .on_hover_text(toggle.tip.as_str());
        let color = match (toggle.on, response.hovered()) {
            (_, true) => crate::toolbar::ICON_ACTIVE,
            (true, false) => widgets::TEXT,
            (false, false) => TOGGLE_OFF,
        };
        paint_icon(painter, spot.center(), &toggle.icon, 17.0, color);
        if response.clicked() {
            out.push(Out::Run(toggle.action.clone()));
        }
    };
    // A track's toggles lead its name; a layer's follow it at the right.
    let mut name_right = place.right() - 8.0;
    if cells_row {
        let mut right = place.right() - 8.0;
        for index in (0..row.toggles.len()).rev() {
            right -= 26.0;
            toggle_ui(ui, right, index, out);
        }
        name_right = right - 4.0;
        if row.indent == 0 || !row.toggles.is_empty() {
            let square = Rect::from_center_size(pos2(x + 9.0, cy), vec2(18.0, 18.0));
            painter.rect_filled(square, 3.0, rgb(row.color));
            x += 28.0;
        }
    } else {
        for index in 0..row.toggles.len() {
            toggle_ui(ui, x - 4.0, index, out);
            x += 28.0;
        }
    }
    if let Some(icon) = &row.icon {
        paint_icon(painter, pos2(x + 9.0, cy), icon, 17.0, widgets::TEXT);
        x += 28.0;
    }
    let galley = painter.layout_no_wrap(row.name.clone(), widgets::body(Scale(1.0)), widgets::TEXT);
    let name = painter.with_clip_rect(Rect::from_min_max(pos2(x, place.top()), pos2(name_right.max(x), place.bottom())).intersect(painter.clip_rect()));
    name.galley(pos2(x, cy - galley.size().y / 2.0), galley, widgets::TEXT);
}

/// An item: its fill and edge in its style, its thumbnail, icon, label,
/// progress, waveform, and end mark.
fn paint_item(ui: &egui::Ui, painter: &Painter, item: &TimelineItem, place: Rect) {
    let radius = 6.0;
    let declared = rgb(item.fill);
    let (fill, text) = match item.style {
        ItemStyle::Solid => (declared, widgets::TEXT),
        ItemStyle::Dashed => (mix(declared, Color32::WHITE, 0.5), widgets::TEXT),
        ItemStyle::Ghost => (mix(declared, Color32::WHITE, 0.55), widgets::MUTED),
        ItemStyle::Gap => (GAP_FILL, widgets::MUTED),
    };
    painter.rect_filled(place, radius, fill);
    match (&item.border, item.style) {
        (_, ItemStyle::Gap) => {
            hatch(painter, place.shrink(1.0), HATCH);
            if let Some(border) = &item.border {
                thick_edge(painter, place, radius, rgb(border.rgb), border.width);
            }
        }
        (_, ItemStyle::Dashed) => dashed_rect(painter, place, mix(declared, Color32::BLACK, 0.35)),
        (Some(border), _) => thick_edge(painter, place, radius, rgb(border.rgb), border.width),
        (None, _) => widgets::outlined(painter, place, radius, Color32::TRANSPARENT, mix(fill, Color32::BLACK, 0.12)),
    }
    if item.selected {
        thick_edge(painter, place, radius, BLUE, 2.0);
    }
    let inner = painter.with_clip_rect(place.shrink(2.0).intersect(painter.clip_rect()));
    let cy = place.center().y;
    let mut x = place.left() + 6.0;
    let end_room = if item.end_mark.is_some() { 26.0 } else { 6.0 };
    if item.progress.is_some() {
        let spot = Rect::from_center_size(pos2(x + 9.0, cy), vec2(16.0, 16.0));
        if inner.clip_rect().contains_rect(spot) {
            egui::Spinner::new().size(15.0).color(widgets::MUTED).paint_at(ui, spot);
        }
        x += 26.0;
    } else if let Some(thumbnail) = &item.thumbnail {
        let height = place.height() - 8.0;
        let ratio = thumbnail.width.max(1) as f32 / thumbnail.height.max(1) as f32;
        let width = (height * ratio).clamp(height * 0.75, height * 2.2);
        if place.width() > width + 24.0
            && let Some(texture) = crate::frames::texture(ui.ctx(), thumbnail)
        {
            let spot = Rect::from_min_size(pos2(place.left() + 4.0, place.top() + 4.0), vec2(width, height));
            let options = egui::ImageOptions { corner_radius: 4.0.into(), ..Default::default() };
            egui::widgets::paint_texture_at(&inner, spot, &options, &egui::load::SizedTexture::new(texture.id(), spot.size()));
            x = spot.right() + 10.0;
        }
    }
    if let Some(icon) = &item.icon {
        paint_icon(&inner, pos2(x + 8.0, cy), icon, 16.0, text);
        x += 24.0;
    }
    if !item.label.is_empty() {
        let galley = inner.layout_no_wrap(item.label.clone(), widgets::body(Scale(1.0)), text);
        let width = galley.size().x;
        inner.galley(pos2(x, cy - galley.size().y / 2.0), galley, text);
        x += width + 14.0;
    }
    if let Some(progress) = item.progress {
        let progress = progress.clamp(0.0, 1.0);
        let track = Rect::from_min_size(pos2(x, cy - 2.0), vec2(120.0, 4.0));
        inner.rect_filled(track, 2.0, widgets::RAIL);
        inner.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * progress, 4.0)), 2.0, BLUE);
        inner.text(
            pos2(track.right() + 10.0, cy),
            Align2::LEFT_CENTER,
            format!("{}%", (progress * 100.0).round()),
            FontId::proportional(12.0),
            widgets::MUTED,
        );
        ui.ctx().request_repaint();
    }
    if !item.waveform.is_empty() {
        // The peaks are spread over the whole item and drawn after its words.
        let color = mix(declared, Color32::BLACK, 0.45);
        let half = (place.height() - 10.0) / 2.0;
        let count = item.waveform.len() as f32;
        let mut px = x.max(place.left() + 4.0);
        while px < place.right() - end_room {
            let from = ((px - place.left()) / place.width() * count) as usize;
            let to = (((px + 1.5 - place.left()) / place.width() * count).ceil() as usize).clamp(from + 1, item.waveform.len());
            let peak = item.waveform[from.min(item.waveform.len() - 1)..to].iter().copied().fold(0.0_f32, f32::max).clamp(0.0, 1.0);
            let reach = (peak * half).max(0.5);
            inner.line_segment([pos2(px, cy - reach), pos2(px, cy + reach)], (1.0, color));
            px += 1.5;
        }
    }
    if let Some(mark) = &item.end_mark {
        let color = item.border.as_ref().map_or(widgets::TEXT, |border| rgb(border.rgb));
        paint_icon(&inner, pos2(place.right() - 14.0, cy), &mark.icon, 16.0, color);
    }
}

/// An item's tips, its menu, and the cursor over its edges.
#[allow(clippy::too_many_arguments)]
fn item_response(ui: &mut egui::Ui, painter: &Painter, g: &Geo, item: &TimelineItem, place: Rect, id: Id, idle: bool, out: &mut Vec<Out>) {
    let shown = place.intersect(g.rows).intersect(painter.clip_rect());
    if !shown.is_positive() {
        return;
    }
    let mut response = ui.interact(shown, id, Sense::click());
    if idle && !item.tip.is_empty() {
        response = response.on_hover_text(item.tip.as_str());
    }
    if idle
        && item.movable
        && place.width() > 3.0 * EDGE
        && let Some(pos) = response.hover_pos()
        && (pos.x - place.left() <= EDGE || place.right() - pos.x <= EDGE)
    {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if let Some(action) = widgets::action_menu(&response, &item.menu).and_then(|entry| entry.action.clone()) {
        out.push(Out::Run(action));
    }
    if let Some(mark) = &item.end_mark {
        let spot = Rect::from_center_size(pos2(place.right() - 14.0, place.center().y), vec2(18.0, 18.0)).intersect(shown);
        if spot.is_positive() && idle {
            ui.interact(spot, id.with("end"), Sense::hover()).on_hover_text(mark.tip.as_str());
        }
    }
}

/// A row's cells: its spans, then its marks. Cells in `runs` are being
/// moved by `delta`: they are drawn faint where they were and chosen where
/// they land. A `stretched` span is drawn chosen.
#[allow(clippy::too_many_arguments)]
fn paint_cells(
    painter: &Painter,
    g: &Geo,
    row: &TimelineRow,
    marks: &[CellMark],
    spans: &[CellSpan],
    runs: &[(u32, u32)],
    delta: i64,
    stretched: Option<usize>,
    top: f32,
    bottom: f32,
) {
    let color = rgb(row.color);
    let cy = (top + bottom) / 2.0;
    let inset = ((bottom - top) * 0.18).max(3.0);
    let moved = |frame: u32| runs.iter().any(|(start, length)| frame >= *start && frame < start + length);
    let shifted = |frame: u32| (frame as i64 + delta).max(0) as u32;
    let chosen_start = |frame: u32| marks.iter().any(|mark| mark.frame == frame && mark.selected);

    for (index, span) in spans.iter().enumerate() {
        let lifted = moved(span.start);
        let chosen = stretched == Some(index) || chosen_start(span.start);
        paint_span(painter, g, span, span.start, color, chosen, if lifted { 0.35 } else { 1.0 }, cy, top + inset, bottom - inset);
        if lifted {
            let place = Rect::from_min_max(
                pos2(g.x(span.start as f32), top + inset),
                pos2(g.x((span.start + span.length) as f32), bottom - inset),
            );
            dashed_rect(painter, place, mix(color, Color32::BLACK, 0.3));
            let landed = shifted(span.start);
            paint_span(painter, g, span, landed, color, true, 1.0, cy, top + inset, bottom - inset);
            let place = place.translate(vec2(landed as f32 * g.zoom - span.start as f32 * g.zoom, 0.0));
            thick_edge(painter, place, 2.0, widgets::INK, 2.0);
        }
    }
    for mark in marks {
        let lifted = moved(mark.frame);
        let chosen = mark.selected;
        paint_mark(painter, g, mark, mark.frame, chosen && !lifted, if lifted { 0.35 } else { 1.0 }, cy, top + inset, bottom - inset);
        if lifted {
            paint_mark(painter, g, mark, shifted(mark.frame), true, 1.0, cy, top + inset, bottom - inset);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_span(painter: &Painter, g: &Geo, span: &CellSpan, start: u32, color: Color32, chosen: bool, alpha: f32, cy: f32, top: f32, bottom: f32) {
    let place = Rect::from_min_max(pos2(g.x(start as f32), top), pos2(g.x((start + span.length) as f32), bottom));
    let last = start + span.length.max(1) - 1;
    match span.kind {
        SpanKind::Hold => {
            let (fill, edge) = if chosen {
                (CELL_BLUE, mix(CELL_BLUE, Color32::BLACK, 0.2))
            } else {
                (mix(color, Color32::WHITE, 0.74), mix(color, Color32::WHITE, 0.45))
            };
            widgets::outlined(painter, place, 2.0, fill.gamma_multiply(alpha), edge.gamma_multiply(alpha));
        }
        SpanKind::Tween | SpanKind::BrokenTween => {
            let (fill, ink) = if chosen { (CELL_BLUE, Color32::WHITE) } else { (TWEEN_FILL, widgets::INK) };
            widgets::outlined(painter, place, 2.0, fill.gamma_multiply(alpha), TWEEN_EDGE.gamma_multiply(alpha));
            let reach = (g.zoom * 0.3).clamp(2.0, 5.0) + 3.0;
            let (from, to) = (g.mid(start) + reach, g.mid(last) - reach);
            if to > from {
                let ink = ink.gamma_multiply(alpha);
                if span.kind == SpanKind::Tween {
                    painter.line_segment([pos2(from, cy), pos2(to - 6.0, cy)], (1.2, ink));
                    fill_polygon(painter, &[pos2(to, cy), pos2(to - 8.0, cy - 4.0), pos2(to - 8.0, cy + 4.0)], ink);
                } else {
                    dashed_line(painter, pos2(from, cy), pos2(to, cy), ink, 1.2);
                }
            }
        }
        SpanKind::HoldBar => {
            let bar = Rect::from_min_max(pos2(g.mid(start), cy - 1.5), pos2(g.mid(last), cy + 1.5));
            let fill = if chosen { CELL_BLUE } else { HOLD_BAR };
            painter.rect_filled(bar, 1.5, fill.gamma_multiply(alpha));
        }
    }
}

/// One mark in its frame's cell. A chosen mark is drawn white on a blue
/// cell, or as a blue diamond.
#[allow(clippy::too_many_arguments)]
fn paint_mark(painter: &Painter, g: &Geo, mark: &CellMark, frame: u32, chosen: bool, alpha: f32, cy: f32, top: f32, bottom: f32) {
    let center = pos2(g.mid(frame), cy);
    let cell = Rect::from_min_max(pos2(g.x(frame as f32), top), pos2(g.x((frame + 1) as f32), bottom));
    let on_blue = chosen && !matches!(mark.kind, MarkKind::Diamond | MarkKind::Label { .. } | MarkKind::Comment { .. });
    if on_blue {
        painter.rect_filled(cell, 0.0, CELL_BLUE.gamma_multiply(alpha));
    }
    let ink = if on_blue { Color32::WHITE } else { widgets::INK }.gamma_multiply(alpha);
    let back = if on_blue { CELL_BLUE } else { Color32::WHITE }.gamma_multiply(alpha);
    let radius = (g.zoom * 0.28).clamp(2.0, 5.0);
    let small = FontId::proportional(12.5);
    match &mark.kind {
        MarkKind::Key => widgets::disc(painter, center, radius, ink),
        MarkKind::BlankKey => {
            widgets::disc(painter, center, radius, back);
            widgets::ring(painter, center, radius, 1.4, ink);
        }
        MarkKind::HoldEnd => {
            let place = Rect::from_center_size(center, vec2((g.zoom * 0.35).clamp(3.0, 6.0), (bottom - top) * 0.7));
            widgets::outlined(painter, place, 1.0, back, ink);
        }
        MarkKind::Label { text } => {
            let red = FLAG.gamma_multiply(alpha);
            let pole = cell.left() + 3.0;
            painter.line_segment([pos2(pole, cy - 8.0), pos2(pole, cy + 8.0)], (1.5, red));
            fill_polygon(painter, &[pos2(pole, cy - 8.0), pos2(pole + 10.0, cy - 4.5), pos2(pole, cy - 1.0)], red);
            let words = if chosen { BLUE } else { widgets::TEXT }.gamma_multiply(alpha);
            painter.text(pos2(pole + 14.0, cy), Align2::LEFT_CENTER, text, small, words);
        }
        MarkKind::Comment { text } => {
            let green = COMMENT.gamma_multiply(alpha);
            let galley = painter.layout_no_wrap("//".to_string(), widgets::semibold(13.0), green);
            let width = galley.size().x;
            painter.galley(pos2(cell.left() + 2.0, cy - galley.size().y / 2.0), galley, green);
            let words = if chosen { BLUE } else { widgets::TEXT }.gamma_multiply(alpha);
            painter.text(pos2(cell.left() + width + 7.0, cy), Align2::LEFT_CENTER, text, small, words);
        }
        MarkKind::Diamond => {
            let reach = (g.zoom * 0.36).clamp(3.0, 6.5);
            let points = [
                center + vec2(0.0, -reach),
                center + vec2(reach, 0.0),
                center + vec2(0.0, reach),
                center + vec2(-reach, 0.0),
            ];
            let (fill, edge) = if chosen {
                (CELL_BLUE, mix(CELL_BLUE, Color32::BLACK, 0.45))
            } else {
                (Color32::WHITE, widgets::INK)
            };
            fill_polygon(painter, &points, fill.gamma_multiply(alpha));
            painter.line(vec![points[0], points[1], points[2], points[3], points[0]], (1.3, edge.gamma_multiply(alpha)));
        }
    }
}

/// The ruler: frame numbers every five, or times at an interval that fits
/// the zoom, over ticks.
fn paint_ruler(painter: &Painter, g: &Geo, timeline: &Timeline) {
    let area = Rect::from_min_max(pos2(g.ruler.left() - 36.0, g.ruler.top()), g.ruler.max);
    let ruler = painter.with_clip_rect(area.intersect(painter.clip_rect()));
    painter.hline(g.whole.x_range(), g.ruler.bottom() - 0.5, (1.0, widgets::BORDER));
    if let Some(strip) = g.loop_strip {
        painter.hline(g.whole.x_range(), strip.bottom() - 0.5, (1.0, widgets::BORDER));
    }
    let font = FontId::proportional(12.0);
    let label_y = g.ruler.top() + 12.0;
    let tick_bottom = g.ruler.bottom() - 2.0;
    let length = timeline.ruler.length.max(1);
    let first = g.cell_at(g.ruler.left()).saturating_sub(1);
    let last = (g.cell_at(g.ruler.right()) + 1).min(length.max(extent(timeline)));
    match g.unit {
        RulerUnit::Frames => {
            let step = [5u32, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000]
                .into_iter()
                .find(|step| *step as f32 * g.zoom >= 30.0)
                .unwrap_or(20000);
            for f in first..=last {
                let x = g.mid(f);
                let labelled = f == 0 || (f + 1) % step == 0;
                if labelled {
                    ruler.vline(x, (tick_bottom - 9.0)..=tick_bottom, (1.0, TICK));
                    let strong = f == timeline.playhead;
                    let face = if strong { widgets::semibold(12.5) } else { font.clone() };
                    ruler.text(pos2(x, label_y), Align2::CENTER_CENTER, (f + 1).to_string(), face, widgets::TEXT);
                } else if g.zoom >= 4.0 {
                    ruler.vline(x, (tick_bottom - 5.0)..=tick_bottom, (1.0, TICK));
                }
            }
        }
        RulerUnit::Timecode => {
            let rate = rate(g.fps);
            let label_w = painter.layout_no_wrap("00:00:00:00".to_string(), font.clone(), widgets::TEXT).size().x + 24.0;
            let mut steps: Vec<u32> = [1, 2, 5, 10].into_iter().filter(|step| *step < rate).collect();
            steps.extend([1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600].into_iter().map(|seconds| seconds * rate));
            let step = steps.into_iter().find(|step| *step as f32 * g.zoom >= label_w).unwrap_or(7200 * rate);
            let minor = [10, 8, 5, 4, 2].into_iter().find(|parts| step % parts == 0 && (step / parts) as f32 * g.zoom >= 6.0);
            let mut f = first / step * step;
            while f <= last {
                let x = g.x(f as f32);
                ruler.vline(x, (tick_bottom - 9.0)..=tick_bottom, (1.0, TICK));
                ruler.text(pos2(x, label_y), Align2::CENTER_CENTER, timecode(f, g.fps), font.clone(), widgets::MUTED);
                if let Some(parts) = minor {
                    for part in 1..parts {
                        let at = g.x((f + part * step / parts) as f32);
                        ruler.vline(at, (tick_bottom - 4.0)..=tick_bottom, (1.0, TICK));
                    }
                }
                f += step;
            }
        }
    }
}

/// Ranges: a band in the ruler with an earlier and a later handle, and a
/// loop's bar with square handles in its strip.
fn paint_ranges(painter: &Painter, g: &Geo, timeline: &Timeline, preview: &Preview) {
    let ruler = painter.with_clip_rect(Rect::from_min_max(g.ruler.min, g.loop_strip.map_or(g.ruler.max, |strip| strip.max)).intersect(painter.clip_rect()));
    for (index, range) in timeline.ranges.iter().enumerate() {
        let (start, end) = match preview {
            Preview::Range { index: held, start, end } if *held == index => (*start, *end),
            _ => (range.start, range.end),
        };
        let [(_, left), (_, right)] = handles(g, range.kind, start, end);
        match range.kind {
            RangeKind::OnionSkin => {
                let band = Rect::from_min_max(pos2(left.x, g.ruler.top() + 22.0), pos2(right.x, g.ruler.bottom()));
                ruler.rect_filled(band, 3.0, BAND);
                for (at, color) in [(left, EARLIER), (right, LATER)] {
                    widgets::disc(&ruler, at, 5.0, color);
                    widgets::ring(&ruler, at, 5.0, 1.0, Color32::WHITE);
                }
            }
            RangeKind::Loop => {
                let bar = Rect::from_min_max(pos2(left.x, left.y - 3.0), pos2(right.x, left.y + 3.0));
                widgets::outlined(&ruler, bar, 1.0, LOOP_FILL, LOOP);
                for at in [left, right] {
                    widgets::outlined(&ruler, Rect::from_center_size(at, vec2(8.0, 12.0)), 1.0, Color32::WHITE, widgets::INK);
                }
            }
        }
    }
}

/// The playhead: a purple line under a head on the ruler.
fn paint_playhead(painter: &Painter, g: &Geo, frame: u32) {
    let x = g.playhead_x(frame);
    if x < g.rows.left() - 1.0 || x > g.rows.right() + 1.0 {
        return;
    }
    let top = g.ruler.bottom() - 12.0;
    let line = painter.with_clip_rect(Rect::from_min_max(pos2(g.rows.left(), g.ruler.top()), g.rows.max).intersect(painter.clip_rect()));
    line.vline(x, (top + 10.0)..=g.rows.bottom(), (1.5, PLAYHEAD));
    fill_polygon(
        &line,
        &[pos2(x - 6.0, top), pos2(x + 6.0, top), pos2(x + 6.0, top + 6.0), pos2(x, top + 12.0), pos2(x - 6.0, top + 6.0)],
        PLAYHEAD,
    );
}

/// The muted words in the middle of the rows when nothing is on them.
fn paint_empty_hint(painter: &Painter, timeline: &Timeline, g: &Geo) {
    if timeline.empty_hint.is_empty() {
        return;
    }
    let empty = timeline.rows.iter().all(|row| match &row.content {
        RowContent::Items { items } => items.is_empty(),
        RowContent::Cells { marks, spans } => marks.is_empty() && spans.is_empty(),
    });
    if empty {
        painter.text(g.rows.center(), Align2::CENTER_CENTER, &timeline.empty_hint, widgets::body(Scale(1.0)), widgets::MUTED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{Ruler, ToolItem};

    const AREA: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1000.0, y: 320.0 } };

    fn sample() -> Timeline {
        let row = |id: &str, name: &str, content: RowContent| TimelineRow {
            id: id.into(),
            name: name.into(),
            color: [120, 160, 230],
            icon: None,
            indent: 0,
            open: None,
            disclose: None,
            selected: false,
            toggles: Vec::new(),
            read_only: false,
            menu: Vec::new(),
            drop: DropRule::Replace,
            content,
        };
        let item = TimelineItem {
            id: "a".into(),
            start: 10,
            length: 10,
            fill: [210, 225, 250],
            border: None,
            style: ItemStyle::Solid,
            label: "First".into(),
            icon: None,
            selected: false,
            thumbnail: None,
            waveform: Vec::new(),
            progress: None,
            end_mark: None,
            tip: String::new(),
            menu: Vec::new(),
            open: Some(Action::new("open")),
            movable: true,
        };
        Timeline {
            tab: "t1".into(),
            header: vec![ToolItem::Button {
                icon: "play".into(),
                tip: "Play".into(),
                enabled: true,
                selected: false,
                action: Some(Action::new("play")),
                menu: Vec::new(),
            }],
            pages: Vec::new(),
            back: None,
            ruler: Ruler { fps: 24.0, length: 48, unit: RulerUnit::Frames },
            rows: vec![
                row("r1", "Row one", RowContent::Items { items: vec![item] }),
                row(
                    "r2",
                    "Row two",
                    RowContent::Cells {
                        marks: vec![CellMark { frame: 0, kind: MarkKind::Key, selected: false }],
                        spans: vec![CellSpan { start: 0, length: 6, kind: SpanKind::Hold }],
                    },
                ),
            ],
            playhead: 0,
            ranges: Vec::new(),
            add_row: None,
            empty_hint: String::new(),
        }
    }

    /// Drives `show` headlessly, one frame per call.
    struct Driver {
        ctx: egui::Context,
        timeline: Timeline,
        locked: bool,
    }

    impl Driver {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::toolbar::install_fonts(&ctx);
            let mut driver = Self { ctx, timeline: sample(), locked: false };
            driver.frame(Vec::new());
            driver
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<Out> {
            let mut outs = Vec::new();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 400.0))),
                events,
                ..Default::default()
            };
            let (timeline, locked) = (&self.timeline, self.locked);
            let mut output = self.ctx.run_ui(input, |ui| outs = show(ui, timeline, AREA, locked));
            output.textures_delta.clear();
            outs
        }

        /// Where a frame lies on the timeline, and a row's middle.
        fn place(&self, frame: f32, row: usize) -> Pos2 {
            let mut view: View = self.ctx.data(|data| data.get_temp(Id::new(("timeline", "t1")))).unwrap_or_default();
            let g = geo(&self.timeline, AREA, AREA.top() + HEAD_H, &mut view);
            let (top, bottom) = g.tops[row];
            pos2(g.x(frame), (top + bottom) / 2.0)
        }

        fn ruler_at(&self, frame: f32) -> Pos2 {
            let mut view = View::default();
            let g = geo(&self.timeline, AREA, AREA.top() + HEAD_H, &mut view);
            pos2(g.x(frame), g.ruler.center().y)
        }

        fn button(&mut self, at: Pos2, pressed: bool) -> Vec<Out> {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
            ])
        }

        /// A press at `from`, moves to `to`, and a release there: every
        /// frame's outputs, in order.
        fn drag(&mut self, from: Pos2, to: Pos2) -> Vec<Out> {
            let mut outs = self.button(from, true);
            for step in 1..=4 {
                let at = from + (to - from) * (step as f32 / 4.0);
                outs.extend(self.frame(vec![egui::Event::PointerMoved(at)]));
            }
            outs.extend(self.button(to, false));
            outs
        }

        fn click(&mut self, at: Pos2) -> Vec<Out> {
            let mut outs = self.button(at, true);
            outs.extend(self.button(at, false));
            outs
        }
    }

    fn gestures(outs: &[Out]) -> Vec<&TimelineGesture> {
        outs.iter()
            .filter_map(|out| match out {
                Out::Gesture(gesture) => Some(gesture),
                _ => None,
            })
            .collect()
    }

    fn item_pick() -> TimelinePick {
        TimelinePick { row: "r1".into(), item: Some("a".into()), frame: None }
    }

    /// Catches a click on an item that picks nothing.
    #[test]
    fn a_click_on_an_item_selects_it() {
        let mut driver = Driver::new();
        let outs = driver.click(driver.place(15.0, 0));
        assert_eq!(
            gestures(&outs),
            vec![&TimelineGesture::Select { picks: vec![item_pick()], add: false, toggle: false }]
        );
    }

    /// Catches a drag that moves by the wrong number of frames, or sends a
    /// message on every frame of the drag.
    #[test]
    fn dragging_an_item_moves_it_by_whole_frames_once() {
        let mut driver = Driver::new();
        let outs = driver.drag(driver.place(15.0, 0), driver.place(20.2, 0));
        assert_eq!(
            gestures(&outs),
            vec![&TimelineGesture::Move { picks: vec![item_pick()], frames: 5, row: None, push: false }]
        );
    }

    /// Catches an edge drag taken for a move.
    #[test]
    fn dragging_an_edge_resizes_the_item() {
        let mut driver = Driver::new();
        let edge = driver.place(20.0, 0) - vec2(2.0, 0.0);
        let outs = driver.drag(edge, edge + vec2(3.0 * (driver.place(1.0, 0).x - driver.place(0.0, 0).x), 0.0));
        assert_eq!(
            gestures(&outs),
            vec![&TimelineGesture::Resize { pick: item_pick(), edge: Edge::End, frames: 3 }]
        );
    }

    /// Catches a playhead that seeks only on release, or never lets go.
    #[test]
    fn dragging_the_playhead_seeks_live_then_once_more() {
        let mut driver = Driver::new();
        let outs = driver.drag(driver.ruler_at(20.5), driver.ruler_at(30.5));
        let seeks: Vec<(u32, bool)> = outs
            .iter()
            .filter_map(|out| match out {
                Out::Seek { frame, live } => Some((*frame, *live)),
                _ => None,
            })
            .collect();
        assert_eq!(seeks.first(), Some(&(20, true)));
        assert!(seeks[..seeks.len() - 1].iter().all(|(_, live)| *live), "{seeks:?}");
        assert_eq!(seeks.last(), Some(&(30, false)));
        assert!(seeks.iter().any(|(frame, live)| *live && *frame > 20 && *frame <= 30), "{seeks:?}");
        assert!(gestures(&outs).is_empty());
    }

    /// Catches a header that draws its items but sends nothing.
    #[test]
    fn a_header_button_runs_its_action() {
        let mut driver = Driver::new();
        let at = pos2(12.0 + crate::toolbar::BUTTON_W / 2.0, HEAD_H / 2.0);
        let outs = driver.click(at);
        assert_eq!(outs, vec![Out::Run(Action::new("play"))]);
    }

    /// Catches a double-click that does not open the item.
    #[test]
    fn a_double_click_opens_the_item() {
        let mut driver = Driver::new();
        let at = driver.place(15.0, 0);
        let mut outs = driver.click(at);
        outs.extend(driver.click(at));
        assert!(outs.contains(&Out::Run(Action::new("open"))), "{outs:?}");
    }

    /// Catches a drag over empty cells that chooses nothing, or only one row.
    #[test]
    fn dragging_over_empty_cells_across_rows_chooses_a_block() {
        let mut driver = Driver::new();
        let outs = driver.drag(driver.place(30.5, 1), driver.place(34.5, 0));
        assert_eq!(
            gestures(&outs),
            vec![&TimelineGesture::SelectCells { rows: vec!["r1".into(), "r2".into()], start: 30, end: 34 }]
        );
    }

    /// Catches a timeline that acts while the assistant works.
    #[test]
    fn a_locked_timeline_sends_nothing() {
        let mut driver = Driver::new();
        driver.locked = true;
        let mut outs = driver.click(driver.place(15.0, 0));
        outs.extend(driver.drag(driver.ruler_at(20.5), driver.ruler_at(30.5)));
        outs.extend(driver.click(pos2(12.0 + crate::toolbar::BUTTON_W / 2.0, HEAD_H / 2.0)));
        assert!(outs.is_empty(), "{outs:?}");
    }

    /// Catches tooltips that read wrong for the ruler's unit.
    #[test]
    fn resize_tips_read_in_the_ruler_unit() {
        assert_eq!(resize_text(RulerUnit::Frames, 24.0, 10, 12, false), "+12 f");
        assert_eq!(resize_text(RulerUnit::Timecode, 24.0, 64, 12, false), "3 s 4 f (+12 f)");
        assert_eq!(resize_text(RulerUnit::Frames, 24.0, 8, 8, true), "\u{d7}2");
        assert_eq!(timecode(24 * 61 + 5, 24.0), "00:01:01:05");
    }
}
