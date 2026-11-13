//! Writing an MP4: H.264 from RGBA frames and AAC from stereo samples. The
//! file is written beside its place and renamed when finished, so a
//! cancelled or failed export leaves nothing behind.

use crate::colour;
use crate::{Result, error, named};
use ffmpeg_next as ff;
use std::path::{Path, PathBuf};

/// What an export is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Even numbers, as 4:2:0 video needs.
    pub width: u32,
    pub height: u32,
    /// Frames per second, as a fraction.
    pub rate: (u32, u32),
    /// A keyframe at least this often, in frames, so the file scrubs well.
    pub keyframes: u32,
    /// The sound's rate and channels, 1 or 2, or None for a film without
    /// sound.
    pub sound: Option<(u32, u16)>,
}

/// An MP4 being written. Dropping it before `finish` cancels it and
/// removes what was written.
pub struct Encoder {
    /// Closed before the file is removed, as Windows needs.
    output: std::mem::ManuallyDrop<ff::format::context::Output>,
    video: ff::encoder::Video,
    /// The sound's encoder, the samples waiting for a whole frame of it,
    /// the next frame's time, and its channels.
    audio: Option<(ff::encoder::Audio, Vec<f32>, i64, usize)>,
    settings: Settings,
    /// Each stream's index and time base in the file.
    video_base: ff::Rational,
    audio_base: ff::Rational,
    written: u32,
    scratch: PathBuf,
    path: PathBuf,
    done: bool,
}

