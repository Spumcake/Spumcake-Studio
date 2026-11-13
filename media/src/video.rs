//! Reading a video's frames: any frame by its index, or frames in order,
//! as RGBA at the size they are shown, converted by the stream's own
//! colour tag on every core.

use crate::colour::{self, Planes, Tag};
use crate::{Result, error, named};
use ffmpeg_next as ff;
use std::path::Path;

/// What a video is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    /// Frames per second, as a fraction: 24/1, or 30000/1001.
    pub rate: (u32, u32),
    pub frames: u32,
    pub sound: bool,
}

impl Info {
    pub fn fps(&self) -> f64 {
        f64::from(self.rate.0) / f64::from(self.rate.1.max(1))
    }
}

/// One frame: its index in the video, its size, and its pixels, RGBA with
/// straight alpha, rows top to bottom.
#[derive(Debug, Clone)]
pub struct Frame {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A frame this far ahead is reached by decoding on rather than seeking.
const DECODE_ON: u32 = 24;

/// An open video. One thread uses it at a time; it moves between threads,
/// so it can live on a decoder thread of its own.
pub struct Video {
    input: ff::format::context::Input,
    stream: usize,
    decoder: ff::decoder::Video,
    /// Seconds per tick of the stream's timestamps, and its first one.
    tick: f64,
    start: i64,
    info: Info,
    ended: bool,
    /// The index of the last frame decoded, when the decoder is still
    /// where it left it.
    at: Option<u32>,
    /// A scaler to 4:2:0 at the size last asked, with its sizes and format.
    scaler: Option<((ff::format::Pixel, u32, u32, u32, u32), ff::software::scaling::Context)>,
    /// The frame last given, so asking for it again, as a paused stage
    /// redrawn at full size does, decodes nothing.
    last: Option<Frame>,
    name: String,
}

// FFmpeg's contexts belong to whichever single thread holds the video.
unsafe impl Send for Video {}

impl Video {
    pub fn open(path: &Path) -> Result<Video> {
        crate::init()?;
        let name = named(path);
        let input = ff::format::input(path).map_err(|_| error(format!("{name} could not be opened as a video.")))?;
        let stream = input.streams().best(ff::media::Type::Video).ok_or_else(|| error(format!("{name} has no video.")))?;
        let index = stream.index();
        let tick = f64::from(stream.time_base());
        let start = if stream.start_time() == i64::MIN { 0 } else { stream.start_time() };
        // The stream's own rate; the average is worked out from a duration
        // that can leave out the last frame, and reads 24.5 for 24.
        let rate = [stream.rate(), stream.avg_frame_rate()]
            .into_iter()
            .find(|rate| rate.numerator() > 0 && rate.denominator() > 0 && f64::from(*rate) <= 240.0)
            .map_or((24, 1), |rate| (rate.numerator() as u32, rate.denominator() as u32));
        let fps = f64::from(rate.0) / f64::from(rate.1);
        let frames = if stream.frames() > 0 {
            stream.frames() as u32
        } else if stream.duration() > 0 {
            (stream.duration() as f64 * tick * fps).round() as u32
        } else {
            (input.duration().max(0) as f64 / f64::from(ff::ffi::AV_TIME_BASE) * fps).round() as u32
        };
        let mut context = ff::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|_| error(format!("{name}'s video is in a format Studio cannot read.")))?;
        context.set_threading(ff::codec::threading::Config { kind: ff::codec::threading::Type::Frame, count: 0 });
        let decoder = context.decoder().video().map_err(|_| error(format!("{name}'s video is in a format Studio cannot read.")))?;
        let sound = input.streams().best(ff::media::Type::Audio).is_some();
        let info = Info { width: decoder.width(), height: decoder.height(), rate, frames, sound };
        Ok(Video { input, stream: index, decoder, tick, start, info, ended: false, at: None, scaler: None, last: None, name })
    }

    pub fn info(&self) -> Info {
        self.info
    }

    /// The frame at `index`, or the nearest the file holds, at `size`.
    /// A frame a little ahead of the last one decodes on; any other seeks
    /// to the keyframe before it and decodes from there.
    pub fn frame(&mut self, index: u32, size: (u32, u32)) -> Result<Frame> {
        if let Some(last) = &self.last
            && last.index == index.min(self.info.frames.saturating_sub(1))
            && (last.width, last.height) == (size.0.max(2) & !1, size.1.max(2) & !1)
        {
            return Ok(last.clone());
        }
        let frame = self.find(index, size)?;
        self.last = Some(frame.clone());
        Ok(frame)
    }

