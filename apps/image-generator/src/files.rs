//! Files: the image's own `.ipx` file, PNG and JPEG in and out, and the
//! raw images the host reads. The `.ipx` format is described in this
//! application's AGENTS.md.

use crate::layered::{Blend, Curves, Grade, Kind, LayerMask, Locks, Node, LayeredImage, Pixels};
use crate::text::Text;
use image::{ImageDecoder, ImageEncoder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The first line of every `.ipx` file.
const MAGIC: &[u8] = b"IPX1\n";
/// Version 3 keeps each layer's kind and settings, and masks; version 2,
/// from 0.11, a gallery of images; version 1, from 0.10, one.
const VERSION: u32 = 3;
/// The largest image read or made: about 100 megapixels, so a damaged or
/// huge file cannot take all the memory.
const MOST_PIXELS: u64 = 100_000_000;

/// Only the version, read first to know which header follows.
#[derive(Deserialize)]
struct Version {
    version: u32,
}

/// Version 1's header: one image.
#[derive(Serialize, Deserialize)]
struct HeaderV1 {
    version: u32,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<u64>,
    /// Bottom to top.
    layers: Vec<NodeFile>,
}

/// Version 2's and 3's header: every image of the gallery, in order, and
/// which is shown.
#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    #[serde(default)]
    shown: usize,
    #[serde(rename = "pictures")]
    images: Vec<ImageFile>,
    /// The cell and image the gallery was opened from as new work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<protocol::OpenedFrom>,
}

#[derive(Serialize, Deserialize)]
struct ImageFile {
    name: String,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<u64>,
    /// Bottom to top.
    layers: Vec<NodeFile>,
}

#[derive(Serialize, Deserialize)]
struct NodeFile {
    id: u64,
    name: String,
    visible: bool,
    blend: Blend,
    opacity: f32,
    fill: f32,
    #[serde(default)]
    locks: Locks,
    #[serde(default)]
    offset: [i32; 2],
    /// A layer's PNG: where it starts after the header line, and its length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    png: Option<[u64; 2]>,
    /// A group's layers, bottom to top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    children: Option<Vec<NodeFile>>,
    /// From version 3: `pixels`, `group`, `text`, `curves`, or `grade`.
    /// Version 2's nodes have none, and are pixels or groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    /// A text's words, style, and box width; its box's corner is `offset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    curves: Option<Curves>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grade: Option<Grade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<MaskFile>,
}

/// A node's mask: its own id and settings, and its grey PNG among the bytes
/// after the header line.
#[derive(Serialize, Deserialize)]
struct MaskFile {
    id: u64,
    name: String,
    #[serde(default)]
    feather: f32,
    #[serde(default = "shown")]
    visible: bool,
    png: [u64; 2],
}

fn shown() -> bool {
    true
}

/// One image read from a file: its name, layers, and active layer.
pub struct ReadImage {
    pub name: String,
    pub image: LayeredImage,
    pub active: Option<u64>,
}

/// A gallery read from its file, never empty, and which image is shown.
pub struct Read {
    pub images: Vec<ReadImage>,
    pub shown: usize,
    pub source: Option<protocol::OpenedFrom>,
}

/// An encoded layer, shared by an image's cache and the writes using it.
pub type Png = std::sync::Arc<Vec<u8>>;

/// Encoded layers kept by layer id and the fingerprint of the pixels they
/// were made from, so a file written again encodes only the layers that
/// changed; and the document's pixel generation when they were made, so a
/// image whose pixels are unchanged since, such as after a change to an
/// adjustment or a layer's settings only, is not copied or looked at again.
#[derive(Default)]
pub struct PngCache {
    layers: HashMap<u64, (u64, Png)>,
    made_at: Option<u64>,
}

impl PngCache {
    /// Keeps what a write encoded for this image.
    pub fn keep(&mut self, encoded: Encoded) {
        self.layers = encoded.layers;
        self.made_at = Some(encoded.generation);
    }
}

/// One layer as a write takes it: its pixels to look at, or none when its
/// image has not changed since it was encoded; and what was encoded for
/// it last.
struct LayerSnapshot {
    pixels: Option<Pixels>,
    kept: Option<(u64, Png)>,
}

