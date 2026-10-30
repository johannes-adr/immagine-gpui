//! Small vector icons drawn as paths (the bundled font has no chevron or heart glyphs).

use crate::components::glass::glass_div_faded;
use gpui_kit::*;

/// A "<" chevron, `color` stroked.
pub(super) fn chevron(color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * k), px(y + py_ * k));
            let mut b = PathBuilder::stroke(px(2.4));
            b.move_to(at(15., 4.));
            b.line_to(at(7., 12.));
            b.line_to(at(15., 20.));
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(22.))
}

/// A round glass back button with a chevron.
pub(super) fn back_chip(tint: Hsla, id: &'static str, color: Rgba) -> Stateful<Div> {
    back_chip_faded(tint, id, color, 1.)
}

/// `back_chip` shown at `k` (1 = full, 0 = invisible).
pub(super) fn back_chip_faded(tint: Hsla, id: &'static str, color: Rgba, k: f32) -> Stateful<Div> {
    let color = Rgba { a: color.a * k, ..color };
    glass_div_faded(tint, 18., k).id(id).h(px(36.)).w(px(48.)).flex().items_center().justify_center().active(|s| s.opacity(0.6)).child(chevron(color))
}

/// Glyphs of the photo viewer's bars (Apple Photos look), drawn as strokes on a 24-unit grid.
#[derive(Clone, Copy)]
pub(super) enum Glyph {
    Share,
    Info,
    Sliders,
    Trash,
    Search,
    Close,
    Play,
    Pause,
}

/// A `Glyph` of `size` px, stroked in `color`.
pub(super) fn glyph(kind: Glyph, size: f32, color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * k), px(y + py_ * k));
            let stroke = |pts: &[(f32, f32)], window: &mut Window| {
                let mut b = PathBuilder::stroke(px(1.9 * k));
                for (i, (a, c)) in pts.iter().enumerate() {
                    if i == 0 { b.move_to(at(*a, *c)) } else { b.line_to(at(*a, *c)) }
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            let fill = |pts: &[(f32, f32)], window: &mut Window| {
                let mut b = PathBuilder::fill();
                for (i, (a, c)) in pts.iter().enumerate() {
                    if i == 0 { b.move_to(at(*a, *c)) } else { b.line_to(at(*a, *c)) }
                }
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            let circle = |cx_: f32, cy_: f32, r: f32| -> Vec<(f32, f32)> {
                (0..=28).map(|i| { let a = i as f32 / 28. * std::f32::consts::TAU; (cx_ + r * a.cos(), cy_ + r * a.sin()) }).collect()
            };
            match kind {
                Glyph::Share => {
                    stroke(&[(8.5, 10.), (5.5, 10.), (5.5, 21.), (18.5, 21.), (18.5, 10.), (15.5, 10.)], window);
                    stroke(&[(12., 15.), (12., 2.5)], window);
                    stroke(&[(8., 6.5), (12., 2.5), (16., 6.5)], window);
                }
                Glyph::Info => {
                    stroke(&circle(12., 12., 9.5), window);
                    stroke(&[(12., 11.), (12., 17.)], window);
                    let mut b = PathBuilder::fill();
                    for (i, (a, c)) in circle(12., 7.4, 1.25).iter().enumerate() {
                        if i == 0 { b.move_to(at(*a, *c)) } else { b.line_to(at(*a, *c)) }
                    }
                    b.close();
                    if let Ok(path) = b.build() {
                        window.paint_path(path, color);
                    }
                }
                Glyph::Sliders => {
                    for (ly, kx) in [(6., 15.), (12., 8.), (18., 16.)] {
                        stroke(&[(3., ly), (kx - 2.6, ly)], window);
                        stroke(&[(kx + 2.6, ly), (21., ly)], window);
                        stroke(&circle(kx, ly, 2.4), window);
                    }
                }
                Glyph::Search => {
                    stroke(&circle(10.5, 10.5, 6.8), window);
                    stroke(&[(15.5, 15.5), (21., 21.)], window);
                }
                Glyph::Play => fill(&[(7., 4.), (20., 12.), (7., 20.)], window),
                Glyph::Pause => {
                    fill(&[(6., 4.), (10., 4.), (10., 20.), (6., 20.)], window);
                    fill(&[(14., 4.), (18., 4.), (18., 20.), (14., 20.)], window);
                }
                Glyph::Close => {
                    stroke(&[(5., 5.), (19., 19.)], window);
                    stroke(&[(19., 5.), (5., 19.)], window);
                }
                Glyph::Trash => {
                    stroke(&[(3.5, 6.), (20.5, 6.)], window);
                    stroke(&[(9., 6.), (9., 3.), (15., 3.), (15., 6.)], window);
                    stroke(&[(5.5, 6.), (6.8, 21.), (17.2, 21.), (18.5, 6.)], window);
                    stroke(&[(10., 10.), (10., 17.)], window);
                    stroke(&[(14., 10.), (14., 17.)], window);
                }
            }
        },
    )
    .size(px(size))
}
