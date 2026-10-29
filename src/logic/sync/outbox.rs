//! Owner: B1. Swift source: `../immagine/Immagine/Sync/AlbumOutbox.swift`.
//! Spec: `../immagine/ImmagineTests/` (ModelTests, OfflineTests, FavoritesTests sync/outbox parts).
//!
//! Album changes from the sort mode: applied to the local store at once, kept in the store's
//! album-op queue, and sent to the server strictly in order. A transient error pauses the queue;
//! a permanent one (the server will never take it) undoes the change locally.
//!
//! Port of `AlbumOutbox`. The actor, the retry timer and the failure callback are not ported:
//! `flush` returns what happened (`FlushReport`) and the caller schedules the retry and shows
//! the messages. `apply` is the Swift `AlbumOutbox.apply` (infallible here).

use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::album_op::{AlbumOp, AlbumOpKind};
use crate::logic::ports::HttpTransport;
use crate::logic::store::Store;

/// Result of one `flush` pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlushReport {
    /// Ops the server confirmed (and removed from the queue).
    pub sent: usize,
    /// Messages for ops the server refused for good; they were undone locally.
    pub dropped: Vec<String>,
    /// Set when the queue paused on a transient error: seconds until the next attempt.
    pub retry_in_secs: Option<f64>,
}

/// Applies one op to the local store (Swift `AlbumOutbox.apply`). Applying `op.inverse()`
/// afterwards undoes it.
pub fn apply<S: Store>(op: &AlbumOp, store: &mut S) {
    let album = op.album_id.as_deref().unwrap_or("");
    match op.kind {
        AlbumOpKind::Add => store.add_album_assets(album, &op.asset_ids),
        AlbumOpKind::Remove => store.remove_album_assets(album, &op.asset_ids),
        AlbumOpKind::Trash => {
            store.trash_assets(&op.asset_ids);
            // The device photo must not come back with the next upload pass.
            store.exclude_local_assets(&op.asset_ids);
        }
        AlbumOpKind::Restore => {
            store.untrash_assets(&op.asset_ids);
            store.include_local_assets(&op.asset_ids);
        }
        AlbumOpKind::Favorite => store.set_favorite(&op.asset_ids, true),
        AlbumOpKind::Unfavorite => store.set_favorite(&op.asset_ids, false),
    }
}

/// Applies the ops locally and queues them in order (Swift `enqueue`). An op without an album
/// that is not asset-only is skipped. Nothing is sent here: call `flush`.
pub fn enqueue<S: Store>(store: &mut S, ops: &[AlbumOp], now: f64) {
    for op in ops {
        if op.album_id.is_none() && !op.kind.affects_assets_only() {
            continue;
        }
        apply(op, store);
        store.enqueue_album_op(op, now);
    }
}

/// Sends the queue in order. Stops at the first transient error (the op stays queued with its
/// attempt counted); a permanent error drops the op and undoes it locally.
pub async fn flush<S: Store, T: HttpTransport>(store: &mut S, client: &ImmichClient<T>, token: &str) -> FlushReport {
    let mut report = FlushReport::default();
    for op in store.pending_album_ops() {
        let Some(id) = op.id else { continue };
        match send(&op, client, token).await {
            Ok(()) => {
                store.delete_album_op(id);
                report.sent += 1;
            }
            Err(error) if is_permanent(&error) => {
                store.delete_album_op(id);
                apply(&op.inverse(), store);
                report.dropped.push(format!("Eine Änderung konnte nicht gespeichert werden: {error}"));
            }
            Err(error) => {
                store.mark_album_op_failed(id, &error.to_string());
                report.retry_in_secs = Some(backoff_secs(op.attempts + 1));
                return report;
            }
        }
    }
    report
}

