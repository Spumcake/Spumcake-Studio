//! Property ids and values stored on a [`crate::document::Step`].
//! Sections and pages stay in the Storyboard Generator.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropId {
    Preset,
    PressureSize,
    Curve,
    Smoothing,
    Stabilization,
    Streamline,
    EraserSize,
    Name,
    Hex,
    Rgb,
    Opacity,
    ColorSize,
    Dash,
    Cap,
    /// Whether a color is drawn: on every color page, and a layer's eye.
    Visible,
    Locked,
    /// Canvas padding, 0 through 128.
    Padding,
    /// Under Stroke, on a tool's color that has a size: the next mark follows
    /// the color's size until the Size slider is moved.
    LockSize,
    /// A palette as rows of its colors. Each row opens that color.
    Palette,
    CornerRadius,
    /// A shape tool's setting. Not stored on the object.
    Constrain,
    /// A shape tool's setting. Not stored on the object.
    Snap,
    Family,
    FontSize,
    Bold,
    Italic,
    Underline,
    Align,
    VAlign,
    /// Every line as a bullet, number, or checkbox item, or none.
    List,
    /// Space between a text box's edge and its words.
    TextPadding,
    /// A text box's corner radius.
    TextRadius,
    LineHeight,
    LetterSpacing,
    AllCaps,
    AutoSize,
    WrapText,
}

impl PropId {
    /// The name a property message uses. It is the declared property.
    pub fn key(self) -> &'static str {
        match self {
            PropId::Preset => "preset",
            PropId::PressureSize => "pressure_size",
            PropId::Curve => "curve",
            PropId::Smoothing => "smoothing",
            PropId::Stabilization => "stabilization",
            PropId::Streamline => "streamline",
            PropId::EraserSize => "eraser_size",
            PropId::Name => "name",
            PropId::Hex => "hex",
            PropId::Rgb => "rgb",
            PropId::Opacity => "opacity",
            PropId::ColorSize => "color_size",
            PropId::Dash => "dash",
            PropId::Cap => "cap",
            PropId::Visible => "visible",
            PropId::Locked => "locked",
            PropId::Padding => "padding",
            PropId::LockSize => "lock_size",
            PropId::Palette => "palette",
            PropId::CornerRadius => "corner_radius",
            PropId::Constrain => "constrain",
            PropId::Snap => "snap",
            PropId::Family => "family",
            PropId::FontSize => "font_size",
            PropId::Bold => "bold",
            PropId::Italic => "italic",
            PropId::Underline => "underline",
            PropId::Align => "align",
            PropId::VAlign => "valign",
            PropId::List => "list",
            PropId::TextPadding => "text_padding",
            PropId::TextRadius => "text_radius",
            PropId::LineHeight => "line_height",
            PropId::LetterSpacing => "letter_spacing",
            PropId::AllCaps => "all_caps",
            PropId::AutoSize => "auto_size",
            PropId::WrapText => "wrap_text",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "preset" => PropId::Preset,
            "pressure_size" => PropId::PressureSize,
            "curve" => PropId::Curve,
            "smoothing" => PropId::Smoothing,
            "stabilization" => PropId::Stabilization,
            "streamline" => PropId::Streamline,
            "eraser_size" => PropId::EraserSize,
            "name" => PropId::Name,
            "hex" => PropId::Hex,
            "rgb" => PropId::Rgb,
            "opacity" => PropId::Opacity,
            "color_size" => PropId::ColorSize,
            "dash" => PropId::Dash,
            "cap" => PropId::Cap,
            "visible" => PropId::Visible,
            "locked" => PropId::Locked,
            "padding" => PropId::Padding,
            "lock_size" => PropId::LockSize,
            "palette" => PropId::Palette,
            "corner_radius" => PropId::CornerRadius,
            "constrain" => PropId::Constrain,
            "snap" => PropId::Snap,
            "family" => PropId::Family,
            "font_size" => PropId::FontSize,
            "bold" => PropId::Bold,
            "italic" => PropId::Italic,
            "underline" => PropId::Underline,
            "align" => PropId::Align,
            "valign" => PropId::VAlign,
            "list" => PropId::List,
            "text_padding" => PropId::TextPadding,
            "text_radius" => PropId::TextRadius,
            "line_height" => PropId::LineHeight,
            "letter_spacing" => PropId::LetterSpacing,
            "all_caps" => PropId::AllCaps,
            "auto_size" => PropId::AutoSize,
            "wrap_text" => PropId::WrapText,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Bool(bool),
    Number(f64),
    Text(String),
    Index(u8),
    Rgb([u8; 3]),
    Curve(Vec<(f64, f64)>),
    Idle,
    /// The selected objects do not share one value.
    Mixed,
}

pub trait Inspect {
    fn read(&self, id: PropId) -> Option<PropValue>;
    /// Clamps numbers into range. Returns false when the value cannot be
    /// applied, and leaves the previous value in place.
    fn write(&mut self, id: PropId, value: &PropValue) -> bool;
}

/// A whole number clamped into `min..=max`. Non-finite input is refused.
/// The only caller is the `Inspect` implementation in the document model.
pub fn clamp_u8(number: f64, min: u8, max: u8) -> Option<u8> {
    if !number.is_finite() {
        return None;
    }
    Some(number.round().clamp(f64::from(min), f64::from(max)) as u8)
}
