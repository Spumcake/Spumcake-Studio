//! The Image Generator as its own program, driven through the protocol alone.

use protocol::{AppMessage, HostMessage, RasterGesture, RasterView};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

struct Program {
    child: Child,
    input: ChildStdin,
    output: Receiver<AppMessage>,
}

impl Program {
    fn start(restore: bool) -> Self {
        Self::start_with(restore, |_| {})
    }

    /// Started with its environment changed by `set`.
    fn start_with(restore: bool, set: impl FnOnce(&mut Command)) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_image-generator"));
        if restore {
            command.arg(protocol::RESTORE_FLAG);
        }
        set(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("image-generator starts");
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
        Self { child, input, output }
    }

    fn send(&mut self, message: HostMessage) {
        writeln!(self.input, "{}", serde_json::to_string(&message).expect("json")).expect("sent");
    }

    fn command(&mut self, id: &str, arg: serde_json::Value) {
        self.send(HostMessage::Command {
            id: id.into(),
            tab: Some("1".into()),
            arg: Some(arg),
            node: None,
            viewport: [1200.0, 800.0],
        });
    }

    /// The first message `pick` takes, within a few seconds.
    fn wait<T>(&self, mut pick: impl FnMut(&AppMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self.output.recv_timeout(left).expect("the awaited message never came");
            if let Some(found) = pick(&message) {
                return found;
            }
        }
    }

    fn raster(&self, newer_than: u64) -> RasterView {
        self.wait(|message| match message {
            AppMessage::Raster(view) if view.revision > newer_than => Some(view.clone()),
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

/// The host's `project` for `folder`, read from its line, so it says
/// nothing but the folder: every project is organized one way.
fn in_project(folder: &Path) -> HostMessage {
    let line = json!({ "type": "project", "folder": folder.display().to_string() }).to_string();
    protocol::parse_host_message(&line).expect("a project")
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("image-generator-session-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir.canonicalize().expect("canonical")
}

fn pixel(path: &Path, width: u32, x: u32, y: u32) -> [u8; 4] {
    let bytes = std::fs::read(path).expect("the image file");
    let at = ((y * width + x) * 4) as usize;
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

/// Where the images for the host are: in memory when the system has a
/// runtime folder, or else in the project's cache.
fn display_folder(project: &Path) -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|runtime| runtime.is_absolute() && runtime.is_dir()) {
        Some(runtime) => runtime.join("inpainter/image-generator"),
        None => project.join(".spumcake/cache/image-generator"),
    }
}

/// Catches an image the host could not read, a stroke that reaches no
/// pixel, an undo that does not restore it, and an image that waits for
/// Save: inside a project, the program, given only protocol lines, sends a
/// `raster` whose image file is width × height × 4 bytes in memory or the
/// project's cache; a stroke paints it; undo restores it; and a second
/// later the image is in the project as an `.ipx` file that reopens with
/// its layers.
#[test]
fn a_stroke_reaches_the_image_and_the_project() {
    let folder = scratch("stroke");
    let mut program = Program::start(false);
    let first = program.raster(0);
    assert_eq!((first.width, first.height), (1920, 1080));
    assert!(first.camera.is_some(), "a new image came without a camera");

    program.send(in_project(&folder));
    let view = program.raster(0);
    let file = PathBuf::from(&view.image);
    assert!(file.starts_with(display_folder(&folder)), "{}", file.display());
    assert_eq!(std::fs::metadata(&file).unwrap().len(), 1920 * 1080 * 4);
    assert_eq!(pixel(&file, 1920, 500, 300), [0, 0, 0, 0]);

    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture {
        tab: "1".into(),
        gesture: RasterGesture::Stroke { points: vec![[400.0, 300.0, 1.0], [600.0, 300.0, 1.0]] },
    });
    let painted = program.raster(view.revision);
    assert_eq!(painted.image, view.image);
    assert_eq!(std::fs::metadata(&file).unwrap().len(), 1920 * 1080 * 4);
    assert_eq!(pixel(&file, 1920, 500, 300), [0, 0, 0, 255], "the stroke did not reach the image");
    assert_eq!(pixel(&file, 1920, 500, 400), [0, 0, 0, 0]);

    program.command("undo", json!({}));
    let undone = program.raster(painted.revision);
    assert_eq!(pixel(&PathBuf::from(&undone.image), 1920, 500, 300), [0, 0, 0, 0]);
    program.command("redo", json!({}));
    program.raster(undone.revision);

    // A second after the change it is written by itself.
    let written = folder.join("work/Untitled Image.ipx");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !written.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(written.is_file(), "nothing was written into the project");
    drop(program);

    let mut again = Program::start(true);
    again.send(in_project(&folder));
    again.send(HostMessage::Claim { id: "c1".into(), path: written.display().to_string() });
    assert!(again.wait(|message| match message {
        AppMessage::Claimed { id, yours } if id == "c1" => Some(*yours),
        _ => None,
    }));
    again.send(HostMessage::Open { path: Some(written.display().to_string()) });
    let reopened = again.raster(0);
    assert_eq!((reopened.width, reopened.height), (1920, 1080));
    assert_eq!(pixel(&PathBuf::from(&reopened.image), 1920, 500, 300), [0, 0, 0, 255], "the stroke was not kept");
    let rows = again.wait(|message| match message {
        AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.len()),
        _ => None,
    });
    assert_eq!(rows, 1);
    drop(again);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches the images for the host written to disk while the system has
/// a folder in memory, or left behind there: with one, the image is in
/// `inpainter/image-generator` under it, stays there when a project is named,
/// and goes when the program ends, and a folder left by a program no longer
/// running is removed; without one, the image is in the project's cache.
#[test]
fn the_images_for_the_host_are_in_memory_when_there_is_some() {
    let folder = scratch("memory");
    let runtime = folder.join("run");
    let project = folder.join("project");
    let left = runtime.join("inpainter/image-generator/999999999-1");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let mut program = Program::start_with(false, |command| {
        command.env("XDG_RUNTIME_DIR", &runtime);
    });
    let first = program.raster(0);
    let file = PathBuf::from(&first.image);
    assert!(file.starts_with(runtime.join("inpainter/image-generator")), "{}", file.display());
    assert_eq!(std::fs::metadata(&file).unwrap().len(), 1920 * 1080 * 4);
    assert!(!left.exists(), "a folder left by a program no longer running stayed");
    program.send(in_project(&project));
    let view = program.wait(|message| match message {
        AppMessage::Raster(view) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(view.image, first.image, "naming a project moved the images out of memory");
    assert!(!project.join(".spumcake/cache/image-generator").exists(), "images were written into the project's cache too");
    program.send(HostMessage::Shutdown);
    let gone = file.parent().unwrap().to_path_buf();
    let deadline = Instant::now() + Duration::from_secs(5);
    while gone.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!gone.exists(), "the folder in memory stayed after the program ended");
    drop(program);

    let mut program = Program::start_with(false, |command| {
        command.env_remove("XDG_RUNTIME_DIR");
    });
    program.raster(0);
    program.send(in_project(&project));
    let view = program.wait(|message| match message {
        AppMessage::Raster(view) => Some(view.clone()),
        _ => None,
    });
    assert!(PathBuf::from(&view.image).starts_with(project.join(".spumcake/cache/image-generator")), "{}", view.image);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches Open and Export that do not go through the host's file dialog,
/// or that lose the image's size or pixels on the way.
#[test]
fn open_and_export_go_through_the_file_dialog() {
    let folder = scratch("files");
    let source = folder.join("Sunset.png");
    let mut rgba = Vec::new();
    for y in 0..30u32 {
        for x in 0..40u32 {
            rgba.extend_from_slice(&[(x * 6) as u8, (y * 8) as u8, 77, 255]);
        }
    }
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        &rgba,
        40,
        30,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    std::fs::write(&source, png).unwrap();

    let mut program = Program::start(false);
    program.send(in_project(&folder));
    program.raster(0);
    program.command("open", json!({}));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id: asked, path: Some(source.display().to_string()) });
    let opened = program.raster(0);
    assert_eq!((opened.width, opened.height), (40, 30));
    assert_eq!(std::fs::read(&opened.image).unwrap(), rgba);

    program.send(HostMessage::Command {
        id: "export_png".into(),
        tab: Some(opened.tab.clone()),
        arg: None,
        node: None,
        viewport: [1200.0, 800.0],
    });
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Save, .. } => Some(id.clone()),
        _ => None,
    });
    let exported = folder.join("Out.png");
    program.send(HostMessage::FileResult { id: asked, path: Some(exported.display().to_string()) });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !exported.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let back = image::open(&exported).expect("the export reads").into_rgba8();
    assert_eq!(back.into_raw(), rgba);

