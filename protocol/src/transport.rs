//! A transport row: play, step back and forward, the time, and a scrubber
//! with its cues, drawn by the host under a tab's area, the same for a 3D
//! view's animation and for video. The workspace declares it; the host sends
//! play, pause, and step as `command`s, and a scrub as `transport_time`.

use serde::{Deserialize, Serialize};

use crate::{Cue, Thumbnail};

/// What the transport row shows and how its time moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    /// The length, in seconds.
    #[serde(default)]
    pub length: f64,
    /// Frames per second: the scrubber's ticks, and a step's size.
    #[serde(default = "twenty_four")]
    pub fps: f64,
    /// The time shown, in seconds from the start.
    #[serde(default)]
    pub time: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub playing: bool,
    /// Who moves `time` while playing.
    #[serde(default, skip_serializing_if = "TransportClock::is_default")]
    pub clock: TransportClock,
    /// Marks on the scrubber, such as "door" at 3.0 s, or a numbered note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
    /// Thin rows under the scrubber, its width, such as a sound's waveform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<TransportRow>,
    /// Sent as `command` when the play button is used, with `{"value": playing}`.
    pub play: crate::Action,
    /// Sent as `command` with `{"value": -1}` or `{"value": 1}`, a frame back or forward.
    pub step: crate::Action,
}

fn twenty_four() -> f64 {
    24.0
}

/// Who moves the time while the transport plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportClock {
    /// The host advances `time` at `fps` while playing, and reports where it
    /// stopped with `transport_time`, as a 3D view's animation needs.
    #[default]
    Host,
    /// The workspace moves `time`, and the host draws the time its latest
    /// declaration gives, so a video's frames follow its sound.
    Workspace,
}

impl TransportClock {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A thin row under the scrubber: its label, and an image the workspace
/// drew, the scrubber's width, such as a sound's waveform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportRow {
    #[serde(default)]
    pub label: String,
    pub image: Thumbnail,
}