/// One image to write, owned, so the writer's thread can write it while
/// the image is worked on.
pub struct Snapshot {
    /// The image's number in its tab, to give its encoded layers back.
    pub uid: u64,
    name: String,
    width: u32,
    height: u32,
    active: Option<u64>,
    nodes: Vec<Node>,
    layers: HashMap<u64, LayerSnapshot>,
    generation: u64,
}

impl Snapshot {
    /// The image as it is: its layers' pixels copied, but only when its
    /// pixels changed since they were encoded, as `generation`, the
    /// document's, says.
    pub fn take(uid: u64, name: &str, image: &LayeredImage, active: Option<u64>, generation: u64, cache: &PngCache) -> Self {
        let unchanged = cache.made_at == Some(generation);
        let mut layers = HashMap::new();
        for id in image.buffers() {
            let kept = cache.layers.get(&id).cloned();
            let pixels = if unchanged && kept.is_some() { None } else { image.pixels.get(&id).cloned() };
            layers.insert(id, LayerSnapshot { pixels, kept });
        }
        Self {
            uid,
            name: name.to_string(),
            width: image.width,
            height: image.height,
            active,
            nodes: image.nodes.clone(),
            layers,
            generation,
        }
    }
}

#[cfg(test)]
impl Snapshot {
    /// How many buffers' pixels it copied.
    pub fn copied(&self) -> usize {
        self.layers.values().filter(|layer| layer.pixels.is_some()).count()
    }
}

/// What a write encoded for one image, for its cache.
pub struct Encoded {
    pub uid: u64,
    generation: u64,
    layers: HashMap<u64, (u64, Png)>,
}

/// A fingerprint of a layer's pixels: cheap next to encoding them.
fn fingerprint(pixels: &Pixels) -> u64 {
    // FNV-1a over 8-byte words.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ (u64::from(pixels.width) << 32 | u64::from(pixels.height));
    for chunk in pixels.data.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        hash ^= u64::from_le_bytes(word);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Writes a gallery: every image in order, and which is shown. Returns
/// what was encoded for each image.
pub fn write_ipx(path: &Path, images: Vec<Snapshot>, shown: usize, source: Option<protocol::OpenedFrom>) -> Result<Vec<Encoded>, String> {
    let mut blobs: Vec<u8> = Vec::new();
    let mut files = Vec::new();
    let mut encoded = Vec::new();
    for image in images {
        let mut made = HashMap::new();
        let mut layers = Vec::new();
        for node in &image.nodes {
            layers.push(node_file(node, &image.layers, &mut made, &mut blobs)?);
        }
        files.push(ImageFile { name: image.name, width: image.width, height: image.height, active: image.active, layers });
        encoded.push(Encoded { uid: image.uid, generation: image.generation, layers: made });
    }
    let header = Header { version: VERSION, shown, images: files, source };
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(&header).map_err(|error| error.to_string())?);
    bytes.push(b'\n');
    bytes.extend(blobs);
    write_atomic(path, &bytes)?;
    Ok(encoded)
}

/// One buffer's PNG, encoded again only when its pixels changed: a layer's
/// in colour, a mask's in grey.
fn buffer_png(id: u64, grey: bool, layers: &HashMap<u64, LayerSnapshot>, made: &mut HashMap<u64, (u64, Png)>, blobs: &mut Vec<u8>) -> Result<[u64; 2], String> {
    let layer = layers.get(&id).ok_or_else(|| format!("Layer {id} has no pixels."))?;
    let (print, png) = match (&layer.pixels, &layer.kept) {
        (None, Some(kept)) => kept.clone(),
        (Some(pixels), kept) => {
            let print = fingerprint(pixels);
            match kept {
                Some((kept, png)) if *kept == print => (print, png.clone()),
                _ if grey => (print, Png::new(encode_grey(pixels)?)),
                _ => (print, Png::new(encode_png(pixels.width, pixels.height, &pixels.data)?)),
            }
        }
        (None, None) => return Err(format!("Layer {id} has no pixels.")),
    };
    let place = [blobs.len() as u64, png.len() as u64];
    blobs.extend_from_slice(&png);
    made.insert(id, (print, png));
    Ok(place)
}