    // The same image opened as a layer of the image is a second layer.
    program.send(HostMessage::Command {
        id: "open_layer".into(),
        tab: Some(opened.tab.clone()),
        arg: None,
        node: None,
        viewport: [1200.0, 800.0],
    });
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id: asked, path: Some(source.display().to_string()) });
    let rows = program.wait(|message| match message {
        AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.len()).filter(|rows| *rows > 1),
        _ => None,
    });
    assert_eq!(rows, 2);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a + tile that adds nothing the host can show, or a stroke on an
/// added image that reaches another: through protocol lines alone, the
/// + row's action adds a white image shown on the strip and the canvas,
/// a stroke paints only it, a click on the first thumbnail shows the
/// first image, still white, again, and the + tile's open row adds a PNG
/// as an image of its own.
#[test]
fn an_image_added_from_the_strip_is_painted_alone() {
    let mut program = Program::start(false);
    let first = program.raster(0);
    let film = first.film.clone().expect("no filmstrip");
    assert_eq!(film.items.len(), 1);
    let first_thumb = std::fs::read(&film.items[0].thumbnail.path).expect("the first thumbnail");

    let send = |program: &mut Program, action: &protocol::Action| {
        program.command(&action.id, action.arg.clone().unwrap_or(serde_json::Value::Null));
    };
    send(&mut program, &film.focus);
    send(&mut program, film.add[0].action.as_ref().expect("the + row has no action"));
    let added = program.wait(|message| match message {
        AppMessage::Raster(view) if view.film.as_ref().is_some_and(|film| film.items.len() == 2) => Some(view.clone()),
        _ => None,
    });
    let strip = added.film.clone().unwrap();
    assert_eq!(strip.shown, 1, "the added image is not the one shown");
    assert_eq!(strip.items[1].name, "Untitled Image 2");
    assert_eq!((added.width, added.height), (1920, 1080));

    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture {
        tab: "1".into(),
        gesture: RasterGesture::Stroke { points: vec![[400.0, 300.0, 1.0], [600.0, 300.0, 1.0]] },
    });
    let painted = program.raster(added.revision);
    assert_eq!(pixel(Path::new(&painted.image), 1920, 500, 300), [0, 0, 0, 255], "the stroke missed the added image");
    let strip = painted.film.clone().unwrap();
    assert_eq!(std::fs::read(&strip.items[0].thumbnail.path).unwrap(), first_thumb, "the stroke reached the first image");
    let dark = std::fs::read(&strip.items[1].thumbnail.path).unwrap();
    assert!(dark.chunks(4).any(|pixel| pixel[0] < 128), "the added image's thumbnail does not show the stroke");

    send(&mut program, &strip.focus);
    send(&mut program, &strip.select.with_value(json!(0)));
    // The first image comes back as the file and revision first sent,
    // which the host still holds.
    let shown = program.wait(|message| match message {
        AppMessage::Raster(view) if view.film.as_ref().is_some_and(|film| film.shown == 0) => Some(view.clone()),
        _ => None,
    });
    assert_eq!((shown.image.as_str(), shown.revision), (first.image.as_str(), first.revision));
    assert_eq!(pixel(Path::new(&shown.image), 1920, 500, 300), [0, 0, 0, 0], "the first image was painted");

    // The + tile's second row opens a PNG through the host's file dialog
    // as a third image, shown, and named after its file.
    let folder = scratch("film-open");
    let source = folder.join("Sunset.png");
    image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 90, 30, 255])).save(&source).unwrap();
    let strip = shown.film.clone().unwrap();
    send(&mut program, &strip.focus);
    send(&mut program, strip.add[1].action.as_ref().expect("the + tile's open row has no action"));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id: asked, path: Some(source.display().to_string()) });
    let opened = program.wait(|message| match message {
        AppMessage::Raster(view) if view.film.as_ref().is_some_and(|film| film.items.len() == 3) => Some(view.clone()),
        _ => None,
    });
    let strip = opened.film.unwrap();
    assert_eq!(strip.shown, 2, "the opened image is not the one shown");
    assert_eq!(strip.items[2].name, "Sunset");
    assert_eq!((opened.width, opened.height), (40, 30));
    assert_eq!(pixel(Path::new(&opened.image), 40, 5, 5), [200, 90, 30, 255]);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// A gallery file's header: the line of JSON after its magic.
fn header(path: &Path) -> serde_json::Value {
    let bytes = std::fs::read(path).unwrap();
    let start = bytes.iter().position(|byte| *byte == b'{').expect("no header");
    let end = start + bytes[start..].iter().position(|byte| *byte == b'\n').expect("no header's end");
    serde_json::from_slice(&bytes[start..end]).unwrap()
}

fn until_file(path: &Path, changed_after: Option<std::time::SystemTime>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let ready = || std::fs::metadata(path).and_then(|meta| meta.modified()).is_ok_and(|modified| changed_after.is_none_or(|after| modified > after));
    while !ready() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ready(), "{} was never written", path.display());
}

/// Catches an image opened as new work that is not kept, broken by its
/// name, kept beside the project's activities, or with a record of what
/// made it: `open_as_new` with a name holding "/" and a line break, and
/// where it came from, makes a tab of one image called by a safe name,
/// written in the project's `work/` folder with the image's pixels and no
/// `source`, as Spumcake-Skills keeps what made a file, and nothing at the
/// project's root; opened again, painted, and saved, it still has none.
#[test]
fn an_image_opened_as_new_work_is_kept_in_work_without_its_source() {
    let folder = scratch("open-as-new");
    let source = folder.join("lamp.png");
    image::RgbaImage::from_pixel(20, 10, image::Rgba([10, 120, 200, 255])).save(&source).unwrap();
    let from = protocol::OpenedFrom { cell: "c7".into(), image: "images/lamp.png".into(), session: Some("s3".into()), recipe: None };

    let mut program = Program::start(false);
    program.send(in_project(&folder));
    program.raster(0);
    program.send(HostMessage::OpenAsNew { path: source.display().to_string(), name: "AC/DC lamp\nposter".into(), from: Some(from) });
    let opened = program.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (20, 10) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(opened.film.as_ref().unwrap().items.len(), 1);
    assert_eq!(opened.film.as_ref().unwrap().items[0].name, "AC DC lamp poster");
    assert_eq!(pixel(Path::new(&opened.image), 20, 3, 3), [10, 120, 200, 255]);
    let written = folder.join("work/AC DC lamp poster.ipx");
    until_file(&written, None);
    assert!(header(&written).get("source").is_none(), "the image keeps where it came from");
    assert!(!folder.join("AC DC lamp poster.ipx").exists(), "the image was kept at the project's root");
    drop(program);

    // Opened again, painted, and saved: still no source.
    let saved = std::fs::metadata(&written).unwrap().modified().unwrap();
    let mut again = Program::start(false);
    again.send(in_project(&folder));
    again.raster(0);
    again.send(HostMessage::Open { path: Some(written.display().to_string()) });
    let shown = again.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (20, 10) => Some(view.clone()),
        _ => None,
    });
    again.send(HostMessage::Command { id: "tool".into(), tab: Some(shown.tab.clone()), arg: Some(json!({ "tool": "brush" })), node: None, viewport: [1200.0, 800.0] });
    again.send(HostMessage::RasterGesture { tab: shown.tab.clone(), gesture: RasterGesture::Stroke { points: vec![[2.0, 2.0, 1.0], [18.0, 8.0, 1.0]] } });
    until_file(&written, Some(saved));
    assert!(header(&written).get("source").is_none(), "a save in a project wrote where the work came from");
    drop(again);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches typing that disappoints, through protocol lines alone: with the
