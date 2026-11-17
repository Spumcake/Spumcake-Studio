//! The second slice's checks of sound and the animatic: an export is
//! checked by decoding the video it wrote, against what the test drew and
//! where it put a click, never against the code that wrote it; the cut and
//! each clip's frames change exactly when what the animatic would show or
//! play does.

use crate::board::{self, Boards};
use crate::look::Looker;
use crate::registry::Tool;
use crate::session::Session;
use crate::soundtrack::{TrackFlag, TrackRef};
use crate::sounds::{RATE, clicks, wav_bytes};
use draw::document::Sample;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The film's frame size in these checks: small, so they run quickly.
const SIZE: [u32; 2] = [320, 180];

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("storyboard-sound-checks-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A thick stroke by hand, a short line to the right of `(x, y)`.
fn dab(session: &mut Session, x: f64, y: f64) {
    session.set_tool(Tool::Brush);
    session.set_size(draw::defaults::SIZE_MAX).expect("a brush size");
    session.begin().expect("the brush draws here");
    for step in 0..4 {
        session.add_sample(Sample { x: x + f64::from(step) * 4.0, y, pressure: Some(1.0) }).expect("a sample");
    }
    session.commit().expect("the stroke lands");
}

/// A film in a project: clip 1 on frames 0..24 with a dab on the left;
/// a gap on 24..36; clip 2 on 36..60 with a dab on the right and, from its
/// frame 6, one at the bottom too; and a sound with a click on frame 30.
fn film(label: &str) -> (Session, PathBuf) {
    let project = scratch(label);
    let mut session = Session::new();
    session.set_project(&project);
    session.set_film_settings(Some("Checks"), None, Some(SIZE), None);
    let first = session.add_clip(None, 24).unwrap();
    session.enter_clip(first);
    dab(&mut session, 60.0, 60.0);
    session.leave_clip();
    let second = session.add_clip(Some(36), 24).unwrap();
    session.enter_clip(second);
    dab(&mut session, 250.0, 60.0);
    let layer = session.view().layer.unwrap();
    assert!(session.insert_key(layer, 6, false));
    session.seek_clip(6);
    dab(&mut session, 150.0, 150.0);
    session.leave_clip();
    let source = project.join("click.wav");
    std::fs::write(&source, wav_bytes(RATE, &clicks(60, &[30]))).unwrap();
    session.import_sound(&source, None, Some(0)).unwrap();
    (session, project)
}

/// Exports and waits for it. Returns the export's path.
fn export(session: &mut Session, project: &Path) -> PathBuf {
    let file = session.start_export().expect("the export starts");
    let started = Instant::now();
    while session.exporting().is_some() {
        assert!(started.elapsed() < Duration::from_secs(120), "the export never finished");
        std::thread::sleep(Duration::from_millis(20));
        session.poll_export();
    }
    assert!(session.active_storyboard().exported().is_some(), "{:?}", session.sound_notices());
    project.join(file)
}

/// How dark the decoded frame is around `(x, y)`, 0 white to 1 black.
fn darkness(frame: &media::Frame, x: u32, y: u32) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for dy in 0..4 {
        for dx in 0..4 {
            let at = (((y + dy) * frame.width + x + dx) * 4) as usize;
            let light = (f64::from(frame.rgba[at]) + f64::from(frame.rgba[at + 1]) + f64::from(frame.rgba[at + 2])) / (3.0 * 255.0);
            sum += 1.0 - light;
            count += 1.0;
        }
    }
    sum / count
}

fn frames(path: &Path) -> Vec<media::Frame> {
    let mut video = media::Video::open(path).expect("the export opens");
    let info = video.info();
    video.frames(0, (info.width, info.height)).map(|frame| frame.expect("a frame")).collect()
}

