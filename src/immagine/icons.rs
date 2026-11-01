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
    Gear,
    Download,
    Downloaded,
    Play,
    Pause,
    AddToAlbum,
    Check,
    /// The jobs list: arrow into a tray beside list lines.
    Jobs,
    /// Three dots in a row ("..." menu).
    More,
    /// A box with a lid (archive).
    Archive,
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
                Glyph::Gear => {
                    let mut pts = Vec::new();
                    for i in 0..8 {
                        let c = i as f32 * std::f32::consts::TAU / 8.;
                        for (da, r) in [(-0.2_f32, 7.6_f32), (-0.13, 10.), (0.13, 10.), (0.2, 7.6)] {
                            pts.push((12. + r * (c + da).cos(), 12. + r * (c + da).sin()));
                        }
                    }
                    pts.push(pts[0]);
                    stroke(&pts, window);
                    stroke(&circle(12., 12., 3.2), window);
                }
                Glyph::Download | Glyph::Downloaded => {
                    stroke(&[(4., 15.), (4., 20.5), (20., 20.5), (20., 15.)], window);
                    if matches!(kind, Glyph::Download) {
                        stroke(&[(12., 3.), (12., 15.)], window);
                        stroke(&[(7., 10.5), (12., 15.5), (17., 10.5)], window);
                    } else {
                        stroke(&[(6.5, 9.5), (10.8, 14.), (17.5, 4.5)], window);
                    }
                }
                Glyph::AddToAlbum => {
                    stroke(&[(6., 3.5), (18., 3.5)], window);
                    stroke(&[(3.5, 7.), (20.5, 7.), (20.5, 20.5), (3.5, 20.5), (3.5, 7.)], window);
                    stroke(&[(12., 10.5), (12., 17.)], window);
                    stroke(&[(8.7, 13.75), (15.3, 13.75)], window);
                }
                Glyph::Jobs => {
                    for y in [5., 12., 19.] {
                        stroke(&[(3., y), (4.2, y)], window);
                    }
                    stroke(&[(8., 5.), (21., 5.)], window);
                    stroke(&[(8., 12.), (13., 12.)], window);
                    stroke(&[(8., 19.), (13., 19.)], window);
                    stroke(&[(18., 9.5), (18., 20.)], window);
                    stroke(&[(15., 17.), (18., 20.), (21., 17.)], window);
                }
                Glyph::More => {
                    for cx_ in [5., 12., 19.] {
                        fill(&circle(cx_, 12., 1.9), window);
                    }
                }
                Glyph::Archive => {
                    stroke(&[(3., 4.5), (21., 4.5), (21., 9.), (3., 9.), (3., 4.5)], window);
                    stroke(&[(4.5, 9.), (4.5, 20.), (19.5, 20.), (19.5, 9.)], window);
                    stroke(&[(9.5, 13.5), (14.5, 13.5)], window);
                }
                Glyph::Check => stroke(&[(5., 12.5), (10., 17.5), (19., 7.)], window),
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

/// A small filled heart (grid badge).
pub(super) fn heart_badge(size: f32, color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * k), px(y + py_ * k));
            let mut b = PathBuilder::fill();
            b.move_to(at(12., 21.5));
            b.curve_to(at(2.2, 8.2), at(2., 14.5));
            b.curve_to(at(7.2, 3.), at(2.5, 3.));
            b.curve_to(at(12., 6.8), at(10.8, 3.));
            b.curve_to(at(16.8, 3.), at(13.2, 3.));
            b.curve_to(at(21.8, 8.2), at(21.5, 3.));
            b.curve_to(at(12., 21.5), at(22., 14.5));
            b.close();
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(size))
}

/// A progress ring: a faint track and an arc from the top, clockwise, for `fraction` (0..1) of the circle.
pub(super) fn progress_ring(fraction: f32, size: f32, color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let r = size / 2. - 2.;
            let (cx_, cy_) = (x + size / 2., y + size / 2.);
            let arc = |from: f32, sweep: f32, color: Rgba, window: &mut Window| {
                let steps = ((sweep / std::f32::consts::TAU) * 48.).ceil().max(2.) as usize;
                let mut b = PathBuilder::stroke(px(2.4));
                for i in 0..=steps {
                    let a = from + sweep * i as f32 / steps as f32;
                    let p = point(px(cx_ + r * a.cos()), px(cy_ + r * a.sin()));
                    if i == 0 { b.move_to(p) } else { b.line_to(p) }
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            let top = -std::f32::consts::FRAC_PI_2;
            arc(top, std::f32::consts::TAU, Rgba { a: color.a * 0.3, ..color }, window);
            arc(top, std::f32::consts::TAU * fraction.clamp(0.04, 1.), color, window);
        },
    )
    .size(px(size))
}
