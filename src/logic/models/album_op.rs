//! An album change the UI already shows but the server has not confirmed yet.
//!
//! Port of `Immagine/Models/AlbumOp.swift`. The Swift nested `AlbumOp.Kind` is `AlbumOpKind`
//! here. The `affectsAssetsOnly` extension lives in `Sync/AlbumOutbox.swift` in Swift; it is
//! a method on the kind here. The GRDB row mapping is left out (see the port plan).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlbumOpKind {
    Add,
    Remove,
    Trash,
    Restore,
    /// The favorites, sorted into like an album.
    Favorite,
    Unfavorite,
}

impl AlbumOpKind {
    /// Changes of the photos themselves, not of an album's links.
    pub fn affects_assets_only(self) -> bool {
        matches!(
            self,
            AlbumOpKind::Trash | AlbumOpKind::Restore | AlbumOpKind::Favorite | AlbumOpKind::Unfavorite
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumOp {
    /// Row id in the local queue, `None` before it is stored.
    #[serde(default)]
    pub id: Option<i64>,
    pub kind: AlbumOpKind,
    pub album_id: Option<String>,
    pub asset_ids: Vec<String>,
    #[serde(default)]
    pub attempts: u32,
}

impl AlbumOp {
    fn new(kind: AlbumOpKind, album_id: Option<String>, asset_ids: Vec<String>) -> Self {
        AlbumOp { id: None, kind, album_id, asset_ids, attempts: 0 }
    }

    pub fn add(ids: Vec<String>, to_album: &str) -> Self {
        Self::new(AlbumOpKind::Add, Some(to_album.to_string()), ids)
    }

    pub fn remove(ids: Vec<String>, from_album: &str) -> Self {
        Self::new(AlbumOpKind::Remove, Some(from_album.to_string()), ids)
    }

    pub fn trash(ids: Vec<String>) -> Self {
        Self::new(AlbumOpKind::Trash, None, ids)
    }

    pub fn restore(ids: Vec<String>) -> Self {
        Self::new(AlbumOpKind::Restore, None, ids)
    }

    pub fn favorite(ids: Vec<String>) -> Self {
        Self::new(AlbumOpKind::Favorite, None, ids)
    }

    pub fn unfavorite(ids: Vec<String>) -> Self {
        Self::new(AlbumOpKind::Unfavorite, None, ids)
    }

    /// The change that cancels this one (undo). A missing album becomes `""`, as in Swift.
    pub fn inverse(&self) -> AlbumOp {
        let ids = self.asset_ids.clone();
        let album = self.album_id.clone().unwrap_or_default();
        match self.kind {
            AlbumOpKind::Add => AlbumOp::remove(ids, &album),
            AlbumOpKind::Remove => AlbumOp::add(ids, &album),
            AlbumOpKind::Trash => AlbumOp::restore(ids),
            AlbumOpKind::Restore => AlbumOp::trash(ids),
            AlbumOpKind::Favorite => AlbumOp::unfavorite(ids),
            AlbumOpKind::Unfavorite => AlbumOp::favorite(ids),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The pure part of `testFavoriteOpsQueueLikeAlbumChanges` (the queue itself is store work).
    #[test]
    fn favorite_op_kind_and_inverse() {
        let fav = AlbumOp::favorite(ids(&["a"]));
        assert_eq!(fav.kind, AlbumOpKind::Favorite);
        assert!(fav.album_id.is_none());
        assert!(AlbumOpKind::Favorite.affects_assets_only());
        assert!(!AlbumOpKind::Add.affects_assets_only());
        assert_eq!(fav.inverse(), AlbumOp::unfavorite(ids(&["a"])));
        assert_eq!(AlbumOp::unfavorite(ids(&["b"])).kind, AlbumOpKind::Unfavorite);
    }

    #[test]
    fn add_and_remove_are_each_other_inverse() {
        let add = AlbumOp::add(ids(&["x"]), "in");
        let remove = add.inverse();
        assert_eq!(remove, AlbumOp::remove(ids(&["x"]), "in"));
        assert_eq!(remove.inverse(), AlbumOp::add(ids(&["x"]), "in"));
    }

    #[test]
    fn trash_and_restore_are_each_other_inverse() {
        assert_eq!(AlbumOp::trash(ids(&["x"])).inverse(), AlbumOp::restore(ids(&["x"])));
        assert_eq!(AlbumOp::restore(ids(&["x"])).inverse(), AlbumOp::trash(ids(&["x"])));
    }
}
