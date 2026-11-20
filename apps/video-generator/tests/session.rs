//! The Video Generator as its own program, driven through the protocol
//! alone, on short videos made for the test in a scratch project.

use protocol::{AppMessage, HostMessage, RasterView};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The working area every test reports.
const VIEWPORT: [f64; 2] = [1200.0, 800.0];
const SIZE: (u32, u32) = (128, 72);

struct Program {
    child: Child,
    input: ChildStdin,
    output: Receiver<AppMessage>,
    calls: u64,
}

impl Program {
    fn start(restore: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_video-generator"));
        if restore {
            command.arg(protocol::RESTORE_FLAG);
        }
        // Tests never play sound.
        command.env("INPAINTER_NO_SOUND", "1");
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("video-generator starts");
        let input = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (lines, output) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let message = protocol::parse_app_message(&line).expect("every line is a message");
                if lines.send(message).is_err() {
                    break;
                }
            }
        });
        Self { child, input, output, calls: 0 }
    }

    fn send(&mut self, message: HostMessage) {
        writeln!(self.input, "{}", serde_json::to_string(&message).expect("json")).expect("sent");
    }

    fn command(&mut self, id: &str, arg: Value) {
        self.send(HostMessage::Command { id: id.into(), tab: Some("1".into()), arg: Some(arg), node: None, viewport: VIEWPORT });
    }

    /// The first message `pick` takes, within a few seconds.
    fn wait<T>(&self, what: &str, mut pick: impl FnMut(&AppMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self.output.recv_timeout(left).unwrap_or_else(|_| panic!("never came: {what}"));
            if let Some(found) = pick(&message) {
                return found;
            }
        }
    }

    /// The first view, with its image as it is now, that `check` passes.
    fn view(&self, what: &str, mut check: impl FnMut(&RasterView, &Image) -> bool) -> (RasterView, Image) {
        self.wait(what, |message| match message {
            AppMessage::Raster(view) => {
                let image = Image::read(view);
                check(view, &image).then(|| (view.clone(), image))
            }
            _ => None,
        })
    }

    /// Opens a video file into the gallery from the + tile's row, as the
    /// host's file dialog answers it.
    fn open_take(&mut self, path: &Path) {
        self.command("film_open", json!({}));
        let id = self.wait("a file dialog", |message| match message {
            AppMessage::File { id, .. } => Some(id.clone()),
            _ => None,
        });
        self.send(HostMessage::FileResult { id, path: Some(path.display().to_string()) });
    }

    /// One of the assistant's calls, and its output.
    fn invoke(&mut self, tool: &str, input: Value) -> Result<Value, Value> {
        self.calls += 1;
        let call = format!("call-{}", self.calls);
        self.send(HostMessage::Invoke {
            call: call.clone(),
            tab: "1".into(),
            tool: protocol::tool_key("video-generator", "takes", tool),
            input,
            group: "request-1".into(),
        });
        self.wait(tool, |message| match message {
            AppMessage::ToolResult(result) if result.call == call => Some(if result.ok { Ok(result.output.clone()) } else { Err(result.output.clone()) }),
            _ => None,
        })
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A view's image, read whole.
struct Image {
    width: u32,
    bytes: Vec<u8>,
}

impl Image {
    fn read(view: &RasterView) -> Image {
        Image { width: view.width, bytes: std::fs::read(&view.image).unwrap_or_default() }
    }

    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * self.width + x) * 4) as usize;
        self.bytes.get(at..at + 3).map_or([0, 0, 0], |rgb| [rgb[0], rgb[1], rgb[2]])
    }
}

/// Frame `i` of the first video: a flat colour of its own.
fn dawn(i: u32) -> [u8; 3] {
    [(20 + 4 * i) as u8, (220 - 4 * i) as u8, (40 + 4 * i) as u8]
}

/// Frame `i` of the second, as unlike the first as can be.
fn dusk(i: u32) -> [u8; 3] {
    [230, (20 + 16 * i) as u8, (220 - 16 * i) as u8]
}

const DAWN_FRAMES: u32 = 48;
const DUSK_FRAMES: u32 = 12;

fn write_video(path: &Path, frames: u32, colour: fn(u32) -> [u8; 3]) {
    write_video_at(path, frames, colour, 24);
}

/// A video of `frames` flat colours at `fps` frames a second.
fn write_video_at(path: &Path, frames: u32, colour: fn(u32) -> [u8; 3], fps: u32) {
    let settings = media::Settings { width: SIZE.0, height: SIZE.1, rate: (fps, 1), keyframes: 12, sound: None };
    let mut encoder = media::Encoder::create(path, settings).expect("an encoder");
    for i in 0..frames {
        let rgb = colour(i);
        encoder.frame(&[rgb[0], rgb[1], rgb[2], 255].repeat((SIZE.0 * SIZE.1) as usize)).expect("a frame");
    }
    encoder.finish().expect("the video");
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(y))).max().unwrap_or(0)
}

/// Whether a pixel is that frame of that video, and no other frame.
fn is(pixel: [u8; 3], colour: fn(u32) -> [u8; 3], frames: u32, index: u32) -> bool {
    let nearest = (0..frames).min_by_key(|&i| distance(pixel, colour(i))).unwrap_or(0);
    nearest == index && distance(pixel, colour(index)) <= 6
}

/// The host's `project` for `folder`, read from its line, so it says
/// nothing but the folder: every project is organized one way.
fn in_project(folder: &Path) -> HostMessage {
    let line = json!({ "type": "project", "folder": folder.display().to_string() }).to_string();
    protocol::parse_host_message(&line).expect("a project")
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("video-generator-session-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("takes")).expect("a scratch folder");
    dir.canonicalize().expect("canonical")
}

/// A project with the two videos in its `takes` folder.
fn project(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = scratch(label);
    let (first, second) = (dir.join("takes/Dawn.mp4"), dir.join("takes/Dusk.mp4"));
    write_video(&first, DAWN_FRAMES, dawn);
    write_video(&second, DUSK_FRAMES, dusk);
    (dir, first, second)
}

fn names(view: &RasterView) -> Vec<String> {
    view.film.as_ref().map(|film| film.items.iter().map(|item| item.name.clone()).collect()).unwrap_or_default()
}

/// Where the takes are drawn in a 1200 × 800 area with 40 around it: one
/// take, small enough to keep its own 128 × 72 pixels, in the middle; two
/// side by side, each in the middle of its half.
const ALONE: (u32, u32) = (600, 400);
const LEFT: (u32, u32) = (316, 400);
const RIGHT: (u32, u32) = (884, 400);

/// The middle of the frame strips' row `row`, column `column`: eight cells
/// of 133 × 75 across 1120 with 8 between them, the rows packed from the
/// top, each 75 tall with 18 for its times and 8 between.
fn strip(row: u32, column: u32) -> (u32, u32) {
    (40 + 141 * column + 2 + 64, 40 + 101 * row + 1 + 36)
}

/// Catches a gallery that loses a take, a frame drawn from the wrong take
/// or the wrong time, a view laid out other than as approved, an undo that
/// does not put a change back, a file that forgets the gallery, and a
/// linked video written to: two videos open into the gallery with their
/// thumbnails; the frame shown follows the keys; side by side, zoomed, and
/// frame strips show the right frames in the right places; rename, reorder,
/// delete, and the view undo and redo; and the `.vgx` reopens in a new
/// program as it was left, with the videos' sha256 unchanged.
#[test]
fn a_gallery_of_takes_is_shown_kept_and_reopened() {
    let (dir, first, second) = project("gallery");
    let sums = [layers::link::sha256_file(&first).unwrap(), layers::link::sha256_file(&second).unwrap()];
    let mut program = Program::start(false);
    let label = program.wait("the tab", |message| match message {
        AppMessage::Tab { label, .. } => Some(label.clone()),
        _ => None,
    });
    assert_eq!(label, "Untitled Video");
    let groups = program.wait("the tool groups", |message| match message {
        AppMessage::Tools { groups } => Some(groups.clone()),
        _ => None,
    });
    let tools: Vec<&str> = groups[0].tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!((groups[0].name.as_str(), tools), ("Video Generator", vec!["list_takes", "open_video", "show_take", "set_view"]));
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    let (empty, image) = program.view("an empty view at the area's size", |view, _| view.width == 1200);
    assert_eq!((empty.width, empty.height), (1200, 800));
    assert_eq!(image.bytes.len(), 1200 * 800 * 4, "the image file is not the area's size");
    assert_eq!(image.at(600, 400), [255, 255, 255]);
    assert!(names(&empty).is_empty());
    assert_eq!(empty.camera_owner, protocol::RasterCamera::Workspace, "the view is not the workspace's own");
    let camera = empty.camera.expect("the first view did not put the host's camera on the image");
    assert_eq!((camera.origin, camera.zoom), ([0.0, 0.0], 1.0), "the host would fit the image with its margin");
    // The area changing makes a new image, and its view puts the camera
    // on it again; back at the area's first size, so does the next.
    for (size, label) in [([1300.0, 900.0], "the view after the area grew"), (VIEWPORT, "the view back at the area's size")] {
        program.send(HostMessage::Viewport { viewport: size });
        let (resized, _) = program.view(label, |view, _| f64::from(view.width) == size[0] && view.camera.is_some());
        let camera = resized.camera.unwrap();
        assert_eq!((camera.origin, camera.zoom), ([0.0, 0.0], 1.0));
    }

    // Two takes, each shown as it opens, with its thumbnail: its first
    // frame, 44 tall.
    program.open_take(&first);
    program.view("the first take", |view, image| names(view) == ["Dawn"] && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    let (view, _) = program.view("the first take's thumbnail", |view, _| view.film.as_ref().unwrap().items[0].thumbnail.revision > 0);
    let thumbnail = &view.film.as_ref().unwrap().items[0].thumbnail;
    assert_eq!((thumbnail.width, thumbnail.height), (78, 44));
    let bytes = std::fs::read(&thumbnail.path).unwrap();
    assert_eq!(bytes.len(), 78 * 44 * 4);
    assert!(distance([bytes[0], bytes[1], bytes[2]], dawn(0)) <= 6, "the thumbnail is not the take's first frame");
    program.open_take(&second);
    program.view("the second take", |view, image| {
        names(view) == ["Dawn", "Dusk"] && view.film.as_ref().unwrap().shown == 1 && is(image.at(ALONE.0, ALONE.1), dusk, DUSK_FRAMES, 0)
    });

    // The keys step the frame: one frame, the last, the first, and a
    // second, which stops at the last.
    for (arg, frame) in [
        (json!({"step": 1}), 1),
        (json!({"step": 1}), 2),
        (json!({"to": "last"}), 11),
        (json!({"to": "first"}), 0),
        (json!({"seconds": 1}), 11),
    ] {
        program.command("frame", arg.clone());
        program.view(&format!("frame {frame} after {arg}"), |_, image| is(image.at(ALONE.0, ALONE.1), dusk, DUSK_FRAMES, frame));
    }
    // The first take keeps the frame, and a second is 24 frames.
    program.command("film_select", json!({"value": 0}));
    program.view("the first take at frame 11", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 11));
    program.command("frame", json!({"seconds": 1}));
    program.view("frame 35", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 35));
    program.command("frame", json!({"step": -1}));
    program.view("frame 34", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 34));

    // A second take chosen with Ctrl or Shift: side by side at the same
    // frame, the shorter at its last.
    program.command("film_select", json!({"value": 1, "extend": true}));
    program.view("side by side", |_, image| {
        is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 34) && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 11) && image.at(ALONE.0, ALONE.1) == [255, 255, 255]
    });
    // At 200%, each take is drawn twice as large about its middle.
    program.command("view_zoom", json!({"zoom": 2.0}));
    program.view("side by side at 200%", |_, image| is(image.at(LEFT.0 - 100, LEFT.1), dawn, DAWN_FRAMES, 34) && is(image.at(RIGHT.0 + 100, RIGHT.1), dusk, DUSK_FRAMES, 11));
    program.command("view_zoom", json!({"zoom": null}));
    program.view("side by side fitted", |_, image| image.at(LEFT.0 - 100, LEFT.1) == [255, 255, 255]);

    // Frame strips: a row of eight frames for each take, in filmstrip
    // order, at equal times, each ending at the take's last frame.
    let strips_shown = |image: &Image| {
        (0..8).all(|column| {
            let (x, y) = strip(0, column);
            let (u, v) = strip(1, column);
            is(image.at(x, y), dawn, DAWN_FRAMES, 6 * (column + 1) - 1)
                && is(image.at(u, v), dusk, DUSK_FRAMES, ((column + 1) * DUSK_FRAMES / 8).saturating_sub(1))
        })
    };
    program.command("view_strips", json!({}));
    program.view("frame strips", |_, image| strips_shown(image));
    // The view is a step: undone, side by side again; redone, strips.
    program.command("undo", json!({}));
    program.view("side by side again", |_, image| is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 34) && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 11));
    program.command("redo", json!({}));
    program.view("frame strips again", |_, image| strips_shown(image));

    // Rename, reorder, and delete, each undone and redone.
    program.command("film_rename", json!({"take": 0, "value": "Dawn take"}));
    program.view("the rename", |view, _| names(view) == ["Dawn take", "Dusk"]);
    program.command("undo", json!({}));
    program.view("the rename undone", |view, _| names(view) == ["Dawn", "Dusk"]);
    program.command("redo", json!({}));
    program.view("the rename redone", |view, _| names(view) == ["Dawn take", "Dusk"]);
    program.command("film_reorder", json!({"from": 0, "to": 1}));
    program.view("the reorder", |view, image| {
        names(view) == ["Dusk", "Dawn take"]
            && is(image.at(strip(0, 7).0, strip(0, 7).1), dusk, DUSK_FRAMES, 11)
            && is(image.at(strip(1, 7).0, strip(1, 7).1), dawn, DAWN_FRAMES, 47)
    });
    program.command("undo", json!({}));
    program.view("the reorder undone", |view, _| names(view) == ["Dawn take", "Dusk"]);
    program.command("redo", json!({}));
    program.view("the reorder redone", |view, _| names(view) == ["Dusk", "Dawn take"]);
    program.command("film_delete", json!({"take": 0}));
    program.view("the delete", |view, _| names(view) == ["Dawn take"]);
    program.command("undo", json!({}));
    program.view("the delete undone", |view, _| names(view) == ["Dusk", "Dawn take"]);

    // Left as: "Dawn take" shown beside "Dusk", at frame 20.
    program.command("view_strips", json!({}));
    program.command("film_select", json!({"value": 1}));
    program.command("film_select", json!({"value": 0, "extend": true}));
    program.command("frame", json!({"to": "first"}));
    for _ in 0..20 {
        program.command("frame", json!({"step": 1}));
    }
    program.view("the view to keep", |_, image| is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 20) && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 11));
    let listed = program.invoke("list_takes", json!({})).unwrap();
    assert_eq!((listed["view"].as_str(), listed["shown"].as_u64(), listed["with"].as_u64(), listed["frame"].as_u64()), (Some("side_by_side"), Some(1), Some(0), Some(20)));
    assert_eq!(listed["takes"][1]["file"], "takes/Dawn.mp4");
    assert_eq!(listed["takes"][1]["frames"], 48);
    assert_eq!(listed["takes"][1]["seconds"], 2.0);

    // Nothing asks it to save: in a project, the gallery is written by
    // itself about a second after its last change.
    let kept = dir.join("work/Untitled Video.vgx");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !kept.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = std::fs::read_to_string(&kept).expect("the .vgx file");
    assert!(text.starts_with("VGX1\n{\"version\":3,"), "{text}");
    assert!(text.contains("\"path\":\"takes/Dawn.mp4\""), "a take inside the project is not linked by its path in it: {text}");
    assert!(text.contains(&sums[0]), "a take's sha256 is not kept: {text}");
    program.send(HostMessage::Shutdown);
    drop(program);

    // A new program, as a restored workspace, finds it all again, even
    // with Studio's own folder gone, as a harness would leave the project.
    let _ = std::fs::remove_dir_all(dir.join(protocol::PROJECT_DIR));
    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some(kept.display().to_string()) });
    let (view, _) = program.view("the gallery reopened", |view, image| {
        names(view) == ["Dusk", "Dawn take"]
            && view.film.as_ref().unwrap().shown == 1
            && is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 20)
            && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 11)
    });
    assert_eq!((view.width, view.height), (1200, 800));
    let listed = program.invoke("list_takes", json!({})).unwrap();
    assert_eq!((listed["view"].as_str(), listed["with"].as_u64(), listed["frame"].as_u64()), (Some("side_by_side"), Some(0), Some(20)));
    drop(program);

    assert_eq!([layers::link::sha256_file(&first).unwrap(), layers::link::sha256_file(&second).unwrap()], sums, "a linked video was written to");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches an assistant's tool that does nothing or acts other than the
