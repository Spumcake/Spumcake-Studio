//! The transport row: play or pause, a frame back and forward, the time,
//! and the scrubber with its cues and thin rows, drawn under a 3D view or a
//! raster tab that declares one. With the host's clock the host moves the
//! time while playing; with the workspace's, it draws the declared time.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};

/// The row's height, and each thin row's under it.
pub(crate) const HEIGHT: f32 = 56.0;
pub(crate) const ROW: f32 = 18.0;
const PICKED: Color32 = Color32::from_rgb(37, 99, 235);
const CAPTION_GREY: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
/// A cue's mark when the workspace gives no colour: the door cue's orange.
const CUE: Color32 = Color32::from_rgb(234, 88, 12);

/// The height a transport takes, its thin rows included.
pub(crate) fn height(t: &protocol::Transport) -> f32 {
    HEIGHT + t.rows.len() as f32 * ROW
}

/// One tab's clock.
#[derive(Default)]
pub(crate) struct Clock {
    /// While the host's clock plays: when it started, and from which time.
    running: Option<(Instant, f64)>,
    /// Where the host's clock stopped, held until the workspace declares
    /// that it is no longer playing.
    stopped: Option<f64>,
}

/// What the row asks of the workspace.
#[derive(Debug)]
pub(crate) enum Event {
    /// The user scrubbed to a time, or the host's clock stopped there.
    Time(f64),
    /// Play, pause, or a step, as the transport's actions with their value.
    Command(protocol::Action),
}

