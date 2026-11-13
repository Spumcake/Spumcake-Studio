//! Sound decoded whole into memory: a sound file of any kind FFmpeg reads,
//! or a video's own sound, as stereo samples at the rate asked, so tracks
//! from files of different rates mix. A video's sound is placed where its
//! timestamps put it against the video's first frame, with silence where
//! it starts late or has gaps, so it stays with its frames.

use crate::{Result, error, named};
use ffmpeg_next as ff;
use std::path::Path;

/// Stereo samples, left and right in turn, from -1 to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Sound {
    pub const CHANNELS: usize = 2;

    pub fn open(path: &Path, rate: u32) -> Result<Sound> {
        crate::init()?;
        let name = named(path);
        let unreadable = || error(format!("{name}'s sound is in a format Studio cannot read."));
        let mut input = ff::format::input(path).map_err(|_| error(format!("{name} could not be opened.")))?;
        let stream = input.streams().best(ff::media::Type::Audio).ok_or_else(|| error(format!("{name} has no sound.")))?;
        let index = stream.index();
        let tick = f64::from(stream.time_base());
        // Sample 0 is the video's first frame when there are frames,
        // and the sound's own start when there is not.
        let seconds = |stream: &ff::format::stream::Stream| {
            let start = if stream.start_time() == i64::MIN { 0 } else { stream.start_time() };
            start as f64 * f64::from(stream.time_base())
        };
        let origin = input.streams().best(ff::media::Type::Video).map_or_else(|| seconds(&stream), |video| seconds(&video));
        let context = ff::codec::context::Context::from_parameters(stream.parameters()).map_err(|_| unreadable())?;
        let mut decoder = context.decoder().audio().map_err(|_| unreadable())?;
        let mut resampler: Option<ff::software::resampling::Context> = None;
        let mut samples = Vec::new();
        // Sample frames to drop from the start, for sound that began before
        // the frames.
        let mut early = 0usize;
        let mut take = |decoder: &mut ff::decoder::Audio, samples: &mut Vec<f32>| -> Result<()> {
            let mut frame = ff::frame::Audio::empty();
            while decoder.receive_frame(&mut frame).is_ok() {
                // Where this frame belongs, against what is decoded so far:
                // late is filled with silence, and early, only at the very
                // start, is dropped. A gap under a hundredth of a second is
                // the resampler's own and is left.
                if let Some(stamp) = frame.timestamp().or(frame.pts()) {
                    let due = ((stamp as f64 * tick - origin) * f64::from(rate)).round() as i64;
                    let have = (samples.len() / Sound::CHANNELS) as i64 - early as i64;
                    if due > have + i64::from(rate / 100) {
                        samples.resize(samples.len() + (due - have) as usize * Sound::CHANNELS, 0.0);
                    } else if samples.is_empty() && early == 0 && due < 0 {
                        early = (-due) as usize;
                    }
                }
                // A frame without a layout is taken as mono or stereo by its
                // channel count.
                if frame.channel_layout().is_empty() {
                    frame.set_channel_layout(ff::ChannelLayout::default(i32::from(frame.channels())));
                }
                if resampler.is_none() {
                    resampler = Some(
                        ff::software::resampling::Context::get(
                            frame.format(),
                            frame.channel_layout(),
                            frame.rate(),
                            ff::format::Sample::F32(ff::format::sample::Type::Packed),
                            ff::ChannelLayout::STEREO,
                            rate,
                        )
                        .map_err(|_| unreadable())?,
                    );
                }
                // Room for every sample at the new rate: ffmpeg-next would
                // make room for only as many as came in, and a higher rate
                // would lose the rest.
                let room = frame.samples() * rate as usize / frame.rate().max(1) as usize + 256;
                let mut out = ff::frame::Audio::new(ff::format::Sample::F32(ff::format::sample::Type::Packed), room, ff::ChannelLayout::STEREO);
                if let Some(resampler) = &mut resampler {
                    resampler.run(&frame, &mut out).map_err(|_| unreadable())?;
                }
                push(&out, samples);
            }
            Ok(())
        };
        for (stream, packet) in input.packets() {
            if stream.index() == index && decoder.send_packet(&packet).is_ok() {
                take(&mut decoder, &mut samples)?;
            }
        }
        let _ = decoder.send_eof();
        take(&mut decoder, &mut samples)?;
        // What the resampler still holds.
        if let Some(resampler) = &mut resampler {
            loop {
                let mut out = ff::frame::Audio::new(ff::format::Sample::F32(ff::format::sample::Type::Packed), 4096, ff::ChannelLayout::STEREO);
                if resampler.flush(&mut out).is_err() || out.samples() == 0 {
                    break;
                }
                push(&out, &mut samples);
            }
        }
        samples.drain(..(early * Sound::CHANNELS).min(samples.len()));
        if samples.is_empty() {
            return Err(error(format!("{name}'s sound is empty.")));
        }
        Ok(Sound { rate, samples })
    }

    /// The length in sample frames, each a left and right sample.
    pub fn len(&self) -> usize {
        self.samples.len() / Self::CHANNELS
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

fn push(frame: &ff::frame::Audio, samples: &mut Vec<f32>) {
    let count = frame.samples() * Sound::CHANNELS;
    if count == 0 || frame.planes() == 0 {
        return;
    }
    let bytes = frame.data(0);
    let count = count.min(bytes.len() / 4);
    samples.extend(bytes[..count * 4].chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])));
}
