//! Owner: A6. Swift source: `../immagine/Immagine/Net/ImmichClient.swift`.
//! Spec: `../immagine/ImmagineTests/` -> MockTransport with real JSON; CoreTests, UploadTests (bulk check).
//! Port of the stateless Immich REST client: request building (path, query, JSON body, bearer token),
//! error mapping and the typed responses. Transport is the `HttpTransport` seam from `ports`.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::json;
use super::models::album::Album;
use super::models::asset::Asset;
use super::models::asset_edit::{AssetEdit, AssetEditsRequest, AssetEditsResponse};
use super::models::person::{AssetFace, Person, PersonUpdate};
use super::models::server::{AuthResponse, SearchMetadataResponse, ServerVersion, User};
use super::ports::{HttpRequest, HttpResponse, HttpTransport, Method, TransportError};

/// Errors of the Immich client. The Display texts are Swift's `APIError` (German).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The server address is not an http(s) origin with a valid host and port.
    BadUrl,
    /// Non-2xx answer (other than 401): status code and the server's message or the status text.
    Http(u16, String),
    /// The answer could not be read as the expected JSON.
    Decoding(String),
    /// 401: the login is refused or has expired.
    Unauthorized,
    /// No answer at all (offline, DNS, aborted).
    Transport(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::BadUrl => write!(f, "Ungültige Server-Adresse."),
            ClientError::Http(code, message) => write!(f, "Serverfehler {code}: {message}"),
            ClientError::Decoding(message) => write!(f, "Antwort konnte nicht gelesen werden: {message}"),
            ClientError::Unauthorized => write!(f, "Anmeldung fehlgeschlagen oder abgelaufen."),
            ClientError::Transport(message) => write!(f, "Netzwerkfehler: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A photo on the map (`MapMarkerResponseDto`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapMarker {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
}

/// One entry of a bulk response (`BulkIdResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkIdResult {
    pub id: String,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// One page of people (`PeopleResponseDto`), hidden ones included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleResponse {
    pub people: Vec<Person>,
    pub total: i64,
    pub hidden: i64,
    #[serde(default)]
    pub has_next_page: Option<bool>,
}

/// Answer of `POST /assets/bulk-upload-check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkUploadCheckResponse {
    #[serde(default)]
    pub results: Option<Vec<BulkUploadCheckResult>>,
    #[serde(default)]
    pub ignored: Option<Vec<String>>,
}

impl BulkUploadCheckResponse {
    /// Ids the server wants uploaded (`accept`).
    pub fn needed(&self) -> Vec<String> {
        self.ids_with_action("accept")
    }

    /// Ids the server already has (`reject`).
    pub fn existing(&self) -> Vec<String> {
        self.ids_with_action("reject")
    }

