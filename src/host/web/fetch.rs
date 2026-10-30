use crate::logic::ports::{HttpRequest, HttpResponse, HttpTransport, Method, TransportError};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, RequestCredentials, RequestInit, Response};

/// `fetch` against the server origin. Same-origin (proxy) the Immich cookie rides along; a server on another
/// origin is authorized by the bearer header alone.
pub struct FetchTransport;

fn network(error: JsValue) -> TransportError {
    TransportError::Network(error.as_string().unwrap_or_else(|| format!("{error:?}")))
}

impl HttpTransport for FetchTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let headers = Headers::new().map_err(network)?;
        for (name, value) in &request.headers {
            headers.set(name, value).map_err(network)?;
        }
        let init = RequestInit::new();
        init.set_method(match request.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        });
        init.set_credentials(RequestCredentials::SameOrigin);
        init.set_headers(&headers);
        if let Some(body) = &request.body {
            init.set_body(&Uint8Array::from(body.as_slice()));
        }
        let web_request = web_sys::Request::new_with_str_and_init(&format!("{}{}", super::origin(), request.path), &init).map_err(network)?;
        let window = web_sys::window().ok_or_else(|| TransportError::Network("no window".into()))?;
        let response: Response = JsFuture::from(window.fetch_with_request(&web_request))
            .await
            .map_err(network)?
            .dyn_into()
            .map_err(network)?;

        let mut response_headers = Vec::new();
        if let Ok(Some(entries)) = js_sys::try_iter(&response.headers()) {
            for entry in entries.flatten() {
                let pair: Array = entry.unchecked_into();
                if let (Some(k), Some(v)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                    response_headers.push((k, v));
                }
            }
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(network)?).await.map_err(network)?;
        Ok(HttpResponse {
            status: response.status(),
            headers: response_headers,
            body: Uint8Array::new(&buffer).to_vec(),
        })
    }
}

/// Downloads `url` with the session token as bearer (works cross-origin; same-origin the cookie applies too).
pub async fn bytes(url: &str) -> Result<Vec<u8>, String> {
    let js = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
    let init = RequestInit::new();
    init.set_credentials(RequestCredentials::SameOrigin);
    if let Some(token) = super::local_storage::get(super::TOKEN_KEY) {
        let headers = Headers::new().map_err(js)?;
        headers.set("Authorization", &format!("Bearer {token}")).map_err(js)?;
        init.set_headers(&headers);
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js)?.dyn_into().map_err(js)?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

/// `Range: bytes=start-end` GET with the session token. 206 gives the range and the file size from
/// `Content-Range`; 200 means the server ignored the header and the body is the whole file.
pub async fn range(url: &str, start: u64, end_inclusive: u64) -> Result<crate::host::RangeResponse, String> {
    let js = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
    let init = RequestInit::new();
    init.set_credentials(RequestCredentials::SameOrigin);
    let headers = Headers::new().map_err(js)?;
    headers.set("Range", &format!("bytes={start}-{end_inclusive}")).map_err(js)?;
    if let Some(token) = super::local_storage::get(super::TOKEN_KEY) {
        headers.set("Authorization", &format!("Bearer {token}")).map_err(js)?;
    }
    init.set_headers(&headers);
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js)?.dyn_into().map_err(js)?;
    let status = response.status();
    if status != 200 && status != 206 {
        return Err(format!("HTTP {status}"));
    }
    let total = if status == 206 { response.headers().get("Content-Range").ok().flatten().and_then(|v| crate::host::content_range_total(&v)) } else { None };
    let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
    let bytes = Uint8Array::new(&buffer).to_vec();
    let total = if status == 206 { total } else { Some(bytes.len() as u64) };
    Ok(crate::host::RangeResponse { bytes, total, partial: status == 206 })
}
