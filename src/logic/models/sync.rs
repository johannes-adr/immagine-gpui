//! Payloads of the `/api/sync/stream` JSON-Lines stream.
//!
//! Port of `Immagine/Models/Sync.swift`. Field names follow the OpenAPI spec (`SyncAssetV2`,
//! `AlbumV2`) as verified against the v3.0.3 server. The Swift test-only memberwise inits are
//! not needed: the structs are plain literals in Rust.
//!
//! `SyncEventEnvelope.type` is `event_type` in Rust (`type` is a keyword) with the wire name kept.

use serde::{Deserialize, Serialize};

use super::asset::{capture_date_of, date_bucket_of, lenient_duration};

/// Asset payload delivered by the sync stream (AssetV2 entity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAsset {
    pub id: String,
    #[serde(rename = "type", default)]
    pub asset_type: Option<String>,
    #[serde(default)]
    pub thumbhash: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default)]
    pub file_created_at: Option<String>,
    #[serde(default)]
    pub file_modified_at: Option<String>,
    #[serde(default)]
    pub local_date_time: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub is_favorite: Option<bool>,
    #[serde(default)]
    pub is_edited: Option<bool>,
    /// Display string. V2 sends integer milliseconds, V1 the display string.
    #[serde(default, deserialize_with = "lenient_duration")]
    pub duration: Option<String>,
    #[serde(default)]
    pub live_photo_video_id: Option<String>,
    #[serde(default)]
    pub original_file_name: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
}

impl SyncAsset {
    /// Raw ISO string of the capture instant (see `Asset::capture_date`).
    pub fn capture_date(&self) -> Option<&str> {
        capture_date_of(
            self.file_created_at.as_deref(),
            self.local_date_time.as_deref(),
            self.created_at.as_deref(),
        )
    }

    /// `YYYY-MM-DD` of the capture date (UTC day, see the `asset` module docs).
    pub fn date_bucket(&self) -> Option<String> {
        self.capture_date().and_then(date_bucket_of)
    }
}

/// Album payload delivered by the sync stream (AlbumV2 entity). No `assetCount`: counts are
/// refreshed from the album-asset links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAlbum {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thumbnail_asset_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
}

/// One album-to-asset link event (AlbumToAssetV1 / AlbumToAssetDeleteV1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAlbumAssetLink {
    pub album_id: String,
    pub asset_id: String,
}

/// Generic event envelope of the JSON-Lines stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEventEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub ack: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `testSyncAssetMillisecondDuration` (ModelTests).
    #[test]
    fn sync_asset_millisecond_duration() {
        let json = r#"{"id":"v1","type":"VIDEO","duration":41598,"visibility":"hidden","livePhotoVideoId":null}"#;
        let asset: SyncAsset = serde_json::from_str(json).unwrap();
        assert_eq!(asset.duration.as_deref(), Some("0:42"));
        assert_eq!(asset.visibility.as_deref(), Some("hidden"));
    }

    /// `testSyncAssetDecoding` (CoreTests), fixture captured from the live v3.0.3 server.
    #[test]
    fn sync_asset_decoding() {
        let json = r#"{"id":"8ae196ac-b89f-4cc7-8af6-995f65dd529f","ownerId":"74e9e10f-5d8e-4343-97eb-bef11df33d34",
         "originalFileName":"pexels-jonathan-rios-388322-1033730.jpg",
         "fileCreatedAt":"2026-10-01T20:17:21.062Z","fileModifiedAt":"2026-10-01T20:17:21.062Z",
         "createdAt":"2026-10-01T20:17:38.359Z","localDateTime":"2026-10-01T20:17:21.062Z",
         "type":"IMAGE","deletedAt":null,"isFavorite":false,"visibility":"timeline","duration":null,
         "livePhotoVideoId":null,"stackId":null,"libraryId":null,"width":4000,"height":6000,
         "isEdited":false,"checksum":"G2pWcB+alDjUjKXNAMxV+yws8IE=",
         "thumbhash":"DSkGDQKwl22XV3eIh4mIWbBuBF14"}"#;
        let asset: SyncAsset = serde_json::from_str(json).unwrap();
        assert_eq!(asset.id, "8ae196ac-b89f-4cc7-8af6-995f65dd529f");
        assert_eq!(asset.asset_type.as_deref(), Some("IMAGE"));
        assert_eq!(asset.checksum.as_deref(), Some("G2pWcB+alDjUjKXNAMxV+yws8IE="));
        assert_eq!(asset.date_bucket().as_deref(), Some("2026-10-01"));
        assert!(asset.deleted_at.is_none());
        assert!(asset.duration.is_none());
    }

    #[test]
    fn sync_event_envelope_keeps_wire_type_name() {
        let env: SyncEventEnvelope =
            serde_json::from_str(r#"{"type":"AssetV2","ack":"abc","data":{"id":"x"}}"#).unwrap();
        assert_eq!(env.event_type, "AssetV2");
        assert_eq!(env.ack.as_deref(), Some("abc"));
        assert_eq!(env.data.unwrap()["id"], "x");
    }
}
