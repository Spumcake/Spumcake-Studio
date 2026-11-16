//! The pre-vis check over the image shown: the pre-vis render's edges in
//! cyan, or the render blended see-through at an opacity, as
//! `layers::edges` draws them. It is drawn into the view the host reads,
//! never into the image's layers, its file's pixels, or an export.

use crate::layered::Rect;
use layers::Rgba;
use layers::link::Link;

/// What a person chose for an image's check, kept in its file: the
/// render, named as a link names its file, Edges or See-through, the
/// see-through opacity from 0 to 1, and whether the check is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub render: Option<Link>,
    pub see_through: bool,
    pub opacity: f32,
    pub on: bool,
}

impl Default for Settings {
    /// Edges, and 80% see-through, as ig-previs-overlay-settings shows.
    fn default() -> Self {
        Self { render: None, see_through: false, opacity: 0.8, on: false }
    }
}

/// An image's check: its settings, the render as read from its file, and
/// the render fitted to the image with its edges, made once for each size.
#[derive(Debug, Clone, Default)]
pub struct Previs {
    pub settings: Settings,
    /// None when no render is chosen, or its file could not be read.
    pub image: Option<Rgba>,
    fitted: Option<(Rgba, Vec<bool>)>,
}

impl Previs {
    pub fn new(settings: Settings, image: Option<Rgba>) -> Self {
        Self { settings, image, fitted: None }
    }

    /// A render chosen: the check draws it from now.
    pub fn set_render(&mut self, link: Link, image: Rgba) {
        self.settings.render = Some(link);
        self.image = Some(image);
        self.fitted = None;
    }

    /// The check is on and has a render to draw.
    pub fn showing(&self) -> bool {
        self.settings.on && self.image.is_some()
    }

    /// Draws the check over `rows`, the part `rect` of a `width` × `height`
    /// image, as `layers::edges::check` or `see_through` draws it over the
    /// whole image.
    pub fn draw(&mut self, width: u32, height: u32, rect: Rect, rows: &mut Vec<u8>) {
        if !self.settings.on {
            return;
        }
        let Some(image) = &self.image else {
            return;
        };
        if self.fitted.as_ref().is_none_or(|(fitted, _)| (fitted.width, fitted.height) != (width, height)) {
            let fitted = layers::layout::fit_exactly(image, width, height);
            let edges = layers::edges::find(&layers::edges::grey(&fitted), width, height);
            self.fitted = Some((fitted, edges));
        }
        let Some((fitted, edges)) = &self.fitted else {
            return;
        };
        let (w, h) = (rect.w.max(0) as usize, rect.h.max(0) as usize);
        let mut part = Rgba { width: w as u32, height: h as u32, data: std::mem::take(rows) };
        let row = |y: usize| (rect.y as usize + y) * width as usize + rect.x as usize;
        if self.settings.see_through {
            let mut over = Rgba { width: w as u32, height: h as u32, data: Vec::with_capacity(w * h * 4) };
            for y in 0..h {
                over.data.extend_from_slice(&fitted.data[row(y) * 4..(row(y) + w) * 4]);
            }
            layers::edges::see_through(&mut part, &over, self.settings.opacity);
        } else {
            let mut found = Vec::with_capacity(w * h);
            for y in 0..h {
                found.extend_from_slice(&edges[row(y)..row(y) + w]);
            }
            layers::edges::draw(&mut part, &found, layers::edges::CYAN, layers::edges::STRENGTH);
        }
        *rows = part.data;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A render with a light square on dark, the image's size or not.
    fn render(width: u32, height: u32) -> Rgba {
        let mut image = Rgba::new(width, height, [20, 20, 20, 255]);
        for y in height / 4..height * 3 / 4 {
            for x in width / 4..width * 3 / 4 {
                image.set(x, y, [230, 230, 230, 255]);
            }
        }
        image
    }

    /// Catches a check drawn over part of the image that differs from the
    /// whole image's: Edges and See-through over a band of rows are those
    /// rows of `layers::edges` over the whole image.
    #[test]
    fn a_part_is_drawn_as_the_whole_is() {
        let (width, height) = (64, 48);
        let image = Rgba::new(width, height, [90, 140, 200, 255]);
        for see_through in [false, true] {
            let mut previs = Previs::new(Settings { see_through, opacity: 0.4, on: true, ..Settings::default() }, Some(render(32, 24)));
            let mut whole = image.clone();
            if see_through {
                layers::edges::see_through(&mut whole, &render(32, 24), 0.4);
            } else {
                layers::edges::check(&mut whole, &render(32, 24), layers::edges::CYAN);
            }
            let rect = Rect::new(5, 10, 40, 20);
            let mut rows = Vec::new();
            for y in rect.y..rect.bottom() {
                let start = ((y * width as i32 + rect.x) * 4) as usize;
                rows.extend_from_slice(&image.data[start..start + rect.w as usize * 4]);
            }
            previs.draw(width, height, rect, &mut rows);
            for y in 0..rect.h as usize {
                let start = (((rect.y as usize + y) * width as usize) + rect.x as usize) * 4;
                assert_eq!(&rows[y * rect.w as usize * 4..(y + 1) * rect.w as usize * 4], &whole.data[start..start + rect.w as usize * 4], "row {y}");
            }
            assert!(whole != image, "the check drew nothing");
        }
    }
}