/// The transport row under the view: play or pause, a frame back and
/// forward, the time and the length, and the scrubber with its frame ticks,
/// its cues, and its thin rows, as the approved views (vp-door-swing) show,
/// under a 3D view or a raster tab.
/// With the host's clock, the time runs on while playing, snapped to frames,
/// and stops at the end; returns the time to draw.
pub(crate) fn row(ui: &mut egui::Ui, clock: &mut Clock, t: &protocol::Transport, band: Rect, out: &mut Vec<Event>) -> f64 {
    let fps = if t.fps > 0.0 { t.fps } else { 24.0 };
    let snap = |time: f64| ((time * fps).round() / fps).clamp(0.0, t.length.max(0.0));
    let host_clock = t.clock == protocol::TransportClock::Host;
    let mut time = t.time;
    if !t.playing {
        clock.stopped = None;
    }
    if let Some(at) = clock.stopped.filter(|_| t.playing) {
        // Stopped here, and the workspace has not yet said it stopped: hold
        // still rather than start the clock again from its older time.
        time = at;
    } else if host_clock && t.playing {
        let (since, from) = *clock.running.get_or_insert((Instant::now(), t.time));
        let run = from + since.elapsed().as_secs_f64();
        time = ((run * fps).floor() / fps).min(t.length);
        if run >= t.length {
            clock.running = None;
            clock.stopped = Some(t.length);
            out.push(Event::Time(t.length));
            out.push(Event::Command(t.play.with_value(serde_json::json!(false))));
        } else {
            ui.ctx().request_repaint();
        }
    } else {
        clock.running = None;
    }

    let painter = ui.painter_at(band);
    let row = Rect::from_min_size(band.min, vec2(band.width(), HEIGHT));
    let y = row.center().y - 6.0;
    let mut x = row.min.x + 22.0;
    for (icon, tip, action) in [
        (if t.playing { "pause" } else { "play" }, "Play", t.play.with_value(serde_json::json!(!t.playing))),
        ("skip-prev", "Step back", t.step.with_value(serde_json::json!(-1))),
        ("skip-next", "Step forward", t.step.with_value(serde_json::json!(1))),
    ] {
        let spot = Rect::from_center_size(pos2(x, y), vec2(28.0, 28.0));
        let response = ui.interact(spot, ui.id().with(("transport", icon)), Sense::click()).on_hover_text(tip);
        crate::toolbar::paint_icon(&painter, spot.center(), icon, 20.0, Color32::from_rgb(20, 20, 20));
        if response.clicked() {
            if icon == "pause" && host_clock {
                clock.running = None;
                clock.stopped = Some(time);
                out.push(Event::Time(time));
            }
            out.push(Event::Command(action));
        }
        x += 36.0;
    }
    let words = format!("{:.2} s / {:.2} s", time, t.length);
    let placed = painter.text(pos2(x + 4.0, y), Align2::LEFT_CENTER, words, FontId::proportional(14.0), Color32::from_rgb(20, 20, 20));

    // The scrubber: a track with its played part blue, a tick per frame
    // where there is room for one, or per second, each cue a coloured mark
    // with its label under it, and the handle at the time.
    let track = Rect::from_min_max(pos2(placed.right() + 24.0, y - 2.0), pos2(row.max.x - 20.0, y + 2.0));
    if track.width() > 20.0 && t.length > 0.0 {
        let at_x = |time: f64| track.min.x + (time / t.length).clamp(0.0, 1.0) as f32 * track.width();
        painter.rect_filled(track, 2.0, Color32::from_rgb(226, 230, 236));
        painter.rect_filled(Rect::from_min_max(track.min, pos2(at_x(time), track.max.y)), 2.0, PICKED);
        let frames = (t.length * fps).round() as usize;
        let every = if track.width() / frames.max(1) as f32 >= 6.0 { 1 } else { fps.round().max(1.0) as usize };
        for frame in (0..=frames).step_by(every) {
            let tx = at_x(frame as f64 / fps);
            painter.vline(tx, (track.max.y + 6.0)..=(track.max.y + 11.0), Stroke::new(1.0, Color32::from_rgb(196, 200, 206)));
        }
        for cue in &t.cues {
            let colour = cue.colour.map_or(CUE, |[r, g, b]| Color32::from_rgb(r, g, b));
            let cx = at_x(cue.at);
            painter.vline(cx, (track.min.y - 7.0)..=(track.max.y + 7.0), Stroke::new(2.0, colour));
            if !cue.label.is_empty() {
                painter.text(pos2(cx, track.max.y + 14.0), Align2::CENTER_TOP, &cue.label, FontId::proportional(12.0), Color32::from_rgb(60, 60, 60));
            }
        }
        painter.circle_filled(pos2(at_x(time), track.center().y), 7.0, PICKED);
        let grab = track.expand2(vec2(8.0, 10.0));
        let response = ui.interact(grab, ui.id().with("scrubber"), Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let to = snap(f64::from((pointer.x - track.min.x) / track.width()) * t.length);
            clock.running = None;
            time = to;
            if response.clicked() || response.drag_stopped() || to != t.time {
                out.push(Event::Time(to));
            }
        }
        // The thin rows, the scrubber's width, each with its label at left.
        for (i, line) in t.rows.iter().enumerate() {
            let top = row.max.y + i as f32 * ROW;
            let image = Rect::from_min_max(pos2(track.min.x, top + 2.0), pos2(track.max.x, top + ROW - 2.0));
            painter.text(pos2(row.min.x + 22.0, image.center().y), Align2::LEFT_CENTER, &line.label, FontId::proportional(12.0), CAPTION_GREY);
            if let Some((texture, _)) =
                crate::raster::raw_image(ui.ctx(), &line.image.path, line.image.width, line.image.height, line.image.revision, &[], false)
            {
                painter.image(texture, image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
    }
    time
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_holds_where_it_stopped() {
        let transport = protocol::Transport {
            length: 0.1,
            fps: 24.0,
            time: 0.0,
            playing: true,
            clock: protocol::TransportClock::Host,
            cues: Vec::new(),
            rows: Vec::new(),
            play: protocol::Action::new("transport.play"),
            step: protocol::Action::new("transport.step"),
        };
        let ctx = egui::Context::default();
        crate::toolbar::install_fonts(&ctx);
        let mut clock = Clock::default();
        let frame = |clock: &mut Clock| {
            let mut out = Vec::new();
            let mut time = 0.0;
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 100.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, HEIGHT));
                time = row(ui, clock, &transport, band, &mut out);
            });
            // Nothing renders here, so the texture uploads are dropped.
            output.textures_delta.clear();
            (time, out)
        };
        let (first, out) = frame(&mut clock);
        assert_eq!((first, out.len()), (0.0, 0));
        std::thread::sleep(std::time::Duration::from_millis(150));
        let (at_end, out) = frame(&mut clock);
        assert!((at_end - 0.1).abs() < 1e-9, "{at_end}");
        assert!(matches!(out.as_slice(), [Event::Time(t), Event::Command(action)] if (*t - 0.1).abs() < 1e-9 && action.id == "transport.play"), "{out:?}");
        // The workspace has not answered: its declaration still says playing
        // from 0. The view stays at the end and says nothing more.
        for _ in 0..3 {
            let (held, out) = frame(&mut clock);
            assert!((held - 0.1).abs() < 1e-9 && out.is_empty(), "{held} {out:?}");
        }
    }
}
