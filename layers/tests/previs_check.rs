//! The pre-vis check against Spumcake-Skills' own method: `expected.png`
//! is what `build-animatic/scripts/overlay.py`'s lines make from the same
//! render and pre-vis (`fixtures/make_previs_check.py`); and a take over
//! its animatic, `expected-line-art.png` and `expected-over-animatic.png`,
//! from the same lines with one animatic frame
//! (`fixtures/make_take_over_animatic.py`).

use layers::{Rgba, edges};

fn load(name: &str) -> Rgba {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let image = image::open(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display())).to_rgba8();
    Rgba { width: image.width(), height: image.height(), data: image.into_raw() }
}

/// Catches a pre-vis check that only looks like overlay.py's: every pixel,
/// edges and the 85% mix toward cyan alike, matches the script's output.
#[test]
fn the_check_matches_overlay_py() {
    compare("previs.png", "expected.png");
}

/// Catches a grey conversion other than Pillow's: a coloured pre-vis's
/// edges are found where the script finds them.
#[test]
fn a_coloured_previs_matches_overlay_py() {
    compare("previs-colour.png", "expected-colour.png");
}

/// Catches line art that only looks like overlay.py's: every pixel darker
/// than 128 in grey is white, at the threshold too, and no other changes.
#[test]
fn the_line_art_matches_overlay_py() {
    let mut render = load("render.png");
    edges::line_art(&mut render, &load("animatic.png"), edges::WHITE);
    same(&render, "expected-line-art.png");
}

/// Catches a take over its animatic drawn in another order or way: the
/// animatic's edges in cyan, then its lines in white over them, match the
/// script's output pixel for pixel.
#[test]
fn a_take_over_its_animatic_matches_overlay_py() {
    let mut render = load("render.png");
    edges::over_animatic(&mut render, &load("animatic.png"));
    same(&render, "expected-over-animatic.png");
}

fn compare(previs: &str, expected: &str) {
    let mut render = load("render.png");
    edges::check(&mut render, &load(previs), edges::CYAN);
    same(&render, expected);
}

fn same(render: &Rgba, expected: &str) {
    let expected = load(expected);
    let mut differing = Vec::new();
    for y in 0..render.height {
        for x in 0..render.width {
            if render.pixel(x, y) != expected.pixel(x, y) {
                differing.push((x, y, render.pixel(x, y), expected.pixel(x, y)));
            }
        }
    }
    assert!(differing.is_empty(), "{} pixels differ, first {:?}", differing.len(), &differing[..differing.len().min(5)]);
}
