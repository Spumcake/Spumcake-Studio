//! The toolbar's tools and the notes tool, as the Image Generator's: a Pin
//! click on the frame pauses, then puts a pin numbered after the highest at
//! the time shown, or opens the pin there, whose words are typed through
//! the host's `raster_text`; a Pin drag from a pin moves it; Draw makes a
//! freehand mark at the time shown, part by part. Each new pin with its
//! words, each mark, a pin moved, and a pin taken away is one step of the
//! gallery's history. A note shows over the frame only within half a
//! second of its time, `layers::notes::NEAR`, and each pin is a cue on the
//! transport row.
//!
//! Notes belong to a take's file, by its path and sha256, the gallery's
//! takes and a round's alike, and are kept in the `.vgx` as `take_notes`.

use super::*;
use crate::notes::{MARK_WIDTH, NotesOver};
use layers::notes::{Mark, Notes, Pin};
use protocol::Cue;
use protocol::{TextEditing, TextInput, TextKey};

/// The toolbar's tools, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// A drag pans a zoomed view.
    #[default]
    Move,
    Hand,
    Zoom,
    /// Notes after the magnifier, as the Image Generator's, as
    /// vg-notes-at-time shows.
    Notes,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Move, Tool::Hand, Tool::Zoom, Tool::Notes];

    pub fn key(self) -> &'static str {
        match self {
            Tool::Move => "move",
            Tool::Hand => "hand",
            Tool::Zoom => "zoom",
            Tool::Notes => "notes",
        }
    }

    pub fn named(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.key() == name)
    }
}

/// The tool chosen, and the notes tool's settings row: Pin or Draw, and
/// the marks' colour.
#[derive(Debug, Clone)]
pub struct Tools {
    pub tool: Tool,
    pub draw: bool,
    pub rgb: [u8; 3],
}

impl Default for Tools {
    fn default() -> Self {
        let [r, g, b, _] = layers::notes::RED;
        Self { tool: Tool::Move, draw: false, rgb: [r, g, b] }
    }
}

/// A pin's words being typed: its tab, its take's file, the take's notes
/// as they are being typed and as they were, which become one step when
/// the typing ends.
#[derive(Debug, Clone)]
pub(super) struct Typing {
    tab: String,
    path: String,
    sha256: String,
    number: u32,
    notes: Notes,
    before: Notes,
}

/// A mark being drawn, part by part, on its tab's take, and how many of
/// the stroke's points have come.
#[derive(Debug, Clone)]
pub(super) struct Marking {
    tab: String,
    path: String,
    sha256: String,
    over: NotesOver,
    mark: Mark,
    came: usize,
}

/// The take the notes tool works on: the take shown, alone or first of
/// two side by side, at the time shown; never frame strips.
struct Target {
    tab: String,
    path: String,
    sha256: String,
    time: f64,
    over: NotesOver,
}

impl Session {
    /// The take shown's notes, where its frame is drawn, and the time shown.
    fn notes_target(&self) -> Option<Target> {
        let tab = self.tabs.get(self.active)?;
        let shown = tab.round_view();
        let gallery = shown.as_ref().unwrap_or(&tab.gallery);
        if gallery.view() == Mode::Strips {
            return None;
        }
        let take = gallery.take()?;
        let info = take.info?;
        let scene = view::scene(gallery, self.image_size(), tab.look());
        let place = scene.places.first()?;
        let (path, sha256) = (take.link.path.clone(), take.link.sha256.clone());
        let notes = match &self.typing {
            Some(typing) if typing.tab == tab.id && typing.path == path && typing.sha256 == sha256 => typing.notes.clone(),
            _ => tab.gallery.notes_of(&path, &sha256),
        };
        let open = self.typing.as_ref().filter(|typing| typing.tab == tab.id && typing.path == path && typing.sha256 == sha256).map(|typing| typing.number);
        Some(Target {
            tab: tab.id.clone(),
            path,
            sha256,
            time: view::seconds_at(take, tab.frame_shown()),
            over: NotesOver { notes, size: (info.width, info.height), rect: place.rect, clip: place.clip, open },
        })
    }