fn node_file(node: &Node, layers: &HashMap<u64, LayerSnapshot>, made: &mut HashMap<u64, (u64, Png)>, blobs: &mut Vec<u8>) -> Result<NodeFile, String> {
    let mut file = NodeFile {
        id: node.id,
        name: node.name.clone(),
        visible: node.visible,
        blend: node.blend,
        opacity: node.opacity,
        fill: node.fill,
        locks: node.locks,
        offset: node.offset,
        png: None,
        children: None,
        kind: None,
        text: None,
        curves: None,
        grade: None,
        mask: None,
    };
    if let Some(mask) = &node.mask {
        let png = buffer_png(mask.id, true, layers, made, blobs)?;
        file.mask = Some(MaskFile { id: mask.id, name: mask.name.clone(), feather: mask.feather, visible: mask.visible, png });
    }
    match (&node.children, &node.kind) {
        (Some(children), _) => {
            let mut inner = Vec::new();
            for child in children {
                inner.push(node_file(child, layers, made, blobs)?);
            }
            file.children = Some(inner);
            file.kind = Some("group".into());
        }
        (None, Kind::Pixels) => {
            file.png = Some(buffer_png(node.id, false, layers, made, blobs)?);
            file.kind = Some("pixels".into());
        }
        // A text keeps its words, style, and box, not its pixels.
        (None, Kind::Text(text)) => {
            file.text = Some(text.clone());
            file.kind = Some("text".into());
        }
        (None, Kind::Curves(curves)) => {
            file.curves = Some(curves.clone());
            file.kind = Some("curves".into());
        }
        (None, Kind::Grade(grade)) => {
            file.grade = Some(*grade);
            file.kind = Some("grade".into());
        }
    }
    Ok(file)
}

/// Whether the file starts as an `.ipx` file does.
pub fn is_ipx(path: &Path) -> bool {
    use std::io::Read as _;
    let mut start = [0u8; 5];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut start))
        .is_ok_and(|_| start == MAGIC)
}

pub fn read_ipx(path: &Path) -> Result<Read, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let damaged = || String::from("It is not an image file, or it is damaged.");
    let rest = bytes.strip_prefix(MAGIC).ok_or_else(damaged)?;
    let end = rest.iter().position(|byte| *byte == b'\n').ok_or_else(damaged)?;
    let line = &rest[..end];
    let blobs = &rest[end + 1..];
    let version: Version = serde_json::from_slice(line).map_err(|_| damaged())?;
    match version.version {
        // 0.10's file: a gallery of one, named as the file is.
        1 => {
            let header: HeaderV1 = serde_json::from_slice(line).map_err(|_| damaged())?;
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|stem| !stem.is_empty())
                .unwrap_or_else(|| protocol::untitled(crate::session::ITEM));
            let file = ImageFile { name, width: header.width, height: header.height, active: header.active, layers: header.layers };
            Ok(Read { images: vec![read_image(file, blobs)?], shown: 0, source: None })
        }
        2 | VERSION => {
            let header: Header = serde_json::from_slice(line).map_err(|_| damaged())?;
            let mut images = Vec::new();
            for file in header.images {
                images.push(read_image(file, blobs)?);
            }
            if images.is_empty() {
                return Err(damaged());
            }
            let shown = header.shown.min(images.len() - 1);
            Ok(Read { images, shown, source: header.source })
        }
        other => Err(format!(
            "It is version {other} of an image file, and this Image Generator reads versions 1 to {VERSION}."
        )),
    }
}

fn read_image(file: ImageFile, blobs: &[u8]) -> Result<ReadImage, String> {
    let damaged = || String::from("It is not an image file, or it is damaged.");
    if file.width == 0 || file.height == 0 || u64::from(file.width) * u64::from(file.height) > MOST_PIXELS {
        return Err(damaged());
    }
    let mut image = LayeredImage::empty(file.width, file.height);
    for layer in file.layers {
        let node = read_node(layer, blobs, &mut image)?;
        image.nodes.push(node);
    }
    if image.nodes.is_empty() {
        return Err(damaged());
    }
    image.sync_texts();
    let active = file.active.filter(|id| image.find(*id).is_some());
    Ok(ReadImage { name: file.name, image, active })
}

