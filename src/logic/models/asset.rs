//! A photo or video on the Immich server.
//!
//! Port of `Immagine/Models/Asset.swift` and `Asset+TimelineItem.swift`.
//! Spec: `../immagine/ImmagineTests/ModelTests.swift` and the asset JSON in `CoreTests.swift`.
//!
//! Differences from Swift, on purpose:
//! - Dates stay raw ISO strings (no `ISODate` dependency). `capture_date` picks the same
//!   source field as Swift but without validating it, and `date_bucket` is the UTC day
//!   prefix (`YYYY-MM-DD`), not the device time zone. The timeline layer must convert with
//!   `iso_date` when it needs local days.
//! - Durations are formatted here (`m:ss` / `h:mm:ss`) from either the v3 millisecond number
//!   or the server string; this duplicates a little of `media_duration` until that port lands.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::timeline::TimelineItem;

/// A photo/video on the Immich server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    #[serde(rename = "type", default)]
    pub asset_type: Option<String>,
    #[serde(default)]
    pub original_path: Option<String>,
    #[serde(default)]
    pub original_file_name: Option<String>,
    #[serde(default)]
    pub original_mime_type: Option<String>,
    #[serde(default)]
    pub thumbhash: Option<String>,
    #[serde(default)]
    pub file_created_at: Option<String>,
    #[serde(default)]
    pub file_modified_at: Option<String>,
    #[serde(default)]
    pub local_date_time: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default, deserialize_with = "bool_or_false")]
    pub is_favorite: bool,
    #[serde(default, deserialize_with = "bool_or_false")]
    pub is_archived: bool,
    #[serde(default, deserialize_with = "bool_or_false")]
    pub is_trashed: bool,
    #[serde(default)]
    pub visibility: Option<String>,
    /// Display string ("0:12"). The wire value is milliseconds on v3.
    #[serde(default, deserialize_with = "lenient_duration")]
    pub duration: Option<String>,
    #[serde(default)]
    pub live_photo_video_id: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default, deserialize_with = "bool_or_false")]
    pub is_offline: bool,
    #[serde(default, deserialize_with = "bool_or_false")]
    pub is_edited: bool,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default)]
    pub resized: Option<bool>,
    #[serde(default)]
    pub has_metadata: Option<bool>,
    #[serde(default)]
    pub owner_id: Option<String>,
    /// An unreadable exif block is dropped, never fatal for the whole asset.
    #[serde(default, deserialize_with = "lenient_option")]
    pub exif_info: Option<ExifInfo>,
}

impl Asset {
    pub fn is_video(&self) -> bool {
        self.asset_type.as_deref() == Some("VIDEO")
    }

    pub fn is_image(&self) -> bool {
        !self.is_video()
    }

    /// The raw ISO string of the instant the photo was taken: `fileCreatedAt`, then
    /// `localDateTime`, then `createdAt`.
    pub fn capture_date(&self) -> Option<&str> {
        capture_date_of(
            self.file_created_at.as_deref(),
            self.local_date_time.as_deref(),
            self.created_at.as_deref(),
        )
    }

    /// `YYYY-MM-DD` of the capture date (UTC day, see the module docs).
    pub fn date_bucket(&self) -> Option<String> {
        self.capture_date().and_then(date_bucket_of)
    }

    /// Width / height, `None` when the height is missing or zero.
    pub fn aspect_ratio(&self) -> Option<f64> {
        match (self.width, self.height) {
            (Some(w), Some(h)) if h != 0 => Some(w as f64 / h as f64),
            _ => None,
        }
    }

    /// Lightweight grid row for the timeline and album grids.
    pub fn timeline_item(&self) -> TimelineItem {
        TimelineItem {
            id: self.id.clone(),
            asset_type: self.asset_type.clone().unwrap_or_default(),
            thumbhash: self.thumbhash.clone(),
            ratio: self.aspect_ratio(),
            is_favorite: self.is_favorite,
            is_trashed: self.is_trashed,
            date_bucket: self.date_bucket().unwrap_or_default(),
            sort_date: self.capture_date().map(str::to_owned),
            duration: self.duration.clone(),
        }
    }
}

/// Shared by `Asset` and `SyncAsset`: first non-empty of the three capture fields.
pub(crate) fn capture_date_of<'a>(
    file_created_at: Option<&'a str>,
    local_date_time: Option<&'a str>,
    created_at: Option<&'a str>,
) -> Option<&'a str> {
    [file_created_at, local_date_time, created_at]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
}

/// `YYYY-MM-DD` prefix of an ISO-8601 string, `None` if it does not start with a date.
pub(crate) fn date_bucket_of(iso: &str) -> Option<String> {
    let b = iso.as_bytes();
    let is_date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    is_date.then(|| iso[..10].to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfo {
    #[serde(default)]
    pub make: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub exif_image_width: Option<i64>,
    #[serde(default)]
    pub exif_image_height: Option<i64>,
    #[serde(default)]
    pub file_size_in_byte: Option<i64>,
    /// EXIF orientation as sent by the server (a number in a string, e.g. "6").
    #[serde(default)]
    pub orientation: Option<String>,
    #[serde(default)]
    pub date_time_original: Option<String>,
    #[serde(default)]
    pub lens_model: Option<String>,
    #[serde(default)]
    pub f_number: Option<f64>,
    #[serde(default)]
    pub focal_length: Option<f64>,
    #[serde(default)]
    pub iso: Option<f64>,
    #[serde(default)]
    pub exposure_time: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub projection_type: Option<String>,
    #[serde(default)]
    pub rating: Option<i64>,
}

impl ExifInfo {
    /// EXIF orientations 5 to 8 store the sensor image rotated by 90 degrees.
    pub fn is_rotated_90(&self) -> bool {
        self.orientation
            .as_deref()
            .and_then(|o| o.trim().parse::<i64>().ok())
            .is_some_and(|v| (5..=8).contains(&v))
    }

    /// Pixel size of the unedited original as displayed (orientation applied), the
    /// coordinate space of the server's crop parameters.
    pub fn oriented_size(&self) -> Option<(i64, i64)> {
        match (self.exif_image_width, self.exif_image_height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => {
                Some(if self.is_rotated_90() { (h, w) } else { (w, h) })
            }
            _ => None,
        }
    }
}

/// `null` or a missing flag reads as `false`.
pub(crate) fn bool_or_false<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<bool>::deserialize(d)?.unwrap_or(false))
}