#[test]
fn the_export_shows_each_clip_in_its_place_with_its_sound() {
    let (mut session, project) = film("export");
    let path = export(&mut session, &project);
    assert_eq!(path.file_name().unwrap().to_string_lossy(), "Checks — animatic.mp4");
    let video = media::Video::open(&path).unwrap();
    let info = video.info();
    assert_eq!((info.width, info.height, info.frames, info.sound), (SIZE[0], SIZE[1], 60, true));
    let frames = frames(&path);
    assert_eq!(frames.len(), 60);
    // Clip 1: its dab on the left, nothing on the right.
    for frame in [0, 12, 23] {
        assert!(darkness(&frames[frame], 62, 58) > 0.5, "frame {frame} lacks clip 1's dab");
        assert!(darkness(&frames[frame], 252, 58) < 0.1, "frame {frame} shows clip 2's dab");
    }
    // The gap is black.
    for frame in [24, 30, 35] {
        assert!(darkness(&frames[frame], 160, 90) > 0.9, "frame {frame} of the gap is not black");
    }
    // Clip 2: its dab on the right; the one at the bottom from its frame 6.
    assert!(darkness(&frames[36], 252, 58) > 0.5);
    assert!(darkness(&frames[36], 62, 58) < 0.1);
    assert!(darkness(&frames[41], 152, 148) < 0.1, "the key at frame 6 shows early");
    assert!(darkness(&frames[42], 152, 148) > 0.5, "the key at frame 6 does not show at 36 + 6");
    // The click is heard on frame 30, give or take one.
    let sound = media::Sound::open(&path, RATE).unwrap();
    let per_frame = (RATE / crate::film::FPS) as usize * media::Sound::CHANNELS;
    let loudness: Vec<f32> = sound.samples.chunks(per_frame).map(|chunk| chunk.iter().fold(0.0f32, |most, sample| most.max(sample.abs()))).collect();
    let loudest = loudness.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    assert!(loudest.abs_diff(30) <= 1, "the click is loudest on frame {loudest}");
    // As loud as the mix the player plays, frame by frame.
    let mix = session.loudness(0, 60);
    for frame in [29, 30, 31] {
        let (heard, mixed) = (f64::from(loudness[frame]), f64::from(mix[frame]));
        if mixed > 0.05 {
            assert!((20.0 * (heard / mixed).log10()).abs() < 1.0, "frame {frame}: {heard} against {mixed}");
        }
    }
}

#[test]
fn muted_sound_and_a_hidden_shots_track_leave_the_export() {
    let (mut session, project) = film("muted");
    let track = session.film().sound_tracks[0].id;
    assert!(session.set_track_flag(TrackRef::Sound(track), TrackFlag::Muted, true));
    assert!(session.set_track_flag(TrackRef::Shots, TrackFlag::Visible, false));
    let path = export(&mut session, &project);
    assert!(!media::Video::open(&path).unwrap().info().sound, "a muted film has no sound");
    for frame in frames(&path).iter().step_by(10) {
        assert!(darkness(frame, 62, 58) < 0.1 && darkness(frame, 160, 90) < 0.1, "a hidden Shots track shows white");
    }
}

#[test]
fn a_cancelled_export_leaves_the_previous_one_whole() {
    let (mut session, project) = film("cancel");
    let path = export(&mut session, &project);
    let before = std::fs::read(&path).unwrap();
    let record = session.active_storyboard().exported().cloned();
    // A changed and longer film, cancelled once it is under way: had the
    // cancel done nothing, the file would differ.
    let first = session.film().clips[0].id;
    assert!(session.resize_clip(first, true, 600));
    session.start_export().unwrap();
    let started = Instant::now();
    while session.exporting().is_some_and(|(written, _)| written == 0) {
        assert!(started.elapsed() < Duration::from_secs(60), "the export never began");
        std::thread::sleep(Duration::from_millis(1));
    }
    let (written, total) = session.exporting().expect("still under way");
    assert!(written < total, "it finished before it could be cancelled");
    session.cancel_export();
    let started = Instant::now();
    while session.exporting().is_some() {
        assert!(started.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
        session.poll_export();
    }
    assert_eq!(std::fs::read(&path).unwrap(), before, "the previous export changed");
    assert_eq!(session.active_storyboard().exported().cloned(), record, "a cancelled export was recorded");
    let left: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left.len(), 1, "a partial file remains: {left:?}");
}

#[test]
fn a_missing_sound_refuses_the_export_and_names_its_file() {
    let (mut session, project) = film("missing");
    // Removed while the storyboard is open, and decoded: it is still missed.
    session.load_sounds();
    std::fs::remove_file(project.join("sounds/click.wav")).unwrap();
    let reason = session.start_export().unwrap_err();
    assert_eq!(reason, "click.wav is missing from the sounds folder.");
    assert!(session.sound_notices().iter().any(|notice| notice.detail == reason));
}

