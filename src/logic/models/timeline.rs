//! Lightweight grid row for the timeline and album grids.
//!
//! Port of `Immagine/Models/Timeline.swift`. The grid reads these from indexed columns in
//! the store, so this is the only shape a scroll-time path should touch. `sort_date` is the
//! raw ISO string of the capture instant (parse it with `iso_date` for ordering).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    pub thumbhash: Option<String>,
    /// Width / height.
    pub ratio: Option<f64>,
    pub is_favorite: bool,
    pub is_trashed: bool,
    /// `YYYY-MM-DD`, empty when the asset has no capture date.
    pub date_bucket: String,
    pub sort_date: Option<String>,
    /// Display string ("0:12").
    pub duration: Option<String>,
}

impl TimelineItem {
    pub fn is_video(&self) -> bool {
        self.asset_type == "VIDEO"
    }

    pub fn is_image(&self) -> bool {
        !self.is_video()
    }
}
