//! A gesture in progress, and the value sent when it finishes.
//! It starts from the live tool settings a sub-application declares.
//! It ends as a [`Gesture`], not as a [`crate::document::Step`].

use crate::document::snapshot::{box_from_value, box_value};
use crate::document::{ColorId, CurvePoint, Dynamics, NodeId, Point, Sample, ShapeKind, TextBox};
use kurbo::Affine;
use serde_json::{Map, Value, json};

/// Why a sample was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureError {
    InvalidSample,
}

/// What is being drawn. A shape stores its [`ShapeKind`] here.
/// The Storyboard Generator maps that kind to a step when it commits.
#[derive(Clone, Debug)]
pub enum GestureKind {
    /// `canvas` is the highlighted canvas, or `None` when the commit makes a
    /// new one for the stroke. `source` is the brush color. `layer` is the
    /// canvas layer that came from it, or `None` when the commit adds that
    /// layer. `size` is the stroke's own size, or `None` to follow the layer.
    Brush {
        canvas: Option<NodeId>,
        source: ColorId,
        layer: Option<ColorId>,
        size: Option<u8>,
        dynamics: Dynamics,
    },
    Erase {
        canvas: NodeId,
        size: u8,
        dynamics: Dynamics,
    },
    Shape {
        kind: ShapeKind,
        origin: Point,
        pointer: Point,
        shift: bool,
    },
    /// A text-tool press that has not been released. `hit` is an existing text object.
    Text {
        origin: Point,
        pointer: Point,
        hit: Option<NodeId>,
    },
    /// A move, scale, or rotate of the selection: each object's local
    /// transform before the drag and after it.
    Transform {
        before: Vec<(NodeId, Affine)>,
        after: Vec<(NodeId, Affine)>,
    },
    /// A handle's drag on a text object's box.
    TextFrame {
        node: NodeId,
        before: TextBox,
        after: TextBox,
    },
}

/// The stroke or drag still open. The host previews it. the Storyboard Generator does not.
#[derive(Clone, Debug)]
pub struct OpenGesture {
    pub kind: GestureKind,
    pub samples: Vec<Sample>,
}

/// One finished gesture, ready to send. Not a step and not history.
#[derive(Clone, Debug)]
pub struct Gesture {
    pub kind: GestureKind,
    pub samples: Vec<Sample>,
}

impl OpenGesture {
    pub fn brush(
        canvas: Option<NodeId>,
        source: ColorId,
        layer: Option<ColorId>,
        size: Option<u8>,
        dynamics: Dynamics,
    ) -> Self {
        Self {
            kind: GestureKind::Brush {
                canvas,
                source,
                layer,
                size,
                dynamics,
            },
            samples: Vec::new(),
        }
    }

    pub fn erase(canvas: NodeId, size: u8, dynamics: Dynamics) -> Self {
        Self {
            kind: GestureKind::Erase {
                canvas,
                size,
                dynamics,
            },
            samples: Vec::new(),
        }
    }

    /// `kind` is the shape, not a tool id from the sub-application's registry.
    pub fn shape(kind: ShapeKind, origin: Point) -> Self {
        Self {
            kind: GestureKind::Shape {
                kind,
                origin,
                pointer: origin,
                shift: false,
            },
            samples: Vec::new(),
        }
    }

    pub fn text(origin: Point, hit: Option<NodeId>) -> Self {
        Self {
            kind: GestureKind::Text {
                origin,
                pointer: origin,
                hit,
            },
            samples: Vec::new(),
        }
    }

    /// The sample handling from the reference session's `add_sample`.
    pub fn add_sample(&mut self, sample: Sample) -> Result<(), GestureError> {
        if !sample.x.is_finite() || !sample.y.is_finite() {
            return Err(GestureError::InvalidSample);
        }
        if let Some(pressure) = sample.pressure {
            if !pressure.is_finite() || !(0.0..=1.0).contains(&pressure) {
                return Err(GestureError::InvalidSample);
            }
        }
        if let Some(last) = self.samples.last_mut() {
            if last.x == sample.x && last.y == sample.y {
                if last.pressure != sample.pressure {
                    last.pressure = sample.pressure;
                }
                return Ok(());
            }
        }
        self.samples.push(sample);
        Ok(())
    }