    fn find(&mut self, index: u32, size: (u32, u32)) -> Result<Frame> {
        if let Some(at) = self.at
            && index > at
            && index - at <= DECODE_ON
        {
            let mut last = None;
            while let Some((found, frame)) = self.decode()? {
                if found >= index {
                    return self.to_frame(found, &frame, size);
                }
                last = Some((found, frame));
            }
            if let Some((found, frame)) = last {
                return self.to_frame(found, &frame, size);
            }
        }
        self.seek(index)?;
        let mut last = None;
        while let Some((found, frame)) = self.decode()? {
            if found >= index {
                return self.to_frame(found, &frame, size);
            }
            last = Some((found, frame));
        }
        match last {
            Some((found, frame)) => self.to_frame(found, &frame, size),
            None => Err(error(format!("{} has no frame {}.", self.name, index + 1))),
        }
    }

    /// The frames in order from `from`, at `size`, until the video ends.
    pub fn frames(&mut self, from: u32, size: (u32, u32)) -> Frames<'_> {
        Frames { video: self, from: Some(from), size }
    }

    fn seek(&mut self, index: u32) -> Result<()> {
        let seconds = f64::from(index) / self.info.fps() + self.start as f64 * self.tick;
        let at = (seconds * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        self.input.seek(at, ..at + 1).map_err(|_| error(format!("Studio could not move to frame {} of {}.", index + 1, self.name)))?;
        self.decoder.flush();
        self.ended = false;
        self.at = None;
        Ok(())
    }

    /// The next frame the decoder gives, with its index; None at the end.
    fn decode(&mut self) -> Result<Option<(u32, ff::frame::Video)>> {
        loop {
            let mut frame = ff::frame::Video::empty();
            if self.decoder.receive_frame(&mut frame).is_ok() {
                let stamp = frame.timestamp().or(frame.pts()).unwrap_or(self.start);
                let index = ((stamp - self.start) as f64 * self.tick * self.info.fps()).round().max(0.0) as u32;
                self.at = Some(index);
                return Ok(Some((index, frame)));
            }
            if self.ended {
                return Ok(None);
            }
            match self.input.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.stream {
                        // A packet the decoder refuses is damage in the
                        // file; the frames around it still come.
                        let _ = self.decoder.send_packet(&packet);
                    }
                }
                None => {
                    let _ = self.decoder.send_eof();
                    self.ended = true;
                }
            }
        }
    }

    /// A decoded frame as RGBA at `size`.
    fn to_frame(&mut self, index: u32, frame: &ff::frame::Video, size: (u32, u32)) -> Result<Frame> {
        let (width, height) = (size.0.max(2) & !1, size.1.max(2) & !1);
        let format = frame.format();
        let full_format = matches!(format, ff::format::Pixel::YUVJ420P);
        let tag = Tag::of(frame.color_space(), frame.color_range(), frame.height(), full_format);
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let direct = matches!(format, ff::format::Pixel::YUV420P | ff::format::Pixel::YUVJ420P) && (frame.width(), frame.height()) == (width, height);
        if direct {
            let planes = Planes { data: [frame.data(0), frame.data(1), frame.data(2)], stride: [frame.stride(0), frame.stride(1), frame.stride(2)] };
            colour::to_rgba(&planes, width as usize, height as usize, tag, &mut rgba);
        } else {
            // Scaled, or in another layout, in 4:2:0 first: the scaler keeps
            // the colours as they are, and the conversion reads the tag.
            let target = if full_format { ff::format::Pixel::YUVJ420P } else { ff::format::Pixel::YUV420P };
            let key = (format, frame.width(), frame.height(), width, height);
            if self.scaler.as_ref().is_none_or(|(k, _)| *k != key) {
                let scaler = ff::software::scaling::Context::get(format, frame.width(), frame.height(), target, width, height, ff::software::scaling::Flags::BILINEAR)
                    .map_err(|_| error(format!("{}'s frames could not be resized.", self.name)))?;
                self.scaler = Some((key, scaler));
            }
            let mut small = ff::frame::Video::empty();
            if let Some((_, scaler)) = &mut self.scaler {
                scaler.run(frame, &mut small).map_err(|_| error(format!("{}'s frames could not be resized.", self.name)))?;
            }
            let planes = Planes { data: [small.data(0), small.data(1), small.data(2)], stride: [small.stride(0), small.stride(1), small.stride(2)] };
            colour::to_rgba(&planes, width as usize, height as usize, tag, &mut rgba);
        }
        Ok(Frame { index, width, height, rgba })
    }
}

/// Frames in order, from `Video::frames`.
pub struct Frames<'a> {
    video: &'a mut Video,
    from: Option<u32>,
    size: (u32, u32),
}

impl Iterator for Frames<'_> {
    type Item = Result<Frame>;

    fn next(&mut self) -> Option<Result<Frame>> {
        if let Some(from) = self.from.take() {
            return Some(self.video.find(from, self.size));
        }
        match self.video.decode() {
            Ok(Some((index, frame))) => Some(self.video.to_frame(index, &frame, self.size)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
