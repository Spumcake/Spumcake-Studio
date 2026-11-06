//! The first slice's checks: what undo brings back and where it shows it,
//! which layers the stage puts above the one worked on, and what changes a
//! clip's frames' fingerprints and what does not.

use crate::board::{self, Boards, Which};
use crate::look::Looker;
use crate::session::Session;
use crate::session::film::LayerFlag;
use crate::registry::Tool;
use draw::document::Sample;

/// A stroke by hand, from `(x, y)` to the right.
fn stroke(session: &mut Session, x: f64, y: f64) {
    session.set_tool(Tool::Brush);
    session.begin().expect("the brush draws here");
    for step in 0..6 {
        session
            .add_sample(Sample { x: x + f64::from(step) * 12.0, y, pressure: Some(0.8) })
            .expect("a sample");
    }
    session.commit().expect("the stroke lands");
}

/// A session inside a new clip of 48 frames.
fn in_a_clip() -> (Session, u64) {
    let mut session = Session::new();
    let clip = session.add_clip(None, 48).expect("a clip");
    assert!(session.enter_clip(clip));
    (session, clip)
}

fn first_frame(session: &Session, boards: &mut Boards, looker: &mut Looker, clip: u64, which: Which) -> String {
    let record = session.active_storyboard();
    let clip = record.film().clip(clip).expect("the clip").clone();
    boards.fingerprint(looker, record, &clip, which).expect("a fingerprint")
}

#[test]
fn deleting_a_clip_and_undoing_brings_its_drawings_back() {
    let (mut session, clip) = in_a_clip();
    stroke(&mut session, 100.0, 100.0);
    assert_eq!(session.stroke_count(), 1);
    session.leave_clip();
    assert!(session.delete_clip(clip));
    session.undo().unwrap();
    assert_eq!(session.film().clips.len(), 1);
    assert!(session.enter_clip(clip));
    assert_eq!(session.stroke_count(), 1, "the clip's drawing did not come back");
}

#[test]
fn clearing_a_key_and_an_overwrite_drag_undo_with_their_drawings() {
    let (mut session, _) = in_a_clip();
    let layer = session.view().layer.unwrap();
    assert!(session.insert_key(layer, 12, true));
    stroke(&mut session, 100.0, 100.0);
    assert!(session.clear_key(layer, 12));
    session.seek_clip(12);
    assert_eq!(session.stroke_count(), 0, "the cleared key still shows");
    session.undo().unwrap();
    assert_eq!(session.view().frame, 12, "undo did not bring the key into view");
    assert_eq!(session.stroke_count(), 1);
    // A key dragged over the one at 12 replaces it; undo brings it back.
    assert!(session.insert_key(layer, 30, true));
    assert!(session.move_frames(&[layer], 30, 31, -18, false));
    session.seek_clip(12);
    assert_eq!(session.stroke_count(), 0);
    session.undo().unwrap();
    session.seek_clip(12);
    assert_eq!(session.stroke_count(), 1);
}

#[test]
fn undo_brings_its_change_into_view() {
    let (mut session, clip) = in_a_clip();
    stroke(&mut session, 100.0, 100.0);
    session.seek_clip(10);
    session.leave_clip();
    session.undo().unwrap();
    let view = session.view();
    assert_eq!(view.clip, Some(clip), "undo left the person on the film");
    assert_eq!(view.frame, 0);
    assert_eq!(session.stroke_count(), 0);
}

