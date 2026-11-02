//! Starts Storyboard Generator with no host and no window, and makes one storyboard
//! entirely through the protocol.
//!
//! The process is sent `new_storyboard`, a stroke, an erase across part of that
//! stroke, the canvas's properties page and a change to its padding, undo,
//! redo, and save, answering Storyboard Generator's own file request. Every reply
//! must equal what an in-process `Server` (the same code behind the process)
//! sends for the same messages, and the two saved files must be identical.
//! The storyboard must not exist before `new_storyboard`. Temporary files are removed.
//!
//! Then it drives every tool of Storyboard Generator's skills through `invoke`, the
//! way the assistant does, comparing every reply, the undo of a whole
//! request, and a saved file with the in-process session (`assist.rs`);
//! reopens files and keeps an untitled storyboard as a workspace does, with
//! `--restore`, `open`, `keep`, and `kept` (`assist.rs`); and checks that
//! every part of Storyboard Generator's hand surface that changes a storyboard's
//! contents has a tool (`coverage.rs`).

mod assist;
mod coverage;

use draw::document::{ColorId, Dynamics, NodeId, Sample};
use draw::gesture::{Gesture, GestureKind};
use protocol::{AppMessage, FileKind, HostMessage};
use serde_json::Value;
use storyboard_generator::serve::Server;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

pub(crate) const VIEWPORT: [f64; 2] = [1280.0, 720.0];

