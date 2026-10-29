//! Owner: B1. Swift source: `../immagine/Immagine/Sync/SyncStream.swift`.
//! Spec: `../immagine/ImmagineTests/` (ModelTests, OfflineTests, FavoritesTests sync/outbox parts).
//!
//! Delta sync: `POST /api/sync/stream` answers with JSON-Lines change events, each with an `ack`
//! checkpoint. Processed events are confirmed with `ack_sync`; the next connect resumes after them.
//!
//! Port of `SyncStream`. The whole body arrives as one `Vec<u8>` (streaming is the browser layer's
//! job). Events are applied in order to the store. The Swift UI callbacks (coalesced, ~0.8 s) are
//! returned as data in `StreamReport`; the caller decides when to notify. Assets new to the store
//! get their full detail fetched (`GET /api/assets/{id}`), since the sync payload lacks some fields.
//!
//! `ImmichClient` has no stream method, so the request is built here (`stream_request`) and sent
//! through the `HttpTransport` the caller passes in.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::sync::{SyncAlbum, SyncAlbumAssetLink, SyncAsset, SyncEventEnvelope};
use crate::logic::ports::{HttpRequest, HttpTransport, Method, TransportError};
use crate::logic::store::Store;

/// Acks are sent in batches of this size during a long stream.
const ACK_BATCH: usize = 1000;

/// Entity types the stream is asked for (Swift `StreamRequest`).
const ENTITY_TYPES: [&str; 4] = ["AssetsV2", "AlbumsV2", "AlbumToAssetsV1", "AlbumAssetsV2"];

/// How the stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamResult {
    /// Ran to the end. `changes` counts the entity events (completion markers and acks excluded).
    Ok { changes: usize },
    /// The server demands a full sync (`SyncResetV1`). The acks before it were sent; the events
    /// after it were not read.
    Reset,
}

/// What one stream run did, for the caller to refresh its views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamReport {
    pub result: StreamResult,
    /// Asset rows were added, changed or removed.
    pub assets_changed: bool,
    /// Albums whose membership or existence changed.
    pub touched_albums: BTreeSet<String>,
    /// Assets whose rendered image changed (thumbhash or size), so cached thumbnails can go.
    pub changed_renditions: BTreeSet<String>,
    /// Events per wire type, completion markers included.
    pub event_counts: BTreeMap<String, usize>,
}

impl StreamReport {
    /// One log line for this run, see `summary`.
    pub fn summary(&self) -> String {
        summary(&self.event_counts)
    }
}