async fn send<T: HttpTransport>(op: &AlbumOp, client: &ImmichClient<T>, token: &str) -> Result<(), ClientError> {
    let album = op.album_id.as_deref().unwrap_or("");
    match op.kind {
        // Per-asset "duplicate" results (already inside the album) count as done.
        AlbumOpKind::Add => client.add_assets(&op.asset_ids, album, token).await.map(|_| ()),
        AlbumOpKind::Remove => client.remove_assets(&op.asset_ids, album, token).await.map(|_| ()),
        AlbumOpKind::Trash => client.delete_assets(&op.asset_ids, token).await,
        AlbumOpKind::Restore => client.restore_assets(&op.asset_ids, token).await,
        AlbumOpKind::Favorite => client.set_favorite(&op.asset_ids, true, token).await,
        AlbumOpKind::Unfavorite => client.set_favorite(&op.asset_ids, false, token).await,
    }
}

/// Retry delay after `attempts` failures: 2, 4, 8 … seconds, at most 300 (Swift `scheduleRetry`).
pub fn backoff_secs(attempts: u32) -> f64 {
    (2.0 * 2f64.powi(attempts.min(8) as i32)).min(300.0)
}

/// Client errors the server will repeat for ever: 4xx except the ones that can change
/// (401 login, 408 timeout, 425 too early, 429 rate limit).
pub fn is_permanent(error: &ClientError) -> bool {
    match error {
        ClientError::Http(code, _) => (400..500).contains(code) && ![401, 408, 425, 429].contains(code),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from FavoritesTests and UploadTests (outbox parts); same inputs and expectations.

    use std::collections::BTreeSet;
    use std::rc::Rc;

    use futures_lite::future::block_on;

    use super::*;
    use crate::logic::models::sync::{SyncAlbum, SyncAsset};
    use crate::logic::ports::{HttpResponse, MockTransport};
    use crate::logic::store::MemoryStore;

    const BASE: &str = "http://immich.test:2284";
    const TOKEN: &str = "tok";

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn sync_asset(id: &str) -> SyncAsset {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": "IMAGE", "fileCreatedAt": "2026-10-01T20:17:30.923Z",
            "isFavorite": false, "visibility": "timeline", "originalFileName": format!("{id}.jpg"),
        }))
        .unwrap()
    }

    fn sync_album(id: &str, name: &str) -> SyncAlbum {
        serde_json::from_value(serde_json::json!({ "id": id, "name": name })).unwrap()
    }

    fn album_count(store: &MemoryStore, id: &str) -> i64 {
        store.albums().iter().find(|a| a.id == id).map(|a| a.asset_count).unwrap()
    }

    fn no_content() -> HttpResponse {
        HttpResponse::json(204, "")
    }

    /// `testOutboxAppliesLocallyAndInverseUndoes` (UploadTests).
    #[test]
    fn apply_and_inverse_move_undo_and_trash_restore() {
        let mut store = MemoryStore::new();
        store.upsert_sync_assets(&[sync_asset("x")]);
        store.upsert_sync_album(&sync_album("in", "Posteingang"));
        store.upsert_sync_album(&sync_album("t", "Reisen/Italien"));
        store.add_album_assets("in", &ids(&["x"]));
        store.upsert_local_asset("p", None, 0.0, false);
        store.mark_local_asset_uploaded("p", "x");

        // Move: into the target, out of the inbox.
        let move_ops = [AlbumOp::add(ids(&["x"]), "t"), AlbumOp::remove(ids(&["x"]), "in")];
        for op in &move_ops {
            apply(op, &mut store);
        }
        assert_eq!(store.album_ids_containing("x"), BTreeSet::from(["t".to_string()]));
        assert_eq!(album_count(&store, "in"), 0);
        // Undo in reverse order.
        for op in move_ops.iter().rev() {
            apply(&op.inverse(), &mut store);
        }
        assert_eq!(store.album_ids_containing("x"), BTreeSet::from(["in".to_string()]));

        // Trash hides the photo and stops its re-upload; restore brings both back.
        apply(&AlbumOp::trash(ids(&["x"])), &mut store);
        assert!(store.album_assets("in").is_empty());
        assert_eq!(store.local_asset("p").unwrap().upload_status, "excluded");
        apply(&AlbumOp::trash(ids(&["x"])).inverse(), &mut store);
        let shown: Vec<String> = store.album_assets("in").into_iter().map(|item| item.id).collect();
        assert_eq!(shown, ids(&["x"]));
        assert_eq!(store.local_asset("p").unwrap().upload_status, "uploaded");
    }

    /// `testSortingIntoFavoritesAndUndo` (FavoritesTests).
    #[test]
    fn sorting_into_favorites_and_undo() {
        let mut store = MemoryStore::new();
        store.upsert_sync_assets(&[sync_asset("x")]);
        store.upsert_sync_album(&sync_album("in", "Posteingang"));
        store.add_album_assets("in", &ids(&["x"]));

        let move_ops = [AlbumOp::favorite(ids(&["x"])), AlbumOp::remove(ids(&["x"]), "in")];
        for op in &move_ops {
            apply(op, &mut store);
        }
        assert!(store.is_favorite("x"));
        assert!(store.album_assets("in").is_empty());
        let favorites: Vec<String> = store.favorite_items().into_iter().map(|item| item.id).collect();
        assert_eq!(favorites, ids(&["x"]));

        for op in move_ops.iter().rev() {
            apply(&op.inverse(), &mut store);
        }
        assert!(!store.is_favorite("x"));
        let shown: Vec<String> = store.album_assets("in").into_iter().map(|item| item.id).collect();
        assert_eq!(shown, ids(&["x"]));
    }

    /// `testFavoriteOpsQueueLikeAlbumChanges` (FavoritesTests), through `enqueue`.
    #[test]
    fn favorite_ops_queue_like_album_changes() {
        let mut store = MemoryStore::new();
        enqueue(&mut store, &[AlbumOp::favorite(ids(&["a"])), AlbumOp::unfavorite(ids(&["b"]))], 1.0);
        let ops = store.pending_album_ops();
        assert_eq!(ops.iter().map(|op| op.kind).collect::<Vec<_>>(), [AlbumOpKind::Favorite, AlbumOpKind::Unfavorite]);
        assert!(ops[0].album_id.is_none());
        assert!(AlbumOpKind::Favorite.affects_assets_only());
        assert!(!AlbumOpKind::Add.affects_assets_only());
        assert_eq!(AlbumOp::favorite(ids(&["a"])).inverse(), AlbumOp::unfavorite(ids(&["a"])));
    }

    /// Ops without an album are only queued when they are asset-only (Swift `enqueue` filter).
    #[test]
    fn enqueue_skips_album_ops_without_album() {
        let mut store = MemoryStore::new();
        let orphan = AlbumOp { id: None, kind: AlbumOpKind::Add, album_id: None, asset_ids: ids(&["a"]), attempts: 0 };
        enqueue(&mut store, &[orphan], 1.0);
        assert!(store.pending_album_ops().is_empty());
    }

    /// `testOutboxQueuePersistsInOrder` (UploadTests).
    #[test]
    fn queue_persists_in_order_and_counts_attempts() {
        let mut store = MemoryStore::new();
        store.enqueue_album_op(&AlbumOp::add(ids(&["a", "b"]), "t"), 1.0);
        store.enqueue_album_op(&AlbumOp::remove(ids(&["a"]), "in"), 2.0);
        store.enqueue_album_op(&AlbumOp::trash(ids(&["c"])), 3.0);
        let mut ops = store.pending_album_ops();
        assert_eq!(ops.iter().map(|op| op.kind).collect::<Vec<_>>(), [AlbumOpKind::Add, AlbumOpKind::Remove, AlbumOpKind::Trash]);
        assert_eq!(ops[0].asset_ids, ids(&["a", "b"]));
        assert!(ops[2].album_id.is_none());

        let first = ops.remove(0).id.unwrap();
        store.mark_album_op_failed(first, "offline");
        assert_eq!(store.pending_album_ops()[0].attempts, 1);
        store.delete_album_op(first);
        assert_eq!(store.pending_album_ops().iter().map(|op| op.kind).collect::<Vec<_>>(), [AlbumOpKind::Remove, AlbumOpKind::Trash]);
    }

    /// `testOutboxRetriesOnlyTransientErrors` (UploadTests).
    #[test]
    fn only_refusals_are_permanent() {
        let http = |code| ClientError::Http(code, String::new());
        assert!(is_permanent(&http(400)));
        assert!(is_permanent(&http(404)));
        assert!(!is_permanent(&ClientError::Unauthorized), "a new login fixes it");
        assert!(!is_permanent(&http(401)));
        assert!(!is_permanent(&http(429)));
        assert!(!is_permanent(&http(503)));
        assert!(!is_permanent(&ClientError::Transport("offline".into())));
    }

    #[test]
    fn backoff_doubles_and_stops_at_five_minutes() {
        assert_eq!(backoff_secs(1), 4.0);
        assert_eq!(backoff_secs(2), 8.0);
        assert_eq!(backoff_secs(8), 300.0);
        assert_eq!(backoff_secs(50), 300.0);
    }

    #[test]
    fn flush_sends_in_order_and_empties_the_queue() {
        let mock = Rc::new(MockTransport::new());
        mock.push(HttpResponse::json(200, "[]")).push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();
        store.enqueue_album_op(&AlbumOp::add(ids(&["a"]), "t"), 1.0);
        store.enqueue_album_op(&AlbumOp::favorite(ids(&["a"])), 2.0);

        let report = block_on(flush(&mut store, &client, TOKEN));
        assert_eq!(report, FlushReport { sent: 2, dropped: vec![], retry_in_secs: None });
        assert!(store.pending_album_ops().is_empty());
        let requests = mock.requests.borrow();
        assert_eq!(requests[0].path, "/api/albums/t/assets");
        assert_eq!(requests[1].path, "/api/assets");
    }

    #[test]
    fn flush_pauses_on_a_transient_error_and_keeps_the_op() {
        let mock = Rc::new(MockTransport::new());
        mock.push(HttpResponse::json(503, "{\"message\":\"busy\"}"));
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();
        store.enqueue_album_op(&AlbumOp::favorite(ids(&["a"])), 1.0);
        store.enqueue_album_op(&AlbumOp::favorite(ids(&["b"])), 2.0);

        let report = block_on(flush(&mut store, &client, TOKEN));
        assert_eq!(report.sent, 0);
        assert_eq!(report.retry_in_secs, Some(4.0));
        let pending = store.pending_album_ops();
        assert_eq!(pending.len(), 2, "the queue pauses, nothing is skipped");
        assert_eq!(pending[0].attempts, 1);
        assert_eq!(mock.requests.borrow().len(), 1, "the second op is not sent while the first is failing");
    }

    #[test]
    fn flush_drops_a_refused_op_and_undoes_it_locally() {
        let mock = Rc::new(MockTransport::new());
        mock.push(HttpResponse::json(404, "{\"message\":\"Album not found\"}"));
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();
        store.upsert_sync_assets(&[sync_asset("x")]);
        store.upsert_sync_album(&sync_album("t", "Reisen"));
        let add = AlbumOp::add(ids(&["x"]), "t");
        apply(&add, &mut store);
        store.enqueue_album_op(&add, 1.0);
        assert!(store.album_ids_containing("x").contains("t"));

        let report = block_on(flush(&mut store, &client, TOKEN));
        assert_eq!(report.sent, 0);
        assert_eq!(report.retry_in_secs, None);
        assert_eq!(report.dropped.len(), 1);
        assert!(report.dropped[0].contains("Album not found"));
        assert!(store.pending_album_ops().is_empty());
        assert!(store.album_ids_containing("x").is_empty(), "the refused add is undone");
    }
}
