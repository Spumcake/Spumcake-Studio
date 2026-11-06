//! The lifecycle's items, checked with two stand-in workspaces the host was
//! not written for: one keeps references, the other shots made from them.
//! The host learns both only from what they declare.

use super::{StandIn, project_window, scratch, spec};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Keeps two references. `edit` changes the hero's content; a `render` is
/// answered with a file named by the item and its fingerprint.
const EASEL: &str = r#"#!/bin/sh
hero=h1
echo '{"type":"hello","protocol":1}'
echo '{"type":"tab","id":"e1","label":"Cast"}'
echo '{"type":"contents","tab":"e1","kind":"grid"}'
echo '{"type":"end"}'
items() { echo "{\"type\":\"items\",\"path\":\"Cast.easel\",\"items\":[{\"id\":\"hero\",\"name\":\"Hero\",\"media_types\":[\"image/png\"],\"fingerprint\":\"$hero\"},{\"id\":\"street\",\"name\":\"Street\",\"fingerprint\":\"s1\"}]}"; }
while IFS= read -r line; do
  case "$line" in
    *'"type":"ping"'*) echo '{"type":"pong"}' ;;
    *'"type":"shutdown"'*) exit 0 ;;
    *'"type":"quit"'*) echo '{"type":"quit_ready"}' ;;
    *'"type":"project"'*) items ;;
    *'"type":"command"'*) hero=h2; items ;;
    *'"type":"render"'*)
      id=$(printf '%s' "$line" | sed 's/.*"id":"\([^"]*\)".*/\1/')
      echo "{\"type\":\"rendered\",\"id\":\"$id\",\"path\":\".inpainter/renders/easel/hero-$hero.png\",\"fingerprint\":\"$hero\"}" ;;
  esac
done
"#;

/// Keeps three shots: two made from the hero, one from the street. It
/// writes every line the host sends it to `reel.log`, and `command` asks
/// for the hero as a file.
const REEL: &str = r#"#!/bin/sh
log="$(dirname "$0")/reel.log"
echo '{"type":"hello","protocol":1}'
echo '{"type":"tab","id":"r1","label":"Shots"}'
echo '{"type":"contents","tab":"r1","kind":"grid"}'
echo '{"type":"end"}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *'"type":"ping"'*) echo '{"type":"pong"}' ;;
    *'"type":"shutdown"'*) exit 0 ;;
    *'"type":"quit"'*) echo '{"type":"quit_ready"}' ;;
    *'"type":"project"'*)
      echo '{"type":"items","path":"Shots.reel","items":[{"id":"one","name":"Shot 1","fingerprint":"o1","made":{"used":[{"item":{"path":"Cast.easel","item":"hero"},"fingerprint":"h1"}]}},{"id":"two","name":"Shot 2","fingerprint":"t1","made":{"used":[{"item":{"path":"Cast.easel","item":"hero"},"fingerprint":"h1"}]}},{"id":"three","name":"Shot 3","fingerprint":"r1","made":{"used":[{"item":{"path":"Cast.easel","item":"street"},"fingerprint":"s1"}]}}]}' ;;
    *'"type":"command"'*)
      echo '{"type":"need","id":"n1","item":{"path":"Cast.easel","item":"hero"},"media_type":"image/png"}' ;;
  esac
done
"#;

fn program(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// The lines of one type the reel was sent, as JSON.
fn sent(dir: &Path, kind: &str) -> Vec<Value> {
    std::fs::read_to_string(dir.join("reel.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| message["type"] == kind)
        .collect()
}

fn command(id: &str, tab: &str) -> protocol::HostMessage {
    protocol::HostMessage::Command { id: id.into(), tab: Some(tab.into()), arg: None, node: None, viewport: [800.0, 600.0] }
}

/// Catches a host that marks everything, marks nothing, or knows its
/// workspaces: changing the reference two of three shots used marks
/// exactly those two, in the shots' owner, which the host learned only
/// from what each declared.
#[test]
fn an_edit_marks_only_the_items_made_from_it() {
    let dir = scratch("items-stale");
    let easel = program(&dir, "easel", EASEL);
    let reel = program(&dir, "reel", REEL);
    let stand_in = StandIn::start(Vec::new());
    let mut window = project_window(vec![spec("easel", "Easel", &easel), spec("reel", "Reel", &reel)], &stand_in, &dir);
    window.until(|app| app.items.owner("Cast.easel").is_some() && app.items.owner("Shots.reel").is_some());
    assert!(sent(&dir, "stale").is_empty(), "nothing has changed yet");
    window.app.send(0, command("edit", "e1"));
    window.until(|_| !sent(&dir, "stale").is_empty());
    let stale = &sent(&dir, "stale")[0];
    assert_eq!(stale["path"], "Shots.reel");
    let marked: Vec<&str> = stale["items"].as_array().unwrap().iter().map(|item| item["id"].as_str().unwrap()).collect();
    assert_eq!(marked, ["one", "two"]);
    assert_eq!(stale["items"][0]["changed"][0]["item"], "hero");
    let _ = std::fs::remove_dir_all(dir);
}

/// Catches a `need` that goes nowhere or to the wrong workspace: the shots'
/// owner asks for the hero, the host asks the references' owner to render
/// it, and the file and fingerprint come back to the one that asked.
#[test]
fn a_needed_item_is_rendered_by_its_owner_and_handed_back() {
    let dir = scratch("items-need");
    let easel = program(&dir, "easel", EASEL);
    let reel = program(&dir, "reel", REEL);
    let stand_in = StandIn::start(Vec::new());
    let mut window = project_window(vec![spec("easel", "Easel", &easel), spec("reel", "Reel", &reel)], &stand_in, &dir);
    window.until(|app| app.items.owner("Cast.easel").is_some());
    window.app.send(1, command("need", "r1"));
    window.until(|_| !sent(&dir, "provided").is_empty());
    let provided = &sent(&dir, "provided")[0];
    assert_eq!(provided["id"], "n1");
    assert_eq!(provided["path"], ".inpainter/renders/easel/hero-h1.png");
    assert_eq!(provided["fingerprint"], "h1");
    let _ = std::fs::remove_dir_all(dir);
}