/// strip and toolbar do: it opens both videos by their paths in the
/// project, shows one at a frame, and switches the view, and each change
/// is shown; a file that is no video is refused.
#[test]
fn the_assistant_opens_shows_and_switches_takes() {
    let (dir, _, _) = project("assistant");
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    let opened = program.invoke("open_video", json!({"path": "takes/Dawn.mp4"})).unwrap();
    assert_eq!((opened["take"].as_u64(), opened["name"].as_str(), opened["frames"].as_u64()), (Some(0), Some("Dawn"), Some(48)));
    let opened = program.invoke("open_video", json!({"path": dir.join("takes/Dusk.mp4").display().to_string(), "name": "Last light"})).unwrap();
    assert_eq!((opened["take"].as_u64(), opened["name"].as_str()), (Some(1), Some("Last light")));
    std::fs::write(dir.join("notes.txt"), "not a video").unwrap();
    assert!(program.invoke("open_video", json!({"path": "notes.txt"})).is_err(), "a file that is no video became a take");

    let shown = program.invoke("show_take", json!({"take": 0, "frame": 9})).unwrap();
    assert_eq!(shown["frame"], 9);
    program.view("the first take at frame 9", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 9));
    program.invoke("set_view", json!({"view": "side_by_side", "with": 1})).unwrap();
    program.view("side by side", |_, image| is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 9) && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 9));
    assert!(program.invoke("set_view", json!({"view": "side_by_side", "with": 0})).is_err(), "a take was shown beside itself");
    program.invoke("set_view", json!({"view": "strips"})).unwrap();
    program.view("frame strips", |_, image| is(image.at(strip(1, 7).0, strip(1, 7).1), dusk, DUSK_FRAMES, 11));
    let listed = program.invoke("list_takes", json!({})).unwrap();
    assert_eq!(listed["view"], "strips");
    assert_eq!(listed["takes"][1]["name"], "Last light");
    // Each call that changed the gallery is a step of its history.
    program.command("undo", json!({}));
    program.view("side by side again", |_, image| is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 9));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a video reference the host cannot show: `edits` names video
