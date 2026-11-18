//! Recipe Creation names new recipes as the conventions guide sets, driven
//! through the protocol alone.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Catches a name checked against the open tabs alone, or the disk alone:
/// in a project whose folder holds "Untitled Recipe.smx", two new recipes
/// are "Untitled Recipe 2" and "Untitled Recipe 3".
#[test]
fn new_recipes_take_names_free_in_the_tabs_and_the_folder() {
    let folder = std::env::temp_dir().join(format!("recipe-creation-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("a scratch folder");
    std::fs::write(folder.join("Untitled Recipe.smx"), r#"{"version":2,"cells":[]}"#).expect("a recipe on disk");
    let mut child = Command::new(env!("CARGO_BIN_EXE_recipe-creation"))
        .arg(protocol::RESTORE_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("recipe-creation starts");
    let mut input = child.stdin.take().expect("stdin");
    let output = child.stdout.take().expect("stdout");
    let (lines, incoming) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let new_recipe = protocol::HostMessage::Command {
        id: "new_recipe".into(),
        tab: None,
        arg: None,
        node: None,
        viewport: [800.0, 600.0],
    };
    for message in [protocol::HostMessage::Project { folder: folder.display().to_string() }, new_recipe.clone(), new_recipe] {
        writeln!(input, "{}", serde_json::to_string(&message).expect("json")).expect("sent");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut labels = Vec::new();
    while labels.len() < 2 && Instant::now() < deadline {
        let Ok(line) = incoming.recv_timeout(Duration::from_millis(250)) else {
            continue;
        };
        if let Ok(protocol::AppMessage::Tabs(tabs)) = protocol::parse_app_message(&line) {
            labels = tabs.tabs.into_iter().map(|tab| tab.label).collect();
        }
    }
    let _ = writeln!(input, "{}", serde_json::to_string(&protocol::HostMessage::Shutdown).expect("json"));
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(labels, ["Untitled Recipe 2", "Untitled Recipe 3"]);
}