    fn ids_with_action(&self, action: &str) -> Vec<String> {
        self.results
            .iter()
            .flatten()
            .filter(|result| result.action.as_deref() == Some(action))
            .map(|result| result.id.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkUploadCheckResult {
    pub id: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub asset_id: Option<String>,
    /// The existing duplicate lies in the server's trash.
    #[serde(default)]
    pub is_trashed: Option<bool>,
}

/// Stateless client for the Immich v3 REST API. The auth token is passed per call, as in
/// Swift. Only scheme, host and port of the base URL are used (Swift's `makeURL` drops the
/// base path too), and every request path is server-relative (`/api/...`) for the transport.
pub struct ImmichClient<T: HttpTransport> {
    transport: T,
    origin: Origin,
}

impl<T: HttpTransport> ImmichClient<T> {
    /// `base_url` as typed by the user, e.g. `http://10.0.0.44:2284`.
    pub fn new(base_url: &str, transport: T) -> Result<Self, ClientError> {
        Ok(Self { transport, origin: Origin::parse(base_url)? })
    }

    // MARK: - Auth & server

    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, ClientError> {
        let body = json_body(&LoginBody { email, password })?;
        self.fetch_json(request(Method::Post, "/api/auth/login", None, Some(body))).await
    }

    pub async fn server_version(&self) -> Result<ServerVersion, ClientError> {
        self.fetch_json(request(Method::Get, "/api/server/version", None, None)).await
    }

    pub async fn user_me(&self, token: &str) -> Result<User, ClientError> {
        self.fetch_json(request(Method::Get, "/api/users/me", Some(token), None)).await
    }

    // MARK: - Assets

    pub async fn search_assets(&self, page: u32, size: u32, token: &str) -> Result<SearchMetadataResponse, ClientError> {
        let body = json_body(&SearchBody { page, size, with_deleted: false, with_archived: true })?;
        self.fetch_json(request(Method::Post, "/api/search/metadata", Some(token), Some(body))).await
    }

    /// `POST /api/search/smart`: natural-language (CLIP) search. `kind` is `IMAGE` or `VIDEO` to restrict the media type.
    pub async fn smart_search(&self, query: &str, kind: Option<&str>, page: u32, size: u32, token: &str) -> Result<SearchMetadataResponse, ClientError> {
        let body = json_body(&TextSearchBody { query: Some(query), original_file_name: None, kind, page, size, with_archived: true })?;
        self.fetch_json(request(Method::Post, "/api/search/smart", Some(token), Some(body))).await
    }

    /// `POST /api/search/metadata` filtered by (part of) the original file name.
    pub async fn filename_search(&self, name: &str, kind: Option<&str>, page: u32, size: u32, token: &str) -> Result<SearchMetadataResponse, ClientError> {
        let body = json_body(&TextSearchBody { query: None, original_file_name: Some(name), kind, page, size, with_archived: true })?;
        self.fetch_json(request(Method::Post, "/api/search/metadata", Some(token), Some(body))).await
    }

    pub async fn asset(&self, id: &str, token: &str) -> Result<Asset, ClientError> {
        self.fetch_json(request(Method::Get, format!("/api/assets/{}", encode(id)), Some(token), None)).await
    }

    // MARK: - Albums

    pub async fn albums(&self, token: &str) -> Result<Vec<Album>, ClientError> {
        self.fetch_json(request(Method::Get, "/api/albums", Some(token), None)).await
    }

    pub async fn create_album(&self, name: &str, asset_ids: &[String], token: &str) -> Result<Album, ClientError> {
        let body = json_body(&CreateAlbumBody { album_name: name, asset_ids })?;
        self.fetch_json(request(Method::Post, "/api/albums", Some(token), Some(body))).await
    }

    pub async fn rename_album(&self, id: &str, name: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&serde_json::json!({ "albumName": name }))?;
        self.fetch(request(Method::Patch, format!("/api/albums/{}", encode(id)), Some(token), Some(body))).await.map(|_| ())
    }

    /// The photo on the album's tile.
    pub async fn set_album_thumbnail(&self, id: &str, asset_id: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&serde_json::json!({ "albumThumbnailAssetId": asset_id }))?;
        self.fetch(request(Method::Patch, format!("/api/albums/{}", encode(id)), Some(token), Some(body))).await.map(|_| ())
    }

    pub async fn delete_album(&self, id: &str, token: &str) -> Result<(), ClientError> {
        self.fetch(request(Method::Delete, format!("/api/albums/{}", encode(id)), Some(token), None)).await.map(|_| ())
    }

    /// Adds assets to an album; ids already in it come back as `duplicate`.
    pub async fn add_assets(&self, ids: &[String], album_id: &str, token: &str) -> Result<Vec<BulkIdResult>, ClientError> {
        let body = json_body(&Ids { ids })?;
        let path = format!("/api/albums/{}/assets", encode(album_id));
        self.fetch_json(request(Method::Put, path, Some(token), Some(body))).await
    }

    pub async fn remove_assets(&self, ids: &[String], album_id: &str, token: &str) -> Result<Vec<BulkIdResult>, ClientError> {
        let body = json_body(&Ids { ids })?;
        let path = format!("/api/albums/{}/assets", encode(album_id));
        self.fetch_json(request(Method::Delete, path, Some(token), Some(body))).await
    }

    pub async fn album_assets(&self, album_id: &str, page: u32, size: u32, token: &str) -> Result<SearchMetadataResponse, ClientError> {
        let body = json_body(&AlbumSearchBody { album_ids: vec![album_id], page, size, with_deleted: false })?;
        self.fetch_json(request(Method::Post, "/api/search/metadata", Some(token), Some(body))).await
    }

    // MARK: - Binary downloads

    /// `edited=true` returns the server-side edit when one exists, the unedited rendition otherwise.
    pub async fn thumbnail(&self, id: &str, size: &str, token: &str) -> Result<Vec<u8>, ClientError> {
        let path = with_query(format!("/api/assets/{}/thumbnail", encode(id)), &[("size", size), ("edited", "true")]);
        self.fetch(request(Method::Get, path, Some(token), None)).await.map(|r| r.body)
    }

    /// The unedited preview: the editor's canvas, which the crop parameters refer to.
    pub async fn unedited_preview(&self, id: &str, token: &str) -> Result<Vec<u8>, ClientError> {
        let path = with_query(format!("/api/assets/{}/thumbnail", encode(id)), &[("size", "preview"), ("edited", "false")]);
        self.fetch(request(Method::Get, path, Some(token), None)).await.map(|r| r.body)
    }

    /// Absolute video stream URL. It carries no token: Swift authorizes it with the
    /// `immich_access_token` cookie, which the browser has to supply.
    pub fn video_playback_url(&self, id: &str) -> String {
        self.origin.url(&format!("/api/assets/{}/video/playback", encode(id)))
    }

    /// The original (the edited rendition when one exists). Swift checks this download without
    /// its body, so an error message is the status text, never the server's `message`.
    pub async fn original(&self, id: &str, token: &str) -> Result<Vec<u8>, ClientError> {
        let path = with_query(format!("/api/assets/{}/original", encode(id)), &[("edited", "true")]);
        let response = self.exchange(request(Method::Get, path, Some(token), None)).await?;
        check_status(response.status, &[])?;
        Ok(response.body)
    }

    // MARK: - Upload

    /// `(id, checksum)` per local file; the answer says which ones the server needs.
    pub async fn bulk_upload_check(&self, assets: &[(String, String)], token: &str) -> Result<BulkUploadCheckResponse, ClientError> {
        let items = assets
            .iter()
            .map(|(id, checksum)| BulkCheckItem { id: id.as_str(), checksum: checksum.as_str() })
            .collect::<Vec<_>>();
        let body = json_body(&BulkCheckBody { assets: items })?;
        self.fetch_json(request(Method::Post, "/api/assets/bulk-upload-check", Some(token), Some(body))).await
    }

    // MARK: - Sync

    /// Confirms processed sync-stream events; the server resumes right after the last checkpoint.
    pub async fn ack_sync(&self, acks: &[String], token: &str) -> Result<(), ClientError> {
        let body = json_body(&AckBody { acks })?;
        self.fetch(request(Method::Post, "/api/sync/ack", Some(token), Some(body))).await.map(|_| ())
    }

    /// Moves assets to the server trash (restorable there for 30 days).
    pub async fn delete_assets(&self, ids: &[String], token: &str) -> Result<(), ClientError> {
        let body = json_body(&IdsForce { ids, force: false })?;
        self.fetch(request(Method::Delete, "/api/assets", Some(token), Some(body))).await.map(|_| ())
    }

    /// Moves albums, favorite, shared links, stack and sidecar of `source` over to `target`.
    pub async fn copy_asset_metadata(&self, source: &str, target: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&CopyBody { source_id: source, target_id: target })?;
        self.fetch(request(Method::Put, "/api/assets/copy", Some(token), Some(body))).await.map(|_| ())
    }

