//! Owner: A1. Swift source: `../immagine/Immagine/Support/MediaDuration.swift`.
//! Spec: `../immagine/ImmagineTests/` -> CoreTests.
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::media_duration` is green.

//! Video durations. The v3 server sends integer milliseconds, older payloads a display string such
//! as `"0:00:12.345678"`. Both are normalized to the Photos-style display string ("0:12", "1:02:05").

use serde::{Deserialize, Deserializer};

/// Display string for a duration in milliseconds (negative values count as zero).
pub fn text_from_milliseconds(milliseconds: i64) -> String {
    let seconds = (milliseconds.max(0) as f64 / 1000.0).round() as i64;
    text_from_seconds(seconds)
}

/// Display string for a duration in whole seconds: "M:SS", or "H:MM:SS" from one hour on.
pub fn text_from_seconds(seconds: i64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// "H:MM:SS(.ffffff)" or "M:SS" → seconds; `None` for anything else.
pub fn seconds_from_server_string(raw: &str) -> Option<f64> {
    let parts: Vec<&str> = raw.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total = 0.0;
    for part in parts {
        let value: f64 = part.parse().ok()?;
        if !(value >= 0.0) {
            // Rejects negatives and NaN.
            return None;
        }
        total = total * 60.0 + value;
    }
    Some(total)
}

/// Display string for a server string; `None` when it is not a duration.
pub fn text_from_server_string(raw: &str) -> Option<String> {
    seconds_from_server_string(raw).map(|s| text_from_seconds(s.round() as i64))
}

/// Earlier builds formatted v3's milliseconds as if they were seconds ("7:31:14" for 27.074 s).
/// The digits still encode the milliseconds, so re-read them that way.
pub fn repair_misread_milliseconds(raw: &str) -> Option<String> {
    let wrong = seconds_from_server_string(raw)?;
    Some(text_from_milliseconds(wrong as i64))
}

/// serde adapter for a `duration` field that arrives either as integer milliseconds or as a
/// server string. Use with `#[serde(default, deserialize_with = "deserialize_duration")]` on an
/// `Option<String>`. An unparsable string is kept verbatim, as in the Swift original.
pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Milliseconds(i64),
        Text(String),
    }

    let wire = Option::<Wire>::deserialize(deserializer)?;
    Ok(wire.map(|wire| match wire {
        Wire::Milliseconds(ms) => text_from_milliseconds(ms),
        Wire::Text(raw) => text_from_server_string(&raw).unwrap_or(raw),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats() {
        assert_eq!(text_from_seconds(12), "0:12");
        assert_eq!(text_from_seconds(75), "1:15");
        assert_eq!(text_from_seconds(3725), "1:02:05");
    }

    // Added beyond the Swift suite: the other entry points.

    #[test]
    fn duration_from_milliseconds_rounds_to_seconds() {
        assert_eq!(text_from_milliseconds(27_074), "0:27");
        assert_eq!(text_from_milliseconds(-5), "0:00");
    }

    #[test]
    fn duration_from_server_string() {
        assert_eq!(text_from_server_string("0:00:12.345678").as_deref(), Some("0:12"));
        assert_eq!(text_from_server_string("1:02:05.4").as_deref(), Some("1:02:05"));
        assert_eq!(text_from_server_string("0:15").as_deref(), Some("0:15"));
        assert_eq!(text_from_server_string("garbage"), None);
        assert_eq!(text_from_server_string("-1:00"), None);
    }

    #[test]
    fn duration_repairs_misread_milliseconds() {
        assert_eq!(repair_misread_milliseconds("7:31:14").as_deref(), Some("0:27"));
    }

    #[test]
    fn duration_deserializes_both_wire_formats() {
        #[derive(Deserialize)]
        struct Probe {
            #[serde(default, deserialize_with = "deserialize_duration")]
            duration: Option<String>,
        }
        let ms: Probe = serde_json::from_str(r#"{"duration": 3725000}"#).unwrap();
        assert_eq!(ms.duration.as_deref(), Some("1:02:05"));
        let text: Probe = serde_json::from_str(r#"{"duration": "0:00:12.345678"}"#).unwrap();
        assert_eq!(text.duration.as_deref(), Some("0:12"));
        let null: Probe = serde_json::from_str(r#"{"duration": null}"#).unwrap();
        assert_eq!(null.duration, None);
        let missing: Probe = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.duration, None);
    }
}
