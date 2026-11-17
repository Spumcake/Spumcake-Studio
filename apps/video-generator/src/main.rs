//! The Video Generator: a gallery of takes per tab, each a link to a video
//! file. The host draws the working area's image and the filmstrip, and
//! owns the pointer; this process owns the gallery, its history, its file,
//! and every frame it decodes.

mod assist;
mod declare;
mod decode;
mod finishing;
mod files;
mod gallery;
mod notes;
mod preview;
mod rounds;
mod session;
mod view;
mod waveform;

use session::{Session, respond};
use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let restore = std::env::args().any(|arg| arg == protocol::RESTORE_FLAG);
    let mut session = if restore { Session::restored() } else { Session::launch() };
    let greeting = session.greeting();
    send(&greeting);
    // Lines arrive on their own thread, so the loop also wakes with no
    // message: soon while frames are being decoded, to show them as they
    // come, and otherwise to write a project's changes that are due.
    let (lines, incoming) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            // Which revision the host drew, `shown`, is read too: while
            // playing, it keeps the Player's lead and paces the frames.
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        let wait = if session.waiting() { 4 } else { 250 };
        match incoming.recv_timeout(Duration::from_millis(wait)) {
            Ok(line) => {
                // Every line already waiting is read before the frames are
                // looked for, so a held key never falls behind.
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
                let _ = out.flush();
                drop(out);
                if stop {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => send(&session.tick()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        send(&session.poll());
    }
    // The host is gone: whatever a project has not written yet is written.
    session.write_all();
}

fn send(messages: &[protocol::AppMessage]) {
    if messages.is_empty() {
        return;
    }
    let mut out = std::io::stdout().lock();
    for message in messages {
        let _ = out.write_all(protocol::encode_app_message(message).as_bytes());
    }
    let _ = out.flush();
}
