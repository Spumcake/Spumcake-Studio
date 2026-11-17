//! Finishing a round's take, as vg-retime-upscale and vg-before-after show
//! it: Retime… and Upscale… on a take's menu each make a new draft cell for
//! one finishing endpoint, its take the one reference, and ask the host to
//! run it, so the runner's plan card asks the person's yes with its cost;
//! the round it makes is labelled from its settings and its source; and
//! Compare with the source shows a finished take beside the take it was
//! made from. The runner sends the cell through whatever workflow skill
//! names its model; a cell names no skill.
//!
//! Every finishing endpoint, its settings, the label's rules, and the words
//! still pending the owner are here, so only this module changes when the
//! owner decides where finishing lives.

use protocol::{Cell, CellPart, Round};
use serde_json::{Value, json};

/// The endpoint that retimes a take, and the one that upscales it.
pub const RETIME_MODEL: &str = "topaz/interpolate/video";
pub const UPSCALE_MODEL: &str = "topaz/upscale/video/precision";

/// The frame rate a retime asks for, and the width an upscale reaches.
pub const TARGET_FPS: u32 = 60;
pub const TARGET_WIDTH: u32 = 3840;

/// A take's menu rows, the guide's Retime and Upscale, each opening a
/// request.
pub const RETIME: &str = "Retime…";
pub const UPSCALE: &str = "Upscale…";
/// A finished take's menu row, pending the owner's word.
pub const COMPARE: &str = "Compare with the source";
/// A finishing round's label, pending the owner's word: the take, the
/// parts its settings say, and the take it was made from.
pub const LABEL: &str = "Take {take}{parts}, from Take {source}";
/// The reference's role, and the model's input it fills.
const ROLE: &str = "Reference";
const FIELD: &str = "video_url";

/// Which finishing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Retime,
    Upscale,
}

impl Finish {
    pub fn model(self) -> &'static str {
        match self {
            Finish::Retime => RETIME_MODEL,
            Finish::Upscale => UPSCALE_MODEL,
        }
    }

    /// Its settings, as Last order's finishing requests sent them: a
    /// retime to 60 frames a second with Apollo; an upscale with Gaia CG
    /// by the factor that takes a take `width` wide to 3840.
    pub fn settings(self, width: u32) -> Value {
        match self {
            Finish::Retime => json!({ "target_fps": TARGET_FPS, "model": "Apollo", "slowdown_factor": 1, "H264_output": true }),
            Finish::Upscale => json!({
                "model": "Gaia CG",
                "upscale_factor": f64::from(TARGET_WIDTH) / f64::from(width.max(1)),
                "H264_output": true,
            }),
        }
    }
}

/// The finishing endpoints' models.
pub fn is_finishing(model: &str) -> bool {
    model == RETIME_MODEL || model == UPSCALE_MODEL
}

/// A new draft cell of kind video for `finish` on the take at `path`, of
/// `media_type`, `width` wide: no prompt, the take its one reference with
/// the role Reference filling `video_url`, and the request's settings.
pub fn draft(finish: Finish, path: &str, media_type: &str, width: u32, size: (u32, u32)) -> Cell {
    let mut cell = crate::gallery::empty_draft(size);
    cell.model = Some(finish.model().to_string());
    cell.settings = Some(finish.settings(width));
    cell.input = vec![CellPart::Path { path: path.to_string(), media_type: media_type.to_string(), role: Some(ROLE.into()), field: Some(FIELD.into()) }];
    cell
}

/// A finishing round's source: its one reference, when the round's model
/// is a finishing endpoint's.
pub fn source(round: &Round) -> Option<&str> {
    if !is_finishing(&round.model) || round.references.len() != 1 {
        return None;
    }
    Some(round.references[0].path.as_str())
}

/// What a finishing round's take reads as, "Take 2 · 60 fps · 4K, from
/// Take 1": the take's number from 1, `inherited`, the parts its source
/// already has as a finished take, then its own part from `settings`, the
/// cell's when it still has them, or else the request's: "· 60 fps" from
/// a retime's `target_fps`, "· 4K" when an upscale takes the source's
/// `width` to 3840 or more; and its source's number from 1.
pub fn label(round: &Round, take: usize, source_take: usize, inherited: &str, width: Option<u32>, settings: Option<&Value>) -> String {
    let parts = format!("{inherited}{}", own_part(round, width, settings));
    LABEL.replace("{take}", &(take + 1).to_string()).replace("{parts}", &parts).replace("{source}", &(source_take + 1).to_string())
}

