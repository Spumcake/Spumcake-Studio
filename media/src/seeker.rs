//! A video on a thread of its own, so a workspace's message loop never
//! waits on decoding: the newest request wins, and frames come back by
//! `poll` as they are ready.

use crate::video::{Frame, Info, Video};
use crate::{Result, error};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::time::Duration;

enum Request {
    Frame { index: u32, size: (u32, u32), epoch: u64 },
    Play { from: u32, size: (u32, u32), epoch: u64 },
    Stop,
}

/// Frames decoded ahead while playing.
const AHEAD: usize = 4;

pub struct Seeker {
    info: Info,
    requests: Sender<Request>,
    frames: Receiver<(u64, Result<Frame>)>,
    /// Raised by every request, so frames for an older one are dropped.
    epoch: u64,
}

impl Seeker {
    pub fn new(video: Video) -> Seeker {
        let info = video.info();
        let (requests, incoming) = mpsc::channel();
        let (out, frames) = mpsc::sync_channel(AHEAD);
        std::thread::Builder::new()
            .name(String::from("media-seeker"))
            .spawn(move || serve(video, incoming, out))
            .expect("a thread for decoding");
        Seeker { info, requests, frames, epoch: 0 }
    }

    pub fn info(&self) -> Info {
        self.info
    }

    /// Asks for the frame at `index`, at `size`, in place of anything asked
    /// before: dragging the playhead, stepping, or opening a shot.
    pub fn frame(&mut self, index: u32, size: (u32, u32)) {
        self.epoch += 1;
        let _ = self.requests.send(Request::Frame { index, size, epoch: self.epoch });
    }

    /// Asks for frames in order from `from`, decoded a few ahead, for
    /// playback, until another request.
    pub fn play(&mut self, from: u32, size: (u32, u32)) {
        self.epoch += 1;
        let _ = self.requests.send(Request::Play { from, size, epoch: self.epoch });
    }

    /// Stops decoding ahead.
    pub fn stop(&mut self) {
        self.epoch += 1;
        let _ = self.requests.send(Request::Stop);
    }

    /// The next frame for the latest request, if one is ready, without
    /// waiting. Frames for older requests are dropped.
    pub fn poll(&self) -> Option<Result<Frame>> {
        loop {
            match self.frames.try_recv() {
                Ok((epoch, frame)) if epoch == self.epoch => return Some(frame),
                Ok(_) => continue,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Err(error("Decoding the video stopped."))),
            }
        }
    }
}

/// The decoding thread: the newest request waiting is the one served.
fn serve(mut video: Video, incoming: Receiver<Request>, out: SyncSender<(u64, Result<Frame>)>) {
    let mut next = incoming.recv().ok();
    while let Some(mut request) = next.take() {
        while let Ok(newer) = incoming.try_recv() {
            request = newer;
        }
        match request {
            Request::Stop => next = incoming.recv().ok(),
            Request::Frame { index, size, epoch } => {
                if out.send((epoch, video.frame(index, size))).is_err() {
                    return;
                }
                next = incoming.recv().ok();
            }
            Request::Play { from, size, epoch } => {
                let mut frames = video.frames(from, size);
                'play: for frame in frames.by_ref() {
                    let mut item = (epoch, frame);
                    loop {
                        match incoming.try_recv() {
                            Ok(newer) => {
                                next = Some(newer);
                                break 'play;
                            }
                            Err(TryRecvError::Disconnected) => return,
                            Err(TryRecvError::Empty) => {}
                        }
                        match out.try_send(item) {
                            Ok(()) => break,
                            Err(TrySendError::Full(back)) => {
                                item = back;
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            Err(TrySendError::Disconnected(_)) => return,
                        }
                    }
                }
                if next.is_none() {
                    next = incoming.recv().ok();
                }
            }
        }
    }
}