/// T tool, a click makes a text whose box the view carries; typed words
/// reach it; a drag across a word selects it, and Copy answers with
/// `clipboard`; pasting it types it again; Undo while typing takes the
/// typing back, so the image is white again.
#[test]
fn a_text_is_typed_selected_copied_and_undone() {
    let mut program = Program::start(false);
    let first = program.raster(0);
    program.command("tool", json!({ "tool": "text" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Click { at: [300.0, 300.0] } });
    let view = program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_some() => Some(view.clone()),
        _ => None,
    });
    let text = |program: &mut Program, input: protocol::TextInput| program.send(HostMessage::RasterText { tab: "1".into(), input });
    let key = |key: protocol::TextKey, shift: bool| protocol::TextInput::Key { key, shift, word: false };
    text(&mut program, protocol::TextInput::Insert { text: "Hi there".into() });
    let typed = program.raster(view.revision);
    let editing = typed.text.clone().expect("typing ended the editing");
    assert!(std::fs::read(&typed.image).unwrap().chunks(4).any(|pixel| pixel[0] < 128), "the words drew nothing");

    // Where "Hi" ends: the selection's span after Shift+Right twice from the
    // start.
    text(&mut program, key(protocol::TextKey::Home, false));
    text(&mut program, key(protocol::TextKey::Right, true));
    text(&mut program, key(protocol::TextKey::Right, true));
    let selected = |wider: f64| {
        move |message: &AppMessage| match message {
            AppMessage::Raster(view) => view.text.as_ref().and_then(|text| text.selection.first().copied()).filter(|span| span[2] > wider),
            _ => None,
        }
    };
    let one = program.wait(selected(0.0));
    let span = program.wait(selected(one[2]));
    let middle = editing.rect[1] + editing.rect[3] / 2.0;
    // A press at the box's start and a drag to the word's end select it.
    text(&mut program, protocol::TextInput::Place { at: [editing.rect[0] + 0.5, middle], extend: false });
    text(&mut program, protocol::TextInput::Select { at: [span[0] + span[2], middle] });
    text(&mut program, key(protocol::TextKey::Copy, false));
    let copied = program.wait(|message| match message {
        AppMessage::Clipboard { text } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(copied, "Hi");
    text(&mut program, key(protocol::TextKey::End, false));
    text(&mut program, protocol::TextInput::Insert { text: copied });
    let pasted = program.wait(|message| match message {
        AppMessage::Properties(properties) => properties
            .layers
            .as_ref()
            .and_then(|layers| layers.rows.iter().find(|row| row.icon == "text").map(|row| row.name.clone()))
            .filter(|name| name == "Hi thereHi"),
        _ => None,
    });
    assert_eq!(pasted, "Hi thereHi");

    // Ctrl+Z while typing takes back the typing, and the new text with it.
    program.command("undo", json!({}));
    let undone = program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_none() => Some(view.clone()),
        _ => None,
    });
    assert_eq!(std::fs::read(&undone.image).unwrap(), std::fs::read(&first.image).unwrap(), "Undo left the typing");
}

/// Catches a clone stamp and a move of selected pixels that do nothing
/// through the protocol: a red line, Alt-clicked as the source, is stamped
/// elsewhere part by part; then a marquee around the line and the Move
/// tool's drag lift it off, leaving white, set it down lower, and one
/// Undo puts it back.
#[test]
fn the_stamp_and_moving_pixels_work_through_the_protocol() {
    let mut program = Program::start(false);
    let first = program.raster(0);
    program.command("set_color", json!({ "value": [255, 0, 0] }));
    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture {
        tab: "1".into(),
        gesture: RasterGesture::Stroke { points: vec![[400.0, 300.0, 1.0], [600.0, 300.0, 1.0]] },
    });
    let painted = program.raster(first.revision);
    program.command("tool", json!({ "tool": "stamp" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Source { at: [500.0, 300.0] } });
    let points = vec![[900.0, 700.0, 1.0], [920.0, 700.0, 1.0]];
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::StrokePart { from: 0, points: points.clone() } });
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Stroke { points } });
    let stamped = program.wait(|message| match message {
        AppMessage::Raster(view) if view.revision > painted.revision && view.patch.as_ref().is_none_or(|patch| patch.file.is_none()) => Some(view.clone()),
        _ => None,
    });
    assert!(matches!(stamped.tool, protocol::RasterTool::Sampler { source: Some([500.0, 300.0]), .. }));
    assert_eq!(pixel(Path::new(&stamped.image), 1920, 910, 700), [255, 0, 0, 255], "the stamp copied no red");

    program.command("tool", json!({ "tool": "marquee" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Rect { from: [380.0, 280.0], to: [620.0, 320.0] } });
    program.command("tool", json!({ "tool": "move" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::DragPart { from: [500.0, 300.0], to: [500.0, 350.0] } });
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::DragPart { from: [500.0, 300.0], to: [500.0, 400.0] } });
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Drag { from: [500.0, 300.0], to: [500.0, 400.0] } });
    let moved = program.wait(|message| match message {
        AppMessage::Raster(view) if view.revision > stamped.revision && view.patch.as_ref().is_none_or(|patch| patch.file.is_none()) => Some(view.clone()),
        _ => None,
    });
    let file = Path::new(&moved.image);
    assert_eq!(pixel(file, 1920, 500, 300)[3], 0, "the old place is not clear");
    assert_eq!(pixel(file, 1920, 500, 400), [255, 0, 0, 255], "the line did not move");
    assert_eq!(pixel(file, 1920, 910, 700), [255, 0, 0, 255], "the move reached past the selection");
    assert_eq!(moved.selection.len(), 1);
    assert!(moved.selection[0].iter().all(|point| point[1] >= 375.0), "the selection did not move");
    program.command("undo", json!({}));
    let undone = program.raster(moved.revision);
    assert_eq!(pixel(Path::new(&undone.image), 1920, 500, 300), [255, 0, 0, 255], "one undo did not put the line back");
    assert_eq!(pixel(Path::new(&undone.image), 1920, 500, 400), [0, 0, 0, 0]);
}