/// files, so the host asks here; `preview` of a video in the project
/// answers with its first frame, no larger than its own pixels, as a raw
/// image of its size, and its length for the corner, "2.0 s"; a file that
/// is no video answers with why, and no thumbnail.
#[test]
fn a_video_is_previewed_with_its_length() {
    let (dir, _, _) = project("preview");
    let mut program = Program::start(false);
    let edits = program.wait("edits", |message| match message {
        AppMessage::Edits { media_types } => Some(media_types.clone()),
        _ => None,
    });
    assert!(edits.iter().any(|kind| kind == "video/mp4"), "{edits:?}");
    program.send(in_project(&dir));
    program.send(HostMessage::Preview { id: "p1".into(), path: "takes/Dawn.mp4".into(), media_type: "video/mp4".into() });
    let (thumbnail, badge, error) = program.wait("the preview", |message| match message {
        AppMessage::Previewed { id, thumbnail, badge, error, .. } if id == "p1" => Some((thumbnail.clone(), badge.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(error, None);
    assert_eq!(badge, "2.0 s", "48 frames at 24 a second");
    let thumbnail = thumbnail.expect("no thumbnail");
    assert_eq!((thumbnail.width, thumbnail.height), SIZE);
    let bytes = std::fs::read(&thumbnail.path).unwrap();
    assert_eq!(bytes.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    let middle = (((SIZE.1 / 2) * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    assert!(is([bytes[middle], bytes[middle + 1], bytes[middle + 2]], dawn, DAWN_FRAMES, 0), "the preview is not the first frame");

    std::fs::write(dir.join("notes.txt"), "not a video").unwrap();
    program.send(HostMessage::Preview { id: "p2".into(), path: "notes.txt".into(), media_type: "video/mp4".into() });
    let refused = program.wait("a refusal", |message| match message {
        AppMessage::Previewed { id, thumbnail, error, .. } if id == "p2" => Some((thumbnail.is_none(), error.is_some())),
        _ => None,
    });
    assert_eq!(refused, (true, true));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a video the host offers to edit here that opens nothing, or
/// is written to: `open_as_new` with a name holding "/" makes a new tab
/// called by a safe name, whose gallery is one take linking the file,
/// kept as a new `.vgx` naming the file by its path in the project, the
/// video's sha256 unchanged.
#[test]
fn a_video_opens_as_new_work() {
    let (dir, first, _) = project("open-as-new");
    let sum = layers::link::sha256_file(&first).unwrap();
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::OpenAsNew { path: first.display().to_string(), name: "Dawn/take".into(), from: None });
    let tab = program.wait("the new tab", |message| match message {
        AppMessage::Tabs(tabs) => tabs.tabs.iter().find(|tab| tab.label == "Dawn take").map(|tab| tab.id.clone()),
        _ => None,
    });
    let (view, _) = program.view("the take", |view, _| view.tab == tab && names(view) == ["Dawn"]);
    assert_eq!(view.film.as_ref().unwrap().items.len(), 1);
    program.send(HostMessage::Keep { tab: tab.clone() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab: kept, path, error } if *kept == tab => Some((path.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(kept.1, None);
    let file = PathBuf::from(kept.0.expect("no file"));
    assert_eq!(file, dir.join("work/Dawn take.vgx"));
    let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
    assert!(text.contains("\"takes/Dawn.mp4\""), "{text}");
    assert_eq!(layers::link::sha256_file(&first).unwrap(), sum, "the video was written to");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a project's video that cannot be opened to play, or that is
/// kept as work it never was: a `.mp4` in the project is claimed and one
/// outside it is not; opened by its path in the project, as the project
/// panel opens an animatic in `library/`, it is a tab of one take that
/// plays, with its transport row, and nothing is written in `work/`;
/// it has no Prompt page cell, as a delivered film has no prompt or seed,
/// and its Prompt page is off; opened again, its tab is shown and no second
/// one made; and once its gallery changes, it is kept in `work/` as a new
/// tab is.
#[test]
fn a_video_in_the_project_opens_to_play() {
    let (dir, first, _) = project("open-to-play");
    let outside = scratch("open-to-play-elsewhere").join("Elsewhere.mp4");
    write_video(&outside, DUSK_FRAMES, dusk);
    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    let claimed = |program: &mut Program, id: &str, path: &Path| {
        program.send(HostMessage::Claim { id: id.into(), path: path.display().to_string() });
        program.wait("claimed", |message| match message {
            AppMessage::Claimed { id: answered, yours } if answered == id => Some(*yours),
            _ => None,
        })
    };
    assert!(claimed(&mut program, "c1", &first), "a video in the project is not claimed");
    assert!(!claimed(&mut program, "c2", &outside), "a video outside the project was claimed");

    program.send(HostMessage::Open { path: Some("takes/Dawn.mp4".into()) });
    let (view, _) = program.view("the take to play", |view, image| names(view) == ["Dawn"] && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    let tab = view.tab.clone();
    let declared = transport(&view).expect("no transport row");
    assert_eq!((declared.length, declared.fps), (2.0, 24.0));
    program.command("panel_page", json!({ "page": 3 }));
    program.send(HostMessage::Ping);
    let mut said = Vec::new();
    program.wait("pong", |message| {
        said.push(message.clone());
        matches!(message, AppMessage::Pong).then_some(())
    });
    assert!(!said.iter().any(|message| matches!(message, AppMessage::Cells { tab: own, cells, .. } if *own == tab && !cells.is_empty())), "a video opened to play was given a draft");
    let pages = said.iter().rev().find_map(|message| match message {
        AppMessage::Properties(properties) => Some(properties.pages.clone()),
        _ => None,
    });
    assert!(pages.is_some_and(|pages| !pages[3].enabled), "a video opened to play has its Prompt page on");
    program.command("transport_play", json!({ "value": true }));
    program.view("playing", |view, _| transport(view).is_some_and(|transport| transport.playing));
    program.command("transport_play", json!({ "value": false }));

    // Kept, it has no file: nothing was written in `work/`.
    program.send(HostMessage::Keep { tab: tab.clone() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab: kept, path, error } if *kept == tab => Some((path.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(kept, (None, None), "a video opened to play was kept as work");
    let in_work = || std::fs::read_dir(dir.join("work")).map(|entries| entries.count()).unwrap_or(0);
    assert_eq!(in_work(), 0, "opening a video wrote in work/");

    // Opened again: its tab, not a second one.
    program.send(HostMessage::Open { path: Some(first.display().to_string()) });
    program.send(HostMessage::Ping);
    let mut seen = Vec::new();
    program.wait("pong", |message| {
        seen.push(message.clone());
        matches!(message, AppMessage::Pong).then_some(())
    });
    assert!(!seen.iter().any(|message| matches!(message, AppMessage::Tab { .. } | AppMessage::Contents { .. })), "a second tab was made");
    assert!(seen.iter().all(|message| !matches!(message, AppMessage::Tabs(tabs) if tabs.tabs.len() != 1)), "a second tab was made");

    // Changed, it is kept in `work/` as any new tab is.
    program.command("film_rename", json!({ "take": 0, "value": "Morning" }));
    program.send(HostMessage::Keep { tab: tab.clone() });
    let kept = program.wait("kept after a change", |message| match message {
        AppMessage::Kept { tab: kept, path, .. } if *kept == tab => Some(path.clone()),
        _ => None,
    });
    let file = PathBuf::from(kept.expect("no file after a change"));
    assert_eq!(file, dir.join("work/Dawn.vgx"));
    assert!(vgx_header(&file).to_string().contains("\"takes/Dawn.mp4\""), "the take does not link its file");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(outside.parent().unwrap());
}

/// A take whose left half is one colour and right half another, 640 × 360,
/// so where its middle is drawn shows how it was zoomed.
fn write_halves(path: &Path) {
    let (width, height) = (640u32, 360u32);
    let settings = media::Settings { width, height, rate: (24, 1), keyframes: 12, sound: None };
    let mut encoder = media::Encoder::create(path, settings).expect("an encoder");
    let mut frame = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..height {
        for x in 0..width {
            let rgb = if x < width / 2 { LEFT_HALF } else { RIGHT_HALF };
            frame.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    for _ in 0..12 {
        encoder.frame(&frame).expect("a frame");
    }
    encoder.finish().expect("the video");
}

const LEFT_HALF: [u8; 3] = [200, 40, 40];
const RIGHT_HALF: [u8; 3] = [40, 60, 200];

/// Catches a view the workspace does not move as the host asks, or a frame
/// decoded at another size than it is drawn: fitted, the 640 × 360 take is
/// drawn whole in the middle of the 1200 × 800 area; a `view` gesture zooming
/// by 4 about the point a quarter of the way across and down keeps that
/// point there, so the take's middle is drawn at 1080; zooming by an eighth
/// about the middle draws it at 320 × 180 in the middle, decoded at that
/// size, with white around it.
#[test]
fn a_view_gesture_zooms_about_its_point() {
    let dir = scratch("view");
    let file = dir.join("takes/Halves.mp4");
    write_halves(&file);
    let near = |pixel: [u8; 3], colour: [u8; 3]| distance(pixel, colour) <= 30;
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&file);
    // Fitted at its own pixels: 280 to 920 across, 220 to 580 down.
    program.view("the take fitted", |_, image| near(image.at(300, 400), LEFT_HALF) && near(image.at(900, 400), RIGHT_HALF) && image.at(260, 400) == [255, 255, 255]);
    let view = |zoom_by: f64, at: [f64; 2]| HostMessage::RasterGesture { tab: "1".into(), gesture: protocol::RasterGesture::View { zoom_by, pan_by: [0.0, 0.0], at } };
    program.send(view(4.0, [440.0, 310.0]));
    program.view("zoomed by 4 about a quarter of the way", |_, image| {
        near(image.at(1060, 400), LEFT_HALF) && near(image.at(1100, 400), RIGHT_HALF) && near(image.at(440, 310), LEFT_HALF)
    });
    program.send(view(0.125, [600.0, 400.0]));
    // At 50%: 320 × 180, 440 to 760 across, 310 to 490 down.
    program.view("zoomed out to 50%", |_, image| {
        near(image.at(450, 400), LEFT_HALF) && near(image.at(750, 400), RIGHT_HALF) && image.at(420, 400) == [255, 255, 255] && image.at(600, 300) == [255, 255, 255]
    });
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Requests, as the Image Generator's: the Prompt page, rounds of takes,
// and a take's menu.

fn take_file(path: &str, canon: bool) -> protocol::TakeFile {
    protocol::TakeFile { path: path.into(), media_type: "video/mp4".into(), canon, ..Default::default() }
}

fn round(request: &str, number: u32, takes: Vec<protocol::TakeFile>) -> protocol::Round {
    protocol::Round {
        request: request.into(),
        number,
        model: "Model A".into(),
        model_name: String::new(),
        prompt: "The door opens slowly.".into(),
        references: Vec::new(),
        left_out: Vec::new(),
        state: protocol::RoundState::Done,
        started: Some(1_793_146_000),
        count: None,
        takes,
        error: None,
        report: None,
        artifact: None,
    }
}

/// A project with the two videos, and the program in it: tab 1's
/// cell, from the first `cells` that names one.
fn requests(label: &str) -> (PathBuf, Program, String) {
    let (dir, _, _) = project(label);
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    let cell = program.wait("the tab's cell", |message| match message {
        AppMessage::Cells { tab, selected: Some(cell), .. } if tab == "1" => Some(cell.clone()),
        _ => None,
    });
    (dir, program, cell)
}

fn send_takes(program: &mut Program, cell: &str, rounds: Vec<protocol::Round>) {
    let cell = protocol::ItemRef { path: "work/Untitled Video.vgx".into(), item: Some(cell.into()) };
    program.send(HostMessage::Takes { cell, rounds });
}

/// The next view whose filmstrip `pick` takes.
fn film_of(program: &Program, what: &str, mut pick: impl FnMut(&protocol::Film) -> bool) -> protocol::Film {
    program.wait(what, |message| match message {
        AppMessage::Raster(view) => view.film.clone().filter(|film| pick(film)),
        _ => None,
    })
}

fn runs(film: &protocol::Film) -> Vec<(String, usize)> {
    film.groups.iter().map(|group| (group.label.clone(), group.count)).collect()
}

/// The middle pixel of a thumbnail's raw file.
fn middle(thumbnail: &protocol::Thumbnail) -> [u8; 3] {
    let bytes = std::fs::read(&thumbnail.path).unwrap_or_default();
    let at = (((thumbnail.height / 2) * thumbnail.width + thumbnail.width / 2) * 4) as usize;
    bytes.get(at..at + 3).map_or([0, 0, 0], |rgb| [rgb[0], rgb[1], rgb[2]])
}

/// The header of a `.vgx` file: its line of JSON.
fn vgx_header(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(text.strip_prefix("VGX1\n").unwrap().lines().next().unwrap()).unwrap()
}

/// Catches takes kept in the workspace's file, or a strip that does not
/// follow the requests' records: `takes` fills the strip with each round
/// under "Round N · model", each take its first frame with its length, a
/// canon take sealed, a round being made as its waiting places; and the
/// tab's file, in the project's `work/` folder, names no round.
#[test]
fn takes_fill_the_strip_by_round_and_are_never_kept() {
    let (dir, mut program, cell) = requests("takes");
    let running = protocol::Round { state: protocol::RoundState::Running, count: Some(2), ..round("q2", 2, Vec::new()) };
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false), take_file("takes/Dusk.mp4", true)]), running]);
    let film = film_of(&program, "the rounds with their thumbnails", |film| {
        film.groups.len() == 2 && film.items.len() == 4 && !film.items[0].thumbnail.path.is_empty() && !film.items[1].thumbnail.path.is_empty()
    });
    assert_eq!(runs(&film), [("Round 1 · Model A".to_string(), 2), ("Round 2 · Model A".to_string(), 2)]);
    assert_eq!((film.items[0].badge.as_str(), film.items[1].badge.as_str()), ("2.0 s", "0.5 s"));
    assert_eq!((film.items[0].mark.as_deref(), film.items[1].mark.as_deref()), (None, Some("badge-check")));
    assert!(matches!(film.items[2].pending, Some(protocol::FilmPending::Running { since: Some(1_793_146_000) })));
    assert!(is(middle(&film.items[0].thumbnail), dawn, DAWN_FRAMES, 0), "the thumbnail is not the take's first frame");
    let menu: Vec<&str> = film.items[0].menu.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(menu, ["Next round from this", "Use this take", "Retime…", "Upscale…", "Edit", "Deliver…", "Open report"]);
    // No round names its artifact here, so Deliver… is off.
    assert!(!film.items[0].menu[5].enabled);

    // A click shows the take, chosen on the strip.
    program.command("film_select", json!({ "value": 1 }));
    let (view, _) = program.view("the take shown", |view, image| {
        view.film.as_ref().is_some_and(|film| film.shown == 1 && film.items[1].chosen) && is(image.at(ALONE.0, ALONE.1), dusk, DUSK_FRAMES, 0)
    });
    assert!(!view.film.unwrap().items[0].chosen);

    // Edit puts the take in the gallery, so the tab has something to keep.
    program.command("take_edit", json!({ "take": "takes/Dusk.mp4" }));
    film_of(&program, "the take edited into the gallery", |film| film.groups.last().is_some_and(|group| group.label.is_empty() && group.count == 1));
    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => Some(path.clone()),
        _ => None,
    });
    let file = PathBuf::from(kept.expect("no file"));
    assert_eq!(file, dir.join("work/Untitled Video.vgx"), "a project's video is not in work/");
    let header = vgx_header(&file).to_string();
    assert!(!header.contains("\"q1\"") && !header.contains("Model A") && !header.contains("Dawn"), "a round was kept in the file: {header}");
    assert_eq!(vgx_header(&file)["cell"], json!(cell));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a strip that keeps a take the requests' records no longer
/// have: sent again without a take, as when its report is deleted, its
/// tile leaves, and with one added, it appears.
#[test]
fn a_takes_report_gone_makes_its_tile_leave() {
    let (dir, mut program, cell) = requests("report-gone");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false), take_file("takes/Dusk.mp4", false)])]);
    film_of(&program, "two takes", |film| film.groups.first().is_some_and(|group| group.count == 2));
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dusk.mp4", false)])]);
    let film = film_of(&program, "one take", |film| film.groups.first().is_some_and(|group| group.count == 1));
    assert_eq!(film.items[0].badge, "0.5 s", "the take left is not Dusk");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dusk.mp4", false), take_file("takes/Dawn.mp4", false)])]);
    film_of(&program, "a take added", |film| film.groups.first().is_some_and(|group| group.count == 2));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a next round that loses the round or the take: Next round from
/// this makes the tab's cell a new draft of the round's prompt, its
/// references in order with their roles, and the take as Reference, with
/// a Length.
#[test]
fn next_round_from_a_take_makes_a_draft() {
    let (dir, mut program, cell) = requests("next-round");
    let mut first = round("q1", 1, vec![take_file("takes/Dawn.mp4", false)]);
    first.references = vec![protocol::SentReference { path: "takes/Dusk.mp4".into(), role: Some("Start frame, the door".into()) }];
    send_takes(&mut program, &cell, vec![first]);
    film_of(&program, "the round", |film| !film.groups.is_empty());
    program.command("take_next_round", json!({ "take": "takes/Dawn.mp4" }));
    let draft = program.wait("the draft", |message| match message {
        AppMessage::Cells { cells, selected: Some(selected), .. } => cells.iter().find(|own| own.id == *selected && own.id != cell).cloned(),
        _ => None,
    });
    assert!(draft.step.is_none() && draft.kind == protocol::VIDEO_KIND);
    assert_eq!(draft.prompt(), "The door opens slowly.");
    let paths: Vec<(String, Option<String>)> = draft
        .input
        .iter()
        .filter_map(|part| match part {
            protocol::CellPart::Path { path, role, .. } => Some((path.clone(), role.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        paths,
        [("takes/Dusk.mp4".to_string(), Some("Start frame, the door".to_string())), ("takes/Dawn.mp4".to_string(), Some("Reference".to_string()))]
    );
    assert_eq!(draft.settings.as_ref().and_then(|settings| settings["length"].as_f64()), Some(8.0));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a draft kept after it became a step, or a step's artifact kept
/// in the file: `cell_became_step` leaves the cell its pointer alone, and
/// the file keeps that pointer and nothing of the draft or the artifact.
#[test]
fn a_draft_becomes_its_step() {
    let (dir, mut program, cell) = requests("step");
    let words = "The door opens slowly.";
    program.send(HostMessage::EditCell {
        tab: "1".into(),
        cell: cell.clone(),
        input: vec![protocol::CellPart::Text { text: words.into() }],
        settings: None,
        endpoint: None,
        model: Some("a model".into()),
    });
    program.wait("the edit", |message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().any(|own| own.prompt() == words).then_some(()),
        _ => None,
    });
    let step = protocol::StepRef { chain: "shot-12".into(), step: "take".into() };
    program.send(HostMessage::CellBecameStep { tab: "1".into(), cell: cell.clone(), step: step.clone(), artifact: Some("door: take".into()) });
    let pointed = program.wait("the pointer", |message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().find(|own| own.id == cell && own.step.is_some()).cloned(),
        _ => None,
    });
    assert_eq!(pointed.step, Some(step));
    assert!(!pointed.copies_its_step(), "the step's cell kept its draft");
    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => Some(path.clone()),
        _ => None,
    });
    let header = vgx_header(Path::new(&kept.expect("no file")));
    assert_eq!(header["cells"][0]["step"]["chain"], "shot-12", "{header}");
    let text = header.to_string();
    assert!(!text.contains(words) && !text.contains("a model") && !text.contains("door: take"), "{text}");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a take's report that cannot be opened: Open report asks the
/// host for the report of the take's round.
#[test]
fn open_report_asks_for_the_rounds_report() {
    let (dir, mut program, cell) = requests("report");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false)])]);
    let film = film_of(&program, "the round", |film| !film.groups.is_empty());
    let report = film.items[0].menu.iter().find(|row| row.label == "Open report").expect("no Open report");
    assert!(report.enabled);
    program.command("take_report", report.action.as_ref().unwrap().arg.clone().unwrap());
    let request = program.wait("the report", |message| match message {
        AppMessage::OpenReport { request } => Some(request.clone()),
        _ => None,
    });
    assert_eq!(request, "q1");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a Prompt page that drifts from vg-request-inputs and the guide:
/// the panel's fourth page, Prompt, names the tab's cell, a video draft,
/// with the roles Start frame and End frame, each a row of its own, then
/// Reference, Motion reference, and Sound; images, videos, and sounds to
/// add; and Size in words, Length from 1 to 20 seconds, and Seed, plain
/// fields.
#[test]
fn the_prompt_page_is_declared_with_its_roles_and_settings() {
    let (dir, program, cell) = requests("prompt");
    let properties = program.wait("the Prompt page", |message| match message {
        AppMessage::Properties(properties) if properties.cell.is_some() => Some(properties.clone()),
        _ => None,
    });
    let pages: Vec<(&str, bool)> = properties.pages.iter().map(|page| (page.label.as_str(), page.active)).collect();
    assert_eq!(pages, [("Layers", false), ("Properties", false), ("Adjustments", false), ("Prompt", true)]);
    let panel = properties.cell.unwrap();
    assert_eq!(panel.cell, cell);
    assert_eq!(panel.hint, "Describe the shot…");
    let roles: Vec<(&str, bool)> = panel.roles.iter().map(|role| (role.label.as_str(), role.slot)).collect();
    assert_eq!(roles, [("Start frame", true), ("End frame", true), ("Reference", false), ("Motion reference", false), ("Sound", false)]);
    assert_eq!(
        panel.reference_types,
        ["image/png", "image/jpeg", "image/webp", "video/mp4", "video/quicktime", "audio/wav", "audio/mpeg", "audio/flac"]
    );
    let settings: Vec<(&str, &str)> = panel.settings.iter().map(|setting| (setting.key.as_str(), setting.label.as_str())).collect();
    assert_eq!(settings, [("size", "Size"), ("length", "Length"), ("seed", "Seed")]);
    assert!(matches!(panel.settings[0].kind, protocol::RowKind::Text));
    assert!(matches!(panel.settings[1].kind, protocol::RowKind::Field { min, max, .. } if min == 1.0 && max == 20.0));
    assert!(matches!(panel.settings[2].kind, protocol::RowKind::Field { .. }));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Playing a take with its sound, through the transport row.

/// A take with sound: one second, 24 frames of dawn's first colour, and a
/// tone.
fn write_sounding_video(path: &Path) {
    let settings = media::Settings { width: SIZE.0, height: SIZE.1, rate: (24, 1), keyframes: 12, sound: Some((48_000, 2)) };
    let mut encoder = media::Encoder::create(path, settings).expect("an encoder");
    let rgb = dawn(0);
    let frame = [rgb[0], rgb[1], rgb[2], 255].repeat((SIZE.0 * SIZE.1) as usize);
    let tone: Vec<f32> = (0..48_000).flat_map(|i| {
        let sample = 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
        [sample, sample]
    })
    .collect();
    for i in 0..24 {
        encoder.frame(&frame).expect("a frame");
        encoder.sound(&tone[i * 4000..(i + 1) * 4000]).expect("its sound");
    }
    encoder.finish().expect("the video");
}

/// The transport the view declares.
fn transport(view: &RasterView) -> Option<protocol::Transport> {
    view.transport.clone()
}

/// Catches playing that follows no clock, a pause that loses its place, a
/// step or a scrub that lands elsewhere, and a take with no sound that
/// declares a waveform. With `INPAINTER_NO_SOUND`, the program's Player is
/// silent and its clock the wall's: played for a while and paused, the
/// frame has moved on from 0 and the image is that frame; a step moves one
/// frame; a scrub to one second shows frame 24; and the take, which has no
/// sound, has no row.
#[test]
fn a_take_plays_pauses_steps_and_scrubs() {
    let (dir, first, _) = project("play");
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&first);
    let (view, _) = program.view("the take at frame 0", |view, image| {
        transport(view).is_some_and(|transport| transport.time == 0.0) && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0)
    });
    let declared = transport(&view).unwrap();
    assert_eq!((declared.length, declared.fps, declared.playing), (2.0, 24.0, false));
    assert_eq!(declared.clock, protocol::TransportClock::Workspace);
    assert_eq!((declared.play.id.as_str(), declared.step.id.as_str()), ("transport_play", "transport_step"));

    program.command("transport_play", json!({ "value": true }));
    program.view("playing", |view, _| transport(view).is_some_and(|transport| transport.playing));
    std::thread::sleep(Duration::from_millis(600));
    program.command("transport_play", json!({ "value": false }));
    let (paused, _) = program.view("paused past frame 0", |view, image| {
        transport(view).is_some_and(|transport| {
            let frame = (transport.time * 24.0).round() as u32;
            !transport.playing && frame > 0 && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, frame)
        })
    });
    let frame = (transport(&paused).unwrap().time * 24.0).round() as u32;
    assert!(frame < DAWN_FRAMES, "playing ran past the take");

    program.command("transport_step", json!({ "value": 1 }));
    program.view("a step on", |view, image| {
        transport(view).is_some_and(|transport| (transport.time * 24.0).round() as u32 == frame + 1) && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, frame + 1)
    });
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 1.0 });
    let (scrubbed, _) = program.view("scrubbed to one second", |view, image| {
        transport(view).is_some_and(|transport| (transport.time - 1.0).abs() < 1e-6) && is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 24)
    });
    assert!(transport(&scrubbed).unwrap().rows.is_empty(), "a take with no sound has a waveform");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches two takes side by side at different times, or at one frame
