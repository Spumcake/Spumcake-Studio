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
    let settings = media::Settings { width: SIZE.0, height: SIZE.1, rate: (24, 1), keyframes: 12, sound: None };
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
/// of 133 × 75 across 1120 with 8 between them, each take's row 356 tall.
fn strip(row: u32, column: u32) -> (u32, u32) {
    (40 + 141 * column + 2 + 64, 40 + 364 * row + 140 + 1 + 36)
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
    program.send(HostMessage::Project { folder: dir.display().to_string(), skills: false });
    let (empty, image) = program.view("an empty view at the area's size", |view, _| view.width == 1200);
    assert_eq!((empty.width, empty.height), (1200, 800));
    assert_eq!(image.bytes.len(), 1200 * 800 * 4, "the image file is not the area's size");
    assert_eq!(image.at(600, 400), [255, 255, 255]);
    assert!(names(&empty).is_empty());
    let camera = empty.camera.expect("the host's camera was never put on the image");
    assert_eq!((camera.origin, camera.zoom), ([0.0, 0.0], 1.0));

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
    let kept = dir.join("Untitled Video.vgx");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !kept.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = std::fs::read_to_string(&kept).expect("the .vgx file");
    assert!(text.starts_with("VGX1\n{\"version\":1,"), "{text}");
    assert!(text.contains("\"path\":\"takes/Dawn.mp4\""), "a take inside the project is not linked by its path in it: {text}");
    assert!(text.contains(&sums[0]), "a take's sha256 is not kept: {text}");
    program.send(HostMessage::Shutdown);
    drop(program);

    // A new program, as a restored workspace, finds it all again, even
    // with Studio's own folder gone, as a harness would leave the project.
    let _ = std::fs::remove_dir_all(dir.join(protocol::PROJECT_DIR));
    let mut program = Program::start(true);
    program.send(HostMessage::Viewport { viewport: VIEWPORT });
    program.send(HostMessage::Project { folder: dir.display().to_string(), skills: false });
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
    program.send(HostMessage::Project { folder: dir.display().to_string(), skills: false });
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
