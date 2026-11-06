//! A tab of pixels: the image a sub-application composites, what the
//! host's pointer does on it, and the gestures the host sends back. The host
//! draws the image, its selection, crop box, and layer bounds, previews a
//! brush stroke while it is drawn, and owns the camera. The sub-application
//! owns every pixel, layer, and step of history.

use crate::DocumentCamera;
use serde::{Deserialize, Serialize};

/// What the host draws in a tab whose contents are `raster`. Sent after
/// every change; the host reads the image again when `revision` changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterView {
    pub tab: String,
    /// The image's size, in pixels.
    pub width: u32,
    pub height: u32,
    /// A text being typed in, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextEditing>,
    /// The composited image: a file of `width × height × 4` bytes, RGBA
    /// with straight (unpremultiplied) alpha, rows top to bottom. It is
    /// replaced whole, by a write to a scratch file and a rename, so the
    /// host never reads half of one.
    pub image: String,
    /// Rises with every change to the image.
    pub revision: u64,
    /// The part of the image that differs from revision `since` of the
    /// same file. A host that still shows `since` reads only these rows;
    /// any other host reads the whole image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<RasterPatch>,
    pub tool: RasterTool,
    /// The selection, as closed outlines in image pixels, drawn with a
    /// marching edge. Empty when nothing is selected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selection: Vec<Vec<[f64; 2]>>,
    /// The crop box being set, `[x, y, width, height]` in image pixels:
    /// drawn with thirds and its outside dimmed. Its edges and corners drag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    /// The active layer's box, `[x, y, width, height]`, drawn with handles
    /// while the move tool is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
    /// Where the host's camera goes, when a image is opened or made. The
    /// host owns the camera after that and reports it with `camera`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<DocumentCamera>,
    /// The tab's images, as a filmstrip along the bottom of the working
    /// area, as the owner's mockup and 0.11's gallery views show. None draws
    /// no strip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub film: Option<Film>,
}

/// What changed since an earlier revision: `[x, y, width, height]` in
/// image pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterPatch {
    pub since: u64,
    pub rect: [u32; 4],
    /// While a change is still being made, such as a stroke being drawn,
    /// its rows are in a file of their own, `width × height × 4` bytes as
    /// the image's are, and the image file is not written for this
    /// revision. A host that does not hold `since` keeps what it shows
    /// until a view comes without a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// A gallery's filmstrip: every image in the tab, the one shown outlined
/// in blue, and a bordered + tile at the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Film {
    pub items: Vec<FilmItem>,
    /// The index of the image shown.
    pub shown: usize,
    /// A click on a thumbnail sends `select` with `{"value": index}`.
    pub select: crate::Action,
    /// A thumbnail dragged along the strip sends `reorder` with
    /// `{"from": index, "to": index}` on release, `to` counted after the
    /// dragged one is taken out.
    pub reorder: crate::Action,
    /// The + tile's menu: a click opens these rows above it.
    pub add: Vec<crate::MenuEntry>,
    /// A press anywhere on the strip sends `focus` first, so the
    /// application knows the strip, not the image, has the keyboard.
    pub focus: crate::Action,
}

/// One image on the filmstrip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilmItem {
    /// Shown when the pointer rests on the thumbnail.
    pub name: String,
    pub thumbnail: Thumbnail,
    /// The thumbnail's right-click menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<crate::MenuEntry>,
}

/// What the host's pointer does on a raster tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RasterTool {
    /// A drag moves the active layer: `drag`.
    Move,
    /// A drag makes a rectangle: `rect`.
    Marquee,
    /// A drag draws a freehand outline: `lasso`.
    Lasso,
    /// A drag makes or changes the crop box: `crop`.
    Crop,
    /// A click picks a colour: `pick`.
    Eyedropper,
    /// A pressure stroke, previewed as it is drawn: `stroke`. Sizes are in
    /// image pixels; hardness and opacity run from 0 to 1.
    Brush {
        rgb: [u8; 3],
        size: f64,
        hardness: f64,
        opacity: f64,
        /// Pen pressure scales the size.
        pressure: bool,
    },
    /// As the brush, taking colour away: `stroke`.
    Erase {
        size: f64,
        hardness: f64,
        opacity: f64,
        pressure: bool,
    },
    /// A brush that copies from a source, such as a clone stamp or a
    /// healing brush. Alt-click sends `source`; a drag paints as the brush
    /// does. The host draws a crosshair where it copies from: at the
    /// pointer plus `offset` when there is one, as during and after an
    /// aligned stroke, or else at `source`. Sizes are in image pixels.
    Sampler {
        size: f64,
        pressure: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<[f64; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<[f64; 2]>,
    },
    /// A click sends `click`; a drag sends `rect`, a box to wrap in.
    Text,
    /// A drag pans the camera. Nothing is sent.
    Hand,
    /// A click zooms in at the pointer, with Alt out. Nothing is sent.
    Zoom,
}

