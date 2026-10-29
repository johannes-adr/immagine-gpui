//! Owner: A3. Swift source: `../immagine/Immagine/Models/AssetDetails.swift` (incl. `InfoFormat`, `PlaceName`),
//! and `ExifInfo` from `../immagine/Immagine/Models/Asset.swift`.
//! Spec: `../immagine/ImmagineTests/` -> InfoPanelTests.
//!
//! `ExifInfo` is defined here because `asset.rs` (A2) does not exist yet. When `asset.rs` lands, move it
//! there and re-export it from here instead of keeping two copies. `isRotated90`/`orientedSize` of the Swift
//! `ExifInfo` are not ported here for the same reason.
//!
//! Dates are plain strings (ISO 8601 as the server sends them). Formatting takes civil date parts instead of
//! `Date`, so the caller converts to local time first.

use serde::{Deserialize, Serialize};

pub use super::asset::ExifInfo;

impl ExifInfo {
    /// Nothing known yet (a place set before the details arrived).
    pub fn empty() -> Self {
        Self::default()
    }

    /// A copy with the place fields replaced.
    pub fn with(
        &self,
        latitude: Option<f64>,
        longitude: Option<f64>,
        city: Option<String>,
        state: Option<String>,
        country: Option<String>,
    ) -> Self {
        Self { latitude, longitude, city, state, country, ..self.clone() }
    }
}

/// What the info panel shows beyond the local columns: camera, lens,
/// exposure, place and size — Immich's `exifInfo` of one asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDetails {
    pub id: String,
    pub exif: ExifInfo,
    /// When the server was asked (ISO 8601); older details are refreshed in the background.
    pub fetched_at: String,
}

/// A WGS84 coordinate (`CLLocationCoordinate2D`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl AssetDetails {
    /// The place of the asset; `None` for missing, out-of-range and the 0/0 "no place" value.
    pub fn coordinate(&self) -> Option<Coordinate> {
        let (lat, lon) = (self.exif.latitude?, self.exif.longitude?);
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || (lat == 0.0 && lon == 0.0) {
            return None;
        }
        Some(Coordinate { latitude: lat, longitude: lon })
    }

    pub fn place_name(&self) -> Option<String> {
        PlaceName::format(self.exif.city.as_deref(), self.exif.state.as_deref(), self.exif.country.as_deref())
    }

    /// A new location set in the app (`PUT /assets/{id}`), with the names of
    /// the chosen place until the server has geocoded it itself.
    pub fn moved_to(
        &self,
        coordinate: Coordinate,
        city: Option<String>,
        state: Option<String>,
        country: Option<String>,
    ) -> Self {
        let exif = self.exif.with(Some(coordinate.latitude), Some(coordinate.longitude), city, state, country);
        Self { exif, ..self.clone() }
    }
}

const GERMAN_WEEKDAYS: [&str; 7] = ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"];
const GERMAN_MONTHS: [&str; 12] = [
    "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November",
    "Dezember",
];

/// The info panel's lines, worded and formatted like Photos (German).
pub struct InfoFormat;