/// The request that opens the stream (Swift `SyncStream.run`, before the send).
pub fn stream_request(token: &str) -> HttpRequest {
    let body = serde_json::to_vec(&json!({ "types": ENTITY_TYPES })).unwrap_or_default();
    HttpRequest {
        method: Method::Post,
        path: "/api/sync/stream".to_string(),
        headers: vec![
            ("Authorization".to_string(), format!("Bearer {token}")),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        body: Some(body),
    }
}

/// `"12 events: AssetV2 ×10, SyncAckV1, SyncCompleteV1"` (Swift `SyncStream.summary`): the
/// types with the most events first, the count only where it is above one.
pub fn summary(counts: &BTreeMap<String, usize>) -> String {
    let total: usize = counts.values().sum();
    let mut kinds: Vec<(&String, &usize)> = counts.iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let types: Vec<String> = kinds
        .iter()
        .map(|(kind, count)| if **count > 1 { format!("{kind} ×{count}") } else { kind.to_string() })
        .collect();
    let mut text = format!("{total} event{}", if total == 1 { "" } else { "s" });
    if !types.is_empty() {
        text.push_str(": ");
        text.push_str(&types.join(", "));
    }
    text
}

/// Runs one stream session over the whole answer. Errors: network (`Transport`), a refused
/// login (`Unauthorized`), other non-2xx answers (`Http`), a failed ack.
pub async fn run<S: Store, T: HttpTransport, C: HttpTransport>(
    store: &mut S,
    transport: &T,
    client: &ImmichClient<C>,
    token: &str,
) -> Result<StreamReport, ClientError> {
    let response = transport.send(stream_request(token)).await.map_err(|error| match error {
        TransportError::Network(message) => ClientError::Transport(message),
    })?;
    if response.status == 401 {
        return Err(ClientError::Unauthorized);
    }
    if !(200..300).contains(&response.status) {
        return Err(ClientError::Http(response.status, "Sync-Stream".to_string()));
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    // Asset ids in the store, loaded with the first asset event: most runs bring no asset event.
    let mut known: Option<BTreeSet<String>> = None;
    let mut acks: Vec<String> = Vec::new();
    let mut assets_changed = false;
    let mut touched_albums: BTreeSet<String> = BTreeSet::new();
    let mut changed_renditions: BTreeSet<String> = BTreeSet::new();
    let mut reset = false;

    for line in response.body.split(|byte| *byte == b'\n') {
        let Some(envelope) = parse_line(line) else { continue };
        *counts.entry(envelope.event_type.clone()).or_insert(0) += 1;

        match envelope.event_type.as_str() {
            "AssetV2" | "AssetV1" | "AlbumAssetCreateV2" | "AlbumAssetUpdateV2" | "AlbumAssetCreateV1"
            | "AlbumAssetUpdateV1" => {
                // Per-album asset events upsert the assets too; the membership comes as AlbumToAssetV1.
                let assets: Vec<SyncAsset> = decode_all(envelope.data.as_ref());
                let known_ids = known.get_or_insert_with(|| store.asset_ids());
                let changed = apply_assets(store, client, token, &assets, known_ids).await;
                changed_renditions.extend(changed);
                assets_changed = true;
            }
            "AssetDeleteV1" => {
                let ids = decode_ids(envelope.data.as_ref(), "assetId");
                if !ids.is_empty() {
                    store.delete_assets(&ids);
                    if let Some(known_ids) = known.as_mut() {
                        for id in &ids {
                            known_ids.remove(id);
                        }
                    }
                    assets_changed = true;
                }
            }
            "AlbumV2" | "AlbumV1" => {
                for album in decode_all::<SyncAlbum>(envelope.data.as_ref()) {
                    store.upsert_sync_album(&album);
                }
            }
            "AlbumDeleteV1" => {
                for id in decode_ids(envelope.data.as_ref(), "albumId") {
                    store.delete_album(&id);
                    // Its originals may have to go from the iPhone.
                    touched_albums.insert(id);
                }
            }
            "AlbumToAssetV1" => {
                for link in decode_all::<SyncAlbumAssetLink>(envelope.data.as_ref()) {
                    store.add_album_assets(&link.album_id, std::slice::from_ref(&link.asset_id));
                    touched_albums.insert(link.album_id.clone());
                }
            }
            "AlbumToAssetDeleteV1" => {
                for link in decode_all::<SyncAlbumAssetLink>(envelope.data.as_ref()) {
                    store.remove_album_assets(&link.album_id, std::slice::from_ref(&link.asset_id));
                    touched_albums.insert(link.album_id.clone());
                }
            }
            "SyncResetV1" => {
                send_acks(client, token, &mut acks).await?;
                reset = true;
                break;
            }
            // Completion marker and checkpoint entity: nothing to apply.
            "SyncCompleteV1" | "SyncAckV1" => {}
            // user/partner/memory/stack/face/... entities: out of scope.
            _ => {}
        }

        if let Some(ack) = envelope.ack {
            acks.push(ack);
        }
        if acks.len() >= ACK_BATCH {
            send_acks(client, token, &mut acks).await?;
        }
    }

    let result = if reset {
        StreamResult::Reset
    } else {
        send_acks(client, token, &mut acks).await?;
        let total: usize = counts.values().sum();
        let markers = counts.get("SyncCompleteV1").copied().unwrap_or(0) + counts.get("SyncAckV1").copied().unwrap_or(0);
        StreamResult::Ok { changes: total - markers }
    };

    Ok(StreamReport {
        result,
        assets_changed,
        touched_albums,
        changed_renditions,
        event_counts: counts,
    })
}

/// Upserts the sync payloads and fetches the full detail of assets not yet in the store.
/// Returns the ids whose rendered image changed. A failed detail fetch is skipped, as in Swift.
async fn apply_assets<S: Store, C: HttpTransport>(
    store: &mut S,
    client: &ImmichClient<C>,
    token: &str,
    assets: &[SyncAsset],
    known: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    if assets.is_empty() {
        return BTreeSet::new();
    }
    let changed = store.upsert_sync_assets(assets);

    let mut details = Vec::new();
    for asset in assets {
        if known.contains(&asset.id) {
            continue;
        }
        known.insert(asset.id.clone());
        if let Ok(full) = client.asset(&asset.id, token).await {
            details.push(full);
        }
    }
    if !details.is_empty() {
        store.upsert_assets(&details);
    }
    changed
}

async fn send_acks<C: HttpTransport>(client: &ImmichClient<C>, token: &str, acks: &mut Vec<String>) -> Result<(), ClientError> {
    if acks.is_empty() {
        return Ok(());
    }
    client.ack_sync(acks, token).await?;
    acks.clear();
    Ok(())
}

/// One JSON-Lines entry. Blank and malformed lines are skipped, not fatal.
fn parse_line(line: &[u8]) -> Option<SyncEventEnvelope> {
    let text = std::str::from_utf8(line).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// Event data is one object or an array of objects (verified live: one object per event on v3.0.3).
fn payload_items(payload: Option<&Value>) -> Vec<&Value> {
    match payload {
        Some(Value::Array(list)) => list.iter().filter(|value| value.is_object()).collect(),
        Some(value @ Value::Object(_)) => vec![value],
        _ => Vec::new(),
    }
}

/// Decodes every item; items that do not decode are skipped.
fn decode_all<T: DeserializeOwned>(payload: Option<&Value>) -> Vec<T> {
    payload_items(payload)
        .into_iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

/// The string under `key` of every item.
fn decode_ids(payload: Option<&Value>, key: &str) -> Vec<String> {
    payload_items(payload)
        .into_iter()
        .filter_map(|value| value.get(key)?.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    //! Ported from ModelTests (`testSyncLogsOneSummary`). The stream tests are new: they replay
    //! realistic NDJSON through `MockTransport` (responses in request order).

    use std::rc::Rc;

    use futures_lite::future::block_on;

    use super::*;
    use crate::logic::ports::{HttpResponse, MockTransport};
    use crate::logic::store::MemoryStore;

    const BASE: &str = "http://immich.test:2284";
    const TOKEN: &str = "tok";

    fn counts(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn ndjson(lines: &[&str]) -> HttpResponse {
        HttpResponse { status: 200, headers: Vec::new(), body: lines.join("\n").into_bytes() }
    }

    fn no_content() -> HttpResponse {
        HttpResponse::json(204, "")
    }

    fn body_json(request: &HttpRequest) -> Value {
        serde_json::from_slice(request.body.as_deref().unwrap()).unwrap()
    }

    fn asset_detail(id: &str, thumbhash: &str) -> HttpResponse {
        HttpResponse::json(
            200,
            &json!({
                "id": id, "type": "IMAGE", "thumbhash": thumbhash, "originalPath": format!("/data/{id}.jpg"),
                "originalFileName": format!("{id}.jpg"), "fileCreatedAt": "2026-10-01T20:17:30.923Z",
                "width": 4000, "height": 6000
            })
            .to_string(),
        )
    }

    fn sync_asset_event(ack: &str, id: &str, thumbhash: &str) -> String {
        json!({
            "type": "AssetV2", "ack": ack,
            "data": { "id": id, "type": "IMAGE", "thumbhash": thumbhash, "fileCreatedAt": "2026-10-01T20:17:30.923Z",
                      "isFavorite": false, "visibility": "timeline", "width": 4000, "height": 6000 }
        })
        .to_string()
    }

    fn store_with_asset(id: &str, thumbhash: &str) -> MemoryStore {
        let mut store = MemoryStore::new();
        let asset: SyncAsset = serde_json::from_value(json!({
            "id": id, "type": "IMAGE", "thumbhash": thumbhash, "fileCreatedAt": "2026-10-01T20:17:30.923Z",
            "isFavorite": false, "visibility": "timeline", "width": 4000, "height": 6000
        }))
        .unwrap();
        store.upsert_sync_assets(&[asset]);
        store
    }

    /// `testSyncLogsOneSummary` (ModelTests).
    #[test]
    fn sync_stream_logs_one_summary() {
        assert_eq!(summary(&BTreeMap::new()), "0 events");
        assert_eq!(summary(&counts(&[("SyncCompleteV1", 1)])), "1 event: SyncCompleteV1");
        assert_eq!(
            summary(&counts(&[("AssetV2", 10), ("SyncAckV1", 1), ("SyncCompleteV1", 1)])),
            "12 events: AssetV2 ×10, SyncAckV1, SyncCompleteV1"
        );
    }

    #[test]
    fn stream_request_asks_for_the_entity_types() {
        let request = stream_request(TOKEN);
        assert_eq!(request.method, Method::Post);
        assert_eq!(request.path, "/api/sync/stream");
        assert!(request.headers.contains(&("Authorization".to_string(), "Bearer tok".to_string())));
        assert_eq!(body_json(&request), json!({ "types": ["AssetsV2", "AlbumsV2", "AlbumToAssetsV1", "AlbumAssetsV2"] }));
    }

    #[test]
    fn new_asset_is_stored_with_its_detail_and_acked() {
        let mock = Rc::new(MockTransport::new());
        mock.push(ndjson(&[
            &sync_asset_event("c1", "a1", "TH"),
            r#"{"type":"SyncCompleteV1","ack":"c2"}"#,
        ]))
        .push(asset_detail("a1", "TH"))
        .push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();

        let report = block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap();
        assert_eq!(report.result, StreamResult::Ok { changes: 1 });
        assert!(report.assets_changed);
        assert_eq!(store.asset("a1").unwrap().original_path.as_deref(), Some("/data/a1.jpg"));

        let requests = mock.requests.borrow();
        let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["/api/sync/stream", "/api/assets/a1", "/api/sync/ack"]);
        assert_eq!(body_json(&requests[2]), json!({ "acks": ["c1", "c2"] }));
    }

    #[test]
    fn known_asset_is_not_fetched_again_and_a_new_thumbhash_is_reported() {
        let mock = Rc::new(MockTransport::new());
        mock.push(ndjson(&[&sync_asset_event("c1", "a1", "NEW")])).push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = store_with_asset("a1", "OLD");

        let report = block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap();
        assert_eq!(report.changed_renditions, BTreeSet::from(["a1".to_string()]));
        let paths: Vec<String> = mock.requests.borrow().iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, ["/api/sync/stream", "/api/sync/ack"], "no detail fetch for a known asset");
    }

    #[test]
    fn deletes_and_album_links_are_applied() {
        let mock = Rc::new(MockTransport::new());
        mock.push(ndjson(&[
            r#"{"type":"AssetDeleteV1","ack":"d1","data":{"assetId":"gone"}}"#,
            r#"{"type":"AlbumToAssetDeleteV1","ack":"d2","data":[{"albumId":"al","assetId":"x"}]}"#,
            r#"{"type":"AlbumDeleteV1","ack":"d3","data":{"albumId":"old"}}"#,
            r#"{"type":"AlbumV2","ack":"d4","data":{"id":"al","name":"Reisen neu"}}"#,
            r#"{"type":"AlbumToAssetV1","ack":"d5","data":{"albumId":"al","assetId":"z"}}"#,
        ]))
        .push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = store_with_asset("gone", "T");
        for id in ["x", "y", "z"] {
            store.upsert_sync_assets(&[serde_json::from_value(json!({"id": id, "type": "IMAGE"})).unwrap()]);
        }
        store.upsert_sync_album(&serde_json::from_value(json!({"id": "al", "name": "Reisen"})).unwrap());
        store.upsert_sync_album(&serde_json::from_value(json!({"id": "old", "name": "Alt"})).unwrap());
        store.add_album_assets("al", &["x".to_string(), "y".to_string()]);

        let report = block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap();
        assert_eq!(report.result, StreamResult::Ok { changes: 5 });
        assert!(report.assets_changed);
        assert_eq!(report.touched_albums, BTreeSet::from(["al".to_string(), "old".to_string()]));
        assert!(store.asset("gone").is_none());
        assert!(!store.album_ids_containing("x").contains("al"));
        assert!(store.album_ids_containing("z").contains("al"));
        let album = store.albums().into_iter().find(|a| a.id == "al").unwrap();
        assert_eq!(album.album_name, "Reisen neu");
        assert!(store.albums().iter().all(|a| a.id != "old"));
    }

    #[test]
    fn reset_acks_what_came_before_and_stops_reading() {
        let mock = Rc::new(MockTransport::new());
        mock.push(ndjson(&[
            &sync_asset_event("r1", "a1", "TH"),
            r#"{"type":"SyncResetV1","ack":"r2"}"#,
            &sync_asset_event("r3", "b1", "TH"),
        ]))
        .push(asset_detail("a1", "TH"))
        .push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();

        let report = block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap();
        assert_eq!(report.result, StreamResult::Reset);
        assert!(store.asset("a1").is_some());
        assert!(store.asset("b1").is_none(), "events after the reset are not applied");
        let requests = mock.requests.borrow();
        assert_eq!(requests.len(), 3);
        assert_eq!(body_json(&requests[2]), json!({ "acks": ["r1"] }));
    }

    #[test]
    fn refused_login_and_server_errors_fail_the_run() {
        let mock = Rc::new(MockTransport::new());
        mock.push(HttpResponse::json(401, "{}")).push(HttpResponse::json(500, "{}"));
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();

        assert_eq!(block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap_err(), ClientError::Unauthorized);
        assert_eq!(
            block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap_err(),
            ClientError::Http(500, "Sync-Stream".to_string())
        );
    }

    #[test]
    fn malformed_and_blank_lines_are_skipped() {
        let mock = Rc::new(MockTransport::new());
        mock.push(ndjson(&["not json", "", r#"{"type":"SyncCompleteV1","ack":"m1"}"#, r#"{"ack":"no type"}"#]))
            .push(no_content());
        let client = ImmichClient::new(BASE, Rc::clone(&mock)).unwrap();
        let mut store = MemoryStore::new();

        let report = block_on(run(&mut store, &*mock, &client, TOKEN)).unwrap();
        assert_eq!(report.result, StreamResult::Ok { changes: 0 });
        assert_eq!(report.summary(), "1 event: SyncCompleteV1");
        assert_eq!(body_json(&mock.requests.borrow()[1]), json!({ "acks": ["m1"] }));
    }
}
