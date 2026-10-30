use crate::logic::ports::{HttpRequest, HttpResponse, HttpTransport, Method, TransportError};
use std::io::Read;
use std::sync::OnceLock;

use super::blocking;

/// Where the logged-in session token lives (see `immagine::TOKEN_KEY`); sent as a bearer token on image
/// downloads in addition to the cookie jar, since there is no browser to attach the cookie for us.
const TOKEN_KEY: &str = "immagine.token";

/// One agent for the whole process: it keeps the Immich cookie and reuses connections.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::AgentBuilder::new().build())
}

fn read_body(response: ureq::Response) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    let status = response.status();
    let headers = response.headers_names().into_iter().filter_map(|n| response.header(&n).map(|v| (n.clone(), v.to_string()))).collect();
    let mut body = Vec::new();
    response.into_reader().read_to_end(&mut body).map_err(|e| e.to_string())?;
    Ok((status, headers, body))
}

/// `ureq` reports 4xx/5xx as errors; the logic layer wants them as ordinary responses.
fn into_response(result: Result<ureq::Response, ureq::Error>) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => read_body(response),
        Err(e) => Err(e.to_string()),
    }
}

pub struct UreqTransport;

impl HttpTransport for UreqTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let url = format!("{}{}", super::server_origin(), request.path);
        let (status, headers, body) = blocking::run(move || {
            let method = match request.method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Patch => "PATCH",
                Method::Delete => "DELETE",
            };
            let mut call = agent().request(method, &url);
            for (name, value) in &request.headers {
                call = call.set(name, value);
            }
            into_response(match &request.body {
                Some(body) => call.send_bytes(body),
                None => call.call(),
            })
        })
        .await
        .map_err(TransportError::Network)?;
        Ok(HttpResponse { status, headers, body })
    }
}

/// Blocking GET of an image URL with the session's credentials. Called on a worker thread.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut call = agent().get(url);
    if let Some(token) = super::store::setting_get(TOKEN_KEY) {
        call = call.set("Authorization", &format!("Bearer {token}"));
    }
    let (status, _, body) = into_response(call.call())?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    Ok(body)
}
