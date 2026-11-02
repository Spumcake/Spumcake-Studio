//! Window checks for 0.12's Edit on an Image cell's image: the image a
//! cell shows opens as new work in the application that declared it edits
//! images, named after the prompt, and the cell is unchanged. They spend
//! nothing.

use super::{StandIn, Window, built, call_reply, keep_ticking, project_window, scratch, spec, text_reply};
use crate::cells::CardAction;
use eframe::egui::{self, Pos2};
use protocol::{CellPart, CellState};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const PROMPT: &str = "A lighthouse keeper's lamp, AC/DC poster style,\nwith a storm behind it.";

/// A sub-application nothing was written for, with an id of its own, that
/// declares it edits images and writes each line it is sent to a log.
const EASEL: &str = r#"#!/bin/sh
log="{log}"
echo '{"type":"hello","protocol":1}'
echo '{"type":"tab","id":"e1","label":"Easel"}'
echo '{"type":"contents","tab":"e1","kind":"grid"}'
echo '{"type":"edits","media_types":["image/png"]}'
echo '{"type":"end"}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"ping"'*) echo '{"type":"pong"}' ;;
    *'"type":"shutdown"'*) exit 0 ;;
    *'"type":"quit"'*) echo '{"type":"quit_ready"}' ;;
    *'"type":"open_as_new"'*) printf '%s\n' "$line" >> "$log" ;;
  esac
done
"#;