/// number rather than one time: a scrub to a quarter second shows the
/// 24 frames a second take's frame 6 and the 30 frames a second take's
/// frame 8, each its own frame at that time.
#[test]
fn side_by_side_shows_one_time() {
    let (dir, _, second) = project("play-side");
    let first = dir.join("takes/Dawn30.mp4");
    write_video_at(&first, DAWN_FRAMES, dawn, 30);
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&first);
    program.open_take(&second);
    program.view("the second take", |view, _| names(view).len() == 2);
    // A Ctrl-click on the first: side by side, the second shown first.
    program.command("film_select", json!({ "value": 0, "extend": true }));
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 0.25 });
    program.view("both at a quarter second", |view, image| {
        transport(view).is_some_and(|transport| (transport.time - 0.25).abs() < 1e-6)
            && is(image.at(LEFT.0, LEFT.1), dusk, DUSK_FRAMES, 6)
            && is(image.at(RIGHT.0, RIGHT.1), dawn, DAWN_FRAMES, 8)
    });
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a take with sound that shows no waveform: its sound, decoded
/// off the loop, becomes one row, "Sound", an image the working area's
/// width.
#[test]
fn a_take_with_sound_has_its_waveform() {
    let dir = scratch("waveform");
    let file = dir.join("takes/Tone.mp4");
    write_sounding_video(&file);
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&file);
    let (view, _) = program.view("the waveform", |view, _| transport(view).is_some_and(|transport| !transport.rows.is_empty()));
    let rows = transport(&view).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "Sound");
    assert_eq!(rows[0].image.width, VIEWPORT[0] as u32);
    let bytes = std::fs::read(&rows[0].image.path).unwrap();
    assert_eq!(bytes.len(), (rows[0].image.width * rows[0].image.height * 4) as usize);
    assert!(bytes.chunks_exact(4).any(|pixel| pixel[3] > 0), "the waveform is empty");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Notes at a time, as vg-notes-at-time shows them.

const RED: [u8; 3] = [226, 36, 36];
/// A point of the pin's red disc put at the take's middle, below its
/// white number.
const PIN: (u32, u32) = (ALONE.0, ALONE.1 + 6);

fn gesture(program: &mut Program, gesture: protocol::RasterGesture) {
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture });
}

fn text(program: &mut Program, input: protocol::TextInput) {
    program.send(HostMessage::RasterText { tab: "1".into(), input });
}

/// The transport's cues, as (time, label, colour).
fn cues(view: &RasterView) -> Vec<(f64, String, Option<[u8; 3]>)> {
    transport(view).map(|transport| transport.cues.iter().map(|cue| (cue.at, cue.label.clone(), cue.colour)).collect()).unwrap_or_default()
}

/// A program in a project with Dawn shown alone at one second, frame 24.
fn at_one_second(label: &str) -> (PathBuf, PathBuf, Program) {
    let (dir, first, _) = project(label);
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&first);
    program.view("the take", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 1.0 });
    program.view("one second", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 24));
    (dir, first, program)
}

/// Pins a note with `words` at the take's middle, with the notes tool's
/// Pin, and waits for its cue.
fn pin(program: &mut Program, words: &str) {
    program.command("tool", json!({ "tool": "notes" }));
    gesture(program, protocol::RasterGesture::Click { at: [f64::from(ALONE.0), f64::from(ALONE.1)] });
    program.view("the pin's words typed in", |view, _| view.text.is_some());
    text(program, protocol::TextInput::Insert { text: words.into() });
    text(program, protocol::TextInput::Key { key: protocol::TextKey::Escape, shift: false, word: false });
    program.view("the pin's cue", |view, _| view.text.is_none() && cues(view).len() == 1);
}

/// Catches a note that belongs to no moment, or to every moment: a pin put
/// at one second is a cue there, numbered, in the notes' red, and drawn
/// over the frame there and within half a second, but not at the start;
/// and undo and redo take it away and put it back.
#[test]
fn a_pin_at_a_time_is_a_cue_and_shows_near_it() {
    let (dir, _, mut program) = at_one_second("notes-pin");
    program.command("tool", json!({ "tool": "notes" }));
    gesture(&mut program, protocol::RasterGesture::Click { at: [f64::from(ALONE.0), f64::from(ALONE.1)] });
    let (typing, _) = program.view("the pin's words typed in", |view, image| view.text.is_some() && distance(image.at(PIN.0, PIN.1), RED) <= 6);
    assert!(matches!(typing.tool, protocol::RasterTool::Text), "the notes tool's Pin is not the text tool's pointer");
    text(&mut program, protocol::TextInput::Insert { text: "less rain".into() });
    text(&mut program, protocol::TextInput::Key { key: protocol::TextKey::Escape, shift: false, word: false });
    let (view, _) = program.view("the pin, a cue at one second", |view, image| {
        view.text.is_none() && cues(view).len() == 1 && distance(image.at(PIN.0, PIN.1), RED) <= 6
    });
    assert_eq!(cues(&view), [(1.0, "1 — 1.0 s".to_string(), Some(RED))]);

    // Within half a second it shows; at the start it does not, and its cue
    // stays on the scrubber.
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 1.25 });
    program.view("the pin a quarter second on", |view, image| {
        transport(view).is_some_and(|transport| (transport.time - 1.25).abs() < 1e-6) && distance(image.at(PIN.0, PIN.1), RED) <= 6
    });
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 0.0 });
    let (start, _) = program.view("no pin at the start", |_, image| is(image.at(PIN.0, PIN.1), dawn, DAWN_FRAMES, 0));
    assert_eq!(cues(&start).len(), 1, "the cue left the scrubber");

    program.command("undo", json!({}));
    program.view("the pin undone", |view, _| cues(view).is_empty());
    program.command("redo", json!({}));
    let (redone, _) = program.view("the pin redone", |view, _| cues(view).len() == 1);
    assert_eq!(cues(&redone)[0].1, "1 — 1.0 s");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a mark that is not drawn as it comes, or not kept: the notes