    pub async fn restore_assets(&self, ids: &[String], token: &str) -> Result<(), ClientError> {
        let body = json_body(&Ids { ids })?;
        self.fetch(request(Method::Post, "/api/trash/restore/assets", Some(token), Some(body))).await.map(|_| ())
    }

    pub async fn set_favorite(&self, ids: &[String], favorite: bool, token: &str) -> Result<(), ClientError> {
        let body = json_body(&IdsFavorite { ids, is_favorite: favorite })?;
        self.fetch(request(Method::Put, "/api/assets", Some(token), Some(body))).await.map(|_| ())
    }

    /// `"archive"` or `"timeline"` (`PUT /api/assets`, `visibility`).
    pub async fn set_visibility(&self, ids: &[String], visibility: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&IdsVisibility { ids, visibility })?;
        self.fetch(request(Method::Put, "/api/assets", Some(token), Some(body))).await.map(|_| ())
    }

    /// Moves the photo elsewhere; Immich geocodes the place names itself.
    pub async fn set_location(&self, id: &str, latitude: f64, longitude: f64, token: &str) -> Result<Asset, ClientError> {
        let body = json_body(&LocationBody { latitude, longitude })?;
        let path = format!("/api/assets/{}", encode(id));
        self.fetch_json(request(Method::Put, path, Some(token), Some(body))).await
    }

    // MARK: - Map

    /// Every own photo with a place.
    pub async fn map_markers(&self, token: &str) -> Result<Vec<MapMarker>, ClientError> {
        self.fetch_json(request(Method::Get, "/api/map/markers", Some(token), None)).await
    }

    // MARK: - Edits

    pub async fn asset_edits(&self, id: &str, token: &str) -> Result<Vec<AssetEdit>, ClientError> {
        let path = format!("/api/assets/{}/edits", encode(id));
        let response: AssetEditsResponse = self.fetch_json(request(Method::Get, path, Some(token), None)).await?;
        Ok(response.edits)
    }

    /// Replaces all edits of the asset; the server re-renders its thumbnails.
    pub async fn edit_asset(&self, id: &str, edits: &[AssetEdit], token: &str) -> Result<(), ClientError> {
        let body = json_body(&AssetEditsRequest { edits: edits.to_vec() })?;
        let path = format!("/api/assets/{}/edits", encode(id));
        self.fetch(request(Method::Put, path, Some(token), Some(body))).await.map(|_| ())
    }

    pub async fn remove_asset_edits(&self, id: &str, token: &str) -> Result<(), ClientError> {
        let path = format!("/api/assets/{}/edits", encode(id));
        self.fetch(request(Method::Delete, path, Some(token), None)).await.map(|_| ())
    }

    // MARK: - People & faces

    /// One page of people, hidden ones included (the app filters them).
    pub async fn people(&self, page: u32, size: u32, token: &str) -> Result<PeopleResponse, ClientError> {
        let (page, size) = (page.to_string(), size.to_string());
        let path = with_query("/api/people".into(), &[("withHidden", "true"), ("page", page.as_str()), ("size", size.as_str())]);
        self.fetch_json(request(Method::Get, path, Some(token), None)).await
    }

    pub async fn person(&self, id: &str, token: &str) -> Result<Person, ClientError> {
        self.fetch_json(request(Method::Get, format!("/api/people/{}", encode(id)), Some(token), None)).await
    }

    /// The face crop the server keeps for a person (JPEG).
    pub async fn person_thumbnail(&self, id: &str, token: &str) -> Result<Vec<u8>, ClientError> {
        self.fetch(request(Method::Get, format!("/api/people/{}/thumbnail", encode(id)), Some(token), None)).await.map(|r| r.body)
    }

    pub async fn update_person(&self, id: &str, update: &PersonUpdate, token: &str) -> Result<Person, ClientError> {
        let body = json_body(update)?;
        self.fetch_json(request(Method::Put, format!("/api/people/{}", encode(id)), Some(token), Some(body))).await
    }

    pub async fn create_person(&self, name: &str, token: &str) -> Result<Person, ClientError> {
        let body = json_body(&NameBody { name })?;
        self.fetch_json(request(Method::Post, "/api/people", Some(token), Some(body))).await
    }

    /// Moves all faces of `others` to `into`; the others are deleted.
    pub async fn merge_people(&self, into: &str, others: &[String], token: &str) -> Result<Vec<BulkIdResult>, ClientError> {
        let body = json_body(&Ids { ids: others })?;
        let path = format!("/api/people/{}/merge", encode(into));
        self.fetch_json(request(Method::Post, path, Some(token), Some(body))).await
    }

    /// A person's photos.
    pub async fn person_assets(&self, person_id: &str, page: u32, size: u32, token: &str) -> Result<SearchMetadataResponse, ClientError> {
        let body = json_body(&PersonSearchBody { person_ids: vec![person_id], page, size })?;
        self.fetch_json(request(Method::Post, "/api/search/metadata", Some(token), Some(body))).await
    }

    pub async fn faces(&self, asset_id: &str, token: &str) -> Result<Vec<AssetFace>, ClientError> {
        let path = with_query("/api/faces".into(), &[("id", asset_id)]);
        self.fetch_json(request(Method::Get, path, Some(token), None)).await
    }

    /// Gives the face to another person. Immich takes the person in the path and the face in the body.
    pub async fn reassign_face(&self, face_id: &str, person_id: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&FaceIdBody { id: face_id })?;
        self.fetch(request(Method::Put, format!("/api/faces/{}", encode(person_id)), Some(token), Some(body))).await.map(|_| ())
    }

    /// Removes the face from the photo (soft delete: face detection does not bring it back).
    pub async fn delete_face(&self, face_id: &str, token: &str) -> Result<(), ClientError> {
        let body = json_body(&Force { force: false })?;
        self.fetch(request(Method::Delete, format!("/api/faces/{}", encode(face_id)), Some(token), Some(body))).await.map(|_| ())
    }

    // MARK: - Transport plumbing

    async fn exchange(&self, request: HttpRequest) -> Result<HttpResponse, ClientError> {
        self.transport.send(request).await.map_err(|error| match error {
            TransportError::Network(message) => ClientError::Transport(message),
        })
    }

    /// Sends and checks the status; the body of a successful answer is returned.
    async fn fetch(&self, request: HttpRequest) -> Result<HttpResponse, ClientError> {
        let response = self.exchange(request).await?;
        check_status(response.status, &response.body)?;
        Ok(response)
    }

    async fn fetch_json<R: DeserializeOwned>(&self, request: HttpRequest) -> Result<R, ClientError> {
        let response = self.fetch(request).await?;
        json::decode(&response.body).map_err(|error| ClientError::Decoding(error.to_string()))
    }
}