/// Catches the Adjustments and Properties pages doing nothing through the
/// protocol: Curves from the Adjustments page is added and shown on
/// Properties, and its curve, sent as a property, brightens the image.
#[test]
fn an_adjustment_is_added_and_set_from_the_panel() {
    // A white image, as a new one is empty.
    let folder = scratch("adjustment");
    let white = folder.join("white.png");
    image::RgbaImage::from_pixel(40, 30, image::Rgba([255, 255, 255, 255])).save(&white).unwrap();
    let mut program = Program::start(true);
    program.send(HostMessage::Open { path: Some(white.display().to_string()) });
    let first = program.raster(0);
    program.command("panel_page", json!({ "page": 2 }));
    let item = program.wait(|message| match message {
        AppMessage::Properties(properties) if properties.pages[2].active => properties.sections[0].rows.iter().find_map(|row| match &row.kind {
            protocol::RowKind::Item { action, .. } if row.label == "Curves" => Some(action.clone()),
            _ => None,
        }),
        _ => None,
    });
    program.command(&item.id, item.arg.clone().unwrap_or_default());
    let title = program.wait(|message| match message {
        AppMessage::Properties(properties) if properties.pages[1].active => properties.sections.first().map(|section| section.title.clone()),
        _ => None,
    });
    assert_eq!(title, "Curves");
    // The white image, darkened by a curve that ends at half.
    program.send(HostMessage::Property {
        prop: json!("curve"),
        value: serde_json::to_value(protocol::PropValue::Curve(vec![(0.0, 0.0), (1.0, 0.5)])).unwrap(),
        preview: false,
    });
    let graded = program.raster(first.revision);
    let value = pixel(Path::new(&graded.image), 40, 10, 10);
    assert!((126..=129).contains(&value[0]), "the curve made white {value:?}");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches new images named by a rule of the workspace's own, or named for
/// a file the project already holds: with "Untitled Image.ipx" in the
/// project's `work/` folder, two new tabs are "Untitled Image 2" and
/// "Untitled Image 3", the second is kept in its own file beside the first,
/// which is left as it was, and its filmstrip's new image is "Untitled
/// Image 2".
#[test]
fn new_images_are_named_free_of_the_projects_files() {
    let folder = scratch("names");
    let existing = folder.join("work/Untitled Image.ipx");
    std::fs::create_dir_all(folder.join("work")).unwrap();
    std::fs::write(&existing, b"someone's image").unwrap();
    let mut program = Program::start(true);
    program.send(in_project(&folder));
    let new_image = HostMessage::Command { id: "new_image".into(), tab: None, arg: None, node: None, viewport: [1200.0, 800.0] };
    program.send(new_image.clone());
    program.send(new_image);
    let tabs = program.wait(|message| match message {
        AppMessage::Tabs(tabs) if tabs.tabs.len() == 2 => Some(tabs.clone()),
        _ => None,
    });
    let labels: Vec<&str> = tabs.tabs.iter().map(|tab| tab.label.as_str()).collect();
    assert_eq!(labels, ["Untitled Image 2", "Untitled Image 3"]);

    let tab = tabs.tabs[1].id.clone();
    program.send(HostMessage::Command { id: "film_new".into(), tab: Some(tab.clone()), arg: None, node: None, viewport: [1200.0, 800.0] });
    let film = program.wait(|message| match message {
        AppMessage::Raster(view) => view.film.clone().filter(|film| film.items.len() == 2),
        _ => None,
    });
    let names: Vec<&str> = film.items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["Untitled Image", "Untitled Image 2"]);

    program.send(HostMessage::Keep { tab: tab.clone() });
    let kept = program.wait(|message| match message {
        AppMessage::Kept { tab: kept, path, error } if *kept == tab => Some((path.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(kept, (Some(folder.join("work/Untitled Image 3.ipx").display().to_string()), None));
    assert_eq!(std::fs::read(&existing).unwrap(), b"someone's image", "a new image wrote over a file in the project");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a 0.13 image file that no longer opens, or forgets where it came
/// from with no project open: "Lamp from pitch.ipx", as the 0.13 program
/// wrote it, opens with its image, and once changed and saved, with no
/// project to keep that record, its file still names the cell, image, and
/// recipe it was opened from.
#[test]
fn an_image_file_from_0_13_opens_with_its_source() {
    let folder = scratch("v0-13");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../host/tests/fixtures/v0.13/project/Lamp from pitch.ipx");
    let path = folder.join("Lamp from pitch.ipx");
    std::fs::copy(&fixture, &path).expect("the 0.13 fixture");
    let saved = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut program = Program::start(true);
    program.send(HostMessage::Open { path: Some(path.display().to_string()) });
    let opened = program.wait(|message| match message {
        AppMessage::Raster(view) => Some(view.clone()),
        _ => None,
    });
    assert_eq!((opened.width, opened.height), (40, 30));
    let names: Vec<String> = opened.film.as_ref().expect("no filmstrip").items.iter().map(|item| item.name.clone()).collect();
    assert_eq!(names, ["Lamp from pitch"]);

    program.send(HostMessage::Command { id: "add_layer".into(), tab: Some(opened.tab.clone()), arg: None, node: None, viewport: [1200.0, 800.0] });
    program.send(HostMessage::Command { id: "save".into(), tab: Some(opened.tab.clone()), arg: None, node: None, viewport: [1200.0, 800.0] });
    until_file(&path, Some(saved));
    let source: protocol::OpenedFrom = serde_json::from_value(header(&path)["source"].clone()).expect("the file forgot where it came from");
    assert_eq!(source.cell, "5b7e0c1e-0000-4000-8000-000000000201");
    assert_eq!(source.image, "images/lamp.png");
    assert_eq!(source.recipe.as_deref(), Some("pitch.smx"));
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Writes straight RGBA as a PNG file.
fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut png = Vec::new();
    image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut png), rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
    std::fs::write(path, png).unwrap();
}

/// The Layers page's rows as the next panel declares them, once `pick`
/// takes them.
fn layer_rows(program: &Program, mut pick: impl FnMut(&[protocol::LayerRow]) -> bool) -> Vec<protocol::LayerRow> {
    program.wait(|message| match message {
        AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.clone()).filter(|rows| pick(rows)),
        _ => None,
    })
}

/// Keeps the tab, and returns its file.
fn keep(program: &mut Program, tab: &str) -> PathBuf {
    program.send(HostMessage::Keep { tab: tab.into() });
    let kept = program.wait(|message| match message {
        AppMessage::Kept { tab: kept, path, error } if kept == tab => Some((path.clone(), error.clone())),
        _ => None,
    });
    assert_eq!(kept.1, None, "the image was not kept");
    PathBuf::from(kept.0.expect("no file"))
}

/// Catches a linked file written, a linked layer kept twice, or a painted
/// one that loses its painting: a PNG inside the project opens as a layer
/// linked to it by its path in the project, marked with the link, and is
/// kept with no pixels of its own; opened again after the file changed, it
/// shows the file as it is now; painted, its file's sha256 stays, the row
/// shows the pencil, and its pixels are kept; opened again after the file
/// changed once more, it shows its painting.
#[test]
fn a_linked_layer_keeps_its_file_and_its_painting() {
    let folder = scratch("linked");
    std::fs::create_dir_all(folder.join("takes")).unwrap();
    let take = folder.join("takes/take.png");
    write_png(&take, 40, 30, &[200u8; 40 * 30 * 4]);
    let linked = layers::link::sha256_file(&take).unwrap();

    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(take.display().to_string()) });
    let opened = program.raster(0);
    assert_eq!((opened.width, opened.height), (40, 30));
    let rows = layer_rows(&program, |rows| !rows.is_empty());
    assert_eq!(rows[0].mark.as_deref(), Some("link"), "the linked layer has no link mark");
    let file = keep(&mut program, &opened.tab);
    let saved = header(&file)["pictures"][0]["layers"][0].clone(); // the stored spelling
    assert_eq!(saved["link"], json!({ "path": "takes/take.png", "sha256": linked }));
    assert!(saved.get("png").is_none(), "an unedited linked layer kept its pixels");
    drop(program);

    // Opened again, it is read from the file as it is now.
    write_png(&take, 40, 30, &[90u8; 40 * 30 * 4]);
    let changed = layers::link::sha256_file(&take).unwrap();
    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(file.display().to_string()) });
    let reopened = program.raster(0);
    assert_eq!(pixel(Path::new(&reopened.image), 40, 20, 15), [90, 90, 90, 90], "the layer was not read from its file");

    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Stroke { points: vec![[5.0, 15.0, 1.0], [35.0, 15.0, 1.0]] } });
    let painted = program.raster(reopened.revision);
    assert_eq!(pixel(Path::new(&painted.image), 40, 20, 15), [0, 0, 0, 255]);
    let rows = layer_rows(&program, |rows| rows[0].mark.as_deref() == Some("edit-pencil"));
    assert_eq!(rows.len(), 1);
    let file = keep(&mut program, "1");
    assert_eq!(layers::link::sha256_file(&take).unwrap(), changed, "painting wrote the linked file");
    let saved = header(&file)["pictures"][0]["layers"][0].clone(); // the stored spelling
    assert_eq!(saved["edited"], true);
    assert!(saved.get("png").is_some(), "the painting was not kept");
    assert_eq!(saved["link"]["path"], "takes/take.png", "the layer forgot what it was edited from");
    assert_eq!(saved["link"]["sha256"], json!(changed), "the link names bytes the painting was not made on");
    drop(program);

    // Without Studio's own folder, as a harness would leave the project,
    // the image still opens with its link and its painting.
    let _ = std::fs::remove_dir_all(folder.join(protocol::PROJECT_DIR));
    write_png(&take, 40, 30, &[10u8; 40 * 30 * 4]);
    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(file.display().to_string()) });
    let again = program.raster(0);
    assert_eq!(pixel(Path::new(&again.image), 40, 20, 15), [0, 0, 0, 255], "the painting was lost");
    assert_eq!(pixel(Path::new(&again.image), 40, 20, 2), [90, 90, 90, 90], "the edited layer was read from its file");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches notes that are not shown, kept, or reopened, or that reach an
/// export: with the notes tool, a click puts pin 1, its words are typed
/// in, and Draw makes a mark as it is drawn; the image shown has the mark
/// in red and the pin, the PNG exported is the image without them, the
/// file keeps them, and opened again they are shown and the Notes row
/// sits above the layers.
#[test]
fn notes_are_shown_and_kept_but_never_exported() {
    let folder = scratch("notes");
    let plate = folder.join("plate.png");
    let white = vec![255u8; 200 * 120 * 4];
    write_png(&plate, 200, 120, &white);
    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(plate.display().to_string()) });
    let opened = program.raster(0);
    program.command("tool", json!({ "tool": "notes" }));
    assert!(matches!(program.raster(0).tool, protocol::RasterTool::Text), "Pin does not click as the T tool does");
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Click { at: [60.0, 40.0] } });
    let typing = program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_some() => Some(view.clone()),
        _ => None,
    });
    assert_ne!(pixel(Path::new(&typing.image), 200, 54, 40), [255, 255, 255, 255], "the pin is not shown");
    program.send(HostMessage::RasterText { tab: "1".into(), input: protocol::TextInput::Insert { text: "less rain".into() } });
    program.send(HostMessage::RasterText { tab: "1".into(), input: protocol::TextInput::Key { key: protocol::TextKey::Escape, shift: false, word: false } });
    program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_none() && view.revision > typing.revision => Some(()),
        _ => None,
    });

    program.command("notes_mode", json!({ "value": 1 }));
    let points = vec![[20.0, 100.5, 1.0], [180.0, 100.5, 1.0]];
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::StrokePart { from: 0, points: points.clone() } });
    let part = program.wait(|message| match message {
        AppMessage::Raster(view) if view.patch.as_ref().is_some_and(|patch| patch.file.is_some()) => Some(view.clone()),
        _ => None,
    });
    let patch = part.patch.unwrap();
    let rows = std::fs::read(patch.file.unwrap()).unwrap();
    let at = (((100 - patch.rect[1]) * patch.rect[2] + (100 - patch.rect[0])) * 4) as usize;
    assert_eq!(&rows[at..at + 4], layers::notes::RED, "the mark is not shown as it is drawn");
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Stroke { points } });
    let marked = program.wait(|message| match message {
        AppMessage::Raster(view) if view.revision > part.revision && view.patch.as_ref().is_none_or(|patch| patch.file.is_none()) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(pixel(Path::new(&marked.image), 200, 100, 100), layers::notes::RED, "the mark is not shown");
    let rows = layer_rows(&program, |rows| rows.len() == 2);
    assert_eq!(rows[0].name, "Notes");

    program.command("export_png", json!({}));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Save, .. } => Some(id.clone()),
        _ => None,
    });
    let exported = folder.join("Out.png");
    program.send(HostMessage::FileResult { id: asked, path: Some(exported.display().to_string()) });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !exported.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(image::open(&exported).unwrap().into_rgba8().into_raw(), white, "the export has the notes");

    let file = keep(&mut program, &opened.tab);
    let notes = header(&file)["pictures"][0]["notes"].clone(); // the stored spelling
    assert_eq!(notes["pins"][0]["number"], 1);
    assert_eq!(notes["pins"][0]["text"], "less rain");
    assert_eq!(notes["marks"].as_array().map(Vec::len), Some(1));
    drop(program);

    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(file.display().to_string()) });
    let reopened = program.raster(0);
    assert_eq!(pixel(Path::new(&reopened.image), 200, 100, 100), layers::notes::RED, "the notes were not kept");
    let rows = layer_rows(&program, |_| true);
    assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), ["Notes", "plate"]);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a pre-vis check drawn into the image itself, or not as