/// tool's Draw shows the mark part by part, the finished mark is a step
/// and no cue, and undo takes it away.
#[test]
fn a_mark_is_drawn_part_by_part() {
    let (dir, _, mut program) = at_one_second("notes-mark");
    program.command("tool", json!({ "tool": "notes" }));
    program.command("notes_mode", json!({ "value": 1 }));
    let (view, _) = program.view("the brush's pointer", |view, _| matches!(view.tool, protocol::RasterTool::Brush { .. }));
    let protocol::RasterTool::Brush { rgb, .. } = view.tool else { unreachable!() };
    assert_eq!(rgb, RED);
    let y = f64::from(ALONE.1 + 20);
    let points = [[560.0, y, 1.0], [600.0, y, 1.0], [640.0, y, 1.0]];
    gesture(&mut program, protocol::RasterGesture::StrokePart { from: 0, points: points[..2].to_vec() });
    program.view("the mark's first part", |_, image| distance(image.at(580, ALONE.1 + 20), RED) <= 6);
    gesture(&mut program, protocol::RasterGesture::StrokePart { from: 2, points: points[2..].to_vec() });
    gesture(&mut program, protocol::RasterGesture::Stroke { points: points.to_vec() });
    let (done, _) = program.view("the mark", |_, image| distance(image.at(620, ALONE.1 + 20), RED) <= 6);
    assert!(cues(&done).is_empty(), "a mark is a cue");
    program.command("undo", json!({}));
    program.view("the mark undone", |_, image| is(image.at(620, ALONE.1 + 20), dawn, DAWN_FRAMES, 24));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches notes kept with the gallery rather than the take, or lost: a
/// pin's notes are kept in the `.vgx` as `take_notes`, by the take's path
/// and sha256, with its time and words, and come back as its cue when a
/// new program opens the file.
#[test]
fn notes_are_kept_by_take_through_a_restart() {
    let (dir, first, mut program) = at_one_second("notes-kept");
    pin(&mut program, "less rain");
    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => path.clone(),
        _ => None,
    });
    let header = vgx_header(Path::new(&kept));
    let notes = &header["take_notes"][0];
    assert_eq!(notes["path"], "takes/Dawn.mp4");
    assert_eq!(notes["sha256"].as_str(), Some(layers::link::sha256_file(&first).unwrap().as_str()));
    assert_eq!(notes["notes"]["pins"][0]["text"], "less rain");
    assert_eq!(notes["notes"]["pins"][0]["at"], 1.0);
    program.send(HostMessage::Shutdown);
    drop(program);

    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some(kept) });
    let (view, _) = program.view("the pin's cue again", |view, _| !cues(view).is_empty());
    assert_eq!(cues(&view), [(1.0, "1 — 1.0 s".to_string(), Some(RED))]);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The draft `cell`'s parts, from the next `cells` that has it and that
/// `pick` takes.
fn draft_of(program: &Program, what: &str, mut pick: impl FnMut(&protocol::Cell) -> bool) -> protocol::Cell {
    program.wait(what, |message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().find(|own| pick(own)).cloned(),
        _ => None,
    })
}

/// A draft's path with `role`, when it has one.
fn path_with_role(cell: &protocol::Cell, role: &str) -> Option<(String, String)> {
    cell.input.iter().find_map(|part| match part {
        protocol::CellPart::Path { path, media_type, role: Some(own), .. } if own == role => Some((path.clone(), media_type.clone())),
        _ => None,
    })
}

/// Catches a next round that drops a take's notes or their times, or
/// sends a model a file that lives only in `work/`: the prompt ends with
/// the pins' words, each with its time; the frame at the first note's
/// time, marked, is written beside the tab's file in `work/` as a PNG of
/// the take's size with the pin on it, and filed from the tab's file and
/// the take; once `filed` names it, the draft has the filed file with the
/// role "Notes on the last round", and keeps it through a restart.
#[test]
fn next_round_carries_the_notes_and_the_marked_frame() {
    let (dir, mut program, cell) = requests("notes-next");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false)])]);
    film_of(&program, "the round", |film| !film.groups.is_empty());
    program.command("film_select", json!({ "value": 0 }));
    program.view("the round's take", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    program.send(HostMessage::TransportTime { tab: "1".into(), time: 1.0 });
    program.view("one second", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 24));
    pin(&mut program, "less rain");

    program.command("take_next_round", json!({ "take": "takes/Dawn.mp4" }));
    let (filing, path, from) = program.wait("the marked frame filed", |message| match message {
        AppMessage::FileResource { path, from, id: Some(id), .. } => Some((id.clone(), path.clone(), from.clone())),
        _ => None,
    });
    assert_eq!(path, "work/Untitled Video notes.png");
    assert_eq!(from, ["work/Untitled Video.vgx", "takes/Dawn.mp4"]);
    let png = image::load_from_memory(&std::fs::read(dir.join(&path)).unwrap()).unwrap().to_rgba8();
    assert_eq!(png.dimensions(), SIZE, "the marked frame is not the take's size");
    let pixel = |x: u32, y: u32| {
        let rgba = png.get_pixel(x, y).0;
        [rgba[0], rgba[1], rgba[2]]
    };
    assert!(distance(pixel(SIZE.0 / 2, SIZE.1 / 2 + 6), RED) <= 6, "the pin is not on the marked frame");
    assert!(is(pixel(10, 10), dawn, DAWN_FRAMES, 24), "the marked frame is not the frame at the note's time");
    let draft = draft_of(&program, "the draft", |own| own.id != cell && own.step.is_none() && own.prompt().contains("Notes on the last round"));
    assert_eq!(draft.prompt(), "The door opens slowly. Notes on the last round: 1. less rain (1.0 s)");
    assert_eq!(path_with_role(&draft, "Notes on the last round"), None, "a file only in work/ is in the draft");

    program.send(HostMessage::Filed { id: filing, file: "library/notes-1.png".into(), error: None });
    let filed = draft_of(&program, "the filed marked frame in the draft", |own| own.id == draft.id && path_with_role(own, "Notes on the last round").is_some());
    assert_eq!(path_with_role(&filed, "Notes on the last round"), Some(("library/notes-1.png".to_string(), "image/png".to_string())));

    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => path.clone(),
        _ => None,
    });
    program.send(HostMessage::Shutdown);
    drop(program);
    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some(kept) });
    let again = draft_of(&program, "the draft after a restart", |own| own.id == draft.id);
    assert_eq!(path_with_role(&again, "Notes on the last round").map(|(path, _)| path).as_deref(), Some("library/notes-1.png"));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// The shot's animatic, as vg-frame-from and vg-take-over-animatic show it.

const ANIMATIC_FRAMES: u32 = 12;
const LIGHT: u8 = 235;
const DARK_LINE: u8 = 20;

/// Where frame `i` of the animatic has its dark bar, six pixels wide:
/// eight further right each frame.
fn bar(i: u32) -> u32 {
    8 + 8 * i
}

/// An animatic: a light frame with one dark bar, which moves each frame.
fn write_animatic(path: &Path) {
    let settings = media::Settings { width: SIZE.0, height: SIZE.1, rate: (24, 1), keyframes: 12, sound: None };
    let mut encoder = media::Encoder::create(path, settings).expect("an encoder");
    for i in 0..ANIMATIC_FRAMES {
        let mut rgba = Vec::with_capacity((SIZE.0 * SIZE.1 * 4) as usize);
        for _ in 0..SIZE.1 {
            for x in 0..SIZE.0 {
                let value = if (bar(i)..bar(i) + 6).contains(&x) { DARK_LINE } else { LIGHT };
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        encoder.frame(&rgba).expect("a frame");
    }
    encoder.finish().expect("the animatic");
}

/// The next reference menu the Prompt page declares that `pick` takes.
fn reference_menu(program: &Program, what: &str, mut pick: impl FnMut(&[protocol::MenuEntry]) -> bool) -> Vec<protocol::MenuEntry> {
    program.wait(what, |message| match message {
        AppMessage::Properties(properties) => properties.cell.as_ref().map(|cell| cell.reference_menu.clone()).filter(|rows| pick(rows)),
        _ => None,
    })
}

/// The pre-vis check's button on a toolbar: whether it is on, and chosen.
fn previs_button(toolbar: &protocol::Toolbar) -> Option<(bool, bool)> {
    toolbar.trailing.iter().find_map(|item| match item {
        protocol::ToolItem::Button { icon, enabled, selected, .. } if icon == "cube" => Some((*enabled, *selected)),
        _ => None,
    })
}

/// Catches a Start frame taken from somewhere other than the animatic at
/// the playhead, or sent unfiled: with no Motion reference, the row is off
/// and says why; with one, the row writes the animatic's frame at the time
/// shown beside the tab's file in `work/`, its bar where frame 6 has it,
/// and files it from the tab's file and the animatic; once `filed` names
/// it, the filed file is the draft's Start frame; and Open in the Image
/// Generator opens that file, and no video.
#[test]
fn the_start_frame_is_the_animatics_frame_at_the_playhead() {
    let (dir, mut program, cell) = requests("animatic-frame");
    write_animatic(&dir.join("takes/Animatic.mp4"));
    let rows = reference_menu(&program, "the row off", |rows| rows.first().is_some_and(|row| !row.enabled));
    assert_eq!(rows[0].label, "The animatic's frame at the playhead");
    assert_eq!(rows[0].tip.as_deref(), Some("Add the shot's animatic as a Motion reference first."));
    assert_eq!(rows.get(1).map(|row| (row.label.as_str(), row.enabled)), Some(("Open in the Image Generator", true)));

    program.send(HostMessage::EditCell {
        tab: "1".into(),
        cell: cell.clone(),
        input: vec![
            protocol::CellPart::Text { text: "The keeper walks in.".into() },
            protocol::CellPart::Path { path: "takes/Animatic.mp4".into(), media_type: "video/mp4".into(), role: Some("Motion reference".into()), field: None },
        ],
        settings: None,
        endpoint: None,
        model: None,
    });
    let rows = reference_menu(&program, "the row on", |rows| rows.first().is_some_and(|row| row.enabled));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].tip, None);

    // The playhead at a quarter second: the take's frame 6.
    program.open_take(&dir.join("takes/Dawn.mp4"));
    program.view("the take", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    program.command("frame", json!({ "step": 6 }));
    program.view("frame 6", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 6));
    program.command("frame_from_animatic", json!({ "value": 0 }));
    let (filing, path, from) = program.wait("the Start frame filed", |message| match message {
        AppMessage::FileResource { path, from, id: Some(id), .. } => Some((id.clone(), path.clone(), from.clone())),
        _ => None,
    });
    assert_eq!(path, "work/Untitled Video start frame.png");
    assert_eq!(from, ["work/Untitled Video.vgx", "takes/Animatic.mp4"]);
    let png = image::load_from_memory(&std::fs::read(dir.join(&path)).unwrap()).unwrap().to_rgba8();
    assert_eq!(png.dimensions(), SIZE);
    let grey = |x: u32| png.get_pixel(x, SIZE.1 / 2).0[1];
    assert!(grey(bar(6) + 3) < 80, "the frame at the playhead has no bar where frame 6 has it");
    assert!(grey(bar(0) + 3) > 200, "the frame is the animatic's first, not the one at the playhead");
    program.send(HostMessage::Filed { id: filing, file: "library/start-1.png".into(), error: None });
    let draft = draft_of(&program, "the filed Start frame in the draft", |own| own.id == cell && path_with_role(own, "Start frame").is_some());
    let path = path_with_role(&draft, "Start frame").map(|(path, _)| path).unwrap();
    assert_eq!(path, "library/start-1.png", "the Start frame is not the filed file");

    // Open in the Image Generator: the Start frame, the first reference,
    // as its filed file; the Motion reference, a video, opens nothing.
    program.command("frame_open_image", json!({ "value": 0 }));
    let opened = program.wait("the Start frame opened", |message| match message {
        AppMessage::OpenIn { path, media_type } => Some((path.clone(), media_type.clone())),
        _ => None,
    });
    assert_eq!(opened, (path.clone(), String::from("image/png")));
    program.command("frame_open_image", json!({ "value": 1 }));
    program.send(HostMessage::Ping);
    program.wait("nothing opened for a video", |message| match message {
        AppMessage::OpenIn { .. } => panic!("a video reference opened in the Image Generator"),
        AppMessage::Pong => Some(()),
        _ => None,
    });
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a take shown over the wrong frame of its animatic, or without
/// it: the pre-vis check draws the animatic's bar at the take's time in
/// white, with cyan beside it where its edge is, and nothing where an
/// earlier frame had its bar.
#[test]
fn a_take_is_shown_over_its_animatic() {
    let (dir, mut program, cell) = requests("animatic-over");
    write_animatic(&dir.join("takes/Animatic.mp4"));
    let mut first = round("q1", 1, vec![take_file("takes/Dawn.mp4", false)]);
    first.references = vec![protocol::SentReference { path: "takes/Animatic.mp4".into(), role: Some("Motion reference, the walk".into()) }];
    send_takes(&mut program, &cell, vec![first]);
    film_of(&program, "the round", |film| !film.groups.is_empty());
    program.command("film_select", json!({ "value": 0 }));
    // The toolbar comes with the command, before the take's frame does.
    let button = program.wait("the pre-vis check on offer", |message| match message {
        AppMessage::Toolbar(toolbar) => previs_button(toolbar).filter(|(enabled, _)| *enabled),
        _ => None,
    });
    assert_eq!(button, (true, false));
    program.view("the round's take", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    program.command("frame", json!({ "step": 6 }));
    program.view("frame 6", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 6));

    program.command("previs_toggle", json!({}));
    // The toolbar comes with the command, before the frames do.
    program.wait("the pre-vis check chosen", |message| match message {
        AppMessage::Toolbar(toolbar) => previs_button(toolbar).filter(|button| *button == (true, true)),
        _ => None,
    });
    // The take's frame is 128 × 72 at its own pixels: its left edge, and
    // its middle row.
    let (left, row) = (ALONE.0 - SIZE.0 / 2, ALONE.1);
    let white = |pixel: [u8; 3]| pixel.iter().all(|value| *value > 230);
    let cyan = |pixel: [u8; 3]| pixel[0] < 90 && pixel[2] > 200;
    program.view("the take over its animatic", |_, image| {
        let x = left + bar(6);
        (x..x + 6).any(|x| white(image.at(x, row)))
            && (x - 4..x + 1).any(|x| cyan(image.at(x, row)))
            && !(left + bar(0)..left + bar(0) + 6).any(|x| white(image.at(x, row)))
    });
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches the pre-vis check offered with nothing to lay under a take: a
/// round with no references has the button off.
#[test]
fn the_previs_check_is_off_with_no_references() {
    let (dir, mut program, cell) = requests("animatic-none");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false)])]);
    film_of(&program, "the round", |film| !film.groups.is_empty());
    program.command("film_select", json!({ "value": 0 }));
    // The view with the take chosen, then the toolbar the same change
    // declares.
    film_of(&program, "the round's take chosen", |film| film.items.first().is_some_and(|item| item.chosen));
    let button = program.wait("the toolbar", |message| match message {
        AppMessage::Toolbar(toolbar) => previs_button(toolbar),
        _ => None,
    });
    assert_eq!(button, (false, false));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// A round's takes shown together, as vg-frame-strips and vg-takes-compare