/// Scheme and authority (`scheme://host[:port]`) of the server: what Swift's `makeURL` keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    scheme: String,
    authority: String,
}

impl Origin {
    fn parse(base: &str) -> Result<Self, ClientError> {
        let (scheme, rest) = base.split_once("://").ok_or(ClientError::BadUrl)?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(ClientError::BadUrl);
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let authority = authority.rsplit('@').next().unwrap_or_default();
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => {
                let (inside, after) = v6.split_once(']').ok_or(ClientError::BadUrl)?;
                let port = match after {
                    "" => None,
                    _ => Some(after.strip_prefix(':').ok_or(ClientError::BadUrl)?),
                };
                (format!("[{inside}]"), port)
            }
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host.to_string(), Some(port)),
                None => (authority.to_string(), None),
            },
        };
        if host.is_empty() || host == "[]" {
            return Err(ClientError::BadUrl);
        }
        if let Some(port) = port {
            port.parse::<u16>().map_err(|_| ClientError::BadUrl)?;
        }
        let authority = match port {
            Some(port) => format!("{host}:{port}"),
            None => host,
        };
        Ok(Self { scheme, authority })
    }

    fn url(&self, path: &str) -> String {
        format!("{}://{}{}", self.scheme, self.authority, path)
    }
}

fn request(method: Method, path: impl Into<String>, token: Option<&str>, body: Option<Vec<u8>>) -> HttpRequest {
    let mut headers = Vec::new();
    if let Some(token) = token {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    if body.is_some() {
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
    }
    HttpRequest { method, path: path.into(), headers, body }
}

fn json_body<B: Serialize>(value: &B) -> Result<Vec<u8>, ClientError> {
    json::encode(value).map_err(|error| ClientError::Decoding(format!("Anfrage nicht kodierbar: {error}")))
}

/// Non-2xx answers: 401 is `Unauthorized`; anything else carries the server's string `message`,
/// or the status text when the body has none (or `message` is not a string, as with validation arrays).
fn check_status(status: u16, body: &[u8]) -> Result<(), ClientError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    if status == 401 {
        return Err(ClientError::Unauthorized);
    }
    let message = server_message(body).unwrap_or_else(|| status_text(status).to_string());
    Err(ClientError::Http(status, message))
}

fn server_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.get("message")?.as_str().map(str::to_string)
}

/// English reason phrases, lowercase like Apple's `HTTPURLResponse.localizedString(forStatusCode:)`.
fn status_text(status: u16) -> &'static str {
    match status {
        400 => "bad request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not found",
        405 => "method not allowed",
        409 => "conflict",
        415 => "unsupported media type",
        429 => "too many requests",
        500 => "internal server error",
        502 => "bad gateway",
        503 => "service unavailable",
        504 => "gateway timeout",
        _ => "unknown error",
    }
}

/// Percent-encodes everything outside the RFC 3986 unreserved set.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Appends `?k=v&...` (encoded) to a path; no query, no question mark.
fn with_query(path: String, query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return path;
    }
    let pairs: Vec<String> = query.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
    format!("{path}?{}", pairs.join("&"))
}

// MARK: - Request bodies (private, as in Swift)

#[derive(Serialize)]
struct LoginBody<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchBody {
    page: u32,
    size: u32,
    with_deleted: bool,
    with_archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextSearchBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_file_name: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    page: u32,
    size: u32,
    with_archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumSearchBody<'a> {
    album_ids: Vec<&'a str>,
    page: u32,
    size: u32,
    with_deleted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonSearchBody<'a> {
    person_ids: Vec<&'a str>,
    page: u32,
    size: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateAlbumBody<'a> {
    album_name: &'a str,
    asset_ids: &'a [String],
}

#[derive(Serialize)]
struct Ids<'a> {
    ids: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdsFavorite<'a> {
    ids: &'a [String],
    is_favorite: bool,
}

#[derive(Serialize)]
struct IdsVisibility<'a> {
    ids: &'a [String],
    visibility: &'a str,
}

#[derive(Serialize)]
struct IdsForce<'a> {
    ids: &'a [String],
    force: bool,
}

#[derive(Serialize)]
struct Force {
    force: bool,
}

#[derive(Serialize)]
struct AckBody<'a> {
    acks: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CopyBody<'a> {
    source_id: &'a str,
    target_id: &'a str,
}

#[derive(Serialize)]
struct LocationBody {
    latitude: f64,
    longitude: f64,
}

#[derive(Serialize)]
struct FaceIdBody<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct NameBody<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct BulkCheckItem<'a> {
    id: &'a str,
    checksum: &'a str,
}

