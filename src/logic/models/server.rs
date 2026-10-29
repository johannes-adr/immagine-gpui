//! Server, auth and user payloads, plus the paginated search response.
//!
//! Port of `Immagine/Models/Server.swift`.

use serde::{Deserialize, Serialize};

use super::asset::Asset;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerVersion {
    pub major: i64,
    pub minor: i64,
    pub patch: i64,
    #[serde(default)]
    pub prerelease: Option<String>,
}

impl ServerVersion {
    /// "v3.0.3" or "v3.0.3-rc.1".
    pub fn display(&self) -> String {
        let base = format!("v{}.{}.{}", self.major, self.minor, self.patch);
        match &self.prerelease {
            Some(pre) => format!("{base}-{pre}"),
            None => base,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
    #[serde(default)]
    pub should_change_password: Option<bool>,
    #[serde(default)]
    pub profile_image_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub profile_image_path: Option<String>,
    #[serde(default)]
    pub avatar_color: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
}

/// `POST /api/search/metadata` response: a paginated asset listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMetadataResponse {
    pub assets: SearchPage<Asset>,
}

/// One page of a search result. Generic so the same shape serves other listings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage<T> {
    pub total: i64,
    pub count: i64,
    pub items: Vec<T>,
    #[serde(default)]
    pub next_page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetStatistics {
    #[serde(default)]
    pub images: Option<i64>,
    #[serde(default)]
    pub videos: Option<i64>,
    #[serde(default)]
    pub total: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `testAssetPageWithMixedRowsDecodes` (ModelTests): one odd row must not fail the page.
    #[test]
    fn asset_page_with_mixed_rows_decodes() {
        let json = r#"{"assets":{"total":2,"count":2,"nextPage":null,"items":[
          {"id":"a","type":"IMAGE","duration":null,"isFavorite":true,"isTrashed":false},
          {"id":"b","type":"VIDEO","duration":"0:00:05.000000","isFavorite":false,"isTrashed":false,"exifInfo":{"iso":"bogus"}}]}}"#;
        let page: SearchMetadataResponse = serde_json::from_str(json).unwrap();
        assert_eq!(page.assets.items.len(), 2);
        assert_eq!(page.assets.items[1].duration.as_deref(), Some("0:05"));
        assert!(page.assets.items[1].exif_info.is_none(), "an unreadable exif block is dropped, not fatal");
    }

    #[test]
    fn server_version_display() {
        let v = ServerVersion { major: 3, minor: 0, patch: 3, prerelease: None };
        assert_eq!(v.display(), "v3.0.3");
        let pre = ServerVersion { prerelease: Some("rc.1".into()), ..v };
        assert_eq!(pre.display(), "v3.0.3-rc.1");
    }
}