// show them.

/// The toolbar button with `icon` on a toolbar: whether it is on, and
/// chosen.
fn toolbar_button(toolbar: &protocol::Toolbar, icon: &str) -> Option<(bool, bool)> {
    toolbar.items.iter().chain(&toolbar.trailing).find_map(|item| match item {
        protocol::ToolItem::Button { icon: own, enabled, selected, .. } if own == icon => Some((*enabled, *selected)),
        _ => None,
    })
}

/// Where a round's strips are drawn in a 1200 × 800 area with its line
/// and a references column: the line takes 30 at the top, the column 192
/// and 32 beside it at the left, so eight cells of 105 × 59 with 8
/// between run from x 264, each row 85 below the last from y 70. The
/// middle of row `row`, column `column`.
fn round_strip(row: u32, column: u32) -> (u32, u32) {
    (264 + 113 * column + 52, 70 + 85 * row + 29)
}

/// Catches a round that cannot be shown whole, or whose view is kept or
/// becomes a step: in a tab with only rounds, the grid button on a round's
/// take shows each take of its round as a row of frames, with the round's
/// line above and its references at the left; the take stays chosen, the
/// gallery has no step to undo, and the grid off shows the take again.
#[test]
fn a_round_is_shown_whole_as_frame_strips() {
    let (dir, mut program, cell) = requests("round-strips");
    let mut first = round("q1", 1, vec![take_file("takes/Dawn.mp4", false), take_file("takes/Dusk.mp4", false)]);
    first.references = vec![protocol::SentReference { path: "takes/Dusk.mp4".into(), role: Some("Start frame, the door".into()) }];
    send_takes(&mut program, &cell, vec![first]);
    film_of(&program, "the round", |film| !film.groups.is_empty() && film.items.len() == 2);
    program.command("film_select", json!({ "value": 0 }));
    let grid = program.wait("the grid on offer", |message| match message {
        AppMessage::Toolbar(toolbar) => toolbar_button(toolbar, "view-grid").filter(|(enabled, _)| *enabled),
        _ => None,
    });
    assert_eq!(grid, (true, false), "the grid is off in a tab with only rounds");
    program.view("the round's take", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));

    program.command("view_strips", json!({}));
    let toolbar = program.wait("the grid chosen", |message| match message {
        AppMessage::Toolbar(toolbar) => toolbar_button(toolbar, "view-grid").filter(|(_, selected)| *selected).map(|_| toolbar.clone()),
        _ => None,
    });
    assert_eq!(toolbar_button(&toolbar, "undo"), Some((false, false)), "showing a round whole is a step");
    let (view, image) = program.view("the round whole", |_, image| {
        let (first, last) = (round_strip(0, 7), round_strip(1, 7));
        is(image.at(first.0, first.1), dawn, DAWN_FRAMES, 47) && is(image.at(last.0, last.1), dusk, DUSK_FRAMES, 11)
    });
    assert!(view.film.as_ref().is_some_and(|film| film.items[0].chosen), "the round's take is no longer chosen");
    let dark = |x: std::ops::Range<u32>, y: std::ops::Range<u32>| x.flat_map(|x| y.clone().map(move |y| (x, y))).any(|(x, y)| image.at(x, y).iter().all(|value| *value < 170));
    assert!(dark(40..300, 40..60), "the round's line is not over its strips");
    assert!(dark(40..230, 70..90), "the round's references are not at the left");
    let first = round_strip(0, 0);
    assert!(is(image.at(first.0, first.1), dawn, DAWN_FRAMES, 5), "the round's first strip does not start at its eighth");

    program.command("view_strips", json!({}));
    program.view("the take again", |_, image| is(image.at(ALONE.0, ALONE.1), dawn, DAWN_FRAMES, 0));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a round's take that cannot be compared, or a comparison kept:
/// a Ctrl-click shows a take of another round beside the round's take
/// shown, both from one time, and a take of the gallery beside it too,
/// each chosen on the strip; and the tab's file keeps its own view, one
/// take, with nothing of the rounds.
#[test]
fn a_rounds_takes_are_shown_side_by_side() {
    let (dir, mut program, cell) = requests("round-side");
    program.open_take(&dir.join("takes/Dawn.mp4"));
    program.view("the gallery's take", |view, _| names(view) == ["Dawn"]);
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dusk.mp4", false)]), round("q2", 2, vec![take_file("takes/Dawn.mp4", false)])]);
    film_of(&program, "the rounds", |film| film.groups.len() == 3);
    program.command("film_select", json!({ "value": 0 }));
    program.view("the first round's take", |_, image| is(image.at(ALONE.0, ALONE.1), dusk, DUSK_FRAMES, 0));
    program.command("film_select", json!({ "value": 1, "extend": true }));
    program.command("frame", json!({ "step": 6 }));
    let (view, _) = program.view("two rounds' takes side by side", |_, image| {
        is(image.at(LEFT.0, LEFT.1), dusk, DUSK_FRAMES, 6) && is(image.at(RIGHT.0, RIGHT.1), dawn, DAWN_FRAMES, 6)
    });
    let film = view.film.unwrap();
    assert!(film.items[0].chosen && film.items[1].chosen && !film.items[2].chosen);
    assert!(view.transport.is_some(), "the two have no transport row");

    // The gallery's take, beside the round's.
    // Both frames are decoded already, so the first view may be the
    // whole one: the strip and the image are read from the same view.
    program.command("film_select", json!({ "value": 2, "extend": true }));
    let (view, _) = program.view("a round's take beside the gallery's", |view, image| {
        view.film.as_ref().is_some_and(|film| film.items[0].chosen && film.items[2].chosen && !film.items[1].chosen)
            && is(image.at(LEFT.0, LEFT.1), dusk, DUSK_FRAMES, 6)
            && is(image.at(RIGHT.0, RIGHT.1), dawn, DAWN_FRAMES, 6)
    });
    assert_eq!(view.film.unwrap().shown, 0);
    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => path.clone(),
        _ => None,
    });
    let header = vgx_header(Path::new(&kept));
    assert_eq!(header["view"]["mode"], "single", "the comparison was kept: {header}");
    assert!(!header.to_string().contains("Dusk"), "a round's take was kept: {header}");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Previews of sound.

/// A WAV of one second at 8 000 samples a second, 16-bit mono, loud in
/// its first half and silent in its second, with a cue point "Door" at
/// half a second.
fn write_wav_with_cue(path: &Path) {
    fn chunk(id: &[u8], data: Vec<u8>) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        let odd = data.len() % 2 == 1;
        out.extend(data);
        if odd {
            out.push(0);
        }
        out
    }
    let rate = 8000u32;
    let mut fmt = Vec::new();
    for value in [1u16, 1] {
        fmt.extend_from_slice(&value.to_le_bytes());
    }
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 2).to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut samples = Vec::new();
    for i in 0..rate {
        // A 500 Hz square wave, well within what is kept when it is
        // decoded at 48 000 a second, then silence.
        let value: i16 = if i >= rate / 2 { 0 } else if (i / 8) % 2 == 0 { 20_000 } else { -20_000 };
        samples.extend_from_slice(&value.to_le_bytes());
    }
    let mut cue = 1u32.to_le_bytes().to_vec();
    cue.extend_from_slice(&1u32.to_le_bytes());
    cue.extend_from_slice(&0u32.to_le_bytes());
    cue.extend_from_slice(b"data");
    cue.extend_from_slice(&[0; 8]);
    cue.extend_from_slice(&(rate / 2).to_le_bytes());
    let mut label = 1u32.to_le_bytes().to_vec();
    label.extend_from_slice(b"Door\0");
    let mut list = b"adtl".to_vec();
    list.extend(chunk(b"labl", label));
    let mut body = b"WAVE".to_vec();
    body.extend(chunk(b"fmt ", fmt));
    body.extend(chunk(b"data", samples));
    body.extend(chunk(b"cue ", cue));
    body.extend(chunk(b"LIST", list));
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    std::fs::write(path, out).expect("the WAV");
}