#[test]
fn a_sound_replaced_while_open_changes_the_cut() {
    let (mut session, project) = film("replaced");
    let cut = item(&mut session, board::CUT);
    // The same length, a click on another frame: only the content differs.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(project.join("sounds/click.wav"), wav_bytes(RATE, &clicks(60, &[10]))).unwrap();
    assert_ne!(item(&mut session, board::CUT), cut, "the cut kept the old sound");
    assert!(session.loudness(10, 1)[0] > 0.5, "the new sound is not what plays");
}

fn item(session: &mut Session, id: &str) -> String {
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    session
        .items(&mut boards, &mut looker, "Checks.smx")
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.fingerprint)
        .unwrap_or_else(|| panic!("no item {id}"))
}

#[test]
fn the_cut_and_the_clips_frames_change_with_what_the_animatic_would_show() {
    let (mut session, _project) = film("fingerprints");
    let first = session.film().clips[0].id;
    let second = session.film().clips[1].id;
    let (cut, frames_1, frames_2) = (item(&mut session, board::CUT), item(&mut session, &board::frames_id(first)), item(&mut session, &board::frames_id(second)));
    // Looking changes nothing: a selection, the playhead, a rename, the
    // listening volume.
    session.select_clips(vec![first]);
    session.seek_film(40);
    session.set_clip_text(first, crate::session::film::ClipText::Name, "Opening");
    session.set_listening(0.3);
    assert_eq!(item(&mut session, board::CUT), cut);
    assert_eq!(item(&mut session, &board::frames_id(first)), frames_1);
    // A stroke on clip 2's frame 10 changes its frames, not clip 1's or the cut.
    session.enter_clip(second);
    session.seek_clip(10);
    dab(&mut session, 100.0, 100.0);
    session.leave_clip();
    assert_ne!(item(&mut session, &board::frames_id(second)), frames_2);
    assert_eq!(item(&mut session, &board::frames_id(first)), frames_1);
    assert_eq!(item(&mut session, board::CUT), cut);
    // Each of these changes the cut.
    let sound = session.film().sound_tracks[0].clips[0].id;
    let track = session.film().sound_tracks[0].id;
    let changes: Vec<Box<dyn Fn(&mut Session) -> bool>> = vec![
        Box::new(move |session| session.move_clips(&[first], 2, true)),
        Box::new(move |session| session.resize_clip(first, true, -3)),
        Box::new(move |session| session.move_sounds(&[sound], None, 5, true)),
        Box::new(move |session| session.trim_sound(sound, true, -4)),
        Box::new(move |session| session.set_sound_volume(sound, 0.5)),
        Box::new(move |session| session.set_track_flag(TrackRef::Sound(track), TrackFlag::Muted, true)),
        Box::new(|session| session.set_track_flag(TrackRef::Shots, TrackFlag::Visible, false)),
        Box::new(move |session| session.delete_clip(first)),
    ];
    for (index, change) in changes.iter().enumerate() {
        let before = item(&mut session, board::CUT);
        assert!(change(&mut session), "change {index} did nothing");
        assert_ne!(item(&mut session, board::CUT), before, "change {index} left the cut");
    }
    // Undoing them all brings the cut's first fingerprint back.
    for _ in 0..changes.len() + 1 {
        session.undo().unwrap();
    }
    assert_eq!(item(&mut session, board::CUT), cut);
}