/// `layers::edges` draws it: its button with no render asks for one; once
/// chosen, the image shown is the plate with `layers::edges::check`'s cyan
/// edges, the PNG exported is the plate alone, See-through at 40% is
/// `see_through`'s blend, and the file keeps the check's render, by its
/// path in the project, its mode, its opacity, and that it is on.
#[test]
fn the_previs_check_is_drawn_only_where_it_is_shown() {
    let folder = scratch("previs");
    let (width, height) = (120u32, 80u32);
    let mut plate = layers::Rgba::new(width, height, [0, 0, 0, 255]);
    for y in 0..height {
        for x in 0..width {
            plate.set(x, y, [(x * 2) as u8, 140, (y * 3) as u8, 255]);
        }
    }
    let mut render = layers::Rgba::new(60, 40, [20, 20, 20, 255]);
    for y in 10..30 {
        for x in 15..45 {
            render.set(x, y, [230, 230, 230, 255]);
        }
    }
    write_png(&folder.join("plate.png"), width, height, &plate.data);
    write_png(&folder.join("render.png"), 60, 40, &render.data);

    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(folder.join("plate.png").display().to_string()) });
    let opened = program.raster(0);
    program.command("previs_toggle", json!({}));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id: asked, path: Some(folder.join("render.png").display().to_string()) });
    let checked = program.raster(opened.revision);
    let mut edges = plate.clone();
    layers::edges::check(&mut edges, &render, layers::edges::CYAN);
    assert_ne!(edges, plate, "the render has no edges");
    assert!(std::fs::read(&checked.image).unwrap() == edges.data, "the image shown is not the plate with the render's edges");
    let on = program.wait(|message| match message {
        AppMessage::Toolbar(toolbar) => toolbar.trailing.iter().find_map(|item| match item {
            protocol::ToolItem::Button { icon, selected, .. } if icon == "cube" => Some(*selected),
            _ => None,
        }),
        _ => None,
    });
    assert!(on, "the check's button is not shown on");

    program.command("export_png", json!({}));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Save, .. } => Some(id.clone()),
        _ => None,
    });
    let exported = folder.join("Out.png");
    program.send(HostMessage::FileResult { id: asked, path: Some(exported.display().to_string()) });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !exported.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(image::open(&exported).unwrap().into_rgba8().into_raw() == plate.data, "the export has the check");

    let property = |prop: &str, value: protocol::PropValue| HostMessage::Property { prop: json!(prop), value: serde_json::to_value(value).unwrap(), preview: false };
    program.send(property("previs_opacity", protocol::PropValue::Number(0.4)));
    program.send(property("previs_mode", protocol::PropValue::Index(1)));
    let mut through = plate.clone();
    layers::edges::see_through(&mut through, &render, 0.4);
    let seen = program.wait(|message| match message {
        AppMessage::Raster(view) if std::fs::read(&view.image).is_ok_and(|bytes| bytes == through.data) => Some(view.clone()),
        _ => None,
    });
    assert!(seen.revision > checked.revision);

    let file = keep(&mut program, &opened.tab);
    let saved = header(&file)["pictures"][0]["previs"].clone(); // the stored spelling
    assert_eq!(saved["render"]["path"], "render.png");
    assert_eq!(saved["mode"], "see_through");
    assert_eq!(saved["on"], true);
    assert!((saved["opacity"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

// Requests: the Prompt page's cell, rounds of takes, the round views, a
// next round, previews, and the project's `work/` folder.

/// The takes' colours, each a flat image, and a reference's.
const TEAL: [u8; 3] = [30, 160, 200];
const GREEN: [u8; 3] = [40, 190, 70];
const BLUE: [u8; 3] = [50, 60, 210];
const GOLD: [u8; 3] = [230, 180, 20];
const PLATE: [u8; 3] = [120, 110, 100];

fn flat(colour: [u8; 3], width: u32, height: u32) -> Vec<u8> {
    [colour[0], colour[1], colour[2], 255].repeat((width * height) as usize)
}

/// A project with four takes in `library/`, 160 × 90, and a reference in
/// `refs/`.
fn with_takes(label: &str) -> PathBuf {
    let folder = scratch(label);
    std::fs::create_dir_all(folder.join("library")).unwrap();
    std::fs::create_dir_all(folder.join("refs")).unwrap();
    for (name, colour) in [("r1-1", TEAL), ("r1-2", GREEN), ("r2-1", BLUE), ("r2-2", GOLD)] {
        write_png(&folder.join(format!("library/{name}.png")), 160, 90, &flat(colour, 160, 90));
    }
    write_png(&folder.join("refs/plate.png"), 160, 90, &flat(PLATE, 160, 90));
    folder
}

fn take(path: &str, canon: bool) -> protocol::TakeFile {
    protocol::TakeFile { path: path.into(), media_type: "image/png".into(), canon, ..Default::default() }
}

fn round(request: &str, number: u32, takes: Vec<protocol::TakeFile>) -> protocol::Round {
    protocol::Round {
        request: request.into(),
        number,
        model: "Model A".into(),
        model_name: String::new(),
        prompt: "The keeper turns from the window toward the radio as it crackles.".into(),
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

/// The tab's cell, from the first `cells` that names one.
fn cell_of(program: &Program) -> String {
    program.wait(|message| match message {
        AppMessage::Cells { tab, selected: Some(cell), .. } if tab == "1" => Some(cell.clone()),
        _ => None,
    })
}

fn send_takes(program: &mut Program, cell: &str, rounds: Vec<protocol::Round>) {
    let cell = protocol::ItemRef { path: "work/Untitled Image.ipx".into(), item: Some(cell.into()) };
    program.send(HostMessage::Takes { cell, rounds });
}

/// The next view whose filmstrip `pick` takes.
fn strip(program: &Program, mut pick: impl FnMut(&protocol::Film) -> bool) -> (RasterView, protocol::Film) {
    program.wait(|message| match message {
        AppMessage::Raster(view) => view.film.clone().filter(|film| pick(film)).map(|film| (view.clone(), film)),
        _ => None,
    })
}

fn runs(film: &protocol::Film) -> Vec<(String, usize)> {
    film.groups.iter().map(|group| (group.label.clone(), group.count)).collect()
}

/// Where a flat colour is in an image: its box, `[left, top, right,
/// bottom]`, or none.
fn found(bytes: &[u8], width: u32, colour: [u8; 3]) -> Option<[u32; 4]> {
    let mut boxed: Option<[u32; 4]> = None;
    for (index, pixel) in bytes.chunks_exact(4).enumerate() {
        if pixel[..3] == colour {
            let (x, y) = (index as u32 % width, index as u32 / width);
            let held = boxed.get_or_insert([x, y, x, y]);
            *held = [held[0].min(x), held[1].min(y), held[2].max(x), held[3].max(y)];
        }
    }
    boxed
}

/// Catches takes kept in the workspace's file, or a strip that does not
/// follow the requests' records: `takes` fills the strip with each round
/// under "Round N · model", a canon take sealed, a round being made as its
/// waiting places, and the gallery's own images after them; sent again
/// without a take, as when its report is deleted, its tile leaves, and
/// with one added by hand, it appears; and the tab's file, in the
/// project's `work/` folder, names no take.
#[test]
fn takes_fill_the_strip_by_round_and_are_never_kept() {
    let folder = with_takes("takes");
    let mut program = Program::start(false);
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    let running = protocol::Round { state: protocol::RoundState::Running, count: Some(2), ..round("q2", 2, Vec::new()) };
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take("library/r1-1.png", false), take("library/r1-2.png", true)]), running.clone()]);
    let (_, film) = strip(&program, |film| !film.groups.is_empty());
    assert_eq!(runs(&film), [("Round 1 · Model A".to_string(), 2), ("Round 2 · Model A".to_string(), 2), (String::new(), 1)]);
    assert_eq!(film.items.len(), 5);
    assert_eq!(film.items[1].mark.as_deref(), Some("badge-check"), "the canon take has no seal");
    assert_eq!(film.items[0].mark, None);
    assert!(matches!(film.items[2].pending, Some(protocol::FilmPending::Running { since: Some(1_793_146_000) })));
    assert_eq!(film.shown, 4, "the gallery's image is not the one shown");
    let thumbnail = &film.items[0].thumbnail;
    assert_eq!((thumbnail.width, thumbnail.height), (78, 44));
    assert_eq!(pixel(Path::new(&thumbnail.path), 78, 39, 22), [TEAL[0], TEAL[1], TEAL[2], 255], "the thumbnail is not the take's file");
    let menu: Vec<&str> = film.items[0].menu.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(menu, ["Next round from this", "Use this take", "Edit", "Open report"]);

    // An image of the gallery's own, so there is something to keep.
    program.command("film_new", json!({}));
    strip(&program, |film| film.items.len() == 6);
    // The first take's report is gone.
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take("library/r1-2.png", true)]), running.clone()]);
    let (_, film) = strip(&program, |film| film.groups.first().is_some_and(|group| group.count == 1));
    assert_eq!(runs(&film), [("Round 1 · Model A".to_string(), 1), ("Round 2 · Model A".to_string(), 2), (String::new(), 2)]);
    assert_eq!(pixel(Path::new(&film.items[0].thumbnail.path), 78, 39, 22), [GREEN[0], GREEN[1], GREEN[2], 255]);
    // A take added by hand.
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take("library/r1-2.png", true), take("library/r2-1.png", false)]), running]);
    let (_, film) = strip(&program, |film| film.groups.first().is_some_and(|group| group.count == 2));
    assert_eq!(pixel(Path::new(&film.items[1].thumbnail.path), 78, 39, 22), [BLUE[0], BLUE[1], BLUE[2], 255]);

    let file = keep(&mut program, "1");
    assert_eq!(file, folder.join("work/Untitled Image.ipx"), "a project's image is not in work/");
    let kept = header(&file).to_string();
    assert!(!kept.contains("library/") && !kept.contains("\"q1\""), "a take was kept in the image's file: {kept}");
    assert_eq!(header(&file)["cell"], json!(cell), "the Prompt page's cell was not kept");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a next round that loses the notes or the take, or sends a model
