//! The Image Editor as its own program, driven through the protocol alone.

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
        let mut command = Command::new(env!("CARGO_BIN_EXE_image-editor"));
        if restore {
            command.arg(protocol::RESTORE_FLAG);
        }
        set(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("image-editor starts");
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

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("image-editor-session-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir.canonicalize().expect("canonical")
}

fn pixel(path: &Path, width: u32, x: u32, y: u32) -> [u8; 4] {
    let bytes = std::fs::read(path).expect("the picture file");
    let at = ((y * width + x) * 4) as usize;
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

/// Where the pictures for the host are: in memory when the system has a
/// runtime folder, or else in the project's cache.
fn display_folder(project: &Path) -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|runtime| runtime.is_absolute() && runtime.is_dir()) {
        Some(runtime) => runtime.join("inpainter/image-editor"),
        None => project.join(".inpainter/cache/image-editor"),
    }
}

/// Catches a picture the host could not read, a stroke that reaches no
/// pixel, an undo that does not restore it, and an image that waits for
/// Save: inside a project, the program, given only protocol lines, sends a
/// `raster` whose picture file is width × height × 4 bytes in memory or the
/// project's cache; a stroke paints it; undo restores it; and a second
/// later the image is in the project as an `.ipx` file that reopens with
/// its layers.
#[test]
fn a_stroke_reaches_the_picture_and_the_project() {
    let folder = scratch("stroke");
    let mut program = Program::start(false);
    let first = program.raster(0);
    assert_eq!((first.width, first.height), (1920, 1080));
    assert!(first.camera.is_some(), "a new picture came without a camera");

    program.send(HostMessage::Project { folder: folder.display().to_string() });
    let view = program.raster(0);
    let picture = PathBuf::from(&view.picture);
    assert!(picture.starts_with(display_folder(&folder)), "{}", picture.display());
    assert_eq!(std::fs::metadata(&picture).unwrap().len(), 1920 * 1080 * 4);
    assert_eq!(pixel(&picture, 1920, 500, 300), [0, 0, 0, 0]);

    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture {
        tab: "1".into(),
        gesture: RasterGesture::Stroke { points: vec![[400.0, 300.0, 1.0], [600.0, 300.0, 1.0]] },
    });
    let painted = program.raster(view.revision);
    assert_eq!(painted.picture, view.picture);
    assert_eq!(std::fs::metadata(&picture).unwrap().len(), 1920 * 1080 * 4);
    assert_eq!(pixel(&picture, 1920, 500, 300), [0, 0, 0, 255], "the stroke did not reach the picture");
    assert_eq!(pixel(&picture, 1920, 500, 400), [0, 0, 0, 0]);

    program.command("undo", json!({}));
    let undone = program.raster(painted.revision);
    assert_eq!(pixel(&PathBuf::from(&undone.picture), 1920, 500, 300), [0, 0, 0, 0]);
    program.command("redo", json!({}));
    program.raster(undone.revision);

    // A second after the change it is written by itself.
    let written = folder.join("Untitled Asset.ipx");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !written.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(written.is_file(), "nothing was written into the project");
    drop(program);

    let mut again = Program::start(true);
    again.send(HostMessage::Project { folder: folder.display().to_string() });
    again.send(HostMessage::Claim { id: "c1".into(), path: written.display().to_string() });
    assert!(again.wait(|message| match message {
        AppMessage::Claimed { id, yours } if id == "c1" => Some(*yours),
        _ => None,
    }));
    again.send(HostMessage::Open { path: Some(written.display().to_string()) });
    let reopened = again.raster(0);
    assert_eq!((reopened.width, reopened.height), (1920, 1080));
    assert_eq!(pixel(&PathBuf::from(&reopened.picture), 1920, 500, 300), [0, 0, 0, 255], "the stroke was not kept");
    let rows = again.wait(|message| match message {
        AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.len()),
        _ => None,
    });
    assert_eq!(rows, 1);
    drop(again);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches the pictures for the host written to disk while the system has