fn read_node(file: NodeFile, blobs: &[u8], image: &mut LayeredImage) -> Result<Node, String> {
    let damaged = || String::from("It is not an image file, or it is damaged.");
    if image.find(file.id).is_some() || image.pixels.contains_key(&file.id) {
        return Err(damaged());
    }
    image.next_id = image.next_id.max(file.id + 1);
    let mut node = Node::layer(file.id, &file.name);
    node.visible = file.visible;
    node.blend = file.blend;
    node.opacity = file.opacity.clamp(0.0, 1.0);
    node.fill = file.fill.clamp(0.0, 1.0);
    node.locks = file.locks;
    node.offset = file.offset;
    let read_png = |place: [u64; 2]| -> Result<Pixels, String> {
        let start = usize::try_from(place[0]).map_err(|_| damaged())?;
        let length = usize::try_from(place[1]).map_err(|_| damaged())?;
        let png = blobs.get(start..start.checked_add(length).ok_or_else(damaged)?).ok_or_else(damaged)?;
        decode(png, Some(image::ImageFormat::Png))
    };
    if let Some(mask) = file.mask {
        if image.find(mask.id).is_some() || image.pixels.contains_key(&mask.id) {
            return Err(damaged());
        }
        image.next_id = image.next_id.max(mask.id + 1);
        image.pixels.insert(mask.id, read_png(mask.png)?);
        let feather = if mask.feather.is_finite() { mask.feather.clamp(0.0, 250.0) } else { 0.0 };
        node.mask = Some(LayerMask { id: mask.id, name: mask.name, feather, visible: mask.visible });
    }
    match file.kind.as_deref() {
        Some("text") => {
            let mut text = file.text.ok_or_else(damaged)?;
            text.style = text.style.clamped();
            node.kind = Kind::Text(text);
            return Ok(node);
        }
        Some("curves") => {
            let mut curves = file.curves.ok_or_else(damaged)?;
            for channel in 0..4 {
                let points = crate::layered::clean_points(curves.channel(channel)).ok_or_else(damaged)?;
                *curves.channel_mut(channel) = points;
            }
            node.kind = Kind::Curves(curves);
            return Ok(node);
        }
        Some("grade") => {
            node.kind = Kind::Grade(file.grade.ok_or_else(damaged)?.clamped());
            return Ok(node);
        }
        None | Some("pixels" | "group") => {}
        Some(_) => return Err(damaged()),
    }
    match (file.children, file.png) {
        (Some(children), _) => {
            let mut inner = Vec::new();
            for child in children {
                inner.push(read_node(child, blobs, image)?);
            }
            node.children = Some(inner);
        }
        (None, Some(place)) => {
            image.pixels.insert(file.id, read_png(place)?);
        }
        (None, None) => return Err(damaged()),
    }
    Ok(node)
}

fn decode(bytes: &[u8], format: Option<image::ImageFormat>) -> Result<Pixels, String> {
    let unreadable = |error: image::ImageError| format!("The image could not be read: {error}.");
    // The size is read first, so a huge image is refused before it is
    // decoded.
    let (width, height) = reader(bytes, format)?.into_dimensions().map_err(unreadable)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MOST_PIXELS {
        return Err("The image is too large to open.".into());
    }
    // A phone's JPEG is often stored on its side with an EXIF orientation
    // that says how to turn it; it is turned so it opens the right way up.
    let mut decoder = reader(bytes, format)?.into_decoder().map_err(unreadable)?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    Ok(Pixels {
        width: rgba.width(),
        height: rgba.height(),
        data: rgba.into_raw(),
    })
}

fn reader(bytes: &[u8], format: Option<image::ImageFormat>) -> Result<image::ImageReader<std::io::Cursor<&[u8]>>, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    match format {
        Some(format) => reader.set_format(format),
        None => reader = reader.with_guessed_format().map_err(|error| error.to_string())?,
    }
    if reader.format().is_none() {
        return Err("It is not a PNG or JPEG image.".into());
    }
    Ok(reader)
}