/// a file only in `work/`: with a pin's words typed on a take, Next round
/// from this writes the marked image, the take with the pin on it, beside
/// the tab's file in `work/` and files it from the tab's file and the take;
/// it makes the tab's cell a draft whose prompt is the round's, then "Notes
/// on the last round: 1. …", and whose references are the round's, in
/// order, and the take with the role Reference; and once `filed` names the
/// marked image, it joins the draft as that file, with the role "Notes on
/// the last round".
#[test]
fn next_round_from_a_take_carries_its_notes() {
    let folder = with_takes("next-round");
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: [1200.0, 800.0] });
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    let mut first = round("q1", 1, vec![take("library/r1-1.png", false), take("library/r1-2.png", false)]);
    first.references = vec![protocol::SentReference { path: "refs/plate.png".into(), role: Some("Control, the room's layout".into()) }];
    send_takes(&mut program, &cell, vec![first]);
    strip(&program, |film| !film.groups.is_empty());

    program.command("film_select", json!({ "value": 0 }));
    let shown = program.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (160, 90) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(shown.film.as_ref().unwrap().shown, 0);
    assert!(shown.film.as_ref().unwrap().items[0].chosen, "the take shown is not chosen");
    assert_eq!(pixel(Path::new(&shown.image), 160, 80, 45), [TEAL[0], TEAL[1], TEAL[2], 255], "the take is not read from its file");
    program.command("tool", json!({ "tool": "notes" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Click { at: [40.0, 30.0] } });
    program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_some() => Some(()),
        _ => None,
    });
    program.send(HostMessage::RasterText { tab: "1".into(), input: protocol::TextInput::Insert { text: "less rain".into() } });
    program.command("take_next_round", json!({ "take": "library/r1-1.png" }));
    let (filing, written, from) = program.wait(|message| match message {
        AppMessage::FileResource { path, from, id: Some(id), .. } => Some((id.clone(), path.clone(), from.clone())),
        _ => None,
    });
    assert_eq!(written, "work/Untitled Image notes.png");
    assert_eq!(from, ["work/Untitled Image.ipx", "library/r1-1.png"]);
    let marked = image::open(folder.join(&written)).unwrap().into_rgba8();
    assert_eq!(marked.dimensions(), (160, 90));
    assert_eq!(marked.get_pixel(140, 80).0, [TEAL[0], TEAL[1], TEAL[2], 255], "the marked image is not the take");
    let near: Vec<[u8; 4]> = (36..45).map(|x| marked.get_pixel(x, 30).0).collect();
    assert!(near.iter().any(|pixel| pixel[..3] != TEAL), "the marked image has no pin");
    let draft = program.wait(|message| match message {
        AppMessage::Cells { cells, selected: Some(selected), .. } => {
            cells.iter().find(|own| own.id == *selected && own.prompt().contains("Notes on the last round")).cloned()
        }
        _ => None,
    });
    assert!(draft.step.is_none(), "a next round edited a step");
    assert_eq!(draft.prompt(), "The keeper turns from the window toward the radio as it crackles. Notes on the last round: 1. less rain");
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
        [
            ("refs/plate.png".to_string(), Some("Control, the room's layout".to_string())),
            ("library/r1-1.png".to_string(), Some("Reference".to_string()))
        ]
    );
    program.send(HostMessage::Filed { id: filing, file: "library/notes-1.png".into(), error: None });
    let filed = program.wait(|message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().find(|own| own.id == draft.id).and_then(|own| {
            own.input.iter().find_map(|part| match part {
                protocol::CellPart::Path { path, media_type, role, .. } if role.as_deref() == Some("Notes on the last round") => Some((path.clone(), media_type.clone())),
                _ => None,
            })
        }),
        _ => None,
    });
    assert_eq!(filed, ("library/notes-1.png".to_string(), "image/png".to_string()));
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a round view that draws the wrong takes, or in the wrong
/// places: the grid shows the shown take's round, its reference to the
/// left of its takes, the takes in order, the one shown outlined, and no
/// take of another round; a take of another round chosen with Shift
/// shows both rounds, the older above, each in order; the references
/// sheet shows the reference beside the take.
#[test]
fn the_round_views_draw_the_right_takes_in_the_right_places() {
    let folder = with_takes("round-views");
    let mut program = Program::start(false);
    program.send(HostMessage::Viewport { viewport: [1200.0, 800.0] });
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    let mut first = round("q1", 1, vec![take("library/r1-1.png", false), take("library/r1-2.png", false)]);
    first.references = vec![protocol::SentReference { path: "refs/plate.png".into(), role: Some("Control, the room's layout".into()) }];
    first.left_out = vec![protocol::SentReference { path: "library/r2-2.png".into(), role: None }];
    let second = round("q2", 2, vec![take("library/r2-1.png", false), take("library/r2-2.png", false)]);
    send_takes(&mut program, &cell, vec![first, second]);
    let (_, film) = strip(&program, |film| !film.groups.is_empty());
    program.command("film_select", json!({ "value": 0 }));
    program.command("round_grid", json!({}));
    let grid = program.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (1200, 800) => Some(view.clone()),
        _ => None,
    });
    assert!(matches!(grid.tool, protocol::RasterTool::Hand), "a round view can be drawn on");
    let bytes = std::fs::read(&grid.image).unwrap();
    let (teal, green, plate) = (found(&bytes, 1200, TEAL).unwrap(), found(&bytes, 1200, GREEN).unwrap(), found(&bytes, 1200, PLATE).unwrap());
    assert!(plate[2] < teal[0] && plate[2] < green[0], "the references are not left of the takes");
    assert!(teal[1] < green[1] || (teal[1] == green[1] && teal[2] < green[0]), "the takes are not in order");
    assert!(found(&bytes, 1200, BLUE).is_none() && found(&bytes, 1200, GOLD).is_none(), "another round's take is in the grid");
    let outline = ((teal[1] + teal[3]) / 2 * 1200 + teal[0] - 2) as usize * 4;
    assert_eq!(bytes[outline..outline + 3], [37, 99, 235], "the take shown is not outlined");

    program.send(HostMessage::Command { id: "film_select".into(), tab: Some("1".into()), arg: film.select_message(2, true).arg, node: None, viewport: [1200.0, 800.0] });
    let pair = program.raster(grid.revision);
    let bytes = std::fs::read(&pair.image).unwrap();
    let [teal, green, blue, gold] = [TEAL, GREEN, BLUE, GOLD].map(|colour| found(&bytes, pair.width, colour).expect("a take is missing"));
    assert!(teal[3] < blue[1] && green[3] < gold[1], "the older round is not above");
    assert!(teal[2] < green[0] && blue[2] < gold[0], "a round's takes are not in order");

    program.command("round_sheet", json!({}));
    let sheet = program.raster(pair.revision);
    let bytes = std::fs::read(&sheet.image).unwrap();
    let (plate, teal) = (found(&bytes, sheet.width, PLATE).unwrap(), found(&bytes, sheet.width, TEAL).unwrap());
    assert!(plate[2] < teal[0], "the references are not beside the take");
    assert!(found(&bytes, sheet.width, GOLD).is_some_and(|gold| gold[1] > plate[3]), "the reference not sent is not under those sent");
    assert!(found(&bytes, sheet.width, GREEN).is_none(), "another take is on the sheet");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a reference the host cannot show: `edits` names PNG, JPEG, and