    pub fn finish(self) -> Gesture {
        Gesture {
            kind: self.kind,
            samples: self.samples,
        }
    }
}

impl Gesture {
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        match &self.kind {
            GestureKind::Brush {
                canvas,
                source,
                layer,
                size,
                dynamics,
            } => {
                object.insert("kind".to_string(), json!("brush"));
                object.insert("canvas".to_string(), id_value(canvas.map(NodeId::get)));
                object.insert("source".to_string(), json!(source.get()));
                object.insert("layer".to_string(), id_value(layer.map(ColorId::get)));
                object.insert("size".to_string(), size_value(*size));
                object.insert("dynamics".to_string(), dynamics_value(dynamics));
            }
            GestureKind::Erase {
                canvas,
                size,
                dynamics,
            } => {
                object.insert("kind".to_string(), json!("erase"));
                object.insert("canvas".to_string(), json!(canvas.get()));
                object.insert("size".to_string(), json!(size));
                object.insert("dynamics".to_string(), dynamics_value(dynamics));
            }
            GestureKind::Shape {
                kind,
                origin,
                pointer,
                shift,
            } => {
                object.insert("kind".to_string(), json!("shape"));
                object.insert("shape".to_string(), json!(shape_name(*kind)));
                object.insert("origin".to_string(), point_value(*origin));
                object.insert("pointer".to_string(), point_value(*pointer));
                object.insert("shift".to_string(), json!(shift));
            }
            GestureKind::Text {
                origin,
                pointer,
                hit,
            } => {
                object.insert("kind".to_string(), json!("text"));
                object.insert("origin".to_string(), point_value(*origin));
                object.insert("pointer".to_string(), point_value(*pointer));
                object.insert("hit".to_string(), id_value(hit.map(NodeId::get)));
            }
            GestureKind::Transform { before, after } => {
                object.insert("kind".to_string(), json!("transform"));
                object.insert("before".to_string(), locals_value(before));
                object.insert("after".to_string(), locals_value(after));
            }
            GestureKind::TextFrame {
                node,
                before,
                after,
            } => {
                object.insert("kind".to_string(), json!("text_frame"));
                object.insert("node".to_string(), json!(node.get()));
                object.insert("before".to_string(), box_value(*before));
                object.insert("after".to_string(), box_value(*after));
            }
        }
        object.insert(
            "samples".to_string(),
            Value::Array(self.samples.iter().map(sample_value).collect()),
        );
        Value::Object(object)
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let samples = samples_from(object.get("samples")?)?;
        let kind = match object.get("kind")?.as_str()? {
            "brush" => GestureKind::Brush {
                canvas: opt_node(object.get("canvas")?),
                source: ColorId::new(req_id(object.get("source")?)?),
                layer: opt_color(object.get("layer")?),
                size: opt_size(object.get("size")?),
                dynamics: dynamics_from(object.get("dynamics")?)?,
            },
            "erase" => GestureKind::Erase {
                canvas: NodeId::new(req_id(object.get("canvas")?)?),
                size: object.get("size")?.as_u64()?.try_into().ok()?,
                dynamics: dynamics_from(object.get("dynamics")?)?,
            },
            "shape" => GestureKind::Shape {
                kind: shape_kind(object.get("shape")?.as_str()?)?,
                origin: point_from(object.get("origin")?)?,
                pointer: point_from(object.get("pointer")?)?,
                shift: object.get("shift")?.as_bool()?,
            },
            "text" => GestureKind::Text {
                origin: point_from(object.get("origin")?)?,
                pointer: point_from(object.get("pointer")?)?,
                hit: opt_node(object.get("hit")?),
            },
            "transform" => GestureKind::Transform {
                before: locals_from(object.get("before")?)?,
                after: locals_from(object.get("after")?)?,
            },
            "text_frame" => GestureKind::TextFrame {
                node: NodeId::new(req_id(object.get("node")?)?),
                before: box_from_value(object.get("before")?)?,
                after: box_from_value(object.get("after")?)?,
            },
            _ => return None,
        };
        Some(Self { kind, samples })
    }
}

fn locals_value(locals: &[(NodeId, Affine)]) -> Value {
    Value::Array(
        locals
            .iter()
            .map(|(id, transform)| json!({ "node": id.get(), "transform": transform.as_coeffs() }))
            .collect(),
    )
}