    /// The notes drawn over the take shown: those within half a second of
    /// the time shown, the pin being typed in, and the mark being drawn.
    pub(super) fn notes_over(&self) -> Option<NotesOver> {
        let target = self.notes_target()?;
        let mut over = target.over;
        let mut shown = over.notes.at_time(target.time);
        if let Some(open) = over.open
            && !shown.pins.iter().any(|pin| pin.number == open)
            && let Some(pin) = over.notes.pins.iter().find(|pin| pin.number == open)
        {
            shown.pins.push(pin.clone());
        }
        if let Some(marking) = self.marking.as_ref().filter(|marking| marking.tab == target.tab && marking.path == target.path) {
            shown.marks.push(marking.mark.clone());
        }
        if shown.is_empty() {
            return None;
        }
        over.notes = shown;
        Some(over)
    }

    /// Each pin of the take shown as a cue on the transport row: its bare
    /// number, as the host writes the time beside it, in the notes' red.
    pub fn note_cues(&self) -> Vec<Cue> {
        let Some(target) = self.notes_target() else {
            return Vec::new();
        };
        let [r, g, b, _] = layers::notes::RED;
        let mut pins: Vec<&Pin> = target.over.notes.pins.iter().filter(|pin| pin.at.is_some()).collect();
        pins.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal).then(a.number.cmp(&b.number)));
        pins.iter().map(|pin| Cue { at: pin.at.unwrap_or(0.0), label: pin.number.to_string(), colour: Some([r, g, b]) }).collect()
    }

    /// The pin's words being typed, as the host draws them.
    pub(super) fn text_editing(&self) -> Option<TextEditing> {
        self.typing.as_ref()?;
        self.notes_over()?.editing()
    }

    /// The pointer as the tool chosen gives it to the host: the notes
    /// tool's Draw paints as a brush of the marks' width and colour, and
    /// its Pin clicks and drags as the text tool does.
    pub(super) fn raster_tool(&self) -> RasterTool {
        match self.tools.tool {
            Tool::Move => RasterTool::Move,
            Tool::Hand => RasterTool::Hand,
            Tool::Zoom => RasterTool::Zoom,
            Tool::Notes if self.tools.draw => RasterTool::Brush {
                rgb: self.tools.rgb,
                size: f64::from(MARK_WIDTH),
                hardness: 1.0,
                opacity: 1.0,
                pressure: false,
            },
            Tool::Notes => RasterTool::Text,
        }
    }

    /// Chooses a tool; the words being typed end first.
    pub(super) fn choose_tool(&mut self, tool: Tool) {
        if self.tools.tool != tool {
            self.end_typing();
            self.marking = None;
            self.tools.tool = tool;
        }
    }

    /// A click with the notes tool's Pin: while a pin's words are typed
    /// in, it ends that; on a pin shown, its words are typed in; elsewhere
    /// on the frame a new pin is put there at the time shown, numbered
    /// after the highest, and its words are typed in. Playing pauses first.
    pub(super) fn note_click(&mut self, at: [f64; 2]) {
        self.stop_playing();
        if self.typing.is_some() {
            self.end_typing();
            return;
        }
        let Some(target) = self.notes_target() else {
            return;
        };
        let Some([x, y]) = target.over.to_take(at) else {
            return;
        };
        let before = target.over.notes.clone();
        let mut notes = before.clone();
        let reach = target.over.radius() * 1.2;
        let number = match notes.at_time(target.time).pin_at(x, y, reach) {
            Some(number) => number,
            None => {
                let number = notes.next_number();
                notes.pins.push(Pin { number, x, y, text: String::new(), at: Some(target.time) });
                number
            }
        };
        self.typing = Some(Typing { tab: target.tab, path: target.path, sha256: target.sha256, number, notes, before });
    }

    /// A drag with the notes tool's Pin: from a pin shown, it moves the
    /// pin there, one step; from elsewhere, it is a click where it began.
    pub(super) fn note_drag(&mut self, from: [f64; 2], to: [f64; 2]) {
        self.stop_playing();
        self.end_typing();
        let Some(target) = self.notes_target() else {
            return;
        };
        let reach = target.over.radius() * 1.2;
        let pin = target.over.to_take(from).and_then(|[x, y]| target.over.notes.at_time(target.time).pin_at(x, y, reach));
        let (Some(number), Some([x, y])) = (pin, target.over.to_take(to)) else {
            self.note_click(from);
            return;
        };
        let mut notes = target.over.notes.clone();
        if let Some(pin) = notes.pins.iter_mut().find(|pin| pin.number == number) {
            pin.x = x;
            pin.y = y;
        }
        self.set_take_notes(&target.tab, &target.path, &target.sha256, notes);
    }

    /// What the keyboard does to a pin's words: typed words go after them,
    /// Backspace takes off the last letter, and in a pin with no words
    /// Backspace or Delete takes the pin away; Enter and Escape end the
    /// typing. The caret stays after the last letter.
    pub(super) fn note_text(&mut self, input: &TextInput) {
        let Some(typing) = self.typing.as_mut() else {
            return;
        };
        let number = typing.number;
        let Some(pin) = typing.notes.pins.iter_mut().find(|pin| pin.number == number) else {
            return;
        };
        match input {
            TextInput::Insert { text } => pin.text.push_str(text),
            TextInput::Key { key: TextKey::Backspace | TextKey::Delete, .. } if pin.text.is_empty() => {
                typing.notes.pins.retain(|pin| pin.number != number);
                self.end_typing();
            }
            TextInput::Key { key: TextKey::Backspace, .. } => {
                pin.text.pop();
            }
            TextInput::Key { key: TextKey::Enter | TextKey::Escape, .. } => {
                self.end_typing();
            }
            _ => {}
        }
    }

    /// Ends typing a pin's words: the pin and its words become one step.
    /// Returns whether there was typing.
    pub(super) fn end_typing(&mut self) -> bool {
        let Some(typing) = self.typing.take() else {
            return false;
        };
        if typing.notes != typing.before {
            self.set_take_notes(&typing.tab, &typing.path, &typing.sha256, typing.notes);
        }
        true
    }

    /// The newest points of a mark being drawn with the notes tool's Draw,
    /// drawn at once with no step: `from` is how many points came before.
    /// The first part pauses and starts the mark at the time shown.
    pub(super) fn mark_part(&mut self, from: usize, points: &[[f64; 3]]) {
        if from == 0 {
            self.stop_playing();
            self.end_typing();
            self.marking = None;
            let Some(target) = self.notes_target() else {
                return;
            };
            let [r, g, b] = self.tools.rgb;
            let width = MARK_WIDTH / target.over.scale().max(0.01);
            let mark = Mark { points: Vec::new(), width, colour: [r, g, b, 255], at: Some(target.time) };
            self.marking = Some(Marking { tab: target.tab, path: target.path, sha256: target.sha256, over: target.over, mark, came: 0 });
        }
        let Some(marking) = self.marking.as_mut() else {
            return;
        };
        if marking.came != from {
            return;
        }
        marking.came += points.len();
        let over = &marking.over;
        marking.mark.points.extend(points.iter().filter_map(|point| take_point(over, point)));
    }

    /// A finished mark, one step. The mark drawn as it came becomes the
    /// step as it is.
    pub(super) fn mark_done(&mut self, points: &[[f64; 3]]) {
        let marking = match self.marking.take() {
            Some(marking) => marking,
            None => {
                self.mark_part(0, points);
                match self.marking.take() {
                    Some(marking) => marking,
                    None => return,
                }
            }
        };
        if marking.mark.points.is_empty() {
            return;
        }
        let mut notes = self.tab_notes(&marking.tab, &marking.path, &marking.sha256);
        notes.marks.push(marking.mark);
        self.set_take_notes(&marking.tab, &marking.path, &marking.sha256, notes);
    }

    /// The mark being drawn, let go with Escape.
    pub(super) fn mark_cancel(&mut self) -> bool {
        self.marking.take().is_some()
    }

    fn tab_notes(&self, tab: &str, path: &str, sha256: &str) -> Notes {
        self.tab_index(tab).map(|index| self.tabs[index].gallery.notes_of(path, sha256)).unwrap_or_default()
    }

    /// A take's notes replaced, one step of its tab's gallery, which marks
    /// the tab changed.
    fn set_take_notes(&mut self, tab: &str, path: &str, sha256: &str, notes: Notes) {
        let Some(index) = self.tab_index(tab) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if tab.gallery.set_notes(path, sha256, notes) {
            tab.dirty = true;
        }
    }
}

/// A point of a mark, in the view, as a point of the take; none off the
/// frame shown.
fn take_point(over: &NotesOver, point: &[f64; 3]) -> Option<[f32; 2]> {
    over.to_take([point[0], point[1]])
}
