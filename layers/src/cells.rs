//! How a workspace's file keeps a cell: a pointer to its chain step and
//! nothing else, or a draft's own prompt, references, settings, model, and
//! endpoint before its first run, never a run, a result, or a record of
//! what made anything, as the Studio-as-a-setup contract has it.

use protocol::{Cell, CellState};

/// A cell as a file keeps it.
pub fn stored_cell(cell: &Cell) -> Cell {
    let mut kept = cell.clone();
    kept.versions.clear();
    kept.output.clear();
    kept.current = None;
    kept.error = None;
    kept.state = CellState::Idle;
    kept.effort = None;
    if kept.step.is_some() {
        kept.input.clear();
        kept.settings = None;
        kept.endpoint = None;
        kept.model = None;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a step's copy or a run kept in a file: a step's cell keeps
    /// its pointer alone, and a draft its prompt, with no result.
    #[test]
    fn a_file_keeps_a_pointer_or_a_draft() {
        let mut draft = Cell::text("d1", "A door opens.");
        draft.output = vec![protocol::CellPart::Text { text: "a result".into() }];
        draft.model = Some("a model".into());
        let kept = stored_cell(&draft);
        assert_eq!(kept.prompt(), "A door opens.");
        assert_eq!(kept.model.as_deref(), Some("a model"));
        assert!(kept.output.is_empty());
        let mut step = draft.clone();
        step.step = Some(protocol::StepRef { chain: "door".into(), step: "glow".into() });
        let kept = stored_cell(&step);
        assert!(kept.step.is_some() && !kept.copies_its_step());
    }
}