/// Any value that fails to decode becomes `None` instead of failing the whole record.
pub(crate) fn lenient_option<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Duration as a display string. Accepts v3 milliseconds (a number) or a server string
/// (`"0:00:12.345678"`, `"2:30"`). Anything else becomes `None`.
pub(crate) fn lenient_duration<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match value {
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .filter(|ms| ms.is_finite())
            .map(|ms| duration_text((ms / 1000.0).round() as i64)),
        Some(serde_json::Value::String(s)) => {
            parse_server_duration(&s).map(|secs| duration_text(secs.round() as i64))
        }
        _ => None,
    })
}

/// `"h:mm:ss"`, `"m:ss"` with an optional fraction on the last part, in seconds.
pub(crate) fn parse_server_duration(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let last = parts.len() - 1;
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let value: f64 = if i == last {
            part.parse().ok()?
        } else {
            part.parse::<u32>().ok()? as f64
        };
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        total = total * 60.0 + value;
    }
    Some(total)
}

/// `m:ss`, or `h:mm:ss` from one hour on. Negative values show as zero.
pub(crate) fn duration_text(total_seconds: i64) -> String {
    let s = total_seconds.max(0);
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same shape as `sampleAssetJSON` in `CoreTests.swift`.
    const SAMPLE_ASSET_JSON: &str = r#"{"id":"a0437345-fed4-439f-adbf-f3bd38e1cff4","type":"IMAGE",
     "originalPath":"/data/upload/foo.jpg","originalFileName":"a.jpg",
     "originalMimeType":"image/jpeg","thumbhash":"GikKDQK6iGBbGKaHd3uniPN8M/p2",
     "fileCreatedAt":"2026-10-01T20:17:30.923Z","localDateTime":"2026-10-01T20:17:30.923Z",
     "updatedAt":"2026-10-01T20:17:44.383Z","isFavorite":false,"isArchived":false,
     "isTrashed":false,"visibility":"timeline","duration":null,"checksum":"IciyZg02A2tDHvuXe3ldf71pzOY=",
     "isOffline":false,"hasMetadata":true,"resized":true,"width":5648,"height":8472,"isEdited":false}"#;

    #[test]
    fn asset_decoding() {
        let asset: Asset = serde_json::from_str(SAMPLE_ASSET_JSON).unwrap();
        assert_eq!(asset.id, "a0437345-fed4-439f-adbf-f3bd38e1cff4");
        assert!(asset.is_image());
        assert!((asset.aspect_ratio().unwrap() - 5648.0 / 8472.0).abs() < 0.0001);
        assert_eq!(asset.date_bucket().as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn asset_decodes_v3_video_with_millisecond_duration() {
        let json = r#"{"id":"ef4891d0-4d47-4b26-b0a7-34b675bd17c8","type":"VIDEO","originalFileName":"FullSizeRender.mov",
         "fileCreatedAt":"2026-10-02T22:42:39.000Z","duration":2298,"width":1080,"height":1920,
         "isFavorite":false,"isTrashed":false,"visibility":"timeline",
         "exifInfo":{"fileSizeInByte":2523271,"exifImageWidth":1920,"exifImageHeight":1080,"orientation":"6"}}"#;
        let asset: Asset = serde_json::from_str(json).unwrap();
        assert_eq!(asset.duration.as_deref(), Some("0:02"));
        assert!(asset.is_video());
        let exif = asset.exif_info.as_ref().unwrap();
        assert_eq!(exif.file_size_in_byte, Some(2_523_271));
        assert!(!asset.is_archived, "missing flags default to false");
        assert_eq!(exif.oriented_size(), Some((1080, 1920)), "orientation 6 swaps sensor width and height");
    }

    #[test]
    fn asset_timeline_item_maps_fields() {
        let asset: Asset = serde_json::from_str(SAMPLE_ASSET_JSON).unwrap();
        let item = asset.timeline_item();
        assert_eq!(item.id, asset.id);
        assert_eq!(item.date_bucket, "2026-10-01");
        assert_eq!(item.sort_date.as_deref(), Some("2026-10-01T20:17:30.923Z"));
        assert!(!item.is_video());
    }

    #[test]
    fn duration_text_formats_and_clamps() {
        assert_eq!(duration_text(0), "0:00");
        assert_eq!(duration_text(-5), "0:00");
        assert_eq!(duration_text(3725), "1:02:05");
        assert_eq!(parse_server_duration("abc"), None);
        assert_eq!(parse_server_duration("12"), None);
        assert_eq!(parse_server_duration("1:2:3:4"), None);
        assert_eq!(parse_server_duration("2:30"), Some(150.0));
    }
}
