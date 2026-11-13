//! What the library writes, it reads back: frames by their index, their
//! colours, the sound's length, and nothing left by a cancelled export.

use media::{Encoder, Settings, Sound, Video};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("media-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Frame `i`: a flat colour that differs for every frame, with a bar whose
/// place says the frame's index.
fn pattern(i: u32, w: u32, h: u32) -> Vec<u8> {
    let colour = [(i * 37 % 200 + 30) as u8, (i * 91 % 200 + 30) as u8, (i * 53 % 200 + 30) as u8];
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _y in 0..h {
        for x in 0..w {
            let bar = x / 4 == i % (w / 4);
            let c = if bar { [250, 250, 250] } else { colour };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    rgba
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs() as f64).sum::<f64>() / a.len() as f64
}

fn write_film(path: &std::path::Path, frames: u32, w: u32, h: u32) {
    let rate = 48_000;
    let samples: Vec<f32> = (0..rate * 2).flat_map(|n| {
        let s = ((f64::from(n) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() * 0.5) as f32;
        [s, s]
    }).collect();
    let tone = media::Track { pcm: std::sync::Arc::new(Sound { rate, samples }), at: 0, from: 0, length: 48, volume: 1.0 };
    let mut encoder = Encoder::create(path, Settings { width: w, height: h, rate: (24, 1), keyframes: 12, sound: Some((rate, 2)) }).unwrap();
    for i in 0..frames {
        encoder.frame(&pattern(i, w, h)).unwrap();
        // One frame's sound, mixed as playback mixes it: a tone.
        encoder.sound(&media::mix(&[tone.clone()], i, 1, rate, (24, 1))).unwrap();
    }
    assert_eq!(encoder.frames_written(), frames);
    encoder.finish().unwrap();
}

/// Catches frames read back as the wrong frame or the wrong colour: an
/// export of 48 frames with B-frames, read by any index, forward, back, and
/// in order, gives each frame as it was written.
#[test]
fn frames_come_back_by_their_index() {
    let dir = scratch("frames");
    let path = dir.join("film.mp4");
    let (w, h) = (128, 96);
    write_film(&path, 48, w, h);
    let mut video = Video::open(&path).unwrap();
    let info = video.info();
    assert_eq!((info.width, info.height, info.rate, info.frames, info.sound), (w, h, (24, 1), 48, true));
    for index in [0, 1, 2, 30, 29, 13, 12, 11, 47, 5] {
        let frame = video.frame(index, (w, h)).unwrap();
        assert_eq!(frame.index, index);
        let difference = mean_difference(&frame.rgba, &pattern(index, w, h));
        assert!(difference < 3.0, "frame {index} differs by {difference}");
    }
    let order: Vec<u32> = video.frames(40, (w, h)).map(|frame| frame.unwrap().index).collect();
    assert_eq!(order, (40..48).collect::<Vec<_>>());
    // At half size, still the same frame.
    let small = video.frame(20, (w / 2, h / 2)).unwrap();
    assert_eq!((small.index, small.width, small.height), (20, w / 2, h / 2));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches sound lost or stretched on the way: two seconds written come
/// back as two seconds at the rate asked, as written, lower, and higher.
#[test]
fn sound_comes_back_at_the_rate_asked() {
    let dir = scratch("sound");
    let path = dir.join("film.mp4");
    write_film(&path, 48, 64, 48);
    for rate in [48_000, 32_000, 96_000] {
        let sound = Sound::open(&path, rate).unwrap();
        let seconds = sound.len() as f64 / f64::from(rate);
        assert!((seconds - 2.0).abs() < 0.05, "{seconds} s at {rate}");
        let loudest = sound.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(loudest > 0.3 && loudest < 0.7, "{loudest}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a cancelled export leaving a broken file: dropped before it is
/// finished, it leaves nothing, and a finished one leaves no scratch file.
#[test]
fn a_cancelled_export_leaves_nothing() {
    let dir = scratch("cancel");
    let path = dir.join("film.mp4");
    let mut encoder = Encoder::create(&path, Settings { width: 64, height: 48, rate: (24, 1), keyframes: 24, sound: None }).unwrap();
    encoder.frame(&pattern(0, 64, 48)).unwrap();
    drop(encoder);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a frame file the host could read half of, or a PNG that is not
/// one: both land whole under their names.
#[test]
fn frames_are_written_whole() {
    let dir = scratch("write");
    let rgba = pattern(3, 16, 8);
    let path = media::write_frame(&dir, "shot.rgba", &rgba, (16, 8)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), rgba);
    assert!(media::write_frame(&dir, "short.rgba", &rgba[4..], (16, 8)).is_err());
    let png = dir.join("first.png");
    media::write_png(&png, &rgba, (16, 8)).unwrap();
    assert_eq!(&std::fs::read(&png).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|name| !name.ends_with(".partial")), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The first sample frame louder than a whisper.
fn onset(sound: &Sound) -> f64 {
    let first = sound.samples.chunks(2).position(|pair| pair[0].abs() > 0.05).expect("no sound");
    first as f64 / f64::from(sound.rate)
}

/// Catches sound shifted against its frames: a burst mixed in at frame 24
/// of an export is heard from exactly one second.
#[test]
fn sound_lands_on_its_frame() {
    let dir = scratch("onset");
    let path = dir.join("film.mp4");
    let rate = 48_000;
    let burst = Sound { rate, samples: vec![0.5; 2 * rate as usize / 4] };
    let track = media::Track { pcm: std::sync::Arc::new(burst), at: 24, from: 0, length: 6, volume: 1.0 };
    let mut encoder = Encoder::create(&path, Settings { width: 64, height: 48, rate: (24, 1), keyframes: 24, sound: Some((rate, 2)) }).unwrap();
    for i in 0..48 {
        encoder.frame(&pattern(i, 64, 48)).unwrap();
        encoder.sound(&media::mix(&[track.clone()], i, 1, rate, (24, 1))).unwrap();
    }
    encoder.finish().unwrap();
    let at = onset(&Sound::open(&path, rate).unwrap());
    assert!((at - 1.0).abs() < 0.001, "heard from {at} s");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a video's sound played early when it starts after the frames,
/// as clips from elsewhere can: sound half a second late stays half a
/// second late. Needs the `ffmpeg` program to make the clip.
#[test]
fn late_sound_stays_late() {
    let dir = scratch("late");
    let path = dir.join("clip.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "color=c=red:s=64x48:r=24:d=2", "-itsoffset", "0.5", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
        .arg(&path)
        .status()
        .expect("this check needs the ffmpeg program");
    assert!(made.success());
    let at = onset(&Sound::open(&path, 48_000).unwrap());
    assert!((at - 0.5).abs() < 0.03, "heard from {at} s");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Catches a workspace waiting on decoding, or shown a frame it no longer
/// wants: of two requests one after the other, only the second's frame
/// comes back.
#[test]
fn the_newest_request_wins() {
    let dir = scratch("seeker");
    let path = dir.join("film.mp4");
    write_film(&path, 48, 64, 48);
    let mut seeker = media::Seeker::new(Video::open(&path).unwrap());
    seeker.frame(40, (64, 48));
    seeker.frame(7, (64, 48));
    let started = std::time::Instant::now();
    let frame = loop {
        if let Some(frame) = seeker.poll() {
            break frame.unwrap();
        }
        assert!(started.elapsed().as_secs() < 5, "no frame came");
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert_eq!(frame.index, 7);
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(seeker.poll().is_none(), "an older request's frame came too");
    seeker.play(10, (32, 24));
    let mut got = Vec::new();
    while got.len() < 5 && started.elapsed().as_secs() < 5 {
        if let Some(frame) = seeker.poll() {
            got.push(frame.unwrap().index);
        }
    }
    assert_eq!(got, vec![10, 11, 12, 13, 14]);
    let _ = std::fs::remove_dir_all(&dir);
}
