//! The Image Generator: a gallery of layered images per tab. The host draws
//! the image shown and the filmstrip, and owns the camera and the
//! pointer; this process owns every pixel, layer, selection, step of
//! history, and file.

mod assist;
mod declare;
mod doc;
mod files;
mod gallery;
mod paint;
mod layered;
mod session;
mod text;

use session::{Session, respond};
use std::io::{BufRead, Write};

fn main() {
    let restore = std::env::args().any(|arg| arg == protocol::RESTORE_FLAG);
    let mut session = if restore { Session::restored() } else { Session::launch() };
    let greeting = session.greeting();
    send(&greeting);
    // Lines arrive on their own thread, so the loop also wakes with no
    // message and writes a project's changes that are due. When the
    // image beneath the active layer is to be made, it wakes sooner, as
    // soon as the lines pause, so it is ready before a slider is reached.
    let (lines, incoming) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            // Which revision the host drew is for workspaces that play
            // video. Read as a message, it would come after every image
            // sent and keep the loop from ever being idle, so a finished
            // stroke's image would never be written whole.
            if line.contains("\"type\":\"shown\"") {
                continue;
            }
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        let wait = if session.ahead_due() { 30 } else { 250 };
        let line = match incoming.recv_timeout(std::time::Duration::from_millis(wait)) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !session.keep_ahead() {
                    send(&session.tick());
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        // Every line already waiting is read before the image of a stroke
        // being drawn, or of a held slider, is sent, so painting never falls
        // behind the pen and a slider never falls behind the pointer.
        let mut waiting = vec![line];
        while let Ok(line) = incoming.try_recv() {
            waiting.push(line);
        }
        let mut stop = false;
        let mut out = std::io::stdout().lock();
        for line in waiting {
            if line.contains("\"type\":\"shutdown\"") {
                stop = true;
                break;
            }
            for reply in respond(&mut session, &line) {
                let _ = out.write_all(reply.as_bytes());
            }
        }
        for message in session.take_live() {
            let _ = out.write_all(protocol::encode_app_message(&message).as_bytes());
        }
        let _ = out.flush();
        drop(out);
        if stop {
            break;
        }
    }
    // The host is gone: whatever a project has not written yet is written.
    session.write_all();
}

fn send(messages: &[protocol::AppMessage]) {
    let mut out = std::io::stdout().lock();
    for message in messages {
        let _ = out.write_all(protocol::encode_app_message(message).as_bytes());
    }
    let _ = out.flush();
}