fn main() {
    let dir = std::env::temp_dir().join(format!("inpainter-no-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let result = check(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => println!("no-host check: passed (protocol, every skill tool, one undo per request, workspace, coverage)"),
        Err(message) => {
            eprintln!("no-host check: {message}");
            std::process::exit(1);
        }
    }
}

fn check(dir: &Path) -> Result<(), String> {
    hand_check(dir)?;
    coverage::check()?;
    let saved = assist::check(dir)?;
    assist::workspace(dir, &saved)
}

fn hand_check(dir: &Path) -> Result<(), String> {
    let mut link = Link::start(&[])?;
    let mut oracle = Server::new(None);

    let expected = as_read(oracle.greeting());
    let greeting = link.read_count(expected.len())?;
    if greeting != expected {
        return Err("the declaration does not match the session".into());
    }
    let first = document_of(&greeting).ok_or("the greeting has no document")?;
    let first_tab = first.0.clone();

    let replies = exchange(&mut link, &mut oracle, command("new_storyboard", None, None))?;
    let (tab, storyboard, camera) = document_of(&replies).ok_or("new_storyboard sent no document")?;
    if tab == first_tab || !camera {
        return Err("new_storyboard did not open a new storyboard with its camera".into());
    }
    if storyboard_existed_before(&greeting, &tab) {
        return Err("the new storyboard existed before new_storyboard".into());
    }
    let (canvas, source) = canvas_and_source(&storyboard).ok_or("the new storyboard has no canvas")?;
    let padding_before = canvas_padding(&storyboard).ok_or("the canvas has no padding")?;

    let replies = exchange(
        &mut link,
        &mut oracle,
        gesture(&tab, brush_gesture(canvas, source)),
    )?;
    let storyboard = document_of(&replies)
        .ok_or("the stroke sent no document")?
        .1;
    if stroke_counts(&storyboard) != (1, 0) {
        return Err("the stroke did not land on the canvas".into());
    }

    let replies = exchange(&mut link, &mut oracle, gesture(&tab, erase_gesture(canvas)))?;
    let storyboard = document_of(&replies).ok_or("the erase sent no document")?.1;
    if stroke_counts(&storyboard).1 < 1 {
        return Err("the erase did not cross the stroke".into());
    }

    let open_page = command("object_properties", Some(&tab), Some(canvas.to_string()));
    exchange(&mut link, &mut oracle, open_page)?;
    let padding = HostMessage::Property {
        prop: serde_json::json!("padding"),
        value: serde_json::json!(32),
        preview: false,
    };
    let replies = exchange(&mut link, &mut oracle, padding)?;
    let storyboard = document_of(&replies)
        .ok_or("the property sent no document")?
        .1;
    if canvas_padding(&storyboard) != Some(32.0) {
        return Err("the canvas padding did not become 32".into());
    }

    let replies = exchange(&mut link, &mut oracle, command("undo", Some(&tab), None))?;
    let storyboard = document_of(&replies).ok_or("undo sent no document")?.1;
    if canvas_padding(&storyboard) != Some(padding_before) {
        return Err("undo did not restore the padding".into());
    }
    let replies = exchange(&mut link, &mut oracle, command("redo", Some(&tab), None))?;
    let storyboard = document_of(&replies).ok_or("redo sent no document")?.1;
    if canvas_padding(&storyboard) != Some(32.0) {
        return Err("redo did not put the padding back".into());
    }

    let replies = exchange(&mut link, &mut oracle, command("save", Some(&tab), None))?;
    let Some(AppMessage::File { id, kind, name, .. }) = replies.first().cloned() else {
        return Err("save did not ask for a file".into());
    };
    if kind != FileKind::Save || name.is_none() {
        return Err("save did not ask where to save".into());
    }
    let file_name = name.unwrap_or_default();
    let process_file = dir.join("process").join(&file_name);
    let oracle_file = dir.join("oracle").join(&file_name);
    for path in [&process_file, &oracle_file] {
        std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
    }
    link.send(&HostMessage::FileResult {
        id: id.clone(),
        path: Some(process_file.display().to_string()),
    })?;
    let expected = oracle
        .handle(HostMessage::FileResult {
            id,
            path: Some(oracle_file.display().to_string()),
        })
        .ok_or("the session stopped")?;
    // The two files sit in different folders, which the recent-file menu names.
    let replies = link.read_count(expected.len())?;
    let in_oracle_folder = |messages: &[AppMessage]| -> Vec<String> {
        messages
            .iter()
            .map(|message| {
                protocol::encode_app_message(message).replace(
                    &dir.join("process").display().to_string(),
                    &dir.join("oracle").display().to_string(),
                )
            })
            .collect()
    };
    if in_oracle_folder(&replies) != in_oracle_folder(&expected) {
        return Err("the reply to the saved file does not match the session".into());
    }

    let written = std::fs::read(&process_file).map_err(|_| "storyboard-generator wrote no file")?;
    let reference = std::fs::read(&oracle_file).map_err(|_| "the session wrote no file")?;
    if written != reference {
        return Err("the saved file does not match the session's".into());
    }
    let saved: Value = serde_json::from_slice(&written).map_err(|_| "the file is not a storyboard")?;
    if stroke_counts(&saved).0 != 1 || canvas_padding(&saved) != Some(32.0) {
        return Err("the saved file does not hold the storyboard".into());
    }
    link.send(&HostMessage::Shutdown)?;
    link.finish()
}

/// Sends one message to the process and to the in-process session, and
/// returns the process's replies once they match.
pub(crate) fn exchange(
    link: &mut Link,
    oracle: &mut Server,
    message: HostMessage,
) -> Result<Vec<AppMessage>, String> {
    let expected = as_read(
        oracle
            .handle(message.clone())
            .ok_or("the session stopped")?,
    );
    link.send(&message)?;
    let replies = link.read_count(expected.len())?;
    if replies != expected {
        let differs = replies
            .iter()
            .zip(&expected)
            .find(|(reply, wanted)| reply != wanted)
            .map(|(reply, wanted)| {
                let cut = |message: &AppMessage| -> String {
                    protocol::encode_app_message(message).chars().take(400).collect()
                };
                format!("\n  process: {}\n  session: {}", cut(reply), cut(wanted))
            })
            .unwrap_or_else(|| format!(" ({} replies, not {})", replies.len(), expected.len()));
        return Err(format!(
            "after {} the process replied differently from the session{differs}",
            protocol::encode_host_message(&message).trim_end()
        ));
    }
    Ok(replies)
}

/// The session's messages as they read back after crossing the pipe.
/// `serde_json` may read a number one unit in its last place away from the
/// one it wrote, so both sides are compared after one write and read.
pub(crate) fn as_read(messages: Vec<AppMessage>) -> Vec<AppMessage> {
    messages
        .into_iter()
        .map(|message| {
            let line = protocol::encode_app_message(&message);
            protocol::parse_app_message(line.trim_end()).unwrap_or(message)
        })
        .collect()
}

pub(crate) fn document_of(messages: &[AppMessage]) -> Option<(String, Value, bool)> {
    messages.iter().rev().find_map(|message| match message {
        AppMessage::Document {
            tab,
            drawing,
            camera,
            ..
        } => Some((tab.clone(), drawing.clone(), camera.is_some())),
        _ => None,
    })
}

fn storyboard_existed_before(greeting: &[AppMessage], tab: &str) -> bool {
    greeting.iter().any(|message| match message {
        AppMessage::Tab { id, .. } => id == tab,
        AppMessage::Document { tab: shown, .. } => shown == tab,
        _ => false,
    })
}

pub(crate) fn command(id: &str, tab: Option<&str>, arg: Option<String>) -> HostMessage {
    HostMessage::Command {
        id: id.to_string(),
        tab: tab.map(str::to_string),
        arg: arg.map(Value::String),
        node: None,
        viewport: VIEWPORT,
    }
}

pub(crate) fn gesture(tab: &str, gesture: Gesture) -> HostMessage {
    HostMessage::Gesture {
        tab: tab.to_string(),
        gesture: gesture.to_value(),
    }
}

pub(crate) struct Link {
    child: Child,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    stdin: Option<ChildStdin>,
}

impl Link {
    pub(crate) fn start(args: &[&str]) -> Result<Self, String> {
        // Built beside this program, in the same `target` folder.
        let executable = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("storyboard-generator");
        let mut child = Command::new(&executable)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", executable.display()))?;
        let stdout = std::io::BufReader::new(child.stdout.take().ok_or("no stdout")?);
        let stdin = child.stdin.take().ok_or("no stdin")?;
        Ok(Self {
            child,
            stdout,
            stdin: Some(stdin),
        })
    }

    pub(crate) fn send(&mut self, message: &HostMessage) -> Result<(), String> {
        let encoded = protocol::encode_host_message(message);
        let stdin = self.stdin.as_mut().ok_or("stdin closed")?;
        stdin
            .write_all(encoded.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn read_count(&mut self, count: usize) -> Result<Vec<AppMessage>, String> {
        (0..count).map(|_| self.read_message()).collect()
    }

    fn read_message(&mut self) -> Result<AppMessage, String> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line).unwrap_or(0) == 0 {
            return Err("storyboard-generator closed".into());
        }
        protocol::parse_app_message(line.trim_end())
            .map_err(|error| format!("a reply did not parse: {}", error.line))
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("storyboard-generator exited with {status}"))
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub(crate) fn brush_gesture(canvas: u64, source: u64) -> Gesture {
    Gesture {
        kind: GestureKind::Brush {
            canvas: Some(NodeId::new(canvas)),
            source: ColorId::new(source),
            layer: None,
            size: None,
            dynamics: Dynamics::round(),
        },
        samples: line(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (40.0, 0.0),
            (60.0, 0.0),
            (80.0, 0.0),
        ]),
    }
}

fn erase_gesture(canvas: u64) -> Gesture {
    Gesture {
        kind: GestureKind::Erase {
            canvas: NodeId::new(canvas),
            size: 24,
            dynamics: Dynamics::round(),
        },
        samples: line(&[
            (40.0, -40.0),
            (40.0, -20.0),
            (40.0, 0.0),
            (40.0, 20.0),
            (40.0, 40.0),
        ]),
    }
}

fn line(points: &[(f64, f64)]) -> Vec<Sample> {
    points
        .iter()
        .map(|(x, y)| Sample {
            x: *x,
            y: *y,
            pressure: Some(0.6),
        })
        .collect()
}

fn canvas_node(storyboard: &Value) -> Option<&Value> {
    storyboard
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|node| node.get("kind").and_then(Value::as_str) == Some("canvas"))
}

pub(crate) fn canvas_and_source(storyboard: &Value) -> Option<(u64, u64)> {
    let canvas = canvas_node(storyboard)?;
    let id = canvas.get("id")?.as_u64()?;
    let source = canvas
        .get("colors")?
        .as_array()?
        .iter()
        .find_map(|layer| layer.get("source")?.as_u64())?;
    Some((id, source))
}

fn canvas_padding(storyboard: &Value) -> Option<f64> {
    canvas_node(storyboard)?.get("padding")?.as_f64()
}

pub(crate) fn stroke_counts(storyboard: &Value) -> (usize, usize) {
    let mut strokes = 0;
    let mut erases = 0;
    for node in storyboard
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for layer in node
            .get("colors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for stroke in layer
                .get("strokes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                strokes += 1;
                erases += stroke
                    .get("erases")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
            }
        }
    }
    (strokes, erases)
}