/// WebP, and `preview` of each answers with a raw image of its size, no
/// larger than a reference row needs, of the file's colour, and no words
/// for its corner; a file that is no image answers with why.
#[test]
fn png_jpeg_and_webp_are_previewed() {
    let folder = scratch("preview");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([TEAL[0], TEAL[1], TEAL[2], 255])).save(folder.join("small.png")).unwrap();
    image::RgbImage::from_pixel(640, 360, image::Rgb(GOLD)).save(folder.join("wide.jpg")).unwrap();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([BLUE[0], BLUE[1], BLUE[2], 255])).save(folder.join("square.webp")).unwrap();
    std::fs::write(folder.join("notes.txt"), "not an image").unwrap();
    let mut program = Program::start(false);
    let edits = program.wait(|message| match message {
        AppMessage::Edits { media_types } => Some(media_types.clone()),
        _ => None,
    });
    assert_eq!(edits, ["image/png", "image/jpeg", "image/webp"]);
    program.send(in_project(&folder));
    let mut preview = |id: &str, path: &str, media_type: &str| {
        program.send(HostMessage::Preview { id: id.into(), path: path.into(), media_type: media_type.into() });
        program.wait(|message| match message {
            AppMessage::Previewed { id: answered, thumbnail, badge, error, .. } if answered == id => Some((thumbnail.clone(), badge.clone(), error.clone())),
            _ => None,
        })
    };
    for (path, media_type, size, colour, close) in [
        ("small.png", "image/png", (40, 20), TEAL, 0),
        ("wide.jpg", "image/jpeg", (160, 90), GOLD, 8),
        ("square.webp", "image/webp", (64, 64), BLUE, 0),
    ] {
        let (thumbnail, badge, error) = preview(path, path, media_type);
        assert_eq!((error, badge.as_str()), (None, ""), "{path}");
        let thumbnail = thumbnail.expect("no thumbnail");
        assert_eq!((thumbnail.width, thumbnail.height), size, "{path}");
        let middle = pixel(Path::new(&thumbnail.path), size.0, size.0 / 2, size.1 / 2);
        assert!((0..3).all(|channel| middle[channel].abs_diff(colour[channel]) <= close), "{path}: {middle:?}");
    }
    let (thumbnail, _, error) = preview("text", "notes.txt", "image/png");
    assert!(thumbnail.is_none() && error.is_some(), "a file that is no image was previewed");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches notes on a take lost when the program ends, or a take kept in
/// their place: a pin with its words on a take is kept in the tab's file
/// by the take's path and sha256 alone, and a new program that opens the
/// file and is sent the same rounds shows the take with its pin again.
#[test]
fn notes_on_a_take_survive_a_restart() {
    let folder = with_takes("take-notes");
    let rounds = || vec![round("q1", 1, vec![take("library/r1-1.png", false), take("library/r1-2.png", false)])];
    let mut program = Program::start(false);
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    send_takes(&mut program, &cell, rounds());
    strip(&program, |film| !film.groups.is_empty());
    program.command("film_select", json!({ "value": 0 }));
    program.command("tool", json!({ "tool": "notes" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Click { at: [40.0, 30.0] } });
    program.wait(|message| match message {
        AppMessage::Raster(view) if view.text.is_some() => Some(()),
        _ => None,
    });
    program.send(HostMessage::RasterText { tab: "1".into(), input: protocol::TextInput::Insert { text: "less rain".into() } });
    // Another tool ends the typing, which makes the pin a step.
    program.command("tool", json!({ "tool": "brush" }));
    let file = keep(&mut program, "1");
    drop(program);
    let kept = header(&file);
    let notes = &kept["take_notes"];
    assert_eq!(notes.as_array().map(Vec::len), Some(1), "{kept}");
    assert_eq!(notes[0]["path"], "library/r1-1.png");
    assert_eq!(notes[0]["sha256"], json!(layers::link::sha256_file(&folder.join("library/r1-1.png")).unwrap()));
    assert_eq!(notes[0]["notes"]["pins"][0]["text"], "less rain");
    assert!(!kept.to_string().contains("\"q1\"") && !kept.to_string().contains("Model A"), "a take's record was kept");

    let mut again = Program::start(true);
    again.send(in_project(&folder));
    again.send(HostMessage::Open { path: Some(file.display().to_string()) });
    assert_eq!(cell_of(&again), cell, "the tab's cell was not kept");
    send_takes(&mut again, &cell, rounds());
    strip(&again, |film| !film.groups.is_empty());
    again.command("film_select", json!({ "value": 0 }));
    let shown = again.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (160, 90) => Some(view.clone()),
        _ => None,
    });
    let near: Vec<[u8; 4]> = (36..45).map(|x| pixel(Path::new(&shown.image), 160, x, 30)).collect();
    assert!(near.iter().any(|pixel| pixel[..3] != TEAL), "the take's pin did not come back");
    assert_eq!(pixel(Path::new(&shown.image), 160, 140, 80), [TEAL[0], TEAL[1], TEAL[2], 255]);
    let rows = layer_rows(&again, |rows| rows.first().is_some_and(|row| row.name == "Notes"));
    assert_eq!(rows.len(), 2, "the take's notes have no Notes row");
    drop(again);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches a draft kept after it became a step, a report that cannot be
