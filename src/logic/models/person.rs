//! Owner: A3. Swift source: `../immagine/Immagine/Models/Person.swift`, plus the pure parts of
//! `UI/People/PeopleComponents.swift` (`PeopleOrder`) and `UI/People/FaceEditorView.swift` /
//! `PersonCoverPicker.swift` (`FaceCrop`).
//! Spec: `../immagine/ImmagineTests/` -> PeopleTests.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::asset_edit::{Rect, Size};

/// Missing or `null` JSON values become `T::default()`, as Swift's `decodeIfPresent ?? default`.
fn null_as_default<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A person of Immich's face recognition (`PersonResponseDto`). Unnamed
/// people have an empty name; the server lists only people that still have
/// faces.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub name: String,
    /// "YYYY-MM-DD".
    #[serde(default)]
    pub birth_date: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub is_hidden: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub is_favorite: bool,
    /// Changes when the face thumbnail is regenerated (e.g. a new cover).
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Person {
    /// A visible, not favorite person without birthday.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            birth_date: None,
            is_hidden: false,
            is_favorite: false,
            updated_at: None,
        }
    }

    pub fn has_name(&self) -> bool {
        !self.name.trim().is_empty()
    }

    /// "Anna", or the placeholder for an unnamed person.
    pub fn display_name(&self) -> &str {
        if self.has_name() { &self.name } else { "Unbenannt" }
    }

    pub fn birthday(&self) -> Option<CivilDate> {
        self.birth_date.as_deref().and_then(PersonDate::parse)
    }

    /// Age in whole years on the civil day `on`; `None` without a valid birthday or before it.
    pub fn age_on(&self, on: CivilDate) -> Option<i32> {
        let birthday = self.birthday()?;
        if birthday > on {
            return None;
        }
        let mut years = on.year - birthday.year;
        if (on.month, on.day) < (birthday.month, birthday.day) {
            years -= 1;
        }
        Some(years)
    }

    /// Key of the cached face thumbnail: a new cover face gives a new key.
    pub fn thumbnail_key(&self) -> String {
        format!("{}-{}", self.id, self.updated_at.as_deref().unwrap_or(""))
    }
}

/// A day of the calendar without time zone. Field order makes the derived
/// ordering chronological.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// "YYYY-MM-DD" as Immich stores birth dates (no time zone).
pub struct PersonDate;

const GERMAN_MONTHS: [&str; 12] = [
    "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November",
    "Dezember",
];

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl PersonDate {
    /// Reads the first ten characters as "YYYY-MM-DD"; `None` for anything else.
    pub fn parse(text: &str) -> Option<CivilDate> {
        let head: String = text.chars().take(10).collect();
        let parts: Vec<i32> = head
            .split('-')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect();
        let [year, month, day] = parts.as_slice() else {
            return None;
        };
        let (year, month, day) = (*year, *month, *day);
        if !(1..=12).contains(&month) || day < 1 || day as u32 > days_in_month(year, month as u32) {
            return None;
        }
        Some(CivilDate { year, month: month as u32, day: day as u32 })
    }

    /// "YYYY-MM-DD" with zero padding, the form Immich expects.
    pub fn format(date: CivilDate) -> String {
        format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
    }

    /// "17. Mai 1990"; the input unchanged when it is not a date.
    pub fn display(text: &str) -> String {
        match Self::parse(text) {
            Some(date) => format!("{}. {} {}", date.day, GERMAN_MONTHS[(date.month - 1) as usize], date.year),
            None => text.to_string(),
        }
    }
}

/// A detected (or manually marked) face on a photo (`AssetFaceResponseDto`).
/// The box is in pixels of `imageWidth` × `imageHeight` — the photo as the
/// server renders it, edits included.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFace {
    pub id: String,
    pub image_width: i64,
    pub image_height: i64,
    pub bounding_box_x1: i64,
    pub bounding_box_y1: i64,
    pub bounding_box_x2: i64,
    pub bounding_box_y2: i64,
    #[serde(default)]
    pub person: Option<Person>,
    #[serde(default)]
    pub source_type: Option<String>,
}

impl AssetFace {
    /// The box in unit coordinates (0…1) of the photo, clamped; corners in any order.
    pub fn normalized_box(&self) -> Rect {
        if self.image_width <= 0 || self.image_height <= 0 {
            return Rect::ZERO;
        }
        let w = self.image_width as f64;
        let h = self.image_height as f64;
        let clamp = |v: f64| v.clamp(0.0, 1.0);
        let (x_lo, x_hi) = (self.bounding_box_x1.min(self.bounding_box_x2), self.bounding_box_x1.max(self.bounding_box_x2));
        let (y_lo, y_hi) = (self.bounding_box_y1.min(self.bounding_box_y2), self.bounding_box_y1.max(self.bounding_box_y2));
        let x1 = clamp(x_lo as f64 / w);
        let x2 = clamp(x_hi as f64 / w);
        let y1 = clamp(y_lo as f64 / h);
        let y2 = clamp(y_hi as f64 / h);
        Rect::new(x1, y1, x2 - x1, y2 - y1)
    }
}