#[test]
fn undoing_an_edit_made_before_an_export_keeps_the_export() {
    let (mut session, project) = film("undo-export");
    let first = session.film().clips[0].id;
    assert!(session.move_clips(&[first], 1, true));
    export(&mut session, &project);
    let exported = item(&mut session, board::ANIMATIC);
    session.undo().unwrap();
    assert_eq!(item(&mut session, board::ANIMATIC), exported, "the export was lost or changed by undo");
    assert_eq!(changed_sources(&mut session), [board::CUT], "after the undo the film differs from its export");
    // Rendered, it is copied once and named by its fingerprint.
    let (rendered, fingerprint) = session.render_animatic().unwrap();
    assert_eq!(fingerprint, exported);
    assert!(project.join(&rendered).is_file());
    // Exported again after an edit, the first render is untouched.
    let first_render = std::fs::read(project.join(&rendered)).unwrap();
    session.move_clips(&[first], 3, true);
    export(&mut session, &project);
    assert_eq!(std::fs::read(project.join(&rendered)).unwrap(), first_render);
    // Exporting again clears the mark, and gives the animatic a new fingerprint.
    assert!(changed_sources(&mut session).is_empty());
    assert_ne!(item(&mut session, board::ANIMATIC), exported);
    // The export file gone, render says so and exports nothing.
    let exported = session.active_storyboard().exported().unwrap().clone();
    std::fs::remove_file(project.join(&exported.file)).unwrap();
    assert_eq!(session.render_animatic().unwrap_err(), "The animatic video has been moved or replaced. Export it again.");
    assert!(session.exporting().is_none());
}

/// One tool of a request, through the protocol as the host sends it.
fn invoke(server: &mut crate::serve::Server, tab: &str, tool: &str, input: serde_json::Value, group: &str) -> serde_json::Value {
    let replies = server
        .handle(protocol::HostMessage::Invoke {
            call: format!("{group}-{tool}"),
            tab: tab.to_string(),
            tool: protocol::tool_key("storyboard-generator", "clips", tool),
            input,
            group: group.to_string(),
        })
        .unwrap();
    replies
        .into_iter()
        .find_map(|reply| match reply {
            protocol::AppMessage::ToolResult(result) => Some(result.output),
            _ => None,
        })
        .expect("a result")
}

#[test]
fn one_request_imports_a_sound_and_times_a_clip_to_it_as_one_undo() {
    let project = scratch("request");
    let source = project.join("dialogue.wav");
    std::fs::write(&source, wav_bytes(RATE, &clicks(96, &[40]))).unwrap();
    let mut server = crate::serve::Server::new(None);
    let greeting = server.greeting();
    let tab = greeting
        .iter()
        .find_map(|message| match message {
            protocol::AppMessage::Document { tab, .. } => Some(tab.clone()),
            _ => None,
        })
        .unwrap();
    server.handle(protocol::HostMessage::Project { folder: project.display().to_string(), skills: false });
    invoke(&mut server, &tab, "add_clip", serde_json::json!({ "length": 24 }), "setup");
    let second = invoke(&mut server, &tab, "add_clip", serde_json::json!({ "length": 24 }), "setup")["made"].as_u64().unwrap();
    // The request: import the dialogue, hear where it starts, put the
    // second clip there.
    invoke(&mut server, &tab, "import_sound", serde_json::json!({ "path": source.display().to_string(), "frame": 1 }), "request");
    let heard = invoke(&mut server, &tab, "read_sound", serde_json::json!({ "start": 1, "end": 96 }), "request");
    let first_loud = heard["loudness"].as_array().unwrap().iter().position(|value| value.as_f64().unwrap() > 0.5).unwrap() as i64;
    assert_eq!(first_loud, 40);
    let moved = invoke(&mut server, &tab, "move_clips", serde_json::json!({ "clips": [second], "frames": first_loud - 24 }), "request");
    assert_eq!(moved["clips"][1]["start"], serde_json::json!(41));
    // One undo takes the whole request back.
    server.handle(protocol::HostMessage::Command {
        id: "undo".into(),
        tab: Some(tab.clone()),
        arg: None,
        node: None,
        viewport: [800.0, 600.0],
    });
    let film = invoke(&mut server, &tab, "read_film", serde_json::json!({}), "check");
    assert_eq!(film["sound_tracks"], serde_json::json!([]));
    assert_eq!(film["clips"][1]["start"], serde_json::json!(25));
}

/// The animatic's recorded sources against the items as they are now: what
/// the host's index compares to say it is out of date.
fn changed_sources(session: &mut Session) -> Vec<String> {
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    let items = session.items(&mut boards, &mut looker, "Checks.smx");
    let animatic = items.iter().find(|item| item.id == board::ANIMATIC).expect("an animatic");
    let sources = &animatic.made.as_ref().expect("made").sources;
    sources
        .iter()
        .filter(|source| {
            let id = source.item.item.as_deref().unwrap();
            items.iter().find(|item| item.id == id).is_none_or(|item| item.fingerprint != source.fingerprint)
        })
        .map(|source| source.item.item.clone().unwrap())
        .collect()
}