impl InfoFormat {
    /// "Samstag, 24. Oktober 2026 um 16:42". The arguments are the local civil date and time.
    pub fn date_line(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> String {
        let weekday = GERMAN_WEEKDAYS[weekday_index(year, month, day)];
        let month_name = GERMAN_MONTHS[(month - 1) as usize];
        format!("{weekday}, {day}. {month_name} {year} um {hour:02}:{minute:02}")
    }

    /// The file name without its extension, as Photos shows it.
    pub fn title(file_name: Option<&str>) -> Option<String> {
        let name = file_name.filter(|n| !n.is_empty())?;
        let base = match name.rfind('.') {
            Some(index) if index > 0 => &name[..index],
            _ => name,
        };
        Some(if base.is_empty() { name.to_string() } else { base.to_string() })
    }

    /// "Apple iPhone 17 Pro" — the make only once ("Canon Canon EOS R6"
    /// reads badly).
    pub fn camera(make: Option<&str>, model: Option<&str>) -> Option<String> {
        let make = make.unwrap_or("").trim();
        let model = model.unwrap_or("").trim();
        if model.is_empty() {
            return (!make.is_empty()).then(|| make.to_string());
        }
        if make.is_empty() || model.to_lowercase().starts_with(&make.to_lowercase()) {
            return Some(model.to_string());
        }
        Some(format!("{make} {model}"))
    }

    /// "Rückseitige Kamera — 6,76 mm ƒ1,78": iPhones name their lenses
    /// "iPhone 17 Pro back triple camera 6.765mm f/1.78"; other lenses keep
    /// their own name.
    pub fn lens(
        model: Option<&str>,
        camera_model: Option<&str>,
        focal_length: Option<f64>,
        f_number: Option<f64>,
    ) -> Option<String> {
        let mut name: Option<String> = None;
        if let Some(lens_model) = model.map(str::trim).filter(|s| !s.is_empty()) {
            let lower = lens_model.to_lowercase();
            let camera = camera_model.filter(|c| !c.is_empty());
            name = Some(if lower.contains(" front ") || lower.starts_with("front ") {
                "Frontkamera".to_string()
            } else if lower.contains(" back ") || lower.starts_with("back ") {
                "Rückseitige Kamera".to_string()
            } else if let Some(camera) = camera.filter(|c| lower.starts_with(&c.to_lowercase())) {
                lens_model.chars().skip(camera.chars().count()).collect::<String>().trim().to_string()
            } else {
                lens_model.to_string()
            });
        }
        let optics: Vec<String> = [
            focal_length.map(|f| format!("{} mm", number(f, 2))),
            f_number.map(|f| format!("ƒ{}", number(f, 2))),
        ]
        .into_iter()
        .flatten()
        .collect();
        let optics = optics.join(" ");
        match (name, optics.is_empty()) {
            (Some(name), false) => Some(format!("{name} — {optics}")),
            (Some(name), true) => Some(name),
            (None, false) => Some(optics),
            (None, true) => None,
        }
    }

    /// The exposure strip: ISO, focal length, aperture, shutter speed.
    /// `None` when none of the four is known.
    pub fn exposure(exif: &ExifInfo) -> Option<Vec<String>> {
        let values = [
            exif.iso.map(|iso| format!("ISO {}", iso.round() as i64)),
            exif.focal_length.map(|f| format!("{} mm", number(f, 2))),
            exif.f_number.map(|f| format!("ƒ{}", number(f, 2))),
            shutter(exif.exposure_time.as_deref()),
        ];
        if values.iter().all(Option::is_none) {
            return None;
        }
        Some(values.into_iter().map(|v| v.unwrap_or_else(|| "–".to_string())).collect())
    }

    /// "1/120 s", "2 s" (Immich sends "1/120" or seconds as text).
    pub fn shutter(raw: Option<&str>) -> Option<String> {
        shutter(raw)
    }

    /// German decimal comma, trailing zeros dropped ("6,76", "1,8", "24").
    pub fn number(value: f64, digits: usize) -> String {
        number(value, digits)
    }
}

fn shutter(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.contains('/') {
        return Some(format!("{raw} s"));
    }
    let Ok(seconds) = raw.parse::<f64>() else {
        return Some(raw.to_string());
    };
    if seconds > 0.0 && seconds < 1.0 {
        return Some(format!("1/{} s", (1.0 / seconds).round() as i64));
    }
    Some(format!("{} s", number(seconds, 1)))
}

fn number(value: f64, digits: usize) -> String {
    let mut text = format!("{:.*}", digits, value);
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    text.replace('.', ",")
}

/// 0 = Sunday … 6 = Saturday, for a proleptic Gregorian civil date.
fn weekday_index(year: i32, month: u32, day: u32) -> usize {
    // Days since 1970-01-01 (Howard Hinnant's algorithm).
    let y = i64::from(if month <= 2 { year - 1 } else { year });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    // 1970-01-01 was a Thursday (index 4).
    (days + 4).rem_euclid(7) as usize
}

/// "Berlin, Deutschland" from Immich's reverse geocoding, which names
/// countries in English (GeoNames): they are shown in German.
pub struct PlaceName;

impl PlaceName {
    pub fn format(city: Option<&str>, state: Option<&str>, country: Option<&str>) -> Option<String> {
        let city = clean(city);
        let state = clean(state);
        let country = clean(country).map(|c| localized_country(&c));
        let mut parts: Vec<String> = Vec::new();
        if let Some(city) = city {
            parts.push(city);
        } else if let Some(state) = state {
            parts.push(state);
        }
        if let Some(country) = country {
            if !parts.contains(&country) {
                parts.push(country);
            }
        }
        if parts.is_empty() { None } else { Some(parts.join(", ")) }
    }