/// Queries on a photo's faces.
pub trait AssetFaces {
    /// The people on a photo, each once, in the order of their faces.
    fn people(&self) -> Vec<Person>;
    /// Faces that no person is assigned to yet.
    fn unassigned_count(&self) -> usize;
}

impl AssetFaces for [AssetFace] {
    fn people(&self) -> Vec<Person> {
        let mut seen = HashSet::new();
        self.iter()
            .filter_map(|face| face.person.as_ref())
            .filter(|person| seen.insert(person.id.clone()))
            .cloned()
            .collect()
    }

    fn unassigned_count(&self) -> usize {
        self.iter().filter(|face| face.person.is_none()).count()
    }
}

/// Birth date change: set to a "YYYY-MM-DD" string, or cleared (sent as `null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BirthDateUpdate {
    Set(String),
    Clear,
}

impl Serialize for BirthDateUpdate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            BirthDateUpdate::Set(value) => serializer.serialize_str(value),
            BirthDateUpdate::Clear => serializer.serialize_none(),
        }
    }
}

/// Partial update of a person (`PersonUpdateDto`): only the set fields are
/// sent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<BirthDateUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// The asset whose face of this person becomes the cover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_face_asset_id: Option<String>,
}

/// Geometry of face crops and the cover grid (`FaceCrop` in Swift).
pub struct FaceCrop;

impl FaceCrop {
    /// Where an image of `size` lands when fitted into `boxed`, centered.
    pub fn fitted(size: Size, boxed: Size) -> Rect {
        if size.width <= 0.0 || size.height <= 0.0 || boxed.width <= 0.0 || boxed.height <= 0.0 {
            return Rect::ZERO;
        }
        let scale = (boxed.width / size.width).min(boxed.height / size.height);
        let (width, height) = (size.width * scale, size.height * scale);
        Rect::new((boxed.width - width) / 2.0, (boxed.height - height) / 2.0, width, height)
    }

    /// A square around the box (box size + 40 %), kept inside the photo, in pixels.
    pub fn pixel_rect(bx: Rect, width: i64, height: i64) -> Option<Rect> {
        if width <= 0 || height <= 0 || bx.width <= 0.0 || bx.height <= 0.0 {
            return None;
        }
        let (w, h) = (width as f64, height as f64);
        let side = ((bx.width * w).max(bx.height * h) * 1.4).min(w.min(h));
        let x = (bx.mid_x() * w - side / 2.0).max(0.0).min(w - side);
        let y = (bx.mid_y() * h - side / 2.0).max(0.0).min(h - side);
        Some(Rect::new(x, y, side, side).integral())
    }

    /// The part of a photo (unit coordinates) a square cell shows to put the
    /// face in its middle: twice the face, but never less than 40 % of the
    /// photo's short side. `aspect` is the photo's width / height.
    pub fn contents_rect(bx: Rect, aspect: f64) -> Rect {
        if bx.width <= 0.0 || bx.height <= 0.0 || aspect <= 0.0 {
            return Rect::UNIT;
        }
        // In units where the photo is `aspect` wide and 1 high.
        let short_side = aspect.min(1.0);
        let mut side = (bx.width * aspect).max(bx.height) * 2.0;
        side = side.max(short_side * 0.4).min(short_side);
        let x = (bx.mid_x() * aspect - side / 2.0).max(0.0).min(aspect - side);
        let y = (bx.mid_y() - side / 2.0).max(0.0).min(1.0 - side);
        Rect::new(x / aspect, y, side / aspect, side)
    }
}

/// Order and filtering of people lists (`PeopleOrder` in Swift).
pub struct PeopleOrder;

impl PeopleOrder {
    /// Named people alphabetically (numbers naturally), then the unnamed (server order).
    pub fn for_picking(people: &[Person]) -> Vec<Person> {
        let mut named: Vec<Person> = people.iter().filter(|p| p.has_name()).cloned().collect();
        named.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        named.extend(people.iter().filter(|p| !p.has_name()).cloned());
        named
    }