#[test]
fn the_animatic_is_out_of_date_exactly_while_the_film_differs_from_its_export() {
    let (mut session, project) = film("stale");
    export(&mut session, &project);
    assert!(changed_sources(&mut session).is_empty());
    let first = session.film().clips[0].id;
    let second = session.film().clips[1].id;
    // Moving a clip changes the cut.
    assert!(session.move_clips(&[first], 2, true));
    assert_eq!(changed_sources(&mut session), [board::CUT]);
    // Undone, nothing is out of date.
    session.undo().unwrap();
    assert!(changed_sources(&mut session).is_empty());
    // Drawing on the second clip names its frames.
    session.enter_clip(second);
    dab(&mut session, 100.0, 120.0);
    session.leave_clip();
    assert_eq!(changed_sources(&mut session), [board::frames_id(second)]);
    // The record survives a save and a reopen.
    let path = project.join("Checks.smx");
    crate::store::write_smx(&path, session.active_storyboard(), "Checks").unwrap();
    let reopened = crate::store::read_smx(&path).unwrap();
    let mut ids = draw::document::Allocator::new();
    let record = reopened.into_record(&mut ids, &[]);
    assert_eq!(record.exported(), session.active_storyboard().exported());
}

/// A film to play, with a silent Player whose clock only the test moves.
fn playing_film(label: &str) -> (Session, PathBuf) {
    let (mut session, project) = film(label);
    let player = media::Player::silent(RATE, crate::sounds::fps());
    // From the first advance, only the test moves the clock.
    player.advance(0.0);
    session.use_player(player);
    (session, project)
}

/// The stage as the host is sent it now.
fn stage(session: &Session) -> protocol::Stage {
    session.playing_stage(crate::sheet::stage(session.active_storyboard(), "1".into()))
}

#[test]
fn playing_shows_the_frame_the_players_clock_names() {
    let (mut session, _project) = playing_film("play");
    session.seek_film(0);
    session.start_playing();
    assert!(session.is_playing());
    let first = stage(&session);
    let image = first.image.clone().expect("the first frame shows at once");
    // Only the film's image: no paper, no layers, the drawing hidden.
    assert!(first.underlays.is_empty() && first.overlays.is_empty() && !first.active.visible);
    let mut revisions = vec![image.revision];
    let mut shown = vec![0];
    // Half a second of the clock, a tick each 10 ms.
    for _ in 0..50 {
        session.player().advance(10.0);
        let named = session.player().frame_to_show();
        if session.playback_tick() {
            let now = stage(&session);
            let frame = session.view().film_frame;
            // Each frame sent is the one the clock named when it was sent.
            assert_eq!(Some(frame), named);
            if let Some(image) = now.image {
                assert_eq!(image.path, first.image.as_ref().unwrap().path, "every frame is one path");
                assert!(image.revision > *revisions.last().unwrap(), "revisions rise");
                revisions.push(image.revision);
            } else {
                assert_eq!(now.fill, Some([0, 0, 0]), "a frame with no image is the gap's black");
            }
            shown.push(frame);
        }
    }
    assert_eq!(*shown.last().unwrap(), 12, "half a second is twelve frames");
    // A jump of the Player's own is followed, not a count of ticks.
    session.player().play_from(40);
    session.player().advance(1.0);
    session.playback_tick();
    assert_eq!(session.view().film_frame, 40);
    // The lead runs the image ahead of the sound.
    session.player().set_lead(500.0);
    session.player().advance(1.0);
    session.playback_tick();
    assert_eq!(session.view().film_frame, 52);
    // Pausing keeps the frame and shows the drawing again.
    session.stop_playing();
    assert!(!session.is_playing());
    assert!(stage(&session).image.is_none());
    assert_eq!(session.view().film_frame, 52);
}

