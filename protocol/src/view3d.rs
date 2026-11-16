//! A 3D view tab, as the scene contract's section 4 sets out: a workspace
//! names a scene file, its look, the camera, and a published render to show
//! with it, and the host draws the scene itself. Geometry never crosses the
//! protocol. Positions are in the scene's metres, z up, and a field of view
//! is horizontal, in degrees, as the pre-vis builders keep their cameras.

use serde::{Deserialize, Serialize};

/// How the scene's faces are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View3dLook {
    /// A flat grey per kind.
    #[default]
    Grey,
    /// A flat colour per kind.
    Coded,
    /// White faces and grey edges, keeping only the shell, the counter, the
    /// front door, and the night.
    Simple,
}

/// Which camera a 3D view looks through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraChoice {
    /// The free camera, which the user turns: from `start` when given,
    /// otherwise from where the host left it or from the scene's first
    /// camera. The host reports where the user leaves it with
    /// `view3d_camera`.
    Free {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<CameraPose>,
    },
    /// One of the scene's cameras, by its node's name, letterboxed to its
    /// image size.
    Named {
        #[serde(default)]
        name: String,
    },
}

impl Default for CameraChoice {
    fn default() -> Self {
        Self::Free { start: None }
    }
}

/// Where a camera stands, what it looks at, and its horizontal field of
/// view, in degrees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraPose {
    #[serde(default)]
    pub position: [f64; 3],
    #[serde(default)]
    pub target: [f64; 3],
    #[serde(default)]
    pub fov: f64,
}

/// A published render shown with the view, such as the camera's
/// `reference_file`, which the workspace decoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View3dReference {
    /// The render as a file of `width × height × 4` bytes, RGBA with
    /// straight alpha, rows top to bottom, replaced whole by a write to a
    /// scratch file and a rename, as `RasterView::image` is.
    #[serde(default)]
    pub image: String,
    /// Its size, in pixels.
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub show: ReferenceShow,
    /// Shown beside, the words under each frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<View3dCaption>,
}

/// How a render is shown with the view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceShow {
    /// Side by side, each in its own frame, the view on the left.
    #[default]
    Beside,
    /// Laid over the view, at this opacity from 0 to 1.
    Over {
        #[serde(default = "half")]
        opacity: f64,
    },
}

fn half() -> f64 {
    0.5
}

/// The words under the view's frame and under the render's, such as
/// "3D view" and "Pre-vis render · 10 Nov". Either may be left empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct View3dCaption {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub view: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
}

/// What a drag does in a 3D view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View3dTool {
    /// Turns the free camera around what it looks at.
    #[default]
    Orbit,
    /// Moves the free camera sideways.
    Pan,
}

impl View3dTool {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