// FFmpeg's contexts belong to whichever single thread holds the encoder.
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn create(path: &Path, settings: Settings) -> Result<Encoder> {
        crate::init()?;
        let name = named(path);
        let failed = |why: &str| error(format!("{name} could not be written: {why}."));
        if settings.width < 2 || settings.height < 2 || settings.width % 2 == 1 || settings.height % 2 == 1 {
            return Err(failed("its width and height must be even"));
        }
        let scratch = path.with_extension("mp4.partial");
        let mut output = ff::format::output_as(&scratch, "mp4").map_err(|why| failed(&format!("the file could not be made, {why}")))?;
        let global = output.format().flags().contains(ff::format::Flags::GLOBAL_HEADER);
        let (num, den) = (settings.rate.0 as i32, settings.rate.1.max(1) as i32);

        let codec = ff::encoder::find(ff::codec::Id::H264).ok_or_else(|| failed("this computer has no H.264 encoder"))?;
        let mut video = ff::codec::context::Context::new_with_codec(codec).encoder().video().map_err(|_| failed("the H.264 encoder would not start"))?;
        video.set_width(settings.width);
        video.set_height(settings.height);
        video.set_format(ff::format::Pixel::YUV420P);
        video.set_time_base((den, num));
        video.set_frame_rate(Some((num, den)));
        video.set_gop(settings.keyframes.max(1));
        video.set_colorspace(ff::color::Space::BT709);
        video.set_color_range(ff::color::Range::MPEG);
        unsafe {
            let raw = video.as_mut_ptr();
            (*raw).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*raw).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        }
        if global {
            video.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let mut options = ff::Dictionary::new();
        options.set("preset", "medium");
        options.set("crf", "18");
        let video = video.open_with(options).map_err(|_| failed("the H.264 encoder would not start"))?;
        {
            let mut stream = output.add_stream(codec).map_err(|_| failed("its video could not be added"))?;
            stream.set_parameters(&video);
            stream.set_time_base((den, num));
        }

        let audio = match settings.sound {
            None => None,
            Some((rate, channels)) => {
                if !(1..=2).contains(&channels) {
                    return Err(failed("its sound must have one or two channels"));
                }
                let layout = if channels == 1 { ff::ChannelLayout::MONO } else { ff::ChannelLayout::STEREO };
                let codec = ff::encoder::find(ff::codec::Id::AAC).ok_or_else(|| failed("this computer has no AAC encoder"))?;
                let mut audio = ff::codec::context::Context::new_with_codec(codec).encoder().audio().map_err(|_| failed("the AAC encoder would not start"))?;
                audio.set_rate(rate as i32);
                audio.set_channel_layout(layout);
                audio.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
                audio.set_bit_rate(192_000);
                audio.set_time_base((1, rate as i32));
                if global {
                    audio.set_flags(ff::codec::Flags::GLOBAL_HEADER);
                }
                let audio = audio.open_as(codec).map_err(|_| failed("the AAC encoder would not start"))?;
                let mut stream = output.add_stream(codec).map_err(|_| failed("its sound could not be added"))?;
                stream.set_parameters(&audio);
                stream.set_time_base((1, rate as i32));
                Some((audio, Vec::new(), 0, usize::from(channels)))
            }
        };
        let mut options = ff::Dictionary::new();
        options.set("movflags", "+faststart");
        output.write_header_with(options).map_err(|why| failed(&format!("its header could not be written, {why}")))?;
        let video_base = output.stream(0).map(|s| s.time_base()).unwrap_or(ff::Rational(den, num));
        let audio_base = output.stream(1).map(|s| s.time_base()).unwrap_or(ff::Rational(1, 48000));
        Ok(Encoder { output: std::mem::ManuallyDrop::new(output), video, audio, settings, video_base, audio_base, written: 0, scratch, path: path.to_path_buf(), done: false })
    }

    /// Adds the next frame, `width × height × 4` bytes of RGBA.
    pub fn frame(&mut self, rgba: &[u8]) -> Result<()> {
        let (width, height) = (self.settings.width, self.settings.height);
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(error(format!("A frame for {} must be {width} × {height}.", named(&self.path))));
        }
        let mut frame = ff::frame::Video::new(ff::format::Pixel::YUV420P, width, height);
        let stride = [frame.stride(0), frame.stride(1), frame.stride(2)];
        let (luma, rest) = split_planes(&mut frame);
        let (u, v) = rest;
        colour::from_rgba(rgba, width as usize, height as usize, [luma, u, v], stride);
        frame.set_pts(Some(i64::from(self.written)));
        self.video.send_frame(&frame).map_err(|why| self.broke(why))?;
        self.written += 1;
        self.drain_video()
    }

    /// Adds stereo samples at the settings' rate, left and right in turn,
    /// as `mix` gives them; a film with one channel is written as their
    /// average.
    pub fn sound(&mut self, samples: &[f32]) -> Result<()> {
        let Some((_, waiting, _, channels)) = &mut self.audio else {
            return Err(error(format!("{} was made without sound.", named(&self.path))));
        };
        if *channels == 1 {
            waiting.extend(samples.chunks_exact(2).map(|pair| (pair[0] + pair[1]) / 2.0));
        } else {
            waiting.extend_from_slice(samples);
        }
        self.send_sound(false)
    }

    /// Frames written so far, for progress.
    pub fn frames_written(&self) -> u32 {
        self.written
    }

    /// Ends the file and puts it in its place.
    pub fn finish(mut self) -> Result<()> {
        self.send_sound(true)?;
        if let Some((audio, _, _, _)) = &mut self.audio {
            let _ = audio.send_eof();
        }
        self.drain_audio()?;
        self.video.send_eof().map_err(|why| self.broke(why))?;
        self.drain_video()?;
        self.output.write_trailer().map_err(|why| self.broke(why))?;
        std::fs::rename(&self.scratch, &self.path).map_err(|why| error(format!("{} could not be put in its place: {why}.", named(&self.path))))?;
        self.done = true;
        Ok(())
    }

    fn broke(&self, why: ff::Error) -> crate::Error {
        error(format!("{} could not be written: {why}.", named(&self.path)))
    }

    fn drain_video(&mut self) -> Result<()> {
        let mut packet = ff::Packet::empty();
        while self.video.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(ff::Rational(self.settings.rate.1.max(1) as i32, self.settings.rate.0 as i32), self.video_base);
            packet.write_interleaved(&mut self.output).map_err(|why| self.broke(why))?;
        }
        Ok(())
    }

    /// Sends the waiting samples in the encoder's frames of a fixed length;
    /// at the end, what is left too, padded with silence.
    fn send_sound(&mut self, last: bool) -> Result<()> {
        let Some((audio, waiting, next, channels)) = &mut self.audio else {
            return Ok(());
        };
        let channels = *channels;
        let layout = if channels == 1 { ff::ChannelLayout::MONO } else { ff::ChannelLayout::STEREO };
        let size = (audio.frame_size() as usize).max(1);
        let rate = audio.rate();
        let mut sent = 0;
        while waiting.len() - sent >= size * channels || (last && waiting.len() - sent >= channels) {
            let take = ((waiting.len() - sent) / channels).min(size);
            let mut frame = ff::frame::Audio::new(ff::format::Sample::F32(ff::format::sample::Type::Planar), size, layout);
            frame.set_rate(rate);
            for channel in 0..channels {
                let plane = frame.plane_mut::<f32>(channel);
                plane.fill(0.0);
                for i in 0..take {
                    plane[i] = waiting[sent + i * channels + channel];
                }
            }
            frame.set_pts(Some(*next));
            *next += size as i64;
            sent += take * channels;
            if let Err(why) = audio.send_frame(&frame) {
                return Err(error(format!("{} could not be written: {why}.", named(&self.path))));
            }
            let mut packet = ff::Packet::empty();
            while audio.receive_packet(&mut packet).is_ok() {
                packet.set_stream(1);
                packet.rescale_ts(ff::Rational(1, rate as i32), self.audio_base);
                if let Err(why) = packet.write_interleaved(&mut self.output) {
                    return Err(error(format!("{} could not be written: {why}.", named(&self.path))));
                }
            }
        }
        waiting.drain(..sent);
        Ok(())
    }

    fn drain_audio(&mut self) -> Result<()> {
        let Some((audio, _, _, _)) = &mut self.audio else {
            return Ok(());
        };
        let rate = audio.rate();
        let mut packet = ff::Packet::empty();
        while audio.receive_packet(&mut packet).is_ok() {
            packet.set_stream(1);
            packet.rescale_ts(ff::Rational(1, rate as i32), self.audio_base);
            packet.write_interleaved(&mut self.output).map_err(|why| error(format!("{} could not be written: {why}.", named(&self.path))))?;
        }
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // Safe: the output is never used again.
        unsafe { std::mem::ManuallyDrop::drop(&mut self.output) };
        if !self.done {
            let _ = std::fs::remove_file(&self.scratch);
        }
    }
}

/// The three planes of a 4:2:0 frame, to write at once.
fn split_planes(frame: &mut ff::frame::Video) -> (&mut [u8], (&mut [u8], &mut [u8])) {
    let heights = [frame.height() as usize, frame.height() as usize / 2, frame.height() as usize / 2];
    let strides = [frame.stride(0), frame.stride(1), frame.stride(2)];
    // FFmpeg keeps the planes apart; each is its stride × its height.
    unsafe {
        let raw = frame.as_mut_ptr();
        let plane = |i: usize| std::slice::from_raw_parts_mut((*raw).data[i], strides[i] * heights[i]);
        (plane(0), (plane(1), plane(2)))
    }
}
