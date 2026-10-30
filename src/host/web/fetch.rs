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