#[test]
fn shown_reports_keep_the_lead() {
    let (mut session, _project) = playing_film("lead");
    session.start_playing();
    let revision = stage(&session).image.unwrap().revision;
    let sent = crate::playback::now_ms();
    session.frame_shown(revision, sent + 40.0);
    let lead = session.player().lead();
    assert!((lead - 40.0).abs() < 5.0, "the lead is {lead} ms");
}

#[test]
fn scrubbing_inside_a_clip_plays_the_films_frame() {
    let (mut session, _project) = playing_film("scrub");
    let second = session.film().clips[1].id;
    session.enter_clip(second);
    // Clip 2 starts at the film's frame 36: its frame 5 is the film's 41.
    session.scrub(5, true);
    assert_eq!(session.player().last_scrubbed(), Some(41));
    assert!(stage(&session).image.is_some(), "a live seek shows the image");
    session.scrub(5, false);
    assert!(stage(&session).image.is_none(), "letting go shows the drawing");
    assert_eq!(session.view().frame, 5);
    // Play inside a clip plays its span, and loop repeats it.
    session.toggle_loop();
    session.start_playing();
    let played = session.player().last_played().unwrap();
    assert_eq!((played.from, played.to, played.looping), (36, Some(60), true));
    assert!(!session.can_draw(), "the drawing tools rest while playing");
}

#[test]
fn the_clip_shows_the_films_sound_under_its_layers() {
    let (mut session, _project) = film("film-sound");
    session.load_sounds();
    // The click is on the film's frame 30, which is no clip's: move clip 2
    // over it, to start at 24, so its frame 6 holds the click.
    let second = session.film().clips[1].id;
    session.move_clips(&[second], -12, false);
    session.enter_clip(second);
    let timeline = crate::timeline::declare(&session, &mut Boards::default(), &mut Looker::new(), "1".into());
    let row = timeline.rows.last().unwrap();
    assert_eq!((row.name.as_str(), row.read_only), ("Film sound", true));
    let protocol::RowContent::Items { items } = &row.content else {
        panic!("the film's sound is one item");
    };
    let peaks = &items[0].waveform;
    let loudest = peaks.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    assert_eq!(loudest / crate::sounds::PEAKS_PER_FRAME, 6, "the click shows on the clip's frame 6");
}

#[test]
fn a_step_plays_its_frames_slice_of_sound() {
    let (mut session, _project) = playing_film("step");
    let second = session.film().clips[1].id;
    session.enter_clip(second);
    session.seek_clip(4);
    assert!(crate::timeline::command(&mut session, "step_forward", None));
    // Clip 2 starts at the film's frame 36: its frame 5 is the film's 41.
    assert_eq!(session.player().last_scrubbed(), Some(41));
    assert!(crate::timeline::command(&mut session, "step_back", None));
    assert_eq!(session.player().last_scrubbed(), Some(40));
}

#[test]
fn playback_shows_the_exports_own_image() {
    let (mut session, _project) = playing_film("same-image");
    session.seek_film(40);
    session.start_playing();
    let image = stage(&session).image.expect("an image");
    let shown = std::fs::read(&image.path).unwrap();
    let tracks = session.mix_tracks();
    let mut plan = crate::export::plan(session.active_storyboard(), tracks, session.project());
    let frame = plan.frames[40];
    let exported = crate::export::pixels(&mut Looker::new(), &mut plan, frame);
    assert_eq!((image.width, image.height), plan.size);
    assert!(shown == exported, "playback's image differs from the export's");
}

#[test]
fn playing_through_three_holds_keeps_one_path_with_rising_revisions() {
    let (mut session, _project) = playing_film("holds");
    session.seek_film(0);
    session.start_playing();
    let path = stage(&session).image.unwrap().path;
    let mut revision = 0;
    let mut holds = std::collections::BTreeSet::new();
    // Clip 1's hold, the gap, and clip 2's two holds.
    for _ in 0..300 {
        session.player().advance(10.0);
        if !session.playback_tick() || !session.is_playing() {
            continue;
        }
        let now = stage(&session);
        let frame = session.view().film_frame;
        if let Some(image) = now.image {
            assert_eq!(image.path, path, "every frame is one path");
            assert!(image.revision > revision, "revisions rise");
            revision = image.revision;
            assert!(now.underlays.is_empty() && !now.active.visible);
            holds.insert(if frame < 24 { 1 } else if frame < 42 { 2 } else { 3 });
        }
    }
    assert_eq!(holds.len(), 3, "playback did not pass through every hold: {holds:?}");
}