/// Catches a sound that cannot be previewed, or a workspace that would
/// take a sound for a take: `previews` declares the sounds beside the
/// videos while `edits` keeps to videos; a WAV's preview is its waveform,
/// 160 × 32, with its length and its cue; and a file that is no sound
/// answers with why.
#[test]
fn a_sound_is_previewed_with_its_waveform_and_cues() {
    let dir = scratch("sound-preview");
    write_wav_with_cue(&dir.join("takes/Rain.wav"));
    let mut program = Program::start(false);
    let edits = program.wait("edits", |message| match message {
        AppMessage::Edits { media_types } => Some(media_types.clone()),
        _ => None,
    });
    assert!(edits.iter().all(|kind| kind.starts_with("video/")), "a sound would open as a take: {edits:?}");
    let previews = program.wait("previews", |message| match message {
        AppMessage::Previews { media_types } => Some(media_types.clone()),
        _ => None,
    });
    for kind in ["video/mp4", "audio/wav", "audio/mpeg", "audio/flac"] {
        assert!(previews.iter().any(|own| own == kind), "{kind} is not previewed: {previews:?}");
    }
    program.send(in_project(&dir));
    program.send(HostMessage::Preview { id: "s1".into(), path: "takes/Rain.wav".into(), media_type: "audio/wav".into() });
    let (thumbnail, badge, cues, error) = program.wait("the sound's preview", |message| match message {
        AppMessage::Previewed { id, thumbnail, badge, cues, error, .. } if id == "s1" => Some((thumbnail.clone(), badge.clone(), cues.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(error, None);
    assert_eq!(badge, "1.0 s");
    let thumbnail = thumbnail.expect("no waveform");
    assert_eq!((thumbnail.width, thumbnail.height), (160, 32));
    let bytes = std::fs::read(&thumbnail.path).unwrap();
    assert_eq!(bytes.len(), 160 * 32 * 4);
    let at = |x: usize, y: usize| bytes[(y * 160 + x) * 4 + 3];
    // The square wave is 0.61 of full scale, so it reaches about ten rows
    // either side of the middle, row 16; silence is the middle alone.
    assert!(at(20, 8) > 0, "the loud half is not tall");
    assert_eq!(at(140, 8), 0, "the silent half is not flat");
    assert_eq!(cues.iter().map(|cue| (cue.at, cue.label.as_str())).collect::<Vec<_>>(), [(0.5, "Door")]);

    std::fs::write(dir.join("notes.txt"), "not a sound").unwrap();
    program.send(HostMessage::Preview { id: "s2".into(), path: "notes.txt".into(), media_type: "audio/wav".into() });
    let refused = program.wait("a refusal", |message| match message {
        AppMessage::Previewed { id, thumbnail, error, .. } if id == "s2" => Some((thumbnail.is_none(), error.is_some())),
        _ => None,
    });
    assert_eq!(refused, (true, true));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Finishing a take, as vg-retime-upscale and vg-before-after show it.

/// A 1280 × 720 take of two frames.
fn write_wide(path: &Path) {
    let settings = media::Settings { width: 1280, height: 720, rate: (24, 1), keyframes: 12, sound: None };
    let mut encoder = media::Encoder::create(path, settings).expect("an encoder");
    for _ in 0..2 {
        encoder.frame(&[60, 90, 140, 255].repeat(1280 * 720)).expect("a frame");
    }
    encoder.finish().expect("the video");
}

/// The next new cell `cells` declares that `pick` takes.
fn new_cell(program: &Program, what: &str, mut pick: impl FnMut(&protocol::Cell) -> bool) -> protocol::Cell {
    program.wait(what, |message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().find(|cell| pick(cell)).cloned(),
        _ => None,
    })
}

/// Catches a finishing request other than Last order's, or one paid for
/// without the person's yes: Retime… makes a new draft of the retime's
/// model and settings, the take its one reference filling `video_url`,
/// and asks the host to run it, whose plan card asks; Upscale… of a
/// 1280-wide take asks a factor of 3, in a draft of its own.
#[test]
fn retime_and_upscale_make_drafts_the_host_runs() {
    let (dir, mut program, cell) = requests("finish");
    write_wide(&dir.join("takes/Wide.mp4"));
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Wide.mp4", false)])]);
    film_of(&program, "the round", |film| !film.groups.is_empty());

    program.command("take_retime", json!({ "take": "takes/Wide.mp4" }));
    let retime = new_cell(&program, "the retime's draft", |cell| cell.model.as_deref() == Some("topaz/interpolate/video"));
    assert_eq!(retime.kind, protocol::VIDEO_KIND);
    assert_eq!(
        retime.settings,
        Some(json!({ "target_fps": 60, "model": "Apollo", "slowdown_factor": 1, "H264_output": true, "enable_safety_checker": true }))
    );
    assert_eq!(
        retime.input,
        [protocol::CellPart::Path { path: "takes/Wide.mp4".into(), media_type: "video/mp4".into(), role: Some("structure".into()), field: Some("video_url".into()) }]
    );
    let run = program.wait("run_cell", |message| match message {
        AppMessage::RunCell { tab, cell, .. } if tab == "1" => Some(cell.clone()),
        _ => None,
    });
    assert_eq!(run, retime.id);

    program.command("take_upscale", json!({ "take": "takes/Wide.mp4" }));
    let upscale = new_cell(&program, "the upscale's draft", |cell| cell.model.as_deref() == Some("topaz/upscale/video/precision"));
    assert_eq!(upscale.settings, Some(json!({ "model": "Gaia CG", "upscale_factor": 3.0, "H264_output": true, "enable_safety_checker": true })));
    assert_ne!(upscale.id, retime.id, "an upscale took the retime's draft");
    let run = program.wait("run_cell", |message| match message {
        AppMessage::RunCell { cell, .. } => Some(cell.clone()),
        _ => None,
    });
    assert_eq!(run, upscale.id);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a finishing round labelled as any round, or a finished take
/// that cannot be set beside its source: a retime of the round's take is
/// labelled "Take 1 · 60 fps, from Take 1", its take's menu has Compare
/// with the source, and that shows the source on the left and the
/// finished take on the right, both chosen.
#[test]
fn a_finished_take_is_labelled_and_compared_with_its_source() {
    let (dir, mut program, cell) = requests("finished");
    let mut retime = round("q2", 2, vec![take_file("takes/Dusk.mp4", false)]);
    retime.model = "topaz/interpolate/video".into();
    retime.references = vec![protocol::SentReference { path: "takes/Dawn.mp4".into(), role: Some("structure".into()) }];
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false)]), retime]);
    let film = film_of(&program, "the two rounds", |film| film.groups.len() == 2);
    assert_eq!(film.groups[1].label, "Take 1 · 60 fps, from Take 1");
    let rows = |item: usize| film.items[item].menu.iter().map(|row| row.label.clone()).collect::<Vec<_>>();
    assert!(rows(1).contains(&"Compare with the source".to_string()));
    assert!(!rows(0).contains(&"Compare with the source".to_string()), "a take of no finishing round can be compared");

    program.command("take_compare", json!({ "take": "takes/Dusk.mp4" }));
    film_of(&program, "both chosen", |film| film.items[0].chosen && film.items[1].chosen);
    program.view("the source beside the finished take", |_, image| {
        is(image.at(LEFT.0, LEFT.1), dawn, DAWN_FRAMES, 0) && is(image.at(RIGHT.0, RIGHT.1), dusk, DUSK_FRAMES, 0)
    });
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Sound layers, as vg-layers shows them.

/// A WAV of half a second at 8 000 samples a second, 16-bit mono, held at
/// half of full scale.
fn write_steady_wav(path: &Path) {
    let rate = 8000u32;
    let mut fmt = Vec::new();
    for value in [1u16, 1] {
        fmt.extend_from_slice(&value.to_le_bytes());
    }
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 2).to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let samples: Vec<u8> = (0..rate / 2).flat_map(|_| 16_384i16.to_le_bytes()).collect();
    let mut body = b"WAVE".to_vec();
    for (id, data) in [(b"fmt ", fmt), (b"data", samples)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend(data);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).expect("a folder");
    }
    std::fs::write(path, out).expect("the WAV");
}

/// Adds the sound at `path` as a layer, through Open sound as layer… and
/// the host's file dialog.
fn open_sound(program: &mut Program, path: &Path) {
    program.command("open_sound", json!({}));
    let id = program.wait("a file dialog", |message| match message {
        AppMessage::File { id, filter, .. } if filter.as_ref().is_some_and(|filter| filter.extensions.contains(&"wav".to_string())) => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id, path: Some(path.display().to_string()) });
}

/// The next Layers page whose rows `pick` takes.
fn layer_rows(program: &Program, what: &str, mut pick: impl FnMut(&[protocol::LayerRow]) -> bool) -> Vec<protocol::LayerRow> {
    program.wait(what, |message| match message {
        AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.clone()).filter(|rows| pick(rows)),
        _ => None,
    })
}

fn property(program: &mut Program, prop: Value, value: Value) {
    program.send(HostMessage::Property { prop, value, preview: false });
}

/// The most opaque pixel of the row under the scrubber labelled `label`,
/// once it is declared.
fn row_alpha(view: &RasterView, label: &str) -> Option<u8> {
    let row = view.transport.as_ref()?.rows.iter().find(|row| row.label == label)?;
    let bytes = std::fs::read(&row.image.path).ok()?;
    bytes.chunks_exact(4).map(|pixel| pixel[3]).max()
}

/// Catches a sound layer that cannot be added, muted, set, or undone, or
/// is lost: the Layers page shows the take as its linked layer, with its
/// thumbnail, its file's name, the plain eye, and its gain, and the sound
/// added from a WAV under it at "0 dB"; the layer is a row under the
/// scrubber, and stays one, fainter, while muted; its eye mutes it; its
/// gain shows at its right; its start is on the Properties page, and an
/// undo takes it back; and a new program opening the `.vgx` finds the layer
/// with its gain and mute, and the take's own sound muted at its gain.
#[test]
fn a_sound_layer_is_added_set_undone_and_kept() {
    let (dir, _, _) = project("sound-layer");
    let first = dir.join("takes/Tone.mp4");
    write_sounding_video(&first);
    let rain = dir.join("sounds/rain.wav");
    write_steady_wav(&rain);
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.open_take(&first);
    program.view("the take", |view, _| names(view) == ["Tone"]);
    program.command("panel_page", json!({ "page": 0 }));
    open_sound(&mut program, &rain);
    let rows = layer_rows(&program, "the layer added", |rows| rows.len() == 2 && rows[0].thumbnail.is_some());
    assert_eq!((rows[0].id, rows[0].name.as_str(), rows[0].mark.as_deref(), rows[0].visible), (0, "Tone", Some("link"), true));
    assert_eq!((rows[0].detail.as_deref(), rows[0].toggle_icon.as_deref(), rows[0].value.as_deref()), (Some("Tone.mp4"), None, Some("0 dB")));
    assert_eq!((rows[1].name.as_str(), rows[1].visible, rows[1].value.as_deref(), rows[1].toggle_icon.as_deref()), ("rain", true, Some("0 dB"), None));
    let layer = rows[1].id;
    // The sound may have been decoded, and its row drawn, before the panel
    // above was read: a frame step of nothing draws the view again.
    program.command("frame", json!({ "step": 0 }));
    program.view("the layer's row under the scrubber", |view, _| row_alpha(view, "rain") == Some(255));

    // The view comes before the panel the same change declares.
    program.command("sound_mute", json!({ "sound": layer }));
    program.view("the muted layer's row, fainter", |view, _| row_alpha(view, "rain").is_some_and(|alpha| alpha > 0 && alpha <= 128));
    layer_rows(&program, "the layer muted", |rows| rows.get(1).is_some_and(|row| !row.visible));
    property(&mut program, json!({ "mix": layer, "field": "gain_db" }), json!(-6.0));
    layer_rows(&program, "the layer at -6 dB", |rows| rows.get(1).is_some_and(|row| row.value.as_deref() == Some("−6 dB")));
    program.command("sound_click", json!({ "sound": layer }));
    program.command("panel_page", json!({ "page": 1 }));
    let start = |properties: &protocol::Properties| {
        properties.sections.first().and_then(|section| section.rows.iter().find(|row| row.label == "Starts at")).map(|row| row.value.clone())
    };
    // The take is one second long: a start past its last frame is held
    // there, as mix.py refuses a sound starting at the take's end.
    property(&mut program, json!({ "mix": layer, "field": "start" }), json!(1.5));
    program.wait("its start held inside the take", |message| match message {
        AppMessage::Properties(properties) => {
            start(properties).is_some_and(|value| matches!(value, protocol::PropValue::Number(at) if at > 0.9 && at < 1.0)).then_some(())
        }
        _ => None,
    });
    property(&mut program, json!({ "mix": layer, "field": "start" }), json!(0.5));
    program.wait("its start at 0.5 s", |message| match message {
        AppMessage::Properties(properties) => (start(properties) == Some(protocol::PropValue::Number(0.5))).then_some(()),
        _ => None,
    });
    // Both starts are steps: two undos take it back to the beginning.
    program.command("undo", json!({}));
    program.command("undo", json!({}));
    program.wait("its start undone", |message| match message {
        AppMessage::Properties(properties) => (start(properties) == Some(protocol::PropValue::Number(0.0))).then_some(()),
        _ => None,
    });
    program.command("sound_mute", json!({ "sound": 0 }));
    property(&mut program, json!({ "mix": 0, "field": "gain_db" }), json!(-3.0));
    program.send(HostMessage::Keep { tab: "1".into() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab, path, .. } if tab == "1" => path.clone(),
        _ => None,
    });
    let header = vgx_header(Path::new(&kept));
    assert_eq!(header["take_sound"], json!({ "gain_db": -3.0, "muted": true }));
    assert_eq!(header["sounds"][0]["link"]["path"], "sounds/rain.wav");
    assert_eq!((header["sounds"][0]["gain_db"].as_f64(), header["sounds"][0]["start"].as_f64(), header["sounds"][0]["muted"].as_bool()), (Some(-6.0), Some(0.0), Some(true)));
    program.send(HostMessage::Shutdown);
    drop(program);

    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some(kept) });
    program.command("panel_page", json!({ "page": 0 }));
    let rows = layer_rows(&program, "the layers again", |rows| rows.len() == 2);
    assert_eq!((rows[0].visible, rows[0].value.as_deref()), (false, Some("−3 dB")), "the take's own sound was not kept");
    assert_eq!((rows[1].name.as_str(), rows[1].visible, rows[1].value.as_deref()), ("rain", false, Some("−6 dB")));
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

// Deliver, as vg-deliver and vg-delivering show it.

/// The next notice the tab shows, as (words, state); none when it is taken
/// away.
fn notice_of(program: &Program, what: &str) -> Option<(String, protocol::NoticeState)> {
    program.wait(what, |message| match message {
        AppMessage::Notices { notices, .. } => Some(notices.first().map(|notice| (notice.text.clone(), notice.state))),
        _ => None,
    })
}

/// A project with a round whose step names its artifact, its take
/// shown, and the program in it: a take with its own sound, `Tone.mp4`, or
/// one with none, `Dawn.mp4`. Returns the take's path too.
fn delivering(label: &str, sounding: bool) -> (PathBuf, Program, &'static str) {
    let (dir, mut program, cell) = requests(label);
    let take = if sounding { "takes/Tone.mp4" } else { "takes/Dawn.mp4" };
    if sounding {
        write_sounding_video(&dir.join(take));
    }
    let mut final_shot = round("q1", 1, vec![take_file(take, false)]);
    final_shot.artifact = Some("shot 12: final".into());
    send_takes(&mut program, &cell, vec![final_shot]);
    film_of(&program, "the round", |film| !film.groups.is_empty() && film.items[0].menu.iter().any(|row| row.label == "Deliver…" && row.enabled));
    program.command("film_select", json!({ "value": 0 }));
    (dir, program, take)
}

/// Whether the next filmstrip's first take's Deliver… is on.
fn deliver_on(program: &Program, what: &str, on: bool) {
    film_of(program, what, |film| film.items.first().is_some_and(|item| item.menu.iter().any(|row| row.label == "Deliver…" && row.enabled == on)));
}