fn locals_from(value: &Value) -> Option<Vec<(NodeId, Affine)>> {
    let mut locals = Vec::new();
    for item in value.as_array()? {
        let node = NodeId::new(req_id(item.get("node")?)?);
        let numbers = item.get("transform")?.as_array()?;
        if numbers.len() != 6 {
            return None;
        }
        let mut coeffs = [0.0; 6];
        for (slot, number) in coeffs.iter_mut().zip(numbers) {
            *slot = number.as_f64().filter(|value| value.is_finite())?;
        }
        locals.push((node, Affine::new(coeffs)));
    }
    Some(locals)
}

fn id_value(id: Option<u64>) -> Value {
    id.map(|id| json!(id)).unwrap_or(Value::Null)
}

fn size_value(size: Option<u8>) -> Value {
    size.map(|size| json!(size)).unwrap_or(Value::Null)
}

fn point_value(point: Point) -> Value {
    json!([point[0], point[1]])
}

fn sample_value(sample: &Sample) -> Value {
    json!({
        "x": sample.x,
        "y": sample.y,
        "pressure": sample.pressure.map(|pressure| json!(pressure)).unwrap_or(Value::Null),
    })
}

fn dynamics_value(dynamics: &Dynamics) -> Value {
    json!({
        "pressure_size": dynamics.pressure_size,
        "curve": dynamics.curve.iter().map(|point| json!({ "x": point.x, "y": point.y })).collect::<Vec<_>>(),
        "smoothing": dynamics.smoothing,
        "stabilization": dynamics.stabilization,
        "streamline": dynamics.streamline,
    })
}

fn req_id(value: &Value) -> Option<u64> {
    let id = value.as_u64()?;
    if id == 0 { None } else { Some(id) }
}

fn opt_node(value: &Value) -> Option<NodeId> {
    if value.is_null() {
        None
    } else {
        Some(NodeId::new(req_id(value)?))
    }
}

fn opt_color(value: &Value) -> Option<ColorId> {
    if value.is_null() {
        None
    } else {
        Some(ColorId::new(req_id(value)?))
    }
}

fn opt_size(value: &Value) -> Option<u8> {
    if value.is_null() {
        None
    } else {
        value.as_u64()?.try_into().ok()
    }
}

fn point_from(value: &Value) -> Option<Point> {
    let pair = value.as_array()?;
    if pair.len() != 2 {
        return None;
    }
    let x = pair[0].as_f64()?;
    let y = pair[1].as_f64()?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some([x, y])
}

fn samples_from(value: &Value) -> Option<Vec<Sample>> {
    let mut samples = Vec::new();
    for item in value.as_array()? {
        let object = item.as_object()?;
        let x = object.get("x")?.as_f64()?;
        let y = object.get("y")?.as_f64()?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let pressure = match object.get("pressure") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let pressure = value.as_f64()?;
                if !pressure.is_finite() {
                    return None;
                }
                Some(pressure)
            }
        };
        samples.push(Sample { x, y, pressure });
    }
    Some(samples)
}

fn dynamics_from(value: &Value) -> Option<Dynamics> {
    let object = value.as_object()?;
    let mut curve = Vec::new();
    for point in object.get("curve")?.as_array()? {
        let point = point.as_object()?;
        curve.push(CurvePoint {
            x: point.get("x")?.as_f64()?,
            y: point.get("y")?.as_f64()?,
        });
    }
    Some(Dynamics {
        pressure_size: object.get("pressure_size")?.as_bool()?,
        curve,
        smoothing: object.get("smoothing")?.as_u64()?.try_into().ok()?,
        stabilization: object.get("stabilization")?.as_u64()?.try_into().ok()?,
        streamline: object.get("streamline")?.as_u64()?.try_into().ok()?,
    })
}

fn shape_name(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "rectangle",
        ShapeKind::Ellipse => "ellipse",
        ShapeKind::Line => "line",
    }
}

fn shape_kind(name: &str) -> Option<ShapeKind> {
    Some(match name {
        "rectangle" => ShapeKind::Rectangle,
        "ellipse" => ShapeKind::Ellipse,
        "line" => ShapeKind::Line,
        _ => return None,
    })
}
