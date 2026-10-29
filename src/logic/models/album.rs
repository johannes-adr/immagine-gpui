//! An album on the Immich server.
//!
//! Port of `Immagine/Models/Album.swift`. Offline state lives in the local store (`is_offline`)
//! and is merged by the UI, not carried on this wire model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub album_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub album_thumbnail_asset_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub asset_count: i64,
    pub shared: bool,
    pub has_shared_link: bool,
    #[serde(default)]
    pub order: Option<String>,
}

impl Album {
    pub fn display_name(&self) -> &str {
        &self.album_name
    }
}