#[derive(Serialize)]
struct BulkCheckBody<'a> {
    assets: Vec<BulkCheckItem<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::models::asset_edit::MirrorAxis;
    use crate::logic::ports::MockTransport;
    use futures_lite::future::block_on;
    use serde_json::{json, Value};

    const BASE: &str = "http://10.0.0.44:2284";
    const ASSET: &str = "a0437345-fed4-439f-adbf-f3bd38e1cff4";
    const TOKEN: &str = "t0k";

    /// Real v3.0.3 asset payload (same fixture as CoreTests).
    const ASSET_JSON: &str = r#"{"id":"a0437345-fed4-439f-adbf-f3bd38e1cff4","type":"IMAGE",
     "originalPath":"/data/upload/foo.jpg","originalFileName":"a.jpg",
     "originalMimeType":"image/jpeg","thumbhash":"GikKDQK6iGBbGKaHd3uniPN8M/p2",
     "fileCreatedAt":"2026-10-01T20:17:30.923Z","localDateTime":"2026-10-01T20:17:30.923Z",
     "updatedAt":"2026-10-01T20:17:44.383Z","isFavorite":false,"isArchived":false,
     "isTrashed":false,"visibility":"timeline","duration":null,"checksum":"IciyZg02A2tDHvuXe3ldf71pzOY=",
     "isOffline":false,"hasMetadata":true,"resized":true,"width":5648,"height":8472,"isEdited":false}"#;

    const PERSON_JSON: &str = r#"{"id":"p1","name":"Anna","birthDate":"1990-05-17","isHidden":false,
     "isFavorite":true,"updatedAt":"2026-10-01T20:17:44.383Z"}"#;

    fn client_with(response: HttpResponse) -> ImmichClient<MockTransport> {
        let transport = MockTransport::new();
        transport.push(response);
        ImmichClient::new(BASE, transport).unwrap()
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse::json(200, body)
    }

    /// The single request the client sent.
    fn sent(client: &ImmichClient<MockTransport>) -> HttpRequest {
        let requests = client.transport.requests.borrow();
        assert_eq!(requests.len(), 1, "exactly one request per call");
        requests[0].clone()
    }

    fn body_json(req: &HttpRequest) -> Value {
        serde_json::from_slice(req.body.as_deref().expect("request has a body")).unwrap()
    }

    fn header(req: &HttpRequest, name: &str) -> Option<String> {
        req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    fn assert_bearer(req: &HttpRequest, token: &str) {
        assert_eq!(header(req, "Authorization").as_deref(), Some(format!("Bearer {token}").as_str()));
    }

    // MARK: - URL, base and encoding

    #[test]
    fn base_url_keeps_scheme_host_and_port() {
        let client = ImmichClient::new("https://immich.example.com:2443", MockTransport::new()).unwrap();
        assert_eq!(
            client.video_playback_url(ASSET),
            "https://immich.example.com:2443/api/assets/a0437345-fed4-439f-adbf-f3bd38e1cff4/video/playback"
        );
    }

    #[test]
    fn bad_base_url_is_rejected() {
        for bad in ["10.0.0.44:2284", "ftp://immich.local", "http://", "http://host:port", ""] {
            assert!(matches!(ImmichClient::new(bad, MockTransport::new()), Err(ClientError::BadUrl)), "{bad}");
        }
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(with_query("/api/x".into(), &[("q", "a b&c/ü")]), "/api/x?q=a%20b%26c%2F%C3%BC");
        assert_eq!(with_query("/api/x".into(), &[]), "/api/x", "no query, no question mark");
    }

    #[test]
    fn video_playback_url_is_absolute_and_sends_nothing() {
        let client = client_with(ok("{}"));
        assert_eq!(client.video_playback_url(ASSET), format!("{BASE}/api/assets/{ASSET}/video/playback"));
        assert!(client.transport.requests.borrow().is_empty(), "a URL needs no request");
    }

    // MARK: - Auth & server

    #[test]
    fn login_posts_credentials_without_authorization() {
        let client = client_with(ok(r#"{"accessToken":"eyJhbGciOiJIUzI1NiJ9.e30.sig","userId":"74e9e10f","userEmail":"anna@example.com","name":"Anna","isAdmin":true,"shouldChangePassword":false,"profileImagePath":"","isOnboarded":true}"#));
        let auth = block_on(client.login("anna@example.com", "geheim")).unwrap();
        assert_eq!(auth.access_token.as_deref(), Some("eyJhbGciOiJIUzI1NiJ9.e30.sig"));
        assert_eq!(auth.user_email.as_deref(), Some("anna@example.com"));

        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/auth/login");
        assert_eq!(header(&req, "Authorization"), None);
        assert_eq!(header(&req, "Content-Type").as_deref(), Some("application/json"));
        assert_eq!(body_json(&req), json!({"email": "anna@example.com", "password": "geheim"}));
    }

    #[test]
    fn login_maps_401_to_unauthorized() {
        let client = client_with(HttpResponse::json(401, r#"{"message":"Incorrect email or password","error":"Unauthorized","statusCode":401}"#));
        let err = block_on(client.login("a@b.c", "x")).unwrap_err();
        assert_eq!(err, ClientError::Unauthorized);
        assert_eq!(err.to_string(), "Anmeldung fehlgeschlagen oder abgelaufen.");
    }

    #[test]
    fn server_error_message_comes_from_the_body() {
        let client = client_with(HttpResponse::json(500, r#"{"message":"Database is down"}"#));
        let err = block_on(client.server_version()).unwrap_err();
        assert_eq!(err, ClientError::Http(500, "Database is down".into()));
        assert_eq!(err.to_string(), "Serverfehler 500: Database is down");
    }

    #[test]
    fn array_message_and_non_json_body_fall_back_to_status_text() {
        // Immich sends validation errors with an array in `message`; Swift's decode fails then and
        // the localized status text is used instead.
        let client = client_with(HttpResponse::json(400, r#"{"message":["email must be an email"],"error":"Bad Request","statusCode":400}"#));
        assert_eq!(block_on(client.server_version()).unwrap_err(), ClientError::Http(400, "bad request".into()));

        let proxy = client_with(HttpResponse { status: 502, headers: vec![], body: b"<html>".to_vec() });
        assert_eq!(block_on(proxy.server_version()).unwrap_err(), ClientError::Http(502, "bad gateway".into()));
    }

    #[test]
    fn transport_failure_is_reported_as_transport_error() {
        let transport = MockTransport::new();
        transport.push_error(TransportError::Network("offline".into()));
        let client = ImmichClient::new(BASE, transport).unwrap();
        let err = block_on(client.server_version()).unwrap_err();
        assert_eq!(err, ClientError::Transport("offline".into()));
        assert_eq!(err.to_string(), "Netzwerkfehler: offline");
    }

    #[test]
    fn unreadable_success_body_is_a_decoding_error() {
        let client = client_with(ok("{"));
        assert!(matches!(block_on(client.server_version()), Err(ClientError::Decoding(_))));
    }

    #[test]
    fn server_version_gets_version_path_without_auth() {
        let client = client_with(ok(r#"{"major":3,"minor":0,"patch":3}"#));
        let version = block_on(client.server_version()).unwrap();
        assert_eq!(version.display(), "v3.0.3");
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/server/version");
        assert!(req.headers.is_empty());
        assert!(req.body.is_none());
    }

    #[test]
    fn user_me_sends_bearer_token_and_no_body() {
        let client = client_with(ok(r#"{"id":"74e9e10f-5d8e-4343-97eb-bef11df33d34","email":"anna@example.com","name":"Anna","profileImagePath":"","avatarColor":"primary","isAdmin":true}"#));
        let user = block_on(client.user_me(TOKEN)).unwrap();
        assert_eq!(user.name.as_deref(), Some("Anna"));
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/users/me");
        assert_bearer(&req, TOKEN);
        assert!(req.body.is_none());
        assert_eq!(header(&req, "Content-Type"), None, "no body, no content type");
    }

    // MARK: - Assets

    #[test]
    fn search_assets_posts_paged_metadata_request() {
        let client = client_with(ok(&format!(
            r#"{{"assets":{{"total":1,"count":1,"nextPage":null,"items":[{ASSET_JSON}]}}}}"#
        )));
        let page = block_on(client.search_assets(1, 250, TOKEN)).unwrap();
        assert_eq!(page.assets.items[0].id, ASSET);
        assert_eq!(page.assets.next_page, None);

        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/search/metadata");
        assert_bearer(&req, TOKEN);
        assert_eq!(body_json(&req), json!({"page": 1, "size": 250, "withDeleted": false, "withArchived": true}));
    }

    #[test]
    fn asset_fetches_one_asset_by_id() {
        let client = client_with(ok(ASSET_JSON));
        let asset = block_on(client.asset(ASSET, TOKEN)).unwrap();
        assert_eq!(asset.checksum.as_deref(), Some("IciyZg02A2tDHvuXe3ldf71pzOY="));
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, format!("/api/assets/{ASSET}"));
        assert_bearer(&req, TOKEN);
    }

    // MARK: - Albums

    const ALBUM_JSON: &str = r#"[{"id":"al1","albumName":"Urlaub","description":"","albumThumbnailAssetId":"a0437345-fed4-439f-adbf-f3bd38e1cff4","createdAt":"2026-10-01T20:17:38.359Z","updatedAt":"2026-10-01T20:17:44.383Z","assetCount":12,"shared":false,"hasSharedLink":false,"order":"desc"}]"#;

    #[test]
    fn albums_decodes_the_album_list() {
        let client = client_with(ok(ALBUM_JSON));
        let albums = block_on(client.albums(TOKEN)).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].album_name, "Urlaub");
        assert_eq!(albums[0].asset_count, 12);
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/albums");
        assert_bearer(&req, TOKEN);
    }

    #[test]
    fn create_album_posts_name_and_asset_ids() {
        let client = client_with(ok(&ALBUM_JSON[1..ALBUM_JSON.len() - 1]));
        let ids = vec![ASSET.to_string()];
        let album = block_on(client.create_album("Urlaub", &ids, TOKEN)).unwrap();
        assert_eq!(album.id, "al1");
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/albums");
        assert_eq!(body_json(&req), json!({"albumName": "Urlaub", "assetIds": [ASSET]}));
    }

    #[test]
    fn delete_album_sends_delete_without_body() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.delete_album("al1", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.path, "/api/albums/al1");
        assert!(req.body.is_none());
    }

    #[test]
    fn add_assets_puts_ids_and_reports_duplicates() {
        let client = client_with(ok(r#"[{"id":"a1","success":true},{"id":"a2","success":false,"error":"duplicate"}]"#));
        let ids = vec!["a1".to_string(), "a2".to_string()];
        let results = block_on(client.add_assets(&ids, "al1", TOKEN)).unwrap();
        assert!(results[0].success);
        assert_eq!(results[1].error.as_deref(), Some("duplicate"));
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/albums/al1/assets");
        assert_eq!(body_json(&req), json!({"ids": ["a1", "a2"]}));
    }

    #[test]
    fn remove_assets_deletes_with_ids_body() {
        let client = client_with(ok(r#"[{"id":"a1","success":true}]"#));
        let ids = vec!["a1".to_string()];
        assert!(block_on(client.remove_assets(&ids, "al1", TOKEN)).unwrap()[0].success);
        let req = sent(&client);
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.path, "/api/albums/al1/assets");
        assert_eq!(body_json(&req), json!({"ids": ["a1"]}));
    }

    #[test]
    fn album_assets_searches_by_album_id() {
        let client = client_with(ok(r#"{"assets":{"total":0,"count":0,"nextPage":null,"items":[]}}"#));
        let page = block_on(client.album_assets("al1", 2, 100, TOKEN)).unwrap();
        assert!(page.assets.items.is_empty());
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/search/metadata");
        assert_eq!(body_json(&req), json!({"albumIds": ["al1"], "page": 2, "size": 100, "withDeleted": false}));
    }

    // MARK: - Binary downloads

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    #[test]
    fn thumbnail_requests_edited_size_and_returns_bytes() {
        let client = client_with(HttpResponse { status: 200, headers: vec![], body: JPEG.to_vec() });
        let bytes = block_on(client.thumbnail(ASSET, "thumbnail", TOKEN)).unwrap();
        assert_eq!(bytes, JPEG);
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, format!("/api/assets/{ASSET}/thumbnail?size=thumbnail&edited=true"));
        assert_bearer(&req, TOKEN);
    }

    #[test]
    fn unedited_preview_requests_the_original_rendition() {
        let client = client_with(HttpResponse { status: 200, headers: vec![], body: JPEG.to_vec() });
        assert_eq!(block_on(client.unedited_preview(ASSET, TOKEN)).unwrap(), JPEG);
        assert_eq!(sent(&client).path, format!("/api/assets/{ASSET}/thumbnail?size=preview&edited=false"));
    }

    #[test]
    fn original_downloads_the_edited_file() {
        let client = client_with(HttpResponse { status: 200, headers: vec![], body: JPEG.to_vec() });
        assert_eq!(block_on(client.original(ASSET, TOKEN)).unwrap(), JPEG);
        let req = sent(&client);
        assert_eq!(req.path, format!("/api/assets/{ASSET}/original?edited=true"));
        assert_bearer(&req, TOKEN);
    }

    #[test]
    fn original_error_uses_status_text_not_the_body_message() {
        // Swift's download passes no body to its status check, so the message is the status text.
        let client = client_with(HttpResponse::json(404, r#"{"message":"Not found or no asset.read access"}"#));
        assert_eq!(block_on(client.original(ASSET, TOKEN)).unwrap_err(), ClientError::Http(404, "not found".into()));
    }

    // MARK: - Upload

    #[test]
    fn bulk_upload_check_sends_checksums_and_splits_results() {
        let client = client_with(ok(r#"{"results":[{"id":"new1","action":"accept"},{"id":"old1","action":"reject","reason":"duplicate","assetId":"a0","isTrashed":true}],"ignored":["x"]}"#));
        let assets = vec![("new1".to_string(), "c1=".to_string()), ("old1".to_string(), "c2=".to_string())];
        let response = block_on(client.bulk_upload_check(&assets, TOKEN)).unwrap();
        assert_eq!(response.needed(), vec!["new1".to_string()]);
        assert_eq!(response.existing(), vec!["old1".to_string()]);
        assert_eq!(response.ignored.as_deref(), Some(&["x".to_string()][..]));
        assert_eq!(response.results.as_ref().unwrap()[1].is_trashed, Some(true));

        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/assets/bulk-upload-check");
        assert_bearer(&req, TOKEN);
        assert_eq!(body_json(&req), json!({"assets": [
            {"id": "new1", "checksum": "c1="}, {"id": "old1", "checksum": "c2="}
        ]}));
    }

    #[test]
    fn bulk_upload_check_without_results_needs_nothing() {
        let client = client_with(ok("{}"));
        let response = block_on(client.bulk_upload_check(&[], TOKEN)).unwrap();
        assert!(response.needed().is_empty());
        assert!(response.existing().is_empty());
    }

    // MARK: - Sync, trash, favorites, location

    #[test]
    fn ack_sync_posts_the_acknowledged_checkpoints() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.ack_sync(&["c1".to_string(), "c2".to_string()], TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/sync/ack");
        assert_eq!(body_json(&req), json!({"acks": ["c1", "c2"]}));
    }

    #[test]
    fn delete_assets_moves_to_trash_without_force() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.delete_assets(&[ASSET.to_string()], TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.path, "/api/assets");
        assert_eq!(body_json(&req), json!({"ids": [ASSET], "force": false}));
    }

    #[test]
    fn copy_asset_metadata_puts_source_and_target() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.copy_asset_metadata("orig", "edit", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/assets/copy");
        assert_eq!(body_json(&req), json!({"sourceId": "orig", "targetId": "edit"}));
    }

    #[test]
    fn restore_assets_posts_ids_to_trash_restore() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.restore_assets(&[ASSET.to_string()], TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/trash/restore/assets");
        assert_eq!(body_json(&req), json!({"ids": [ASSET]}));
    }

    #[test]
    fn set_favorite_puts_the_flag_for_all_ids() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.set_favorite(&[ASSET.to_string()], true, TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/assets");
        assert_eq!(body_json(&req), json!({"ids": [ASSET], "isFavorite": true}));
    }

    #[test]
    fn set_visibility_puts_the_visibility_for_all_ids() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.set_visibility(&[ASSET.to_string()], "archive", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/assets");
        assert_eq!(body_json(&req), json!({"ids": [ASSET], "visibility": "archive"}));
    }

    #[test]
    fn set_location_puts_coordinates_and_returns_the_asset() {
        let client = client_with(ok(ASSET_JSON));
        let asset = block_on(client.set_location(ASSET, 52.52, 13.405, TOKEN)).unwrap();
        assert_eq!(asset.id, ASSET);
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, format!("/api/assets/{ASSET}"));
        assert_eq!(body_json(&req), json!({"latitude": 52.52, "longitude": 13.405}));
    }

    #[test]
    fn map_markers_decodes_places() {
        let client = client_with(ok(r#"[{"id":"a1","lat":52.52,"lon":13.405,"city":"Berlin","state":"Berlin","country":"Germany"},{"id":"a2","lat":48.1,"lon":11.6,"city":null,"state":null,"country":null}]"#));
        let markers = block_on(client.map_markers(TOKEN)).unwrap();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].city.as_deref(), Some("Berlin"));
        assert_eq!(markers[1].city, None);
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/map/markers");
        assert_bearer(&req, TOKEN);
    }

    // MARK: - Edits

    #[test]
    fn asset_edits_skips_unknown_actions() {
        let client = client_with(ok(r#"{"edits":[{"action":"crop","parameters":{"x":100,"y":200,"width":3000,"height":4000}},{"action":"rotate","parameters":{"angle":90}},{"action":"futureAction","parameters":{}}]}"#));
        let edits = block_on(client.asset_edits(ASSET, TOKEN)).unwrap();
        assert_eq!(edits, vec![
            AssetEdit::Crop { x: 100, y: 200, width: 3000, height: 4000 },
            AssetEdit::Rotate { angle: 90 },
        ]);
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, format!("/api/assets/{ASSET}/edits"));
    }

    #[test]
    fn edit_asset_puts_the_whole_edit_list() {
        let client = client_with(HttpResponse::json(204, ""));
        let edits = [
            AssetEdit::Crop { x: 100, y: 200, width: 3000, height: 4000 },
            AssetEdit::Rotate { angle: 90 },
            AssetEdit::Mirror { axis: MirrorAxis::Horizontal },
        ];
        block_on(client.edit_asset(ASSET, &edits, TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, format!("/api/assets/{ASSET}/edits"));
        assert_eq!(body_json(&req), json!({"edits": [
            {"action": "crop", "parameters": {"x": 100, "y": 200, "width": 3000, "height": 4000}},
            {"action": "rotate", "parameters": {"angle": 90.0}},
            {"action": "mirror", "parameters": {"axis": "horizontal"}},
        ]}));
    }

    #[test]
    fn remove_asset_edits_deletes_them() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.remove_asset_edits(ASSET, TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.path, format!("/api/assets/{ASSET}/edits"));
        assert!(req.body.is_none());
    }

    // MARK: - People & faces

    #[test]
    fn people_query_includes_hidden_people_and_paging() {
        let client = client_with(ok(&format!(r#"{{"people":[{PERSON_JSON}],"total":2,"hidden":1,"hasNextPage":false}}"#)));
        let response = block_on(client.people(1, 1000, TOKEN)).unwrap();
        assert_eq!(response.people[0].name, "Anna");
        assert_eq!(response.total, 2);
        assert_eq!(response.hidden, 1);
        assert_eq!(response.has_next_page, Some(false));
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/people?withHidden=true&page=1&size=1000");
        assert_bearer(&req, TOKEN);
    }

    #[test]
    fn person_fetches_one_person() {
        let client = client_with(ok(PERSON_JSON));
        let person = block_on(client.person("p1", TOKEN)).unwrap();
        assert!(person.is_favorite);
        assert_eq!(person.birth_date.as_deref(), Some("1990-05-17"));
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/people/p1");
    }

    #[test]
    fn person_thumbnail_returns_the_face_crop_bytes() {
        let client = client_with(HttpResponse { status: 200, headers: vec![], body: JPEG.to_vec() });
        assert_eq!(block_on(client.person_thumbnail("p1", TOKEN)).unwrap(), JPEG);
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/api/people/p1/thumbnail");
        assert_bearer(&req, TOKEN);
    }

    #[test]
    fn update_person_sends_only_the_changed_fields() {
        let client = client_with(ok(PERSON_JSON));
        let update = PersonUpdate { name: Some("Anna".into()), ..Default::default() };
        block_on(client.update_person("p1", &update, TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/people/p1");
        assert_eq!(body_json(&req), json!({"name": "Anna"}));
    }

    #[test]
    fn create_person_posts_the_name() {
        let client = client_with(ok(PERSON_JSON));
        block_on(client.create_person("Anna", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/people");
        assert_eq!(body_json(&req), json!({"name": "Anna"}));
    }

    #[test]
    fn merge_people_posts_the_others_to_the_target() {
        let client = client_with(ok(r#"[{"id":"p2","success":true}]"#));
        let others = vec!["p2".to_string()];
        assert!(block_on(client.merge_people("p1", &others, TOKEN)).unwrap()[0].success);
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/people/p1/merge");
        assert_eq!(body_json(&req), json!({"ids": ["p2"]}));
    }

    #[test]
    fn person_assets_searches_by_person_id() {
        let client = client_with(ok(r#"{"assets":{"total":0,"count":0,"nextPage":null,"items":[]}}"#));
        block_on(client.person_assets("p1", 1, 100, TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/search/metadata");
        assert_eq!(body_json(&req), json!({"personIds": ["p1"], "page": 1, "size": 100}));
    }

    #[test]
    fn faces_queries_by_asset_id() {
        let client = client_with(ok(&format!(r#"[{{"id":"f1","imageWidth":4000,"imageHeight":6000,"boundingBoxX1":300,"boundingBoxY1":300,"boundingBoxX2":700,"boundingBoxY2":700,"person":{PERSON_JSON},"sourceType":"manual"}}]"#)));
        let faces = block_on(client.faces(ASSET, TOKEN)).unwrap();
        assert_eq!(faces[0].person.as_ref().map(|p| p.name.as_str()), Some("Anna"));
        let req = sent(&client);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, format!("/api/faces?id={ASSET}"));
    }

    #[test]
    fn reassign_face_puts_the_face_on_the_person_path() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.reassign_face("f1", "p2", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Put);
        assert_eq!(req.path, "/api/faces/p2");
        assert_eq!(body_json(&req), json!({"id": "f1"}));
    }

    #[test]
    fn delete_face_is_a_soft_delete() {
        let client = client_with(HttpResponse::json(204, ""));
        block_on(client.delete_face("f1", TOKEN)).unwrap();
        let req = sent(&client);
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.path, "/api/faces/f1");
        assert_eq!(body_json(&req), json!({"force": false}));
    }

    #[test]
    fn rename_album_sends_patch() {
        let t = MockTransport::new();
        t.push(HttpResponse::json(200, "{}"));
        let c = ImmichClient::new("http://h", t).unwrap();
        block_on(c.rename_album("a1", "Neu", "tok")).unwrap();
        let r = &c.transport.requests.borrow()[0];
        assert_eq!(r.method, Method::Patch);
        assert_eq!(r.path, "/api/albums/a1");
        assert_eq!(String::from_utf8_lossy(r.body.as_ref().unwrap()), "{\"albumName\":\"Neu\"}");
    }
}