/// A PNG or JPEG file as straight RGBA8.
pub fn import(path: &Path) -> Result<Pixels, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    decode(&bytes, None)
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// A mask's grey pixels as a grey PNG.
fn encode_grey(pixels: &Pixels) -> Result<Vec<u8>, String> {
    let grey: Vec<u8> = pixels.data.chunks_exact(4).map(crate::layered::grey).collect();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&grey, pixels.width, pixels.height, image::ExtendedColorType::L8)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// A JPEG of the image over white, as JPEG keeps no transparency.
pub fn encode_jpeg(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in rgba.chunks_exact(4) {
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            let value = (u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255;
            rgb.push(value as u8);
        }
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// Replaces a file whole: a scratch file beside it, then a rename, so no
/// reader ever sees half of one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, path).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        error.to_string()
    })
}

/// Standard base64, for the images the assistant looks at.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16 | u32::from(*chunk.get(1).unwrap_or(&0)) << 8 | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A smaller copy of an image, at most `most` pixels on its longer side,
/// each pixel the average of the ones it covers.
pub fn shrink(width: u32, height: u32, rgba: &[u8], most: u32) -> (u32, u32, Vec<u8>) {
    let longer = width.max(height);
    if longer <= most {
        return (width, height, rgba.to_vec());
    }
    let scale = f64::from(most) / f64::from(longer);
    let small_w = ((f64::from(width) * scale).round() as u32).max(1);
    let small_h = ((f64::from(height) * scale).round() as u32).max(1);
    (small_w, small_h, resample(width, height, rgba, small_w, small_h))
}

