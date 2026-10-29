//! Owner: A3. Swift source: `../immagine/Immagine/Models/AssetInfo.swift`.
//! Spec: `../immagine/ImmagineTests/` -> InfoPanelTests. The Swift file has no tests of its own, so
//! `AssetInfoFormat` is ported without spec. Its file-size wording is an approximation of
//! `ByteCountFormatter` (decimal units, German "Byte"); it is not checked against the real formatter.

use serde::{Deserialize, Serialize};

/// Viewer-side details of one asset, read from local columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    pub id: String,
    /// "IMAGE", "VIDEO", … (`type` in JSON).
    #[serde(rename = "type")]
    pub asset_type: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_size: Option<i64>,
    pub is_favorite: bool,
    pub is_edited: bool,
    pub live_photo_video_id: Option<String>,
    /// ISO 8601 as stored locally.
    pub capture_date: Option<String>,
    pub duration: Option<String>,
}

impl AssetInfo {
    pub fn is_video(&self) -> bool {
        self.asset_type == "VIDEO"
    }

    /// Server-side editing supports still images only (no video, Live Photo or GIF).
    pub fn is_editable(&self) -> bool {
        if self.is_video() || self.live_photo_video_id.is_some() {
            return false;
        }
        let name = self.file_name.as_deref().unwrap_or("").to_lowercase();
        !name.ends_with(".gif") && !name.ends_with(".svg")
    }
}

/// Formatting for the info panel ("4032 × 3024", "12 MP", "2,4 MB").
pub struct AssetInfoFormat;

fn positive_pair(width: Option<i64>, height: Option<i64>) -> Option<(i64, i64)> {
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    }
}

/// German decimal comma, trailing zeros dropped.
fn number(value: f64, digits: usize) -> String {
    let mut text = format!("{:.*}", digits, value);
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    text.replace('.', ",")
}

impl AssetInfoFormat {
    pub fn dimensions(width: Option<i64>, height: Option<i64>) -> Option<String> {
        let (w, h) = positive_pair(width, height)?;
        Some(format!("{w} × {h}"))
    }

    /// "12 MP", "2,4 MP"; `None` below 0.1 MP.
    pub fn megapixels(width: Option<i64>, height: Option<i64>) -> Option<String> {
        let (w, h) = positive_pair(width, height)?;
        let mp = (w * h) as f64 / 1_000_000.0;
        if mp < 0.1 {
            return None;
        }
        let rounded = if mp >= 10.0 { mp.round() } else { (mp * 10.0).round() / 10.0 };
        Some(format!("{} MP", number(rounded, 1)))
    }

    /// "512 Byte", "2,4 MB", "12 GB" (decimal units, as Finder shows file sizes).
    pub fn file_size(bytes: Option<i64>) -> Option<String> {
        let bytes = bytes.filter(|b| *b > 0)?;
        if bytes < 1000 {
            return Some(format!("{bytes} Byte"));
        }
        const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
        let mut value = bytes as f64 / 1000.0;
        let mut unit = 0;
        while value >= 1000.0 && unit < UNITS.len() - 1 {
            value /= 1000.0;
            unit += 1;
        }
        let text = if value < 10.0 { number(value, 1) } else { (value.round() as i64).to_string() };
        Some(format!("{text} {}", UNITS[unit]))
    }

    /// "JPEG", "HEIF", "PNG", "MOV" … from the original file name.
    pub fn file_type(file_name: Option<&str>) -> Option<String> {
        let name = file_name?;
        let ext = match name.rfind('.') {
            Some(index) if index > 0 => name[index + 1..].to_uppercase(),
            _ => return None,
        };
        if ext.is_empty() {
            return None;
        }
        Some(match ext.as_str() {
            "JPG" | "JPEG" => "JPEG".to_string(),
            "HEIC" | "HEIF" => "HEIF".to_string(),
            _ => ext.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(file_name: Option<&str>, asset_type: &str, live: Option<&str>) -> AssetInfo {
        AssetInfo {
            id: "a".into(),
            asset_type: asset_type.into(),
            file_name: file_name.map(str::to_string),
            mime_type: None,
            width: None,
            height: None,
            file_size: None,
            is_favorite: false,
            is_edited: false,
            live_photo_video_id: live.map(str::to_string),
            capture_date: None,
            duration: None,
        }
    }

    #[test]
    fn editable_only_for_still_images() {
        assert!(info(Some("IMG_1.JPG"), "IMAGE", None).is_editable());
        assert!(!info(Some("IMG_1.MOV"), "VIDEO", None).is_editable());
        assert!(!info(Some("IMG_1.JPG"), "IMAGE", Some("v1")).is_editable(), "Live Photo");
        assert!(!info(Some("anim.GIF"), "IMAGE", None).is_editable());
        assert!(!info(Some("logo.svg"), "IMAGE", None).is_editable());
    }

    #[test]
    fn dimensions_megapixels_and_sizes() {
        assert_eq!(AssetInfoFormat::dimensions(Some(4032), Some(3024)).as_deref(), Some("4032 × 3024"));
        assert_eq!(AssetInfoFormat::dimensions(Some(0), Some(3024)), None);
        assert_eq!(AssetInfoFormat::megapixels(Some(4032), Some(3024)).as_deref(), Some("12 MP"));
        assert_eq!(AssetInfoFormat::megapixels(Some(1600), Some(1500)).as_deref(), Some("2,4 MP"));
        assert_eq!(AssetInfoFormat::megapixels(Some(100), Some(100)), None);
        assert_eq!(AssetInfoFormat::file_size(Some(0)), None);
        assert_eq!(AssetInfoFormat::file_size(Some(2_400_000)).as_deref(), Some("2,4 MB"));
    }

    #[test]
    fn file_types_from_names() {
        assert_eq!(AssetInfoFormat::file_type(Some("IMG_1.jpg")).as_deref(), Some("JPEG"));
        assert_eq!(AssetInfoFormat::file_type(Some("IMG_1.HEIC")).as_deref(), Some("HEIF"));
        assert_eq!(AssetInfoFormat::file_type(Some("clip.mov")).as_deref(), Some("MOV"));
        assert_eq!(AssetInfoFormat::file_type(Some(".hidden")), None);
        assert_eq!(AssetInfoFormat::file_type(None), None);
    }
}