    /// The overview: what a case-insensitive search matches, hidden people only on request.
    pub fn overview(people: &[Person], query: &str, show_hidden: bool) -> Vec<Person> {
        let search = query.trim().to_lowercase();
        people
            .iter()
            .filter(|p| (show_hidden || !p.is_hidden) && (search.is_empty() || p.name.to_lowercase().contains(&search)))
            .cloned()
            .collect()
    }
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Digits without leading zeros.
fn significant_digits(run: &[char]) -> &[char] {
    let first = run.iter().position(|c| *c != '0').unwrap_or(run.len());
    &run[first..]
}

/// Case-insensitive comparison in which digit runs compare by value ("Anna 2" < "Anna 10").
/// An approximation of `localizedStandardCompare`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (na, nb) = (significant_digits(&a[start_a..i]), significant_digits(&b[start_b..j]));
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let (ca, cb) = (fold(a[i]), fold(b[j]));
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn person_decodes_server_payload() {
        let json = r##"{"id":"p1","name":"","birthDate":null,"thumbnailPath":"/data/thumbs/p1.jpeg","isHidden":false,
         "isFavorite":true,"updatedAt":"2026-10-26T13:26:55.567Z","color":"#ff0000"}"##;
        let person: Person = serde_json::from_str(json).unwrap();
        assert_eq!(person.id, "p1");
        assert!(!person.has_name());
        assert_eq!(person.display_name(), "Unbenannt");
        assert!(person.is_favorite);
        assert_eq!(person.birth_date, None);
        assert_eq!(person.thumbnail_key(), "p1-2026-10-26T13:26:55.567Z", "a new cover changes the key");
    }

    #[test]
    fn faces_decode_with_and_without_person() {
        let json = r#"[{"id":"f1","imageWidth":1600,"imageHeight":1200,"boundingBoxX1":177,"boundingBoxY1":254,
          "boundingBoxX2":346,"boundingBoxY2":490,"person":null,"sourceType":"machine-learning"},
         {"id":"f2","imageWidth":1600,"imageHeight":1200,"boundingBoxX1":1131,"boundingBoxY1":393,
          "boundingBoxX2":1331,"boundingBoxY2":663,
          "person":{"id":"p1","name":"Anna","birthDate":"1990-05-17","thumbnailPath":"","isHidden":false}},
         {"id":"f3","imageWidth":1600,"imageHeight":1200,"boundingBoxX1":10,"boundingBoxY1":10,
          "boundingBoxX2":20,"boundingBoxY2":20,
          "person":{"id":"p1","name":"Anna","birthDate":"1990-05-17","thumbnailPath":"","isHidden":false}}]"#;
        let faces: Vec<AssetFace> = serde_json::from_str(json).unwrap();
        assert_eq!(faces.len(), 3);
        let people = faces.people();
        assert_eq!(people.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["p1"], "each person once");
        assert_eq!(faces.unassigned_count(), 1);
    }

    #[test]
    fn face_box_is_normalized_and_clamped() {
        let face = AssetFace {
            id: "f".into(),
            image_width: 1000,
            image_height: 500,
            bounding_box_x1: 600,
            bounding_box_y1: 100,
            bounding_box_x2: 200,
            bounding_box_y2: 600,
            person: None,
            source_type: None,
        };
        let bx = face.normalized_box();
        close(bx.min_x(), 0.2);
        close(bx.max_x(), 0.6);
        close(bx.min_y(), 0.2);
        close(bx.max_y(), 1.0);
        let empty = AssetFace { id: "e".into(), image_width: 0, image_height: 0, bounding_box_x1: 1, bounding_box_y1: 1, bounding_box_x2: 2, bounding_box_y2: 2, person: None, source_type: None };
        assert_eq!(empty.normalized_box(), Rect::ZERO);
    }

    #[test]
    fn face_crop_is_a_square_inside_the_photo() {
        // A face in the top left corner: the square is pushed into the photo.
        let rect = FaceCrop::pixel_rect(Rect::new(0.0, 0.0, 0.1, 0.1), 1000, 800).expect("a crop");
        assert_eq!(rect.width, rect.height);
        close(rect.width, 140.0); // box (100 px) plus 40 %, ±1 px after rounding
        assert!((rect.width - 140.0).abs() <= 1.0);
        assert_eq!(rect.min_x(), 0.0);
        assert_eq!(rect.min_y(), 0.0);
        let centered = FaceCrop::pixel_rect(Rect::new(0.4, 0.4, 0.2, 0.2), 1000, 1000).expect("a crop");
        assert!((centered.mid_x() - 500.0).abs() <= 1.0);
        assert!((centered.mid_y() - 500.0).abs() <= 1.0);
        assert_eq!(FaceCrop::pixel_rect(Rect::ZERO, 1000, 1000), None);
    }

    #[test]
    fn boxes_follow_the_fitted_photo() {
        // A portrait photo in a wide frame sits centered with bars left and right.
        let photo = FaceCrop::fitted(Size::new(1200.0, 1600.0), Size::new(400.0, 280.0));
        close(photo.height, 280.0);
        close(photo.width, 210.0);
        close(photo.min_x(), 95.0);
        close(photo.min_y(), 0.0);
    }

    #[test]
    fn person_update_sends_only_what_changes() {
        let json = |update: &PersonUpdate| serde_json::to_value(update).unwrap();
        assert_eq!(json(&PersonUpdate { name: Some("Anna".into()), ..Default::default() }), serde_json::json!({"name": "Anna"}));
        assert_eq!(
            json(&PersonUpdate { birth_date: Some(BirthDateUpdate::Set("1990-05-17".into())), ..Default::default() }),
            serde_json::json!({"birthDate": "1990-05-17"})
        );
        let cleared = json(&PersonUpdate { birth_date: Some(BirthDateUpdate::Clear), ..Default::default() });
        assert!(cleared["birthDate"].is_null(), "null removes the birthday");
        assert_eq!(
            json(&PersonUpdate { is_hidden: Some(true), feature_face_asset_id: Some("a1".into()), ..Default::default() }),
            serde_json::json!({"isHidden": true, "featureFaceAssetId": "a1"})
        );
    }

    #[test]
    fn birth_dates_and_age() {
        let person = Person { birth_date: Some("1990-05-17".into()), ..Person::new("p", "Anna") };
        let day_before = CivilDate { year: 2026, month: 5, day: 16 };
        let birthday = CivilDate { year: 2026, month: 5, day: 17 };
        assert_eq!(person.age_on(day_before), Some(35));
        assert_eq!(person.age_on(birthday), Some(36));
        assert_eq!(PersonDate::display("1990-05-17"), "17. Mai 1990");
        assert_eq!(PersonDate::format(CivilDate { year: 2001, month: 2, day: 3 }), "2001-02-03");
        let broken = Person { birth_date: Some("kaputt".into()), ..Person::new("q", "") };
        assert_eq!(broken.age_on(birthday), None);
    }

    #[test]
    fn people_order_for_picking_and_overview() {
        let mut berta = Person::new("4", "Berta");
        berta.is_hidden = true;
        let people = vec![
            Person::new("1", ""),
            Person::new("2", "Zoe"),
            Person::new("3", "anna"),
            berta,
            Person::new("5", "Anna 10"),
            Person::new("6", "Anna 2"),
        ];
        let ids = |list: Vec<Person>| list.into_iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(
            ids(PeopleOrder::for_picking(&people)),
            vec!["3", "6", "5", "4", "2", "1"],
            "named alphabetically (numbers naturally), unnamed last"
        );
        assert_eq!(ids(PeopleOrder::overview(&people, "", false)), vec!["1", "2", "3", "5", "6"]);
        assert_eq!(ids(PeopleOrder::overview(&people, "ber", true)), vec!["4"]);
        assert_eq!(ids(PeopleOrder::overview(&people, "  anna ", false)), vec!["3", "5", "6"]);
    }

    /// "Titelbild ändern": every cell shows a square around the person's face.
    #[test]
    fn cover_grid_zooms_onto_the_face() {
        let landscape = FaceCrop::contents_rect(Rect::new(0.45, 0.4, 0.1, 0.15), 1.5);
        close(landscape.width * 1.5, landscape.height); // square in pixels
        close(landscape.height, 0.4); // not tighter than 40 % of the short side
        close(landscape.mid_x(), 0.5); // the face in the middle
        close(landscape.mid_y(), 0.475);

        let at_the_edge = FaceCrop::contents_rect(Rect::new(0.9, 0.0, 0.1, 0.05), 0.75);
        assert!(at_the_edge.min_x() >= 0.0);
        assert!(at_the_edge.max_x() <= 1.0 + 1e-9);
        assert!(at_the_edge.min_y() >= 0.0);

        let big_face = FaceCrop::contents_rect(Rect::new(0.1, 0.1, 0.8, 0.8), 1.0);
        assert_eq!(big_face, Rect::UNIT, "capped at the whole short side");
        assert_eq!(FaceCrop::contents_rect(Rect::ZERO, 1.0), Rect::UNIT);
    }

    #[test]
    fn natural_order_is_case_insensitive_and_numeric() {
        assert_eq!(natural_cmp("anna", "Anna 2"), Ordering::Less);
        assert_eq!(natural_cmp("Anna 2", "Anna 10"), Ordering::Less);
        assert_eq!(natural_cmp("Anna 10", "Berta"), Ordering::Less);
        assert_eq!(natural_cmp("Anna", "anna"), Ordering::Equal);
    }
}