/// The part a finishing round adds to its source's: " · 60 fps", " · 4K",
/// or none.
pub fn own_part(round: &Round, width: Option<u32>, settings: Option<&Value>) -> String {
    let number = |key: &str| settings.and_then(|settings| settings.get(key)).and_then(Value::as_f64);
    if round.model == RETIME_MODEL {
        let fps = number("target_fps").unwrap_or(f64::from(TARGET_FPS));
        return format!(" · {} fps", fps.round());
    }
    if round.model == UPSCALE_MODEL {
        let reached = match (width, number("upscale_factor")) {
            (Some(width), Some(factor)) => f64::from(width) * factor >= f64::from(TARGET_WIDTH) - 0.5,
            // An upscale asked here always reaches 3840.
            _ => true,
        };
        return if reached { String::from(" · 4K") } else { String::new() };
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(model: &str, references: &[&str]) -> Round {
        Round {
            request: "q2".into(),
            number: 2,
            model: model.into(),
            model_name: String::new(),
            prompt: String::new(),
            references: references.iter().map(|path| protocol::SentReference { path: path.to_string(), role: Some("Reference".into()) }).collect(),
            left_out: Vec::new(),
            state: protocol::RoundState::Done,
            started: None,
            count: None,
            takes: Vec::new(),
            error: None,
            report: None,
            artifact: None,
        }
    }

    /// Catches a finishing draft that would not send what Last order's
    /// requests sent: the model, the settings, no prompt, and the take as
    /// its one reference filling `video_url`; an upscale's factor takes a
    /// 1280-wide take to 3840.
    #[test]
    fn a_finishing_draft_is_what_last_order_sent() {
        let retime = draft(Finish::Retime, "takes/7.mp4", "video/mp4", 1280, (1280, 720));
        assert_eq!(retime.model.as_deref(), Some(RETIME_MODEL));
        assert_eq!(retime.kind, protocol::VIDEO_KIND);
        assert_eq!(retime.settings, Some(json!({ "target_fps": 60, "model": "Apollo", "slowdown_factor": 1, "H264_output": true })));
        assert_eq!(
            retime.input,
            [CellPart::Path { path: "takes/7.mp4".into(), media_type: "video/mp4".into(), role: Some("Reference".into()), field: Some("video_url".into()) }]
        );
        let upscale = draft(Finish::Upscale, "takes/7.mp4", "video/mp4", 1280, (1280, 720));
        assert_eq!(upscale.model.as_deref(), Some(UPSCALE_MODEL));
        assert_eq!(upscale.settings, Some(json!({ "model": "Gaia CG", "upscale_factor": 3.0, "H264_output": true })));
        assert_ne!(retime.id, upscale.id);
    }

    /// Catches a finishing round labelled other than from its settings and
    /// its source: a retime adds "60 fps", an upscale "4K" when it reaches
    /// 3840 and nothing when it does not, an upscale of a retimed take
    /// keeps the retime's part, and a round of another model has no
    /// source.
    #[test]
    fn a_finishing_round_is_labelled_from_its_settings_and_source() {
        let retime = round(RETIME_MODEL, &["takes/7.mp4"]);
        assert_eq!(source(&retime), Some("takes/7.mp4"));
        assert_eq!(label(&retime, 0, 6, "", Some(1280), Some(&Finish::Retime.settings(1280))), "Take 1 · 60 fps, from Take 7");
        let upscale = round(UPSCALE_MODEL, &["takes/7-60.mp4"]);
        assert_eq!(label(&upscale, 0, 0, " · 60 fps", Some(1344), Some(&json!({ "upscale_factor": 2.857142857 }))), "Take 1 · 60 fps · 4K, from Take 1");
        assert_eq!(label(&upscale, 0, 0, "", Some(1280), Some(&json!({ "upscale_factor": 2.0 }))), "Take 1, from Take 1");
        assert_eq!(source(&round("kling", &["takes/7.mp4"])), None);
        assert_eq!(source(&round(RETIME_MODEL, &["a.mp4", "b.mp4"])), None);
    }
}