#[test]
fn the_stage_puts_upper_layers_above_the_one_worked_on() {
    let (mut session, _) = in_a_clip();
    let lower = session.view().layer.unwrap();
    let upper = session.add_layer(false).unwrap();
    stroke(&mut session, 100.0, 100.0);
    assert!(session.select_layer(lower));
    let stage = crate::sheet::stage(session.active_storyboard(), "1".into());
    // The paper, then the upper layer's drawing, above.
    let above: Vec<bool> = stage.underlays.iter().map(|underlay| underlay.above).collect();
    assert_eq!(above, vec![false, true]);
    assert!(stage.underlays[1].drawing.to_string().contains("\"strokes\""));
    // Hiding the active layer hides it on the stage too.
    assert!(session.set_layer_flag(lower, LayerFlag::Visible, false));
    let stage = crate::sheet::stage(session.active_storyboard(), "1".into());
    assert!(!stage.active.visible);
    assert!(!session.can_draw(), "a hidden layer can be drawn on");
    let _ = upper;
}

#[test]
fn a_frames_fingerprint_follows_what_it_renders() {
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    let (mut session, clip) = in_a_clip();
    let layer = session.view().layer.unwrap();
    let blank = first_frame(&session, &mut boards, &mut looker, clip, Which::First);
    stroke(&mut session, 300.0, 300.0);
    let drawn = first_frame(&session, &mut boards, &mut looker, clip, Which::First);
    assert_ne!(blank, drawn, "a stroke on the first frame did not change it");
    // The view, the name, the place on the film, and outline change nothing.
    session.zoom_toward(2.0, [10.0, 10.0]);
    assert!(session.set_clip_text(clip, crate::session::film::ClipText::Name, "Storm"));
    let second = session.add_clip(None, 24).unwrap();
    assert!(session.move_clips(&[clip], 30, true));
    assert!(session.enter_clip(clip));
    assert!(session.set_layer_flag(layer, LayerFlag::Outline, true));
    assert_eq!(first_frame(&session, &mut boards, &mut looker, clip, Which::First), drawn);
    assert!(session.set_layer_flag(layer, LayerFlag::Outline, false));
    // A stroke on a later frame leaves the first frame alone.
    assert!(session.insert_key(layer, 20, false));
    stroke(&mut session, 600.0, 600.0);
    assert_eq!(first_frame(&session, &mut boards, &mut looker, clip, Which::First), drawn);
    assert_ne!(first_frame(&session, &mut boards, &mut looker, clip, Which::Last), drawn);
    // Opacity and the eye change what renders.
    assert!(session.set_layer_opacity(layer, 0.5));
    let faded = first_frame(&session, &mut boards, &mut looker, clip, Which::First);
    assert_ne!(faded, drawn);
    assert!(session.set_layer_flag(layer, LayerFlag::Visible, false));
    assert_eq!(first_frame(&session, &mut boards, &mut looker, clip, Which::First), blank);
    let _ = second;
}

#[test]
fn a_frames_fingerprint_survives_saving_and_opening() {
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    let (mut session, clip) = in_a_clip();
    stroke(&mut session, 300.0, 300.0);
    let before = first_frame(&session, &mut boards, &mut looker, clip, Which::First);
    let value = crate::store::storyboard_value(session.active_storyboard(), "Kept");
    let stored = crate::store::stored_from_text(&value.to_string()).expect("the file reads back");
    let mut ids = draw::document::Allocator::new();
    let record = stored.into_record(&mut ids, &[]);
    let reopened = record.film().clip(clip).unwrap().clone();
    let mut fresh = Boards::default();
    assert_eq!(fresh.fingerprint(&mut looker, &record, &reopened, Which::First).unwrap(), before);
}