/// a folder in memory, or left behind there: with one, the picture is in
/// `inpainter/image-editor` under it, stays there when a project is named,
/// and goes when the program ends, and a folder left by a program no longer
/// running is removed; without one, the picture is in the project's cache.
#[test]
fn the_pictures_for_the_host_are_in_memory_when_there_is_some() {
    let folder = scratch("memory");
    let runtime = folder.join("run");
    let project = folder.join("project");
    let left = runtime.join("inpainter/image-editor/999999999-1");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let mut program = Program::start_with(false, |command| {
        command.env("XDG_RUNTIME_DIR", &runtime);
    });
    let first = program.raster(0);
    let picture = PathBuf::from(&first.picture);
    assert!(picture.starts_with(runtime.join("inpainter/image-editor")), "{}", picture.display());
    assert_eq!(std::fs::metadata(&picture).unwrap().len(), 1920 * 1080 * 4);
    assert!(!left.exists(), "a folder left by a program no longer running stayed");
    program.send(HostMessage::Project { folder: project.display().to_string() });
    let view = program.wait(|message| match message {
        AppMessage::Raster(view) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(view.picture, first.picture, "naming a project moved the pictures out of memory");
    assert!(!project.join(".inpainter/cache/image-editor").exists(), "pictures were written into the project's cache too");
    program.send(HostMessage::Shutdown);
    let gone = picture.parent().unwrap().to_path_buf();
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
    program.send(HostMessage::Project { folder: project.display().to_string() });
    let view = program.wait(|message| match message {
        AppMessage::Raster(view) => Some(view.clone()),
        _ => None,
    });
    assert!(PathBuf::from(&view.picture).starts_with(project.join(".inpainter/cache/image-editor")), "{}", view.picture);
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches Open and Export that do not go through the host's file dialog,
/// or that lose the picture's size or pixels on the way.
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
    program.send(HostMessage::Project { folder: folder.display().to_string() });
    program.raster(0);
    program.command("open", json!({}));
    let asked = program.wait(|message| match message {
        AppMessage::File { id, kind: protocol::FileKind::Open, .. } => Some(id.clone()),
        _ => None,
    });
    program.send(HostMessage::FileResult { id: asked, path: Some(source.display().to_string()) });
    let opened = program.raster(0);
    assert_eq!((opened.width, opened.height), (40, 30));
    assert_eq!(std::fs::read(&opened.picture).unwrap(), rgba);

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

    // The same picture opened as a layer of the image is a second layer.
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
/// added picture that reaches another: through protocol lines alone, the
/// + row's action adds a white picture shown on the strip and the canvas,
/// a stroke paints only it, a click on the first thumbnail shows the
/// first picture, still white, again, and the + tile's open row adds a PNG
/// as a picture of its own.
#[test]
fn a_picture_added_from_the_strip_is_painted_alone() {
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
    assert_eq!(strip.shown, 1, "the added picture is not the one shown");
    assert_eq!(strip.items[1].name, "Untitled Asset 2");
    assert_eq!((added.width, added.height), (1920, 1080));

    program.command("tool", json!({ "tool": "brush" }));
    program.send(HostMessage::RasterGesture {
        tab: "1".into(),
        gesture: RasterGesture::Stroke { points: vec![[400.0, 300.0, 1.0], [600.0, 300.0, 1.0]] },
    });
    let painted = program.raster(added.revision);
    assert_eq!(pixel(Path::new(&painted.picture), 1920, 500, 300), [0, 0, 0, 255], "the stroke missed the added picture");
    let strip = painted.film.clone().unwrap();
    assert_eq!(std::fs::read(&strip.items[0].thumbnail.path).unwrap(), first_thumb, "the stroke reached the first picture");
    let dark = std::fs::read(&strip.items[1].thumbnail.path).unwrap();
    assert!(dark.chunks(4).any(|pixel| pixel[0] < 128), "the added picture's thumbnail does not show the stroke");

    send(&mut program, &strip.focus);
    send(&mut program, &strip.select.with_value(json!(0)));
    // The first picture comes back as the file and revision first sent,
    // which the host still holds.
    let shown = program.wait(|message| match message {
        AppMessage::Raster(view) if view.film.as_ref().is_some_and(|film| film.shown == 0) => Some(view.clone()),
        _ => None,
    });
    assert_eq!((shown.picture.as_str(), shown.revision), (first.picture.as_str(), first.revision));
    assert_eq!(pixel(Path::new(&shown.picture), 1920, 500, 300), [0, 0, 0, 0], "the first picture was painted");

    // The + tile's second row opens a PNG through the host's file dialog
    // as a third picture, shown, and named after its file.
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
    assert_eq!(strip.shown, 2, "the opened picture is not the one shown");
    assert_eq!(strip.items[2].name, "Sunset");
    assert_eq!((opened.width, opened.height), (40, 30));
    assert_eq!(pixel(Path::new(&opened.picture), 40, 5, 5), [200, 90, 30, 255]);
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

/// Catches a picture opened as new work that is not kept, broken by its
/// name, or forgets where it came from: `open_as_new` with a name holding
/// "/" and a line break makes a tab of one picture called by a safe name,
/// written in the project's own folder with the picture's pixels and its
/// source; opened again, painted, and saved, the file keeps its source.
#[test]
fn a_picture_opened_as_new_work_is_kept_with_its_source() {
    let folder = scratch("open-as-new");
    let source = folder.join("lamp.png");
    image::RgbaImage::from_pixel(20, 10, image::Rgba([10, 120, 200, 255])).save(&source).unwrap();
    let from = protocol::OpenedFrom { cell: "c7".into(), picture: "images/lamp.png".into(), session: Some("s3".into()), notebook: None };

    let mut program = Program::start(false);
    program.send(HostMessage::Project { folder: folder.display().to_string() });
    program.raster(0);
    program.send(HostMessage::OpenAsNew { path: source.display().to_string(), name: "AC/DC lamp\nposter".into(), from: Some(from.clone()) });
    let opened = program.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (20, 10) => Some(view.clone()),
        _ => None,
    });
    assert_eq!(opened.film.as_ref().unwrap().items.len(), 1);
    assert_eq!(opened.film.as_ref().unwrap().items[0].name, "AC DC lamp poster");
    assert_eq!(pixel(Path::new(&opened.picture), 20, 3, 3), [10, 120, 200, 255]);
    let written = folder.join("AC DC lamp poster.ipx");
    until_file(&written, None);
    assert_eq!(header(&written)["source"], serde_json::to_value(&from).unwrap(), "the file forgot where it came from");
    drop(program);

    // Opened again, painted, and saved: the source stays.
    let saved = std::fs::metadata(&written).unwrap().modified().unwrap();
    let mut again = Program::start(false);
    again.send(HostMessage::Project { folder: folder.display().to_string() });
    again.raster(0);
    again.send(HostMessage::Open { path: Some(written.display().to_string()) });
    let shown = again.wait(|message| match message {
        AppMessage::Raster(view) if (view.width, view.height) == (20, 10) => Some(view.clone()),
        _ => None,
    });
    again.send(HostMessage::Command { id: "tool".into(), tab: Some(shown.tab.clone()), arg: Some(json!({ "tool": "brush" })), node: None, viewport: [1200.0, 800.0] });
    again.send(HostMessage::RasterGesture { tab: shown.tab.clone(), gesture: RasterGesture::Stroke { points: vec![[2.0, 2.0, 1.0], [18.0, 8.0, 1.0]] } });
    until_file(&written, Some(saved));
    assert_eq!(header(&written)["source"], serde_json::to_value(&from).unwrap(), "a save lost where the work came from");
    drop(again);
    let _ = std::fs::remove_dir_all(folder);
}

/// Catches typing that disappoints, through protocol lines alone: with the
/// T tool, a click makes a text whose box the view carries; typed words
/// reach it; a drag across a word selects it, and Copy answers with
/// `clipboard`; pasting it types it again; Undo while typing takes the
/// typing back, so the picture is white again.
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
    assert!(std::fs::read(&typed.picture).unwrap().chunks(4).any(|pixel| pixel[0] < 128), "the words drew nothing");

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
    assert_eq!(std::fs::read(&undone.picture).unwrap(), std::fs::read(&first.picture).unwrap(), "Undo left the typing");
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
    assert_eq!(pixel(Path::new(&stamped.picture), 1920, 910, 700), [255, 0, 0, 255], "the stamp copied no red");

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
    let file = Path::new(&moved.picture);
    assert_eq!(pixel(file, 1920, 500, 300)[3], 0, "the old place is not clear");
    assert_eq!(pixel(file, 1920, 500, 400), [255, 0, 0, 255], "the line did not move");
    assert_eq!(pixel(file, 1920, 910, 700), [255, 0, 0, 255], "the move reached past the selection");
    assert_eq!(moved.selection.len(), 1);
    assert!(moved.selection[0].iter().all(|point| point[1] >= 375.0), "the selection did not move");
    program.command("undo", json!({}));
    let undone = program.raster(moved.revision);
    assert_eq!(pixel(Path::new(&undone.picture), 1920, 500, 300), [255, 0, 0, 255], "one undo did not put the line back");
    assert_eq!(pixel(Path::new(&undone.picture), 1920, 500, 400), [0, 0, 0, 0]);
}

/// Catches the Adjustments and Properties pages doing nothing through the
/// protocol: Curves from the Adjustments page is added and shown on
/// Properties, and its curve, sent as a property, brightens the picture.
#[test]
fn an_adjustment_is_added_and_set_from_the_panel() {
    // A white picture, as a new one is empty.
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
    // The white picture, darkened by a curve that ends at half.
    program.send(HostMessage::Property {
        prop: json!("curve"),
        value: serde_json::to_value(protocol::PropValue::Curve(vec![(0.0, 0.0), (1.0, 0.5)])).unwrap(),
        preview: false,
    });
    let graded = program.raster(first.revision);
    let value = pixel(Path::new(&graded.picture), 40, 10, 10);
    assert!((126..=129).contains(&value[0]), "the curve made white {value:?}");
    drop(program);
    let _ = std::fs::remove_dir_all(folder);
}