/// One finished gesture on a raster tab, in image pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RasterGesture {
    /// The newest points of a brush or eraser stroke still being drawn,
    /// sent as they come, so the sub-application paints them at once:
    /// `from` is how many points were sent before. The host draws no
    /// preview of its own; the image shows the stroke.
    StrokePart { from: usize, points: Vec<[f64; 3]> },
    /// The open stroke was let go with Escape: it is taken off the image.
    StrokeCancel,
    /// A finished brush or eraser stroke, every point: `[x, y, pressure]`,
    /// pressure from 0 to 1, and 1 with a mouse.
    Stroke { points: Vec<[f64; 3]> },
    /// A rectangle, corner to corner.
    Rect { from: [f64; 2], to: [f64; 2] },
    /// A freehand outline, closed by its last point.
    Lasso { points: Vec<[f64; 2]> },
    /// A move, from where the press was to where it was released.
    Drag { from: [f64; 2], to: [f64; 2] },
    /// A click, for the eyedropper.
    Pick { at: [f64; 2] },
    /// Alt-click with a tool that copies from a source.
    Source { at: [f64; 2] },
    /// A click with the text tool.
    Click { at: [f64; 2] },
    /// The move tool's drag while it is open: where the press was, and
    /// where the pointer is now. The finished `drag` follows on release.
    DragPart { from: [f64; 2], to: [f64; 2] },
    /// The edited text's box after a side handle was dragged:
    /// `[x, y, width, height]`.
    TextBox { rect: [f64; 4] },
    /// The crop box after a drag made or changed it: `[x, y, width, height]`.
    Crop { rect: [f64; 4] },
}

/// A text being typed in: its box, its caret, and its selected spans, as
/// `[x, y, width, height]` in image pixels. While it is set, the host
/// draws the box with side handles and a blinking caret, and sends the
/// keyboard as `raster_text`, except undo and redo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextEditing {
    pub rect: [f64; 4],
    pub caret: [f64; 4],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selection: Vec<[f64; 4]>,
}

/// What the keyboard did to a text being typed in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextInput {
    /// Typed or pasted words.
    Insert { text: String },
    /// A press inside the box: the caret goes there, or, with `extend`,
    /// the selection runs there.
    Place { at: [f64; 2], extend: bool },
    /// A drag inside the box, after its press: the selection runs to `at`.
    Select { at: [f64; 2] },
    /// A double-click inside the box: the word there is selected.
    Word { at: [f64; 2] },
    /// A key, with Shift held to select, and `word` with Ctrl (Alt on a
    /// Mac) held to move by words.
    Key { key: TextKey, shift: bool, word: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKey {
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Enter,
    Escape,
    SelectAll,
    /// The selection is answered as `clipboard`; Cut also removes it.
    Copy,
    Cut,
}

/// A small image of one layer, for its row: a file of `width × height × 4`
/// bytes, as `RasterView::image` is. The host reads it again when
/// `revision` changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every raster message survives a round trip, so a field cannot be
    /// dropped quietly.
    #[test]
    fn raster_messages_round_trip() {
        let view = RasterView {
            tab: "1".into(),
            width: 1920,
            height: 1080,
            image: "/tmp/1.rgba".into(),
            revision: 3,
            text: Some(TextEditing { rect: [1.0, 2.0, 30.0, 10.0], caret: [5.0, 2.0, 1.0, 10.0], selection: vec![[1.0, 2.0, 4.0, 10.0]] }),
            patch: Some(RasterPatch { since: 2, rect: [1, 2, 3, 4], file: Some("/tmp/1-2.patch".into()) }),
            tool: RasterTool::Brush { rgb: [20, 20, 22], size: 24.0, hardness: 0.8, opacity: 1.0, pressure: true },
            selection: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]],
            crop: Some([1.0, 2.0, 3.0, 4.0]),
            bounds: Some([0.0, 0.0, 1920.0, 1080.0]),
            camera: Some(DocumentCamera { origin: [0.0, 0.0], zoom: 0.5, rotation: 0.0, flip: false }),
            film: Some(Film {
                items: vec![FilmItem {
                    name: "Beach".into(),
                    thumbnail: Thumbnail { path: "/tmp/t.rgba".into(), width: 68, height: 44, revision: 1 },
                    menu: Vec::new(),
                }],
                shown: 0,
                select: crate::Action::new("select_image"),
                reorder: crate::Action::new("move_image"),
                add: Vec::new(),
                focus: crate::Action::new("film_focus"),
            }),
        };
        let text = serde_json::to_string(&crate::AppMessage::Raster(view.clone())).unwrap();
        assert_eq!(crate::parse_app_message(&text).unwrap(), crate::AppMessage::Raster(view));
        for gesture in [
            RasterGesture::StrokePart { from: 3, points: vec![[1.0, 2.0, 0.5]] },
            RasterGesture::Source { at: [3.0, 4.0] },
            RasterGesture::Click { at: [3.0, 4.0] },
            RasterGesture::DragPart { from: [0.0, 0.0], to: [2.0, 3.0] },
            RasterGesture::TextBox { rect: [1.0, 2.0, 30.0, 10.0] },
            RasterGesture::StrokeCancel,
            RasterGesture::Stroke { points: vec![[1.0, 2.0, 0.5]] },
            RasterGesture::Rect { from: [0.0, 0.0], to: [5.0, 5.0] },
            RasterGesture::Lasso { points: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]] },
            RasterGesture::Drag { from: [0.0, 0.0], to: [5.0, 5.0] },
            RasterGesture::Pick { at: [3.0, 4.0] },
            RasterGesture::Crop { rect: [0.0, 0.0, 5.0, 5.0] },
        ] {
            let message = crate::HostMessage::RasterGesture { tab: "1".into(), gesture };
            let text = serde_json::to_string(&message).unwrap();
            assert_eq!(serde_json::from_str::<crate::HostMessage>(&text).unwrap(), message);
        }
    }
}