#[test]
fn a_frame_renders_as_a_png_in_the_project() {
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    let (mut session, clip) = in_a_clip();
    stroke(&mut session, 300.0, 300.0);
    let project = std::env::temp_dir().join(format!("storyboard-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    let (path, fingerprint) =
        board::render(&mut boards, &mut looker, session.active_storyboard(), &project, clip, Which::First).expect("a render");
    assert!(path.starts_with(".inpainter/renders/storyboard-generator/"));
    assert!(path.ends_with(&format!("-{fingerprint}.png")));
    let bytes = std::fs::read(project.join(&path)).expect("the file");
    assert_eq!(&bytes[1..4], b"PNG");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn growing_a_clip_from_inside_moves_the_next_one() {
    let mut session = Session::new();
    let first = session.add_clip(None, 48).unwrap();
    let second = session.add_clip(None, 48).unwrap();
    assert!(session.enter_clip(first));
    let layer = session.view().layer.unwrap();
    assert!(session.insert_frames(&[layer], 3, 5));
    let film = session.film();
    assert_eq!(film.clip(second).unwrap().start, film.clip(first).unwrap().start + film.clip(first).unwrap().length);
    assert!(session.insert_key(layer, 20, true));
    assert!(session.stretch_span(layer, 20, 30));
    let film = session.film();
    assert_eq!(film.clip(second).unwrap().start, film.clip(first).unwrap().length);
}

#[test]
fn trimming_a_clips_start_keeps_its_frames_on_the_film() {
    let (mut session, clip) = in_a_clip();
    let layer = session.view().layer.unwrap();
    assert!(session.insert_key(layer, 12, true));
    session.leave_clip();
    assert!(session.resize_clip(clip, false, 12));
    let at = session.film().clip(clip).unwrap();
    assert_eq!((at.start, at.length), (12, 36));
    // The key that showed at film frame 12 now starts the clip.
    let keys: Vec<u32> = at.layers[0].keys.iter().map(|key| key.frame).collect();
    assert_eq!(keys, vec![0]);
}

#[test]
fn the_look_tool_sees_the_other_layers_and_the_sheet() {
    let (mut session, _) = in_a_clip();
    stroke(&mut session, 300.0, 300.0);
    let upper = session.add_layer(false).unwrap();
    assert!(session.select_layer(upper));
    let marks = crate::sheet::stage_marks(session.active_storyboard());
    let ink = marks.iter().filter(|mark| matches!(mark, draw::document::SceneMark::Ink(_))).count();
    let words = marks.iter().filter(|mark| matches!(mark, draw::document::SceneMark::Text(_))).count();
    assert!(ink > 10, "the lower layer's stroke and the paper's lines are not in what the assistant sees");
    assert!(words >= 6, "the sheet's words are not in what the assistant sees");
}

#[test]
fn a_storyboard_from_before_films_opens_on_its_sheet_unchanged() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../host/tests/fixtures/v0.13/project/Untitled 1.smx");
    let before = std::fs::read(&path).expect("the 0.13 file");
    let stored: serde_json::Value = serde_json::from_slice(&before).expect("its JSON");
    let mut ids = draw::document::Allocator::new();
    let mut record = crate::store::read_smx(&path).expect("it opens").into_record(&mut ids, &[]);
    let film = record.film().clone();
    assert_eq!(film.clips.len(), 1);
    let clip = film.clips[0].clone();
    let key = clip.layers[0].keys[0].drawing;
    let drawing = record.drawing_of(key).expect("its drawing").snapshot_value(draw::document::DrawingId::new(1), "x");
    assert_eq!(drawing["nodes"], stored["nodes"], "the old drawing changed");
    // Going in and out of the clip is not a change, so nothing is written.
    let view = crate::record::View { clip: Some(clip.id), frame: 0, film_frame: 0, layer: None };
    record.go(view);
    record.go(crate::record::View::default());
    assert!(!record.is_dirty() && !record.needs_keep(), "opening the old storyboard changed it");
    assert_eq!(std::fs::read(&path).expect("the 0.13 file"), before);
    // Its field lies on its drawing, so its first frame is not blank.
    let mut boards = Boards::default();
    let mut looker = Looker::new();
    let drawn = boards.fingerprint(&mut looker, &record, &clip, Which::First).unwrap();
    let mut empty = Session::new();
    let blank_clip = empty.add_clip(None, 48).unwrap();
    let blank = first_frame(&empty, &mut boards, &mut looker, blank_clip, Which::First);
    assert_ne!(drawn, blank, "the old drawing is outside its field");
}