/// opened, or a layer written out that the runner is not asked to file:
/// Open report asks for the take's round's report; a layer written out is
/// a PNG in the project's `work/`, and
/// `file_resource` names it and that file; `cell_became_step` leaves the
/// cell its pointer alone, and no undo brings its draft back, in the file
/// too.
#[test]
fn a_draft_becomes_its_step_and_what_leaves_is_filed() {
    let folder = with_takes("steps");
    let mut program = Program::start(false);
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take("library/r1-1.png", false)])]);
    let (_, film) = strip(&program, |film| !film.groups.is_empty());
    let report = film.items[0].menu.iter().find(|row| row.label == "Open report").expect("no Open report");
    assert!(report.enabled, "Open report is off");
    program.command("take_report", report.action.as_ref().unwrap().arg.clone().unwrap());
    let request = program.wait(|message| match message {
        AppMessage::OpenReport { request } => Some(request.clone()),
        _ => None,
    });
    assert_eq!(request, "q1");

    // Two takes opened as layers, each linked to its file, r1-1 above
    // r1-2, and a new layer above both, which is written out.
    for (count, take) in [(2, "library/r1-2.png"), (3, "library/r1-1.png")] {
        program.command("open_layer", json!({}));
        let asked = program.wait(|message| match message {
            AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
            _ => None,
        });
        program.send(HostMessage::FileResult { id: asked, path: Some(folder.join(take).display().to_string()) });
        layer_rows(&program, |rows| rows.len() == count && rows[0].mark.as_deref() == Some("link"));
    }
    program.command("add_layer", json!({}));
    let rows = layer_rows(&program, |rows| rows.len() == 4);
    assert!(rows[0].menu.iter().any(|row| row.label == "Write out as PNG…"));
    let filed = |program: &Program| {
        program.wait(|message| match message {
            AppMessage::FileResource { tab, path, from, .. } if tab == "1" => Some((path.clone(), from.clone())),
            _ => None,
        })
    };
    program.command("layer_write_png", json!({ "node": rows[0].id }));
    let (path, from) = filed(&program);
    assert!(path.starts_with("work/") && path.ends_with(".png"), "{path}");
    assert_eq!(from, ["work/Untitled Image.ipx", "library/r1-2.png", "library/r1-1.png"], "the runner is not told what the layer came from, lowest first");
    assert!(image::open(folder.join(&path)).is_ok(), "the layer was not written out as a PNG");
    // The take r1-1 painted on, and written out: its own file comes
    // second, before the take beneath it.
    let linked = rows[1].id;
    program.command("layer_click", json!({ "node": linked, "value": false }));
    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture { tab: "1".into(), gesture: RasterGesture::Stroke { points: vec![[900.0, 540.0, 1.0], [1000.0, 540.0, 1.0]] } });
    layer_rows(&program, |rows| rows.len() == 4 && rows[1].mark.as_deref() == Some("edit-pencil"));
    program.command("layer_write_png", json!({ "node": linked }));
    let (path, from) = filed(&program);
    assert!(image::open(folder.join(&path)).is_ok(), "the painted layer was not written out as a PNG");
    assert_eq!(from, ["work/Untitled Image.ipx", "library/r1-1.png", "library/r1-2.png"], "the painted take's own file is not second");

    let words = "The keeper turns.";
    program.send(HostMessage::EditCell {
        tab: "1".into(),
        cell: cell.clone(),
        input: vec![protocol::CellPart::Text { text: words.into() }],
        settings: None,
        endpoint: None,
        model: Some("a model".into()),
    });
    program.wait(|message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().any(|own| own.prompt() == words).then_some(()),
        _ => None,
    });
    let step = protocol::StepRef { chain: "shot-12".into(), step: "keyframe".into() };
    program.send(HostMessage::CellBecameStep { tab: "1".into(), cell: cell.clone(), step: step.clone(), artifact: None });
    let pointed = program.wait(|message| match message {
        AppMessage::Cells { cells, .. } => cells.iter().find(|own| own.id == cell && own.step.is_some()).cloned(),
        _ => None,
    });
    assert_eq!(pointed.step, Some(step));
    assert!(!pointed.copies_its_step(), "the step's cell kept its draft");
    program.command("film_focus", json!({}));
    program.command("undo", json!({}));
    let file = keep(&mut program, "1");
    let kept = header(&file);
    assert_eq!(kept["cells"][0]["step"]["chain"], "shot-12", "{kept}");
    assert!(!kept.to_string().contains(words) && !kept.to_string().contains("a model"), "a draft came back: {kept}");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

// In use: a take used, beside the takes in use.

/// Everything the program answers until the host's ping is answered.
fn until_pong(program: &mut Program) -> Vec<AppMessage> {
    program.send(HostMessage::Ping);
    let mut seen = Vec::new();
    program.wait(|message| {
        seen.push(message.clone());
        matches!(message, AppMessage::Pong).then_some(())
    });
    seen
}

/// Catches a take that cannot be put in use, or a use that says nothing
/// when it fails: a take's menu has Use this take, and no way to ask the
/// assistant; it sends `use_take` with the tab, the cell's item as `takes`
/// names it, and the take's path; the host's `take_used` with an error
/// shows that sentence as a failed notice, and one with none says nothing,
/// as the seal comes with the next `takes`.
#[test]
fn use_this_take_asks_the_host_to_use_it() {
    let folder = with_takes("use-take");
    let mut program = Program::start(false);
    program.send(in_project(&folder));
    let cell = cell_of(&program);
    send_takes(&mut program, &cell, vec![round("q1", 1, vec![take("library/r1-1.png", false)])]);
    let (view, _) = strip(&program, |film| !film.groups.is_empty());
    let menu: Vec<&str> = view.film.as_ref().unwrap().items[0].menu.iter().map(|row| row.label.as_str()).collect();
    assert!(menu.contains(&"Use this take") && !menu.contains(&"Ask the assistant to use this take"), "{menu:?}");

    program.command("take_use", json!({ "take": "library/r1-1.png" }));
    let (tab, item, path) = program.wait(|message| match message {
        AppMessage::UseTake { tab, cell, path } => Some((tab.clone(), cell.clone(), path.clone())),
        _ => None,
    });
    assert_eq!((tab.as_str(), path.as_str()), ("1", "library/r1-1.png"));
    assert_eq!(item, protocol::ItemRef { path: "work/Untitled Image.ipx".into(), item: Some(cell.clone()) });

    program.send(HostMessage::TakeUsed { tab: "1".into(), path: "library/r1-1.png".into(), artifact: None, error: Some("The take could not be put in use.".into()) });
    let notice = program.wait(|message| match message {
        AppMessage::Notices { notices, .. } => notices.first().cloned(),
        _ => None,
    });
    assert_eq!((notice.text.as_str(), notice.state), ("The take could not be put in use.", protocol::NoticeState::Failed));
    // A success takes the refusal away, and after that says nothing.
    program.send(HostMessage::TakeUsed { tab: "1".into(), path: "library/r1-1.png".into(), artifact: Some("shot 12: keyframe".into()), error: None });
    let replies = until_pong(&mut program);
    let said: Vec<&Vec<protocol::Notice>> = replies.iter().filter_map(|message| match message {
        AppMessage::Notices { notices, .. } => Some(notices),
        _ => None,
    }).collect();
    assert!(said.len() == 1 && said[0].is_empty(), "the refusal was not taken away: {said:?}");
    program.send(HostMessage::TakeUsed { tab: "1".into(), path: "library/r1-1.png".into(), artifact: Some("shot 12: keyframe".into()), error: None });
    assert!(
        !until_pong(&mut program).iter().any(|message| matches!(message, AppMessage::Notices { .. })),
        "a take put in use said something"
    );
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// The draft output of the tab `pick` takes, from the next `cells` that
/// names a draft for it: the tab, and the selected cell's output.
fn draft_output(program: &Program, mut pick: impl FnMut(&str, &[protocol::CellPart]) -> bool) -> (String, Vec<protocol::CellPart>) {
    program.wait(|message| match message {
        AppMessage::Cells { tab, cells, selected: Some(selected) } => {
            cells.iter().find(|cell| cell.id == *selected).map(|cell| (tab.clone(), cell.output.clone())).filter(|(tab, output)| pick(tab, output))
        }
        _ => None,
    })
}

/// Catches a draft that hides which take its image was opened from, or one
/// that keeps it: a tab whose image links nothing has a draft with no
/// output; an image opened from a project file sends `cells` whose draft's
/// output is that file, as the project names it, so the host can find its
/// round; the `.ipx` keeps no output; and after a restart, `cells` carries
/// it again.
#[test]
fn a_draft_says_the_file_its_image_was_opened_from() {
    let folder = with_takes("draft-output");
    let mut program = Program::start(false);
    let (_, empty) = draft_output(&program, |tab, _| tab == "1");
    assert!(empty.is_empty(), "a draft whose image links nothing has an output");
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(folder.join("library/r1-1.png").display().to_string()) });
    let (tab, output) = draft_output(&program, |tab, output| tab != "1" && !output.is_empty());
    let opened = [protocol::CellPart::Path { path: "library/r1-1.png".into(), media_type: "image/png".into(), role: None, field: None }];
    assert_eq!(output, opened);
    let file = keep(&mut program, &tab);
    let kept = header(&file);
    assert!(
        kept["cells"].as_array().into_iter().flatten().all(|cell| cell["output"].as_array().is_none_or(|output| output.is_empty())),
        "the file kept a draft's output: {kept}"
    );
    program.send(HostMessage::Shutdown);
    drop(program);

    let mut program = Program::start(true);
    program.send(in_project(&folder));
    program.send(HostMessage::Open { path: Some(file.display().to_string()) });
    let (_, again) = draft_output(&program, |_, output| !output.is_empty());
    assert_eq!(again, opened, "after a restart, the draft does not say its file");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}
