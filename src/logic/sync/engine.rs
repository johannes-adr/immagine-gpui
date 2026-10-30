//! Owner: B1. Swift source: `../immagine/Immagine/Sync/SyncEngine.swift`.
//! Spec: `../immagine/ImmagineTests/` (ModelTests, OfflineTests, FavoritesTests sync/outbox parts).
//!
//! One sync run: send the album outbox, do a full sync when the store asks for one (or the stream
//! demands a reset), then read the delta stream. Checkpoints live on the server (acks); the local
//! flags are `needs_full_sync` and `last_sync` in the store's meta table.
//!
//! Port of `SyncEngine` (`fullSync`), the run loop of `SyncStream` and the outbox calls of the app
//! state. Not ported: `repairAlbumLinks` (a self-heal for an old local bug; it can be added on top of
//! `album_assets` / `merge_album_assets` later).
//!
//! `SyncEngine` is generic over the store, the transport and the clock. The transport is shared
//! between the client and the stream request through `Rc` (single-threaded, no `Send` needed).

use std::collections::BTreeSet;
use std::rc::Rc;

use super::outbox::{self, FlushReport};
use super::stream::{self, StreamReport, StreamResult};
use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::album_op::AlbumOp;
use crate::logic::ports::{Clock, HttpRequest, HttpResponse, HttpTransport, TransportError};
use crate::logic::store::Store;

const NEEDS_FULL_SYNC: &str = "needs_full_sync";
const LAST_SYNC: &str = "last_sync";
const PAGE_SIZE: u32 = 1000;
/// Safety net against a server that keeps answering with the same page (Swift `guardCounter`).
const MAX_PAGES: u32 = 1000;

/// `Rc` makes one transport usable by the client and the stream request. Lives here because
/// the engine is the only place that needs it.
impl<T: HttpTransport> HttpTransport for Rc<T> {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        (**self).send(request).await
    }
}

/// What one `sync` call did.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncRun {
    /// A full sync ran (asked for by the store, or by a stream reset).
    pub full_sync: bool,
    /// Outcome of the outbox pass that ran first.
    pub outbox: FlushReport,
    /// Outcome of the (last) stream pass.
    pub stream: StreamReport,
    /// Assets whose rendered image changed in this run (full sync and stream together).
    pub changed_renditions: BTreeSet<String>,
}

pub struct SyncEngine<S: Store, T: HttpTransport, C: Clock> {
    store: S,
    transport: Rc<T>,
    client: ImmichClient<Rc<T>>,
    clock: C,
}