/// Box-averages a straight-alpha image to a new size, weighting colour by
/// alpha so clear pixels do not darken the edges.
pub fn resample(width: u32, height: u32, rgba: &[u8], to_w: u32, to_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; to_w as usize * to_h as usize * 4];
    for ty in 0..to_h as usize {
        let y0 = ty * height as usize / to_h as usize;
        let y1 = ((ty + 1) * height as usize / to_h as usize).max(y0 + 1).min(height as usize);
        for tx in 0..to_w as usize {
            let x0 = tx * width as usize / to_w as usize;
            let x1 = ((tx + 1) * width as usize / to_w as usize).max(x0 + 1).min(width as usize);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            // Large boxes are sampled on a grid, so a thumbnail stays cheap.
            let step_y = ((y1 - y0) / 8).max(1);
            let step_x = ((x1 - x0) / 8).max(1);
            for y in (y0..y1).step_by(step_y) {
                for x in (x0..x1).step_by(step_x) {
                    let at = (y * width as usize + x) * 4;
                    let alpha = u64::from(rgba[at + 3]);
                    for channel in 0..3 {
                        sum[channel] += u64::from(rgba[at + channel]) * alpha;
                    }
                    sum[3] += alpha;
                    count += 1;
                }
            }
            let at = (ty * to_w as usize + tx) * 4;
            if sum[3] > 0 {
                for channel in 0..3 {
                    out[at + channel] = (sum[channel] / sum[3]) as u8;
                }
                out[at + 3] = (sum[3] / count.max(1)) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("image-generator-files-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Catches a file that loses a layer, a group, a setting, an offset, or
    /// a pixel when written and read back.
    #[test]
    fn an_ipx_file_round_trips() {
        let dir = scratch("ipx");
        let mut image = LayeredImage::blank(30, 20);
        let a = image.take_id();
        let b = image.take_id();
        let group = image.take_id();
        let mut pixels = Pixels::clear(10, 5);
        for (index, byte) in pixels.data.iter_mut().enumerate() {
            *byte = (index * 7 % 256) as u8;
        }
        image.pixels.insert(a, pixels);
        image.pixels.insert(b, Pixels::filled(30, 20, [1, 2, 3, 4]));
        let mut first = Node::layer(a, "Speckle");
        first.offset = [-3, 9];
        first.blend = Blend::Overlay;
        first.opacity = 0.25;
        first.fill = 0.5;
        first.locks.position = true;
        let mut second = Node::layer(b, "Haze");
        second.visible = false;
        let mut folder = Node::group(group, "Folder", vec![first, second]);
        folder.locks.all = true;
        folder.opacity = 0.75;
        image.nodes.push(folder);
        // A gallery of three: the grouped image, a plain one of another
        // size, and one painted red, shown second.
        let plain = LayeredImage::blank(12, 40);
        let mut red = LayeredImage::blank(5, 5);
        let red_id = red.nodes[0].id;
        red.pixels.insert(red_id, Pixels::filled(5, 5, [255, 0, 0, 255]));
        let path = dir.join("Round.ipx");
        let none = PngCache::default();
        let writing = vec![
            Snapshot::take(1, "Speckles", &image, Some(a), 0, &none),
            Snapshot::take(2, "Tall", &plain, None, 0, &none),
            Snapshot::take(3, "Red", &red, Some(red_id), 0, &none),
        ];
        let source = protocol::OpenedFrom { cell: "c1".into(), image: "images/a.png".into(), session: Some("s1".into()), recipe: None };
        write_ipx(&path, writing, 1, Some(source.clone())).unwrap();
        assert!(is_ipx(&path));
        let read = read_ipx(&path).unwrap();
        assert_eq!(read.shown, 1);
        assert_eq!(read.source, Some(source), "the gallery forgot where it came from");
        let names: Vec<&str> = read.images.iter().map(|read| read.name.as_str()).collect();
        assert_eq!(names, ["Speckles", "Tall", "Red"]);
        let first = &read.images[0];
        assert_eq!(first.active, Some(a));
        assert_eq!((first.image.width, first.image.height), (30, 20));
        assert_eq!(first.image.nodes, image.nodes);
        assert_eq!(first.image.pixels, image.pixels);
        assert!(first.image.next_id > group);
        for (got, want) in read.images[1..].iter().zip([&plain, &red]) {
            assert_eq!((got.image.width, got.image.height), (want.width, want.height));
            assert_eq!(got.image.nodes, want.nodes);
            assert_eq!(got.image.pixels, want.pixels, "an image's pixels came back as another's");
        }
        assert_eq!(read.images[2].active, Some(red_id));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches a 0.10 file that no longer opens: version 1, one image, opens
    /// as a gallery of one named as its file is, with its pixels.
    #[test]
    fn a_version_1_file_opens_as_one_image() {
        let dir = scratch("v1");
        let mut image = LayeredImage::blank(6, 4);
        let id = image.nodes[0].id;
        image.pixels.insert(id, Pixels::filled(6, 4, [9, 8, 7, 255]));
        let mut blobs = Vec::new();
        let snapshot = Snapshot::take(1, "Old Beach", &image, Some(id), 0, &PngCache::default());
        let layers = vec![node_file(&image.nodes[0], &snapshot.layers, &mut HashMap::new(), &mut blobs).unwrap()];
        let header = HeaderV1 { version: 1, width: 6, height: 4, active: Some(id), layers };
        let mut bytes = MAGIC.to_vec();
        bytes.extend(serde_json::to_vec(&header).unwrap());
        bytes.push(b'\n');
        bytes.extend(blobs);
        let path = dir.join("Old Beach.ipx");
        std::fs::write(&path, bytes).unwrap();
        let read = read_ipx(&path).unwrap();
        assert_eq!(read.images.len(), 1);
        assert_eq!(read.shown, 0);
        assert_eq!(read.images[0].name, "Old Beach");
        assert_eq!(read.images[0].active, Some(id));
        assert_eq!(read.images[0].image.pixels, image.pixels);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches PNG that changes a pixel, and JPEG that cannot be read back
    /// or comes back far from what was written.
    #[test]
    fn png_and_jpeg_round_trip() {
        let dir = scratch("formats");
        let mut rgba = Vec::new();
        for y in 0..16u32 {
            for x in 0..24u32 {
                rgba.extend_from_slice(&[(x * 10) as u8, (y * 15) as u8, 128, if x < 12 { 255 } else { 90 }]);
            }
        }
        let png = dir.join("out.png");
        std::fs::write(&png, encode_png(24, 16, &rgba).unwrap()).unwrap();
        let back = import(&png).unwrap();
        assert_eq!((back.width, back.height), (24, 16));
        assert_eq!(back.data, rgba);

        let jpeg = dir.join("out.jpg");
        std::fs::write(&jpeg, encode_jpeg(24, 16, &rgba).unwrap()).unwrap();
        let back = import(&jpeg).unwrap();
        assert_eq!((back.width, back.height), (24, 16));
        for (index, (got, want)) in back.data.chunks(4).zip(rgba.chunks(4)).enumerate() {
            assert_eq!(got[3], 255);
            // The opaque half comes back close; the clear half over white.
            let alpha = u32::from(want[3]);
            for channel in 0..3 {
                let expected = (u32::from(want[channel]) * alpha + 255 * (255 - alpha)) / 255;
                let difference = (i32::from(got[channel]) - expected as i32).abs();
                assert!(difference <= 24, "pixel {index} channel {channel}: {} vs {expected}", got[channel]);
            }
        }
        // A JPEG stored on its side, 24 × 16, with the EXIF orientation 6
        // ("turn a quarter clockwise"), opens 16 × 24: its top-left pixel,
        // dark, is then at the top right.
        let mut dark_corner = vec![255u8; 24 * 16 * 3];
        for y in 0..8 {
            for x in 0..8 {
                dark_corner[(y * 24 + x) * 3..(y * 24 + x) * 3 + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let mut turned = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut turned, 95);
        // A little-endian TIFF header and one entry: Orientation (0x0112),
        // a SHORT, 6.
        let exif = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder.write_image(&dark_corner, 24, 16, image::ExtendedColorType::Rgb8).unwrap();
        let phone = dir.join("phone.jpg");
        std::fs::write(&phone, turned).unwrap();
        let back = import(&phone).unwrap();
        assert_eq!((back.width, back.height), (16, 24), "the JPEG's orientation was ignored");
        let pixel = |x: usize, y: usize| back.data[(y * 16 + x) * 4];
        assert!(pixel(13, 2) < 60 && pixel(2, 2) > 200, "the JPEG was turned the wrong way");

        assert!(import(&dir.join("missing.png")).is_err());
        std::fs::write(dir.join("words.png"), b"not an image").unwrap();
        assert!(import(&dir.join("words.png")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// Catches a file that loses a kind of layer, or keeps a text's pixels
    /// rather than its words: an image with a pixel layer and its
    /// feathered mask, a text, and Curves and Color Grade layers in a group
    /// with a mask of its own, written and read back, has the same tree
    /// and the same composite, and its header keeps the words.
    #[test]
    fn every_kind_of_layer_round_trips() {
        use crate::layered::{Curves, Grade, LayerMask};
        use crate::text::{Style, Text};
        let dir = scratch("kinds");
        let mut image = LayeredImage::blank(60, 40);
        let background = image.nodes[0].id;
        let mut speckles = Pixels::filled(60, 40, [200, 120, 40, 255]);
        for (index, byte) in speckles.data.iter_mut().enumerate().filter(|(index, _)| index % 4 == 1) {
            *byte = (index * 13 % 256) as u8;
        }
        image.pixels.insert(background, speckles);
        let mask = image.take_id();
        let mut grey = Pixels::filled(60, 40, [255, 255, 255, 255]);
        for pixel in grey.data.chunks_exact_mut(4).take(800) {
            pixel.copy_from_slice(&[0, 0, 0, 255]);
        }
        image.pixels.insert(mask, grey);
        image.nodes[0].mask = Some(LayerMask { id: mask, name: "Subject Mask".into(), feather: 3.0, visible: true });
        let text = image.take_id();
        let words = Text { words: "Golden hour".into(), style: Style { size: 14.0, bold: true, color: [250, 250, 250], ..Style::default() }, width: Some(50.0) };
        let mut node = Node::of_kind(text, "Golden hour", Kind::Text(words));
        node.offset = [4, 6];
        let curves = image.take_id();
        let mut bent = Curves::default();
        bent.red = vec![[0.0, 0.1], [0.4, 0.6], [1.0, 0.9]];
        let grade = image.take_id();
        let group = image.take_id();
        let group_mask = image.take_id();
        image.pixels.insert(group_mask, Pixels::filled(60, 40, [128, 128, 128, 255]));
        let mut folder = Node::group(
            group,
            "Looks",
            vec![
                Node::of_kind(curves, "Curves", Kind::Curves(bent)),
                Node::of_kind(grade, "Color Grade", Kind::Grade(Grade { exposure: 0.5, contrast: 12.0, saturation: -30.0, temperature: 20.0 })),
            ],
        );
        folder.mask = Some(LayerMask { id: group_mask, name: "Looks Mask".into(), feather: 0.0, visible: true });
        image.nodes.push(node);
        image.nodes.push(folder);
        let composite = image.composite();
        let path = dir.join("Kinds.ipx");
        let snapshot = Snapshot::take(1, "Kinds", &image, Some(text), 0, &PngCache::default());
        assert!(!snapshot.layers.contains_key(&text), "the text's pixels were taken for the file");
        write_ipx(&path, vec![snapshot], 0, None).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let line = &bytes[MAGIC.len()..MAGIC.len() + bytes[MAGIC.len()..].iter().position(|byte| *byte == b'\n').unwrap()];
        let header: serde_json::Value = serde_json::from_slice(line).unwrap();
        assert_eq!(header["version"], 3);
        let saved = &header["pictures"][0]["layers"][1]; // The stored spelling of the images.
        assert_eq!(saved["kind"], "text");
        assert_eq!(saved["text"]["words"], "Golden hour");
        assert!(saved.get("png").is_none(), "a text kept its pixels");
        let mut read = read_ipx(&path).unwrap();
        let back = &mut read.images[0].image;
        assert_eq!(back.nodes, image.nodes);
        assert_eq!(back.composite(), composite, "the image read back looks different");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches a 0.12 file that no longer opens: version 2, whose nodes have
    /// no kind, opens with its layers as pixels and groups.
    #[test]
    fn a_version_2_file_still_opens() {
        let dir = scratch("v2");
        let mut image = LayeredImage::blank(6, 4);
        let id = image.nodes[0].id;
        image.pixels.insert(id, Pixels::filled(6, 4, [9, 8, 7, 255]));
        let inner = image.take_id();
        image.pixels.insert(inner, Pixels::filled(2, 2, [1, 2, 3, 255]));
        image.nodes.push(Node::group(image.next_id, "Folder", vec![Node::layer(inner, "Inner")]));
        image.next_id += 1;
        let path = dir.join("Old.ipx");
        write_ipx(&path, vec![Snapshot::take(1, "Old", &image, Some(id), 0, &PngCache::default())], 0, None).unwrap();
        // The same file as 0.12 wrote it: version 2, and no kinds.
        let bytes = std::fs::read(&path).unwrap();
        let end = MAGIC.len() + bytes[MAGIC.len()..].iter().position(|byte| *byte == b'\n').unwrap();
        let mut header: serde_json::Value = serde_json::from_slice(&bytes[MAGIC.len()..end]).unwrap();
        header["version"] = serde_json::json!(2);
        fn strip(layers: &mut serde_json::Value) {
            for layer in layers.as_array_mut().unwrap() {
                layer.as_object_mut().unwrap().remove("kind");
                if let Some(children) = layer.get_mut("children") {
                    strip(children);
                }
            }
        }
        strip(&mut header["pictures"][0]["layers"]); // The stored spelling of the images.
        let mut old = MAGIC.to_vec();
        old.extend(serde_json::to_vec(&header).unwrap());
        old.extend_from_slice(&bytes[end..]);
        assert!(!String::from_utf8_lossy(&old[..end + 40]).contains("\"kind\""));
        std::fs::write(&path, old).unwrap();
        let read = read_ipx(&path).unwrap();
        assert_eq!(read.images[0].image.nodes, image.nodes);
        assert_eq!(read.images[0].image.pixels, image.pixels);
        let _ = std::fs::remove_dir_all(dir);
    }
}