    /// English country name → German name; unknown names stay as they are.
    pub fn localized_country(english: &str) -> String {
        localized_country(english)
    }
}

fn clean(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Lowercase English (GeoNames) country names → German names. Not the full ISO list:
/// countries missing here keep their English name.
const COUNTRIES_DE: &[(&str, &str)] = &[
    ("germany", "Deutschland"),
    ("austria", "Österreich"),
    ("switzerland", "Schweiz"),
    ("liechtenstein", "Liechtenstein"),
    ("france", "Frankreich"),
    ("italy", "Italien"),
    ("spain", "Spanien"),
    ("portugal", "Portugal"),
    ("netherlands", "Niederlande"),
    ("the netherlands", "Niederlande"),
    ("belgium", "Belgien"),
    ("luxembourg", "Luxemburg"),
    ("denmark", "Dänemark"),
    ("sweden", "Schweden"),
    ("norway", "Norwegen"),
    ("finland", "Finnland"),
    ("iceland", "Island"),
    ("ireland", "Irland"),
    ("united kingdom", "Vereinigtes Königreich"),
    ("poland", "Polen"),
    ("czech republic", "Tschechien"),
    ("czechia", "Tschechien"),
    ("slovakia", "Slowakei"),
    ("hungary", "Ungarn"),
    ("slovenia", "Slowenien"),
    ("croatia", "Kroatien"),
    ("bosnia and herzegovina", "Bosnien und Herzegowina"),
    ("serbia", "Serbien"),
    ("greece", "Griechenland"),
    ("turkey", "Türkei"),
    ("russia", "Russland"),
    ("russian federation", "Russland"),
    ("ukraine", "Ukraine"),
    ("romania", "Rumänien"),
    ("bulgaria", "Bulgarien"),
    ("estonia", "Estland"),
    ("latvia", "Lettland"),
    ("lithuania", "Litauen"),
    ("malta", "Malta"),
    ("cyprus", "Zypern"),
    ("monaco", "Monaco"),
    ("andorra", "Andorra"),
    ("united states of america", "Vereinigte Staaten"),
    ("united states", "Vereinigte Staaten"),
    ("canada", "Kanada"),
    ("mexico", "Mexiko"),
    ("brazil", "Brasilien"),
    ("argentina", "Argentinien"),
    ("chile", "Chile"),
    ("peru", "Peru"),
    ("colombia", "Kolumbien"),
    ("cuba", "Kuba"),
    ("greenland", "Grönland"),
    ("japan", "Japan"),
    ("china", "China"),
    ("taiwan", "Taiwan"),
    ("hong kong", "Hongkong"),
    ("india", "Indien"),
    ("pakistan", "Pakistan"),
    ("bangladesh", "Bangladesch"),
    ("nepal", "Nepal"),
    ("sri lanka", "Sri Lanka"),
    ("mongolia", "Mongolei"),
    ("kazakhstan", "Kasachstan"),
    ("south korea", "Südkorea"),
    ("republic of korea", "Südkorea"),
    ("north korea", "Nordkorea"),
    ("vietnam", "Vietnam"),
    ("viet nam", "Vietnam"),
    ("thailand", "Thailand"),
    ("indonesia", "Indonesien"),
    ("philippines", "Philippinen"),
    ("singapore", "Singapur"),
    ("malaysia", "Malaysia"),
    ("israel", "Israel"),
    ("iran", "Iran"),
    ("iraq", "Irak"),
    ("saudi arabia", "Saudi-Arabien"),
    ("united arab emirates", "Vereinigte Arabische Emirate"),
    ("australia", "Australien"),
    ("new zealand", "Neuseeland"),
    ("egypt", "Ägypten"),
    ("morocco", "Marokko"),
    ("tunisia", "Tunesien"),
    ("kenya", "Kenia"),
    ("nigeria", "Nigeria"),
    ("ethiopia", "Äthiopien"),
    ("south africa", "Südafrika"),
];

fn localized_country(english: &str) -> String {
    let key = english.to_lowercase();
    COUNTRIES_DE
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, german)| (*german).to_string())
        .unwrap_or_else(|| english.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(latitude: Option<f64>, longitude: Option<f64>) -> AssetDetails {
        AssetDetails {
            id: "x".into(),
            exif: ExifInfo::empty().with(latitude, longitude, None, None, None),
            fetched_at: "2026-10-24T14:42:00Z".into(),
        }
    }

    #[test]
    fn place_names_in_german() {
        assert_eq!(
            PlaceName::format(Some("Sindelfingen"), Some("Baden-Württemberg"), Some("Germany")).as_deref(),
            Some("Sindelfingen, Deutschland")
        );
        assert_eq!(
            PlaceName::format(Some("Berlin"), Some("Berlin"), Some("Germany")).as_deref(),
            Some("Berlin, Deutschland")
        );
        assert_eq!(PlaceName::format(None, Some("Tyrol"), Some("Austria")).as_deref(), Some("Tyrol, Österreich"));
        assert_eq!(
            PlaceName::format(Some("New York City"), Some("New York"), Some("United States of America")).as_deref(),
            Some("New York City, Vereinigte Staaten")
        );
        assert_eq!(
            PlaceName::format(Some("Atlantis"), None, Some("Atlantis Republic")).as_deref(),
            Some("Atlantis, Atlantis Republic"),
            "unknown names stay as they are"
        );
        assert_eq!(PlaceName::format(Some(" "), None, None), None);
    }

    #[test]
    fn camera_and_lens() {
        assert_eq!(InfoFormat::camera(Some("Apple"), Some("iPhone 14 Pro")).as_deref(), Some("Apple iPhone 14 Pro"));
        assert_eq!(InfoFormat::camera(Some("Canon"), Some("Canon EOS R6")).as_deref(), Some("Canon EOS R6"), "the make only once");
        assert_eq!(InfoFormat::camera(None, None), None);
        assert_eq!(
            InfoFormat::lens(Some("iPhone 14 Pro back triple camera 2.22mm f/2.2"), Some("iPhone 14 Pro"), Some(2.22), Some(2.2))
                .as_deref(),
            Some("Rückseitige Kamera — 2,22 mm ƒ2,2")
        );
        assert_eq!(
            InfoFormat::lens(Some("iPhone 14 Pro front camera 2.69mm f/1.9"), Some("iPhone 14 Pro"), Some(2.69), Some(1.9))
                .as_deref(),
            Some("Frontkamera — 2,69 mm ƒ1,9")
        );
        assert_eq!(
            InfoFormat::lens(Some("RF24-105mm F4 L IS USM"), Some("Canon EOS R6"), Some(50.0), Some(4.0)).as_deref(),
            Some("RF24-105mm F4 L IS USM — 50 mm ƒ4")
        );
        assert_eq!(InfoFormat::lens(None, None, None, None), None);
    }

    #[test]
    fn exposure_strip() {
        assert_eq!(InfoFormat::shutter(Some("1/120")).as_deref(), Some("1/120 s"));
        assert_eq!(InfoFormat::shutter(Some("0.008")).as_deref(), Some("1/125 s"));
        assert_eq!(InfoFormat::shutter(Some("2")).as_deref(), Some("2 s"));
        assert_eq!(InfoFormat::shutter(Some("")), None);
        let exif = ExifInfo::empty().with(None, None, None, None, None);
        assert_eq!(InfoFormat::exposure(&exif), None, "no strip without any value");
        let photo = ExifInfo {
            f_number: Some(1.78),
            focal_length: Some(6.765),
            iso: Some(80.0),
            exposure_time: Some("1/120".into()),
            ..ExifInfo::empty()
        };
        assert_eq!(
            InfoFormat::exposure(&photo),
            Some(vec!["ISO 80".to_string(), "6,76 mm".to_string(), "ƒ1,78".to_string(), "1/120 s".to_string()]),
            "as Photos shows it"
        );
    }

    #[test]
    fn date_line_and_title() {
        assert_eq!(InfoFormat::date_line(2026, 10, 24, 16, 42), "Samstag, 24. Oktober 2026 um 16:42");
        assert_eq!(InfoFormat::title(Some("IMG_1234.HEIC")).as_deref(), Some("IMG_1234"));
        assert_eq!(InfoFormat::title(Some(".hidden")).as_deref(), Some(".hidden"));
        assert_eq!(InfoFormat::title(None), None);
    }

    #[test]
    fn coordinate_needs_a_real_place() {
        assert_eq!(at(Some(0.0), Some(0.0)).coordinate(), None, "0/0 is a missing place, not the Gulf of Guinea");
        assert_eq!(at(None, Some(13.0)).coordinate(), None);
        assert_eq!(at(Some(120.0), Some(13.0)).coordinate(), None);
        assert_eq!(at(Some(52.5), Some(13.4)).coordinate().map(|c| c.latitude), Some(52.5));
    }

    #[test]
    fn details_move_to_a_new_place() {
        let moved = at(None, None).moved_to(Coordinate { latitude: 48.7, longitude: 9.0 }, Some("Sindelfingen".into()), None, Some("Germany".into()));
        assert_eq!(moved.place_name().as_deref(), Some("Sindelfingen, Deutschland"));
        assert_eq!(moved.coordinate().map(|c| c.longitude), Some(9.0));
    }

    #[test]
    fn weekday_of_a_known_date() {
        // 1970-01-01 was a Thursday; 2000-01-01 a Saturday.
        assert_eq!(weekday_index(1970, 1, 1), 4);
        assert_eq!(weekday_index(2000, 1, 1), 6);
    }
}
