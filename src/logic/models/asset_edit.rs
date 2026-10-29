//! Owner: A3. Swift source: `../immagine/Immagine/Models/AssetEdit.swift`.
//! Spec: `../immagine/ImmagineTests/` -> GeometryTests (edit transforms, server edits, wire format).
//!
//! Also holds the geometry primitives (`Point`, `Size`, `Rect`) shared with `person` (face boxes).
//! The crop-box math of the editor (`CropGeometry`) is in `crop_geometry`, not here.

use serde::{Deserialize, Deserializer, Serialize};

/// A point in normalized or pixel space (depending on the caller).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// Axis-aligned rectangle with origin `(x, y)` and size, like `CGRect`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
    /// The whole unit square.
    pub const UNIT: Rect = Rect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };

    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn min_x(&self) -> f64 {
        self.x.min(self.x + self.width)
    }

    pub fn max_x(&self) -> f64 {
        self.x.max(self.x + self.width)
    }

    pub fn min_y(&self) -> f64 {
        self.y.min(self.y + self.height)
    }

    pub fn max_y(&self) -> f64 {
        self.y.max(self.y + self.height)
    }

    pub fn mid_x(&self) -> f64 {
        (self.min_x() + self.max_x()) / 2.0
    }

    pub fn mid_y(&self) -> f64 {
        (self.min_y() + self.max_y()) / 2.0
    }

    /// Overlap of two rectangles; `ZERO` when they do not overlap.
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x0 = self.min_x().max(other.min_x());
        let x1 = self.max_x().min(other.max_x());
        let y0 = self.min_y().max(other.min_y());
        let y1 = self.max_y().min(other.max_y());
        if x1 < x0 || y1 < y0 {
            Rect::ZERO
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// The smallest whole-pixel rectangle that contains `self`.
    pub fn integral(&self) -> Rect {
        let x0 = self.min_x().floor();
        let y0 = self.min_y().floor();
        Rect::new(x0, y0, self.max_x().ceil() - x0, self.max_y().ceil() - y0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorAxis {
    Horizontal,
    Vertical,
}

/// One non-destructive server edit (`AssetEditActionItemDto`). The server
/// applies the crop first (in the orientation-corrected original's pixel
/// space), then mirror/rotate in list order; rotation angles are clockwise.
/// Unknown actions are not representable here; see `AssetEditsResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "WireEdit", into = "WireEdit")]
pub enum AssetEdit {
    Crop { x: i64, y: i64, width: i64, height: i64 },
    Rotate { angle: i64 },
    Mirror { axis: MirrorAxis },
}

/// The JSON shape `{"action": ..., "parameters": ...}`. Angles travel as
/// doubles, as the server sends them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", content = "parameters", rename_all = "lowercase")]
enum WireEdit {
    Crop { x: i64, y: i64, width: i64, height: i64 },
    Rotate { angle: f64 },
    Mirror { axis: MirrorAxis },
}

impl From<AssetEdit> for WireEdit {
    fn from(edit: AssetEdit) -> Self {
        match edit {
            AssetEdit::Crop { x, y, width, height } => WireEdit::Crop { x, y, width, height },
            AssetEdit::Rotate { angle } => WireEdit::Rotate { angle: angle as f64 },
            AssetEdit::Mirror { axis } => WireEdit::Mirror { axis },
        }
    }
}

impl From<WireEdit> for AssetEdit {
    fn from(edit: WireEdit) -> Self {
        match edit {
            WireEdit::Crop { x, y, width, height } => AssetEdit::Crop { x, y, width, height },
            WireEdit::Rotate { angle } => AssetEdit::Rotate { angle: angle.round() as i64 },
            WireEdit::Mirror { axis } => AssetEdit::Mirror { axis },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetEditsRequest {
    pub edits: Vec<AssetEdit>,
}

/// `AssetEditsResponseDto`; edits of unknown kinds are skipped, not fatal.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AssetEditsResponse {
    #[serde(default, deserialize_with = "lenient_edits")]
    pub edits: Vec<AssetEdit>,
}

fn lenient_edits<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<AssetEdit>, D::Error> {
    let raw = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .filter_map(|value| serde_json::from_value::<AssetEdit>(value).ok())
        .collect())
}

/// The orientation part of an edit, normalized to "mirror left-right first,
/// then rotate clockwise" — any server edit list reduces to this form.
/// Points are normalized image coordinates (0…1, y pointing down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditTransform {
    pub mirrored: bool,
    /// Clockwise degrees: 0, 90, 180 or 270. Set through `new`, which normalizes.
    pub rotation: i64,
}

/// 2×2 integer matrix acting on centered coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Matrix {
    a: i64,
    b: i64,
    c: i64,
    d: i64,
}

impl Matrix {
    const IDENTITY: Matrix = Matrix { a: 1, b: 0, c: 0, d: 1 };
    const ROTATE_CW: Matrix = Matrix { a: 0, b: -1, c: 1, d: 0 }; // (x, y) → (-y, x)
    const MIRROR_H: Matrix = Matrix { a: -1, b: 0, c: 0, d: 1 };
    const MIRROR_V: Matrix = Matrix { a: 1, b: 0, c: 0, d: -1 };

    /// `self · r`.
    fn times(self, r: Matrix) -> Matrix {
        Matrix {
            a: self.a * r.a + self.b * r.c,
            b: self.a * r.b + self.b * r.d,
            c: self.c * r.a + self.d * r.c,
            d: self.c * r.b + self.d * r.d,
        }
    }

    fn determinant(self) -> i64 {
        self.a * self.d - self.b * self.c
    }

    fn rotation_matrix(degrees: i64) -> Matrix {
        let turns = normalize_degrees(degrees) / 90;
        (0..turns).fold(Matrix::IDENTITY, |m, _| Matrix::ROTATE_CW.times(m))
    }

    fn map(self, p: Point) -> Point {
        let x = p.x - 0.5;
        let y = p.y - 0.5;
        Point::new(
            self.a as f64 * x + self.b as f64 * y + 0.5,
            self.c as f64 * x + self.d as f64 * y + 0.5,
        )
    }
}

fn normalize_degrees(degrees: i64) -> i64 {
    ((degrees % 360) + 360) % 360
}

fn bounds_of(r: Rect, f: impl Fn(Point) -> Point) -> Rect {
    let a = f(Point::new(r.min_x(), r.min_y()));
    let b = f(Point::new(r.max_x(), r.max_y()));
    Rect::new(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs())
}

impl EditTransform {
    pub const IDENTITY: EditTransform = EditTransform { mirrored: false, rotation: 0 };

    pub fn new(mirrored: bool, rotation: i64) -> Self {
        Self { mirrored, rotation: normalize_degrees(rotation) }
    }

    pub fn is_identity(&self) -> bool {
        !self.mirrored && self.rotation == 0
    }

    pub fn swaps_axes(&self) -> bool {
        self.rotation % 180 != 0
    }

    fn matrix(&self) -> Matrix {
        let mirror = if self.mirrored { Matrix::MIRROR_H } else { Matrix::IDENTITY };
        Matrix::rotation_matrix(self.rotation).times(mirror)
    }

    /// Reduces mirror/rotate edits (applied in list order) to the normal form.
    pub fn from_edits(edits: &[AssetEdit]) -> Self {
        let mut total = Matrix::IDENTITY;
        for edit in edits {
            match *edit {
                AssetEdit::Rotate { angle } => total = Matrix::rotation_matrix(angle).times(total),
                AssetEdit::Mirror { axis: MirrorAxis::Horizontal } => total = Matrix::MIRROR_H.times(total),
                AssetEdit::Mirror { axis: MirrorAxis::Vertical } => total = Matrix::MIRROR_V.times(total),
                AssetEdit::Crop { .. } => {}
            }
        }
        let mirrored = total.determinant() < 0;
        let rotation_part = if mirrored { total.times(Matrix::MIRROR_H) } else { total };
        let rotation = [0, 90, 180, 270]
            .into_iter()
            .find(|&degrees| Matrix::rotation_matrix(degrees) == rotation_part)
            .unwrap_or(0);
        Self::new(mirrored, rotation)
    }

    /// Photos' "Drehen": 90° counterclockwise, as seen on screen.
    pub fn rotated_counterclockwise(&self) -> Self {
        Self::new(self.mirrored, self.rotation - 90)
    }

    /// Photos' "Spiegeln": left-right, as seen on screen.
    pub fn mirrored_on_screen(&self) -> Self {
        // mirrorH · R(θ) = R(−θ) · mirrorH
        Self::new(!self.mirrored, -self.rotation)
    }

    /// Server edit actions for this transform (mirror before rotate).
    pub fn edits(&self) -> Vec<AssetEdit> {
        let mut result = Vec::new();
        if self.mirrored {
            result.push(AssetEdit::Mirror { axis: MirrorAxis::Horizontal });
        }
        if self.rotation != 0 {
            result.push(AssetEdit::Rotate { angle: self.rotation });
        }
        result
    }

    pub fn apply(&self, p: Point) -> Point {
        self.matrix().map(p)
    }

    pub fn invert(&self, p: Point) -> Point {
        // Inverse of R·M is M·R⁻¹ (M is its own inverse).
        let mirror = if self.mirrored { Matrix::MIRROR_H } else { Matrix::IDENTITY };
        mirror.times(Matrix::rotation_matrix(-self.rotation)).map(p)
    }

    pub fn apply_rect(&self, r: Rect) -> Rect {
        bounds_of(r, |p| self.apply(p))
    }

    pub fn invert_rect(&self, r: Rect) -> Rect {
        bounds_of(r, |p| self.invert(p))
    }
}

/// Complete editor state: a crop in the oriented original's normalized space
/// plus the orientation transform shown on top of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditState {
    pub crop: Rect,
    pub transform: EditTransform,
}

impl Default for EditState {
    fn default() -> Self {
        Self::new(Rect::UNIT, EditTransform::IDENTITY)
    }
}

fn is_full(r: Rect) -> bool {
    r.min_x() <= 0.0005 && r.min_y() <= 0.0005 && r.max_x() >= 0.9995 && r.max_y() >= 0.9995
}

impl EditState {
    pub fn new(crop: Rect, transform: EditTransform) -> Self {
        Self { crop, transform }
    }

    pub fn has_edits(&self) -> bool {
        !self.transform.is_identity() || !is_full(self.crop)
    }

    /// Rebuilds the state from server edits; `original_width`/`original_height`
    /// is the oriented original.
    pub fn from_edits(edits: &[AssetEdit], original_width: i64, original_height: i64) -> Self {
        let mut crop = Rect::UNIT;
        if original_width > 0 && original_height > 0 {
            let (w, h) = (original_width as f64, original_height as f64);
            for edit in edits {
                if let AssetEdit::Crop { x, y, width, height } = *edit {
                    crop = Rect::new(x as f64 / w, y as f64 / h, width as f64 / w, height as f64 / h);
                }
            }
        }
        Self::new(crop.intersection(&Rect::UNIT), EditTransform::from_edits(edits))
    }

    /// Server edit list: crop first (pixels of the oriented original), then
    /// mirror and rotate. Empty when nothing is edited.
    pub fn server_edits(&self, original_width: i64, original_height: i64) -> Vec<AssetEdit> {
        let (w, h) = (original_width, original_height);
        let mut result = Vec::new();
        if !is_full(self.crop) && w > 0 && h > 0 {
            let (wf, hf) = (w as f64, h as f64);
            let x = ((self.crop.min_x() * wf) as i64).clamp(0, w - 1);
            let y = ((self.crop.min_y() * hf) as i64).clamp(0, h - 1);
            let width = ((self.crop.width * wf).round() as i64).min(w - x).max(1);
            let height = ((self.crop.height * hf).round() as i64).min(h - y).max(1);
            if !(x == 0 && y == 0 && width == w && height == h) {
                result.push(AssetEdit::Crop { x, y, width, height });
            }
        }
        result.extend(self.transform.edits());
        result
    }

    /// Pixel size `(width, height)` of the rendered result (what the server reports afterwards).
    pub fn output_size(&self, original_width: i64, original_height: i64) -> (i64, i64) {
        let (mut width, mut height) = (original_width, original_height);
        if let Some(AssetEdit::Crop { width: cw, height: ch, .. }) =
            self.server_edits(original_width, original_height).first().copied()
        {
            width = cw;
            height = ch;
        }
        if self.transform.swaps_axes() { (height, width) } else { (width, height) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    fn assert_point(a: Point, b: Point) {
        close(a.x, b.x);
        close(a.y, b.y);
    }

    fn assert_rect(a: Rect, b: Rect, accuracy: f64) {
        assert!((a.min_x() - b.min_x()).abs() < accuracy, "minX {a:?} != {b:?}");
        assert!((a.min_y() - b.min_y()).abs() < accuracy, "minY {a:?} != {b:?}");
        assert!((a.width - b.width).abs() < accuracy, "width {a:?} != {b:?}");
        assert!((a.height - b.height).abs() < accuracy, "height {a:?} != {b:?}");
    }

    #[test]
    fn clockwise_rotation_maps_corners() {
        let t = EditTransform::new(false, 90);
        assert_point(t.apply(Point::new(0.0, 0.0)), Point::new(1.0, 0.0)); // top-left → top-right
        assert_point(t.apply(Point::new(1.0, 0.0)), Point::new(1.0, 1.0)); // top-right → bottom-right
        assert_point(t.apply(Point::new(0.25, 0.5)), Point::new(0.5, 0.25));
    }

    #[test]
    fn apply_invert_round_trip_for_all_transforms() {
        let p = Point::new(0.2, 0.7);
        let r = Rect::new(0.1, 0.2, 0.3, 0.5);
        for mirrored in [false, true] {
            for rotation in [0, 90, 180, 270] {
                let t = EditTransform::new(mirrored, rotation);
                assert_point(t.invert(t.apply(p)), p);
                assert_rect(t.invert_rect(t.apply_rect(r)), r, 1e-9);
            }
        }
    }

    #[test]
    fn normalizes_server_edit_lists() {
        assert_eq!(
            EditTransform::from_edits(&[AssetEdit::Rotate { angle: 90 }, AssetEdit::Rotate { angle: 180 }]),
            EditTransform::new(false, 270)
        );
        assert_eq!(EditTransform::from_edits(&[AssetEdit::Rotate { angle: 360 }]), EditTransform::IDENTITY);
        assert_eq!(
            EditTransform::from_edits(&[
                AssetEdit::Mirror { axis: MirrorAxis::Horizontal },
                AssetEdit::Mirror { axis: MirrorAxis::Horizontal },
            ]),
            EditTransform::IDENTITY
        );
        // A vertical mirror is a horizontal mirror turned by 180°.
        assert_eq!(
            EditTransform::from_edits(&[AssetEdit::Mirror { axis: MirrorAxis::Vertical }]),
            EditTransform::new(true, 180)
        );
        // Rotate-then-mirror equals mirror-then-rotate the other way.
        assert_eq!(
            EditTransform::from_edits(&[AssetEdit::Rotate { angle: 90 }, AssetEdit::Mirror { axis: MirrorAxis::Horizontal }]),
            EditTransform::new(true, 270)
        );
        assert_eq!(
            EditTransform::from_edits(&[
                AssetEdit::Crop { x: 0, y: 0, width: 5, height: 5 },
                AssetEdit::Rotate { angle: 90 },
            ]),
            EditTransform::new(false, 90)
        );
    }

    #[test]
    fn on_screen_buttons_compose() {
        // "Drehen" four times is a full turn.
        let mut t = EditTransform::IDENTITY;
        for _ in 0..4 {
            t = t.rotated_counterclockwise();
        }
        assert_eq!(t, EditTransform::IDENTITY);
        assert_eq!(EditTransform::IDENTITY.rotated_counterclockwise(), EditTransform::new(false, 270));
        // Mirroring on screen after a turn must equal turning the mirrored picture back.
        let turned = EditTransform::new(false, 90).mirrored_on_screen();
        let p = Point::new(0.1, 0.3);
        let base = EditTransform::new(false, 90).apply(p);
        assert_point(turned.apply(p), Point::new(1.0 - base.x, base.y));
        assert_eq!(EditTransform::new(true, 0).mirrored_on_screen(), EditTransform::IDENTITY);
    }

    #[test]
    fn server_edits_order_and_pixels() {
        let state = EditState::new(Rect::new(0.25, 0.1, 0.5, 0.5), EditTransform::new(true, 90));
        let edits = state.server_edits(4000, 3000);
        assert_eq!(
            edits,
            vec![
                AssetEdit::Crop { x: 1000, y: 300, width: 2000, height: 1500 },
                AssetEdit::Mirror { axis: MirrorAxis::Horizontal },
                AssetEdit::Rotate { angle: 90 },
            ],
            "crop must come first (server rule), then mirror, then rotate"
        );
        let (width, height) = state.output_size(4000, 3000);
        assert_eq!(width, 1500);
        assert_eq!(height, 2000);
    }

    #[test]
    fn full_crop_is_omitted_and_crops_stay_in_bounds() {
        assert_eq!(
            EditState::new(Rect::UNIT, EditTransform::new(false, 180)).server_edits(100, 50),
            vec![AssetEdit::Rotate { angle: 180 }]
        );
        assert!(EditState::default().server_edits(100, 50).is_empty());
        assert!(!EditState::default().has_edits());
        // Rounding at the far edge must never exceed the image (server rejects).
        let edge = EditState::new(Rect::new(0.3333, 0.6667, 0.6667, 0.3333), EditTransform::IDENTITY);
        match edge.server_edits(1001, 999).first() {
            Some(AssetEdit::Crop { x, y, width, height }) => {
                assert!(x + width <= 1001);
                assert!(y + height <= 999);
            }
            other => panic!("expected a crop, got {other:?}"),
        }
    }

    #[test]
    fn edit_state_round_trips_through_server_edits() {
        let original = EditState::new(Rect::new(0.1, 0.2, 0.6, 0.5), EditTransform::new(false, 270));
        let edits = original.server_edits(3000, 2000);
        let restored = EditState::from_edits(&edits, 3000, 2000);
        assert_rect(restored.crop, original.crop, 0.001);
        assert_eq!(restored.transform, original.transform);
    }

    #[test]
    fn edit_json_wire_format() {
        let value = serde_json::to_value(AssetEditsRequest {
            edits: vec![AssetEdit::Crop { x: 1, y: 2, width: 3, height: 4 }, AssetEdit::Rotate { angle: 90 }],
        })
        .unwrap();
        let edits = value["edits"].as_array().unwrap();
        assert_eq!(edits[0]["action"], "crop");
        assert_eq!(edits[0]["parameters"]["width"], 3);
        assert_eq!(edits[1]["action"], "rotate");
        assert_eq!(edits[1]["parameters"]["angle"].as_f64(), Some(90.0));

        let response = r#"{"assetId":"x","edits":[{"id":"1","action":"rotate","parameters":{"angle":270}},{"id":"2","action":"sparkle","parameters":{}},{"id":"3","action":"mirror","parameters":{"axis":"vertical"}}]}"#;
        let decoded: AssetEditsResponse = serde_json::from_str(response).unwrap();
        assert_eq!(
            decoded.edits,
            vec![AssetEdit::Rotate { angle: 270 }, AssetEdit::Mirror { axis: MirrorAxis::Vertical }],
            "unknown actions are skipped"
        );
    }
}
