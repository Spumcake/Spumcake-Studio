//! Icon names, shortcut chords, and the face names the toolbar installs.

use eframe::egui;

/// How an icon name is drawn from the Iconoir font. A name ending in
/// `-solid` is the icon's filled style, as Iconoir names it on its site. A
/// name in [`TURNED_ICONS`] is another icon turned by quarter turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IconSpec<'a> {
    pub glyph: &'a str,
    pub filled: bool,
    pub quarter_turns: u8,
}

/// Icons Iconoir has only in one direction: the name, the icon it turns, and
/// the clockwise quarter turns.
const TURNED_ICONS: &[(&str, &str, u8)] = &[("flip-vertical", "flip", 1)];

/// Icons the Iconoir pack does not have, drawn by the toolbar in Iconoir's
/// style instead of taken from the font.
pub(crate) const DRAWN_ICONS: &[&str] = &["brush", "bandage", "stamp"];

pub(crate) fn icon_spec(name: &str) -> IconSpec<'_> {
    if let Some((_, glyph, turns)) = TURNED_ICONS.iter().find(|(alias, _, _)| *alias == name) {
        return IconSpec {
            glyph,
            filled: false,
            quarter_turns: *turns,
        };
    }
    match name.strip_suffix("-solid") {
        Some(glyph) => IconSpec {
            glyph,
            filled: true,
            quarter_turns: 0,
        },
        None => IconSpec {
            glyph: name,
            filled: false,
            quarter_turns: 0,
        },
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    B,
    C,
    E,
    G,
    L,
    N,
    O,
    R,
    S,
    T,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit2,
    Plus,
    Equals,
    Minus,
    BracketLeft,
    BracketRight,
    F4,
    Delete,
    Backspace,
    Left,
    Right,
    Up,
    Down,
}

impl Key {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Key::B => "B",
            Key::C => "C",
            Key::E => "E",
            Key::G => "G",
            Key::L => "L",
            Key::N => "N",
            Key::O => "O",
            Key::R => "R",
            Key::S => "S",
            Key::T => "T",
            Key::V => "V",
            Key::W => "W",
            Key::X => "X",
            Key::Y => "Y",
            Key::Z => "Z",
            Key::Digit0 => "0",
            Key::Digit2 => "2",
            Key::Plus | Key::Equals => "+",
            Key::Minus => "-",
            Key::BracketLeft => "[",
            Key::BracketRight => "]",
            Key::F4 => "F4",
            Key::Delete => "Delete",
            Key::Backspace => "Backspace",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Up => "Up",
            Key::Down => "Down",
        }
    }
}

/// `shift: None` means the shortcut ignores Shift. Ctrl++ is Shift+= on a US keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Chord {
    pub command: bool,
    pub shift: Option<bool>,
    pub alt: bool,
    pub key: Key,
}

impl Chord {
    const fn plain(key: Key) -> Self {
        Self {
            command: false,
            shift: Some(false),
            alt: false,
            key,
        }
    }

    #[allow(dead_code)]
    const fn ctrl(key: Key) -> Self {
        Self {
            command: true,
            shift: Some(false),
            alt: false,
            key,
        }
    }

    #[allow(dead_code)]
    const fn ctrl_shift(key: Key) -> Self {
        Self {
            command: true,
            shift: Some(true),
            alt: false,
            key,
        }
    }

    /// Command plus this key, whether or not Shift is held.
    #[allow(dead_code)]
    const fn ctrl_any_shift(key: Key) -> Self {
        Self {
            command: true,
            shift: None,
            alt: false,
            key,
        }
    }

    #[allow(dead_code)]
    const fn shift(key: Key) -> Self {
        Self {
            command: false,
            shift: Some(true),
            alt: false,
            key,
        }
    }

    pub(crate) fn label(self) -> String {
        let mut parts = Vec::new();
        if self.command {
            parts.push("Ctrl");
        }
        if self.shift == Some(true) {
            parts.push("Shift");
        }
        parts.push(self.key.label());
        parts.join("+")
    }
}

pub(crate) fn hover(label: &str, shortcuts: &[Chord]) -> String {
    match shortcuts.first() {
        Some(chord) => format!("{label} ({})", chord.label()),
        None => label.to_string(),
    }
}

pub(crate) const INTER: &str = "Inter";
pub(crate) const INTER_BOLD: &str = "Inter Bold";
pub(crate) const INTER_ITALIC: &str = "Inter Italic";
pub(crate) const INTER_BOLD_ITALIC: &str = "Inter Bold Italic";

/// The panel shortcut. The edge tab's hover is "Properties (F4)".
const PANEL_LABEL: &str = "Properties";
const PANEL: &[Chord] = &[Chord::plain(Key::F4)];

pub(crate) fn panel_tip() -> String {
    hover(PANEL_LABEL, PANEL)
}

/// The moved panel chord: plain F4, so Shift, Ctrl, and Alt do not count.
pub(crate) fn panel_shortcut_pressed(input: &egui::InputState) -> bool {
    PANEL.iter().any(|chord| {
        if input.modifiers.command != chord.command || input.modifiers.alt != chord.alt {
            return false;
        }
        if let Some(shift) = chord.shift
            && input.modifiers.shift != shift
        {
            return false;
        }
        matches!(chord.key, Key::F4) && input.key_pressed(egui::Key::F4)
    })
}

/// Whether this frame pressed a declared shortcut. `shift: None` ignores Shift.
pub(crate) fn chord_pressed(input: &egui::InputState, chord: &protocol::Chord) -> bool {
    if input.modifiers.command != chord.command || input.modifiers.alt != chord.alt {
        return false;
    }
    if let Some(shift) = chord.shift
        && input.modifiers.shift != shift
    {
        return false;
    }
    key_named(&chord.key).is_some_and(|key| input.key_pressed(key))
}

/// The key a declared shortcut names.
fn key_named(name: &str) -> Option<egui::Key> {
    use egui::Key;
    Some(match name {
        "Digit0" => Key::Num0,
        "Digit1" => Key::Num1,
        "Digit2" => Key::Num2,
        "Digit3" => Key::Num3,
        "Digit4" => Key::Num4,
        "Digit5" => Key::Num5,
        "Digit6" => Key::Num6,
        "Digit7" => Key::Num7,
        "Digit8" => Key::Num8,
        "Digit9" => Key::Num9,
        "BracketLeft" => Key::OpenBracket,
        "BracketRight" => Key::CloseBracket,
        "Left" => Key::ArrowLeft,
        "Right" => Key::ArrowRight,
        "Up" => Key::ArrowUp,
        "Down" => Key::ArrowDown,
        other => return Key::from_name(other),
    })
}