impl<S: Store, T: HttpTransport, C: Clock> SyncEngine<S, T, C> {
    /// `base_url` as typed by the user, e.g. `http://10.0.0.44:2284`.
    pub fn new(store: S, transport: T, clock: C, base_url: &str) -> Result<Self, ClientError> {
        let transport = Rc::new(transport);
        let client = ImmichClient::new(base_url, Rc::clone(&transport))?;
        Ok(Self { store, transport, client, clock })
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// True while the store asks for a full sync (a fresh store does).
    pub fn needs_full_sync(&self) -> bool {
        self.store.meta(NEEDS_FULL_SYNC).as_deref() == Some("1")
    }

    /// Applies album changes locally and queues them. Nothing is sent until the next `sync`.
    pub fn enqueue(&mut self, ops: &[AlbumOp]) {
        let now = self.clock.now_ms() as f64 / 1000.0;
        outbox::enqueue(&mut self.store, ops, now);
    }

    /// One sync run: outbox, then full sync if needed, then the stream. A stream reset triggers
    /// one full sync and one more stream pass; a second reset is left for the next run.
    /// Errors stop the run; the queue and the store keep what they have.
    pub async fn sync(&mut self, token: &str) -> Result<SyncRun, ClientError> {
        let outbox = outbox::flush(&mut self.store, &self.client, token).await;
        let mut changed_renditions = BTreeSet::new();
        let mut full_sync = false;

        if self.needs_full_sync() {
            changed_renditions.extend(self.full_sync(token).await?);
            full_sync = true;
        }
        let mut streamed = stream::run(&mut self.store, &*self.transport, &self.client, token).await?;
        changed_renditions.extend(streamed.changed_renditions.iter().cloned());

        if streamed.result == StreamResult::Reset {
            self.store.set_meta(NEEDS_FULL_SYNC, "1");
            changed_renditions.extend(self.full_sync(token).await?);
            full_sync = true;
            streamed = stream::run(&mut self.store, &*self.transport, &self.client, token).await?;
            changed_renditions.extend(streamed.changed_renditions.iter().cloned());
        }

        Ok(SyncRun { full_sync, outbox, stream: streamed, changed_renditions })
    }

    /// Pulls the whole catalog through `search/metadata` pages and the album list, and drops what
    /// the server no longer has. Stamps `last_sync` and clears `needs_full_sync`. Returns the ids
    /// whose rendered image changed.
    pub async fn full_sync(&mut self, token: &str) -> Result<BTreeSet<String>, ClientError> {
        // Each page goes into the store as it arrives: the work is split into page-sized slices
        // (the awaited fetch between them lets the browser paint) instead of one 26k-asset burst.
        let mut server_ids: BTreeSet<String> = BTreeSet::new();
        let mut changed = BTreeSet::new();
        let mut page = 1u32;
        for _ in 0..MAX_PAGES {
            let response = self.client.search_assets(page, PAGE_SIZE, token).await?;
            let next = response.assets.next_page.as_deref().and_then(|n| n.parse::<u32>().ok());
            let has_items = !response.assets.items.is_empty();
            server_ids.extend(response.assets.items.iter().map(|asset| asset.id.clone()));
            changed.extend(self.store.upsert_assets(&response.assets.items));
            match next {
                Some(next_page) if has_items => page = next_page,
                _ => break,
            }
        }
        // A complete listing is the truth: drop what the server no longer has (deletes the stream
        // missed, e.g. after a reset).
        let stale: Vec<String> = self.store.asset_ids().difference(&server_ids).cloned().collect();
        self.store.delete_assets(&stale);

        let albums = self.client.albums(token).await?;
        self.store.upsert_albums(&albums);
        let server_album_ids: BTreeSet<String> = albums.iter().map(|album| album.id.clone()).collect();
        let stale_albums: Vec<String> = self
            .store
            .albums()
            .into_iter()
            .map(|album| album.id)
            .filter(|id| !server_album_ids.contains(id))
            .collect();
        for id in &stale_albums {
            self.store.delete_album(id);
        }

        let now_secs = self.clock.now_ms() / 1000;
        self.store.set_meta(LAST_SYNC, &now_secs.to_string());
        self.store.set_meta(NEEDS_FULL_SYNC, "0");
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    //! Orchestration tests. Responses are queued in request order on `MockTransport`.

    use futures_lite::future::block_on;
    use serde_json::json;

    use super::*;
    use crate::logic::models::album_op::AlbumOpKind;
    use crate::logic::ports::{FixedClock, HttpResponse, MockTransport};
    use crate::logic::store::MemoryStore;

    const BASE: &str = "http://immich.test:2284";
    const TOKEN: &str = "tok";
    const NOW_MS: i64 = 1_790_000_000_000;

    fn engine() -> SyncEngine<MemoryStore, MockTransport, FixedClock> {
        SyncEngine::new(MemoryStore::new(), MockTransport::new(), FixedClock(NOW_MS), BASE).unwrap()
    }

    fn ndjson(lines: &[&str]) -> HttpResponse {
        HttpResponse { status: 200, headers: Vec::new(), body: lines.join("\n").into_bytes() }
    }

    fn search_page(ids: &[&str]) -> HttpResponse {
        let items: Vec<_> = ids
            .iter()
            .map(|id| json!({ "id": id, "type": "IMAGE", "thumbhash": "TH", "fileCreatedAt": "2026-10-01T20:17:30.923Z",
                              "originalPath": format!("/data/{id}.jpg"), "width": 4000, "height": 6000 }))
            .collect();
        HttpResponse::json(200, &json!({ "assets": { "total": items.len(), "count": items.len(), "items": items, "nextPage": null } }).to_string())
    }

    fn albums(list: &[(&str, &str)]) -> HttpResponse {
        let items: Vec<_> = list
            .iter()
            .map(|(id, name)| json!({ "id": id, "albumName": name, "assetCount": 0, "shared": false, "hasSharedLink": false }))
            .collect();
        HttpResponse::json(200, &serde_json::Value::Array(items).to_string())
    }

    fn no_content() -> HttpResponse {
        HttpResponse::json(204, "")
    }

    fn paths(engine: &SyncEngine<MemoryStore, MockTransport, FixedClock>) -> Vec<(crate::logic::ports::Method, String)> {
        engine.transport().requests.borrow().iter().map(|r| (r.method.clone(), r.path.clone())).collect()
    }

    /// A fresh store asks for a full sync: catalog, albums, then the stream; stale rows go.
    #[test]
    fn first_run_does_a_full_sync_then_streams() {
        let mut engine = engine();
        engine.store_mut().upsert_sync_assets(&[serde_json::from_value(json!({"id": "old", "type": "IMAGE"})).unwrap()]);
        engine.transport().push(search_page(&["a1"])).push(albums(&[])).push(ndjson(&[
            r#"{"type":"SyncCompleteV1","ack":"e1"}"#,
        ])).push(no_content());

        let run = block_on(engine.sync(TOKEN)).unwrap();
        assert!(run.full_sync);
        assert_eq!(run.stream.result, StreamResult::Ok { changes: 0 });
        assert_eq!(run.outbox, FlushReport::default());
        assert!(engine.store().asset("a1").is_some());
        assert!(engine.store().asset("old").is_none(), "the server no longer has it");
        assert_eq!(engine.store().meta("needs_full_sync").as_deref(), Some("0"));
        assert_eq!(engine.store().meta("last_sync").as_deref(), Some("1790000000"));
        assert!(!engine.needs_full_sync());
        assert_eq!(
            paths(&engine).iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(),
            ["/api/search/metadata", "/api/albums", "/api/sync/stream", "/api/sync/ack"]
        );
    }

    /// Once the store is up to date, a run is the stream alone; queued album changes go first.
    #[test]
    fn incremental_run_flushes_the_outbox_then_streams() {
        let mut engine = engine();
        engine.store_mut().set_meta(NEEDS_FULL_SYNC, "0");
        engine.store_mut().upsert_sync_assets(&[serde_json::from_value(json!({"id": "a1", "type": "IMAGE"})).unwrap()]);
        engine.enqueue(&[AlbumOp::favorite(vec!["a1".to_string()])]);
        assert!(engine.store().is_favorite("a1"), "applied at once, before the server hears of it");
        engine.transport().push(no_content()).push(ndjson(&[r#"{"type":"SyncCompleteV1","ack":"i1"}"#])).push(no_content());

        let run = block_on(engine.sync(TOKEN)).unwrap();
        assert!(!run.full_sync);
        assert_eq!(run.outbox.sent, 1);
        assert!(engine.store().pending_album_ops().is_empty());
        assert_eq!(
            paths(&engine),
            [
                (crate::logic::ports::Method::Put, "/api/assets".to_string()),
                (crate::logic::ports::Method::Post, "/api/sync/stream".to_string()),
                (crate::logic::ports::Method::Post, "/api/sync/ack".to_string()),
            ]
        );
    }

    /// A reset in the stream: the store is marked, one full sync runs, and the stream is read again.
    #[test]
    fn reset_triggers_a_full_sync_and_a_second_stream() {
        let mut engine = engine();
        engine.store_mut().set_meta(NEEDS_FULL_SYNC, "0");
        engine.transport().push(ndjson(&[r#"{"type":"SyncResetV1","ack":"x1"}"#]))
            .push(search_page(&["a1"]))
            .push(albums(&[]))
            .push(ndjson(&[r#"{"type":"SyncCompleteV1","ack":"y1"}"#]))
            .push(no_content());

        let run = block_on(engine.sync(TOKEN)).unwrap();
        assert!(run.full_sync);
        assert_eq!(run.stream.result, StreamResult::Ok { changes: 0 });
        assert!(engine.store().asset("a1").is_some());
        assert_eq!(engine.store().meta("needs_full_sync").as_deref(), Some("0"));
        assert_eq!(
            paths(&engine).iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(),
            ["/api/sync/stream", "/api/search/metadata", "/api/albums", "/api/sync/stream", "/api/sync/ack"]
        );
    }

    /// A transient outbox error pauses the queue and reports the retry; the stream still runs.
    #[test]
    fn transient_outbox_error_is_reported_and_the_stream_still_runs() {
        let mut engine = engine();
        engine.store_mut().set_meta(NEEDS_FULL_SYNC, "0");
        engine.store_mut().upsert_sync_assets(&[serde_json::from_value(json!({"id": "a1", "type": "IMAGE"})).unwrap()]);
        engine.enqueue(&[AlbumOp::favorite(vec!["a1".to_string()])]);
        engine.transport().push(HttpResponse::json(503, "{}")).push(ndjson(&[r#"{"type":"SyncCompleteV1"}"#]));

        let run = block_on(engine.sync(TOKEN)).unwrap();
        assert_eq!(run.outbox.sent, 0);
        assert_eq!(run.outbox.retry_in_secs, Some(4.0));
        assert_eq!(engine.store().pending_album_ops().len(), 1);
        assert_eq!(engine.store().pending_album_ops()[0].kind, AlbumOpKind::Favorite);
        assert_eq!(run.stream.result, StreamResult::Ok { changes: 0 });
    }

    /// A network failure ends the run with an error; nothing is lost from the queue.
    #[test]
    fn offline_run_is_an_error_and_keeps_the_queue() {
        let mut engine = engine();
        engine.store_mut().set_meta(NEEDS_FULL_SYNC, "0");
        engine.enqueue(&[AlbumOp::trash(vec!["a1".to_string()])]);
        engine.transport().push_error(TransportError::Network("offline".into()));
        engine.transport().push_error(TransportError::Network("offline".into()));

        let result = block_on(engine.sync(TOKEN));
        assert!(matches!(result, Err(ClientError::Transport(_))));
        assert_eq!(engine.store().pending_album_ops().len(), 1);
    }
}