#[test]
fn a_storyboard_with_sound_reopens_through_the_protocol() {
    let project = scratch("reopen");
    let source = project.join("line.wav");
    std::fs::write(&source, wav_bytes(RATE, &clicks(48, &[6]))).unwrap();
    let mut server = crate::serve::Server::new(None);
    let tab = server
        .greeting()
        .iter()
        .find_map(|message| match message {
            protocol::AppMessage::Document { tab, .. } => Some(tab.clone()),
            _ => None,
        })
        .unwrap();
    server.handle(protocol::HostMessage::Project { folder: project.display().to_string(), skills: false });
    invoke(&mut server, &tab, "add_clip", serde_json::json!({}), "one");
    invoke(&mut server, &tab, "import_sound", serde_json::json!({ "path": source.display().to_string(), "frame": 3 }), "one");
    server.handle(protocol::HostMessage::Command { id: "save".into(), tab: Some(tab.clone()), arg: None, node: None, viewport: [800.0, 600.0] });
    let saved: Vec<_> = std::fs::read_dir(&project).unwrap().filter_map(|entry| entry.ok()).map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "smx")).collect();
    assert_eq!(saved.len(), 1, "the storyboard was not written into the project");
    // A new program opens it: the sound is decoded again from sounds/.
    let mut again = crate::serve::Server::new(None);
    again.greeting();
    again.handle(protocol::HostMessage::Project { folder: project.display().to_string(), skills: false });
    let replies = again.handle(protocol::HostMessage::Open { path: Some(saved[0].display().to_string()) }).unwrap();
    let opened = replies
        .iter()
        .find_map(|message| match message {
            protocol::AppMessage::Document { tab, .. } => Some(tab.clone()),
            _ => None,
        })
        .expect("the storyboard opened");
    let film = invoke(&mut again, &opened, "read_film", serde_json::json!({}), "check");
    let sound = &film["sound_tracks"][0]["sounds"][0];
    // Placed at frame 3, counted from 1, as the tool was told.
    assert_eq!((sound["start"].as_u64(), sound["missing"].as_bool()), (Some(3), Some(false)), "{film}");
    // Its click, 6 frames into the sound, is heard on the film's 9th frame.
    let heard = invoke(&mut again, &opened, "read_sound", serde_json::json!({ "start": 1, "end": 12 }), "check");
    assert!(heard["loudness"][8].as_f64().unwrap() > 0.5, "the reopened sound is not heard: {heard}");
    assert!(heard["loudness"][7].as_f64().unwrap() < 0.01, "the reopened sound is early: {heard}");
}

#[test]
fn looking_and_reopening_leave_the_cut_alone() {
    let (mut session, project) = film("cut-stays");
    let cut = item(&mut session, board::CUT);
    session.set_view([120.0, -40.0], 2.0);
    assert_eq!(item(&mut session, board::CUT), cut, "a zoom changed the cut");
    let path = project.join("Checks.smx");
    crate::store::write_smx(&path, session.active_storyboard(), "Checks").unwrap();
    let record = crate::store::read_smx(&path).unwrap().into_record(&mut draw::document::Allocator::new(), &[]);
    let mut sounds = crate::sounds::Library::default();
    assert_eq!(board::cut_fingerprint(record.film(), &mut sounds, Some(&project)), cut, "a save and reopen changed the cut");
}

#[test]
fn a_hidden_shots_track_shows_faded_on_the_film() {
    let (mut session, _project) = film("faded");
    session.seek_film(5);
    let shown = crate::sheet::stage(session.active_storyboard(), "1".into());
    assert!(session.set_track_flag(TrackRef::Shots, TrackFlag::Visible, false));
    let hidden = crate::sheet::stage(session.active_storyboard(), "1".into());
    let opacity = |stage: &protocol::Stage| stage.underlays.last().map(|underlay| underlay.opacity).unwrap();
    assert!(opacity(&hidden) < opacity(&shown) * 0.5, "hiding the Shots track left the film's stage as it was");
}