/// Catches a Deliver that mixes the wrong sounds, sends a sound from
/// `work/` unfiled, starts twice, or shows the wrong notices: a sound in
/// `work/` is filed first and its filed path mixed; the mix takes the take,
/// its own sound at -3 dB, and each layer with its gain and start, a muted
/// one marked; "Delivering…" shows while it is made; its film goes to the
/// host with what it uses, while "Delivering <film>…" shows; Deliver… is
/// off until the host answers; and the host's answer, a film under
/// `output/`, shows "Delivered <file name>.".
#[test]
fn a_take_is_delivered_with_its_mix() {
    let (dir, mut program, take) = delivering("deliver", true);
    write_steady_wav(&dir.join("sounds/bed.wav"));
    write_steady_wav(&dir.join("work/voice.wav"));
    open_sound(&mut program, &dir.join("sounds/bed.wav"));
    open_sound(&mut program, &dir.join("work/voice.wav"));
    program.command("panel_page", json!({ "page": 0 }));
    let rows = layer_rows(&program, "both layers", |rows| rows.len() == 3);
    let bed = rows.iter().find(|row| row.name == "bed").unwrap().id;
    program.command("sound_mute", json!({ "sound": bed }));
    property(&mut program, json!({ "mix": 0, "field": "gain_db" }), json!(-3.0));
    layer_rows(&program, "the take's sound at -3 dB", |rows| rows.first().is_some_and(|row| row.value.as_deref() == Some("−3 dB")));

    program.command("take_deliver", json!({ "take": take }));
    let (filing, path) = program.wait("the sound in work/ filed", |message| match message {
        AppMessage::FileResource { path, id: Some(id), .. } => Some((id.clone(), path.clone())),
        _ => None,
    });
    assert_eq!(path, "work/voice.wav");
    deliver_on(&program, "Deliver… off while under way", false);
    program.send(HostMessage::Filed { id: filing, file: "sounds/voice.wav".into(), error: None });
    let (script, skill, name, args) = program.wait("the mix", |message| match message {
        AppMessage::RunSkillScript { id, skill, script, args } => Some((id.clone(), skill.clone(), script.clone(), args.clone())),
        _ => None,
    });
    assert_eq!((skill.as_str(), name.as_str()), ("build-animatic", "mix.py"));
    assert_eq!(
        args,
        [
            "--take", take, "--take-sound", "-3", "--sound", "sounds/voice.wav", "0", "0", "--sound", "sounds/bed.wav", "0", "0", "muted", "--name",
            "shot 12: final", "--json",
        ]
    );
    assert_eq!(notice_of(&program, "the mix being made"), Some(("Delivering…".to_string(), protocol::NoticeState::Working)));
    // The film the stand-in mix made: the take itself, so it can be read.
    std::fs::create_dir_all(dir.join("library")).unwrap();
    std::fs::copy(dir.join(take), dir.join("library/shot-12-final.mp4")).unwrap();
    let output = String::from("Mixing…\n{\"path\": \"library/shot-12-final.mp4\", \"sha256\": \"ab\"}\n");
    program.send(HostMessage::SkillScriptRan { id: script, output, error: None });
    let (artifact, file, uses, lines) = program.wait("deliver", |message| match message {
        AppMessage::Deliver { tab, artifact, file, uses, size, frame_rate, format } if tab == "1" => {
            Some((artifact.clone(), file.clone(), uses.clone(), (size.clone(), frame_rate.clone(), format.clone())))
        }
        _ => None,
    });
    assert_eq!((artifact.as_str(), file.as_str(), uses.as_str()), ("shot 12: final", "library/shot-12-final.mp4", "Take 1 · Model A, with voice"));
    assert_eq!(lines, (format!("{} × {}", SIZE.0, SIZE.1), String::from("24 fps"), String::from("H.264 with AAC sound")), "the card's lines of the film");
    assert_eq!(notice_of(&program, "delivering the film"), Some(("Delivering shot-12-final.mp4…".to_string(), protocol::NoticeState::Working)));
    deliver_on(&program, "Deliver… still off", false);
    program.send(HostMessage::Delivered { tab: "1".into(), file: "output/2026-11-17_10-00-001-last-signal.mp4".into(), error: None });
    assert_eq!(notice_of(&program, "delivered"), Some(("Delivered 2026-11-17_10-00-001-last-signal.mp4.".to_string(), protocol::NoticeState::Done)));
    deliver_on(&program, "Deliver… on again", true);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches `--take-sound` sent for a take with no sound, which the mix
/// refuses, or a mix's failure unsaid: a take with none shows its row with
/// its eye off and no gain, takes no gain, and is mixed with no
/// `--take-sound`; and a mix that fails shows why.
#[test]
fn a_silent_take_is_mixed_without_its_sound() {
    let (dir, mut program, take) = delivering("deliver-silent", false);
    program.command("panel_page", json!({ "page": 0 }));
    let rows = layer_rows(&program, "the take's row", |rows| rows.len() == 1);
    assert_eq!((rows[0].visible, rows[0].value.as_deref()), (false, None), "a take with no sound shows its own");
    property(&mut program, json!({ "mix": 0, "field": "gain_db" }), json!(-3.0));
    program.command("take_deliver", json!({ "take": take }));
    let (script, args) = program.wait("the mix", |message| match message {
        AppMessage::RunSkillScript { id, args, .. } => Some((id.clone(), args.clone())),
        _ => None,
    });
    assert_eq!(args, ["--take", take, "--name", "shot 12: final", "--json"]);
    assert_eq!(notice_of(&program, "the mix being made"), Some(("Delivering…".to_string(), protocol::NoticeState::Working)));
    program.send(HostMessage::SkillScriptRan { id: script, output: String::new(), error: Some("The take could not be read.".into()) });
    assert_eq!(notice_of(&program, "the mix's failure"), Some(("The take could not be read.".to_string(), protocol::NoticeState::Failed)));
    deliver_on(&program, "Deliver… on again after a failure", true);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a sound from outside the project sent to the mix, or a Cancel
/// that leaves a notice: Deliver… with such a layer stops with "<file> is
/// not in the project."; and a `delivered` with no file and no error takes
/// the notice away.
#[test]
fn a_sound_outside_the_project_stops_a_deliver() {
    let (dir, mut program, take) = delivering("deliver-outside", false);
    let outside = scratch("deliver-elsewhere").join("door.wav");
    write_steady_wav(&outside);
    open_sound(&mut program, &outside);
    program.command("panel_page", json!({ "page": 0 }));
    layer_rows(&program, "the layer", |rows| rows.len() == 2);
    program.command("take_deliver", json!({ "take": take }));
    assert_eq!(notice_of(&program, "refused"), Some(("door.wav is not in the project.".to_string(), protocol::NoticeState::Failed)));
    program.send(HostMessage::Delivered { tab: "1".into(), file: String::new(), error: None });
    assert_eq!(notice_of(&program, "cancelled"), None);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(outside.parent().unwrap());
}

/// The draft output of the tab `pick` takes, from the next `cells` that
/// names a draft for it: the tab, and the selected cell's output.
fn draft_output(program: &Program, what: &str, mut pick: impl FnMut(&str, &[protocol::CellPart]) -> bool) -> (String, Vec<protocol::CellPart>) {
    program.wait(what, |message| match message {
        AppMessage::Cells { tab, cells, selected: Some(selected) } => {
            cells.iter().find(|cell| cell.id == *selected).map(|cell| (tab.clone(), cell.output.clone())).filter(|(tab, output)| pick(tab, output))
        }
        _ => None,
    })
}

/// Catches a draft that hides which take it was opened from, or one that
/// keeps it: a tab with no take has a draft with no output; a take opened
/// from a project file sends `cells` whose draft's output is that file, as
/// the project names it, so the host can find its round; the `.vgx` keeps
/// no output; and after a restart, `cells` carries it again.
#[test]
fn a_draft_says_the_file_its_take_was_opened_from() {
    let (dir, _, _) = project("draft-output");
    let mut program = Program::start(false);
    let (_, empty) = draft_output(&program, "the first tab's draft", |tab, _| tab == "1");
    assert!(empty.is_empty(), "a draft with no take has an output");
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some("takes/Dawn.mp4".into()) });
    let (tab, output) = draft_output(&program, "the take's draft", |tab, output| tab != "1" && !output.is_empty());
    let opened = [protocol::CellPart::Path { path: "takes/Dawn.mp4".into(), media_type: "video/mp4".into(), role: None, field: None }];
    assert_eq!(output, opened);

    // Changed, so it is kept, and kept with no output.
    program.send(HostMessage::Command { id: "film_rename".into(), tab: Some(tab.clone()), arg: Some(json!({ "take": 0, "value": "Morning" })), node: None, viewport: VIEWPORT });
    program.send(HostMessage::Keep { tab: tab.clone() });
    let kept = program.wait("kept", |message| match message {
        AppMessage::Kept { tab: kept, path, .. } if *kept == tab => path.clone(),
        _ => None,
    });
    let header = vgx_header(Path::new(&kept));
    assert!(
        header["cells"].as_array().into_iter().flatten().all(|cell| cell["output"].as_array().is_none_or(|output| output.is_empty())),
        "the file kept a draft's output: {header}"
    );
    program.send(HostMessage::Shutdown);
    drop(program);

    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(in_project(&dir));
    program.send(HostMessage::Open { path: Some(kept) });
    let (_, again) = draft_output(&program, "the draft after a restart", |_, output| !output.is_empty());
    assert_eq!(again, opened, "after a restart, the draft does not say its file");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a video take drawn as a file rather than as its asset, as the
/// Image Generator's: its latest iteration as a stack whose badge opens the
/// iterations as a run after its round, an out-of-date take outlined with
/// Ignore sending `ignore_take`, and a take nothing records Unverified.
#[test]
fn a_take_shows_its_asset_its_stack_and_its_outline() {
    let (dir, mut program, cell) = requests("asset");
    let iteration = protocol::Iteration { file: "takes/Dusk.mp4".into(), published: None, fix: true };
    let stacked = protocol::TakeFile { iterations: vec![iteration], outlined: true, ..take_file("takes/Dawn.mp4", true) };
    let unverified = protocol::TakeFile { unverified: true, ..take_file("takes/Dusk.mp4", false) };
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![stacked, unverified])]);
    let film = film_of(&program, "the round", |film| !film.groups.is_empty());
    let (first, second) = (&film.items[0], &film.items[1]);
    assert!(first.outlined && first.stack.as_ref().is_some_and(|stack| stack.count == 2), "{first:?}");
    assert!(first.menu.iter().any(|row| row.label == "Ignore"), "an out-of-date take's menu has no Ignore");
    assert!(!second.outlined && second.stack.is_none() && second.caption == "Unverified", "{second:?}");
    program.command("take_ignore", json!({ "take": "takes/Dusk.mp4" }));
    let ignored = program.wait("ignore_take", |message| match message {
        AppMessage::IgnoreTake { path, .. } => Some(path.clone()),
        _ => None,
    });
    assert_eq!(ignored, "takes/Dusk.mp4");
    let open = first.stack.as_ref().unwrap().open.clone();
    program.command(&open.id, open.arg.clone().unwrap());
    let film = film_of(&program, "the stack opened", |film| film.groups.iter().any(|group| group.label == "Take 1 · 2 iterations"));
    let captions: Vec<&str> = film.items.iter().map(|item| item.caption.as_str()).filter(|caption| !caption.is_empty()).collect();
    assert_eq!(captions, ["Unverified", "1", "2"]);
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a take that cannot be put in use, or a use that says nothing
/// when it fails: Use this take sends `use_take` with the tab, the cell's
/// item as `takes` names it, and the take's path; the host's `take_used`
/// with an error shows that sentence as a failed notice, and one with none
/// says nothing, as the seal comes with the next `takes`.
#[test]
fn use_this_take_asks_the_host_to_use_it() {
    let (dir, mut program, cell) = requests("use-take");
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take_file("takes/Dawn.mp4", false)])]);
    film_of(&program, "the round", |film| !film.groups.is_empty());
    program.command("take_use", json!({ "take": "takes/Dawn.mp4" }));
    let (tab, item, path) = program.wait("use_take", |message| match message {
        AppMessage::UseTake { tab, cell, path } => Some((tab.clone(), cell.clone(), path.clone())),
        _ => None,
    });
    assert_eq!((tab.as_str(), path.as_str()), ("1", "takes/Dawn.mp4"));
    assert_eq!(item, protocol::ItemRef { path: "work/Untitled Video.vgx".into(), item: Some(cell.clone()) });

    program.send(HostMessage::TakeUsed { tab: "1".into(), path: "takes/Dawn.mp4".into(), artifact: None, error: Some("The take could not be put in use.".into()) });
    let notice = program.wait("the failure", |message| match message {
        AppMessage::Notices { notices, .. } => notices.first().cloned(),
        _ => None,
    });
    assert_eq!((notice.text.as_str(), notice.state), ("The take could not be put in use.", protocol::NoticeState::Failed));
    // A success takes the refusal away, and after that says nothing.
    let said = |program: &mut Program| {
        program.send(HostMessage::TakeUsed { tab: "1".into(), path: "takes/Dawn.mp4".into(), artifact: Some("shot 12: final".into()), error: None });
        program.send(HostMessage::Ping);
        let mut seen = Vec::new();
        program.wait("pong", |message| {
            if let AppMessage::Notices { notices, .. } = message {
                seen.push(notices.clone());
            }
            matches!(message, AppMessage::Pong).then_some(())
        });
        seen
    };
    let first = said(&mut program);
    assert!(first.len() == 1 && first[0].is_empty(), "the refusal was not taken away: {first:?}");
    assert!(said(&mut program).is_empty(), "a take put in use said something");
    drop(program);
    let _ = std::fs::remove_dir_all(&dir);
}