fn easel_program(dir: &Path, log: &Path) -> PathBuf {
    let path = dir.join("easel");
    std::fs::write(&path, EASEL.replace("{log}", &log.display().to_string())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn image_cell(window: &Window) -> protocol::Cell {
    window
        .app
        .chat
        .conversation
        .iter()
        .rev()
        .find_map(|entry| entry.cell.clone().filter(|cell| cell.kind == protocol::IMAGE_KIND))
        .expect("the conversation has no Image cell")
}

fn version_image(cell: &protocol::Cell, index: usize) -> String {
    cell.versions[index]
        .output
        .iter()
        .find_map(|part| match part {
            CellPart::Path { path, .. } => Some(path.clone()),
            _ => None,
        })
        .expect("the version has no image")
}

/// The Image Generator's tabs, by label.
fn editor_tabs(app: &crate::window::HostWindow) -> Vec<String> {
    app.apps
        .iter()
        .find(|started| started.id == "image-generator")
        .map(|started| started.running.tabs().0.into_iter().map(|tab| tab.label).collect())
        .unwrap_or_default()
}

/// A gallery file's header: the line of JSON after its magic.
fn header(path: &Path) -> Value {
    let bytes = std::fs::read(path).unwrap();
    let start = bytes.iter().position(|byte| *byte == b'{').expect("no header");
    let end = start + bytes[start..].iter().position(|byte| *byte == b'\n').expect("no header's end");
    serde_json::from_slice(&bytes[start..end]).unwrap()
}

fn until_file(window: &mut Window, path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !path.is_file() && std::time::Instant::now() < deadline {
        window.step();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(path.is_file(), "{} was never written", path.display());
}

fn press(window: &mut Window, at: Pos2) {
    for pressed in [true, false] {
        window.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        window.step();
    }
}

/// Catches an Edit button that only looks right, opens the wrong version,
/// names the work badly, forgets where it came from, or changes the cell;
/// and an assistant that cannot do it, or says it did when it could not:
/// with the real Image Generator, a cell's image always shows Edit;
/// on the first of two versions, its click opens a new tab named after the
/// prompt, made safe, holding that image, written to the project with
/// the cell, its image, and the session; the assistant's edit_image
/// opens another; and with the image's file gone, it sends nothing and
/// says why.
#[test]
fn a_cells_image_opens_as_new_work() {
    let dir = scratch("edit-image");
    let tool = protocol::tool_key(crate::route::HOST_ID, "skills", "image_generation");
    let stand_in = StandIn::start(vec![call_reply(&tool, json!({ "prompt": PROMPT })), text_reply("Done.")]);
    let mut window = project_window(vec![spec("image-generator", "Image Generator", &built("image-generator"))], &stand_in, &dir);
    window.app.settings.add_image_skill();
    window.until(|app| app.apps.first().is_some_and(|started| !started.running.declared.edits.is_empty()));
    window.ask("Make an image of a lighthouse keeper's lamp.");
    window.until(|app| app.chat.conversation.iter().any(|entry| entry.cell.as_ref().is_some_and(|cell| cell.kind == protocol::IMAGE_KIND && cell.state == CellState::Done)) && !app.chat.working);
    assert!(stand_in.requests().last().unwrap().to_string().contains("Made the image in cell"), "the assistant was not told the cell");

    // A second version, then the first shown again.
    let id = image_cell(&window).id;
    window.app.note_cell_actions(vec![CardAction::Run(id.clone())]);
    let ran = id.clone();
    window.until(move |app| app.board.running(&ran).is_none() && app.chat.conversation.iter().any(|entry| entry.cell.as_ref().is_some_and(|cell| cell.versions.len() == 2)));
    window.app.note_cell_actions(vec![CardAction::ShowVersion(id.clone(), 0)]);
    // The prompt edited and not run: the image is still the old prompt's.
    window.app.note_cell_actions(vec![CardAction::Edit(id.clone(), vec![CellPart::Text { text: "Something else entirely.".into() }])]);
    let cell = image_cell(&window);
    assert_eq!(cell.current, Some(0));
    let first = version_image(&cell, 0);

    // Edit shows on the image at all times, the pointer away.
    if window.app.workspace_shown() {
        window.app.toggle_workspace();
    }
    window.events.push(egui::Event::PointerGone);
    for _ in 0..3 {
        window.step();
    }
    let card = window.app.board.card_rect.expect("the cell was not drawn");
    let image = window.images.iter().copied().filter(|rect| card.contains_rect(*rect)).max_by(|a, b| a.height().total_cmp(&b.height())).expect("no image drawn");
    let edit = window.texts.iter().find(|(text, _)| text == "Edit").map(|(_, rect)| *rect).expect("Edit does not show on the image");
    assert!(image.contains_rect(edit), "Edit is not on the image");
    let tabs_before = editor_tabs(&window.app).len();
    window.events.push(egui::Event::PointerMoved(edit.center()));
    window.step();
    press(&mut window, edit.center());

    let name = "A lighthouse keeper's lamp, AC";
    assert_eq!(crate::cells::image_name(PROMPT), name);
    window.until(move |app| editor_tabs(app).len() > tabs_before && editor_tabs(app).iter().any(|label| label == name));
    assert!(window.app.workspace_shown(), "the workspace stayed hidden");
    window.until(|app| app.apps[0].running.declared.rasters.values().any(|display| (display.view.width, display.view.height) == (6, 4)));
    let after = image_cell(&window);
    assert_eq!((after.current, after.versions.len()), (Some(0), 2), "the cell changed");

    let folder = window.app.project.as_ref().unwrap().folder.clone();
    let written = folder.join(format!("{name}.ipx"));
    until_file(&mut window, &written);
    let source = &header(&written)["source"];
    let (_, _, session) = window.here();
    assert_eq!(source["cell"], id.as_str());
    assert_eq!(source["image"], first.as_str(), "another version was opened");
    assert_eq!(source["session"], session.as_str());

    // The assistant, with no cell named: the latest Image cell.
    let edit_tool = protocol::tool_key(crate::route::HOST_ID, "prompt", crate::route::EDIT_IMAGE);
    stand_in.script(vec![call_reply(&edit_tool, json!({})), text_reply("Opened.")]);
    let tabs_before = editor_tabs(&window.app).len();
    window.ask("Let me edit that image.");
    window.until(move |app| editor_tabs(app).len() > tabs_before);
    assert!(stand_in.requests().last().unwrap().to_string().contains("Opened the image as new work"));

    // With the image's file gone, nothing is sent and the assistant hears why.
    std::fs::remove_file(folder.join(&first)).unwrap();
    stand_in.script(vec![call_reply(&edit_tool, json!({ "cell": id })), text_reply("It is gone.")]);
    let tabs_before = editor_tabs(&window.app).len();
    window.ask("Edit it again.");
    keep_ticking(&mut window);
    assert_eq!(editor_tabs(&window.app).len(), tabs_before, "a tab opened for a missing image");
    assert!(stand_in.requests().last().unwrap().to_string().contains("cannot be read"), "the assistant was not told why");
    let _ = std::fs::remove_dir_all(dir);
}

/// Catches a host that finds the editor by anything but its declaration: a
/// stand-in nothing was written for declares it edits images, and gets
/// the conversation cell's image with its name.
#[test]
fn the_application_that_declares_it_edits_images_gets_them() {
    let dir = scratch("edit-anywhere");
    let log = dir.join("easel.log");
    let easel = easel_program(&dir, &log);
    let tool = protocol::tool_key(crate::route::HOST_ID, "skills", "image_generation");
    let stand_in = StandIn::start(vec![call_reply(&tool, json!({ "prompt": "A storm." })), text_reply("Done.")]);
    let mut window = project_window(
        vec![spec("aaa-easel", "Easel", &easel), spec("recipe-creation", "Recipe Creation", &built("recipe-creation"))],
        &stand_in,
        &dir,
    );
    window.app.settings.add_image_skill();
    window.until(|app| app.apps.iter().any(|started| started.running.declared.edits.iter().any(|media| media == "image/png")));
    window.ask("Make an image of a storm.");
    let id = image_cell(&window).id;
    window.app.note_cell_actions(vec![CardAction::EditImage(id.clone())]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !std::fs::read_to_string(&log).is_ok_and(|text| text.contains(&id)) && std::time::Instant::now() < deadline {
        window.step();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let line = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(line.contains("\"name\":\"A storm\""), "the stand-in did not get the image: {line}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Catches a recipe's Image cell that Edit leaves out: in a recipe, the
/// cell's image opens in the real Image Generator, its source naming the
/// recipe's file rather than a session.
#[test]
fn a_recipe_cells_image_opens_as_new_work() {
    let dir = scratch("edit-recipe");
    let stand_in = StandIn::start(Vec::new());
    let mut window = project_window(
        vec![
            spec("image-generator", "Image Generator", &built("image-generator")),
            spec("recipe-creation", "Recipe Creation", &built("recipe-creation")),
        ],
        &stand_in,
        &dir,
    );
    window.app.settings.add_image_skill();
    window.select("recipe-creation");
    super::cells_checks::make_recipe_image(&mut window, "A lighthouse in a storm.");
    let cell = super::cells_checks::recipe_cells(&window.app).into_iter().find(|cell| cell.kind == protocol::IMAGE_KIND).unwrap();
    // The recipe is written by itself, and named once it is.
    window.until(|app| {
        app.apps.iter().any(|started| started.id == "recipe-creation" && started.running.tabs().0.iter().any(|tab| tab.path.is_some()))
    });
    window.app.note_cell_actions(vec![CardAction::EditImage(cell.id.clone())]);
    window.until(|app| editor_tabs(app).iter().any(|label| label == "A lighthouse in a storm"));
    let folder = window.app.project.as_ref().unwrap().folder.clone();
    let written = folder.join("A lighthouse in a storm.ipx");
    until_file(&mut window, &written);
    let source = &header(&written)["source"];
    assert_eq!(source["cell"], cell.id.as_str());
    assert!(source["recipe"].as_str().is_some_and(|recipe| !recipe.starts_with('/')), "the source does not name the recipe: {source}");
    assert!(source.get("session").is_none(), "a recipe's cell was taken for the conversation's");
    let _ = std::fs::remove_dir_all(dir);
}

/// Catches a name that breaks a file or reads badly: separators and line
/// breaks become spaces, a long prompt stops at a word, trailing
/// punctuation goes, one long word is cut, and nothing becomes "Untitled Image".
#[test]
fn an_images_name_is_safe_and_short() {
    use crate::cells::image_name;
    assert_eq!(image_name("50/50 split\\of a\tsea, at dusk."), "50 50 split of a sea, at dusk");
    assert_eq!(image_name("A girl in a straw hat on a beach at sunset, smiling"), "A girl in a straw hat on a");
    assert_eq!(image_name(&"x".repeat(80)).chars().count(), 31);
    assert_eq!(image_name("  \n ..."), "Untitled Image");
    assert_eq!(image_name(".hidden harbour"), "hidden harbour");
}
