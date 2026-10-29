//! Seams between platform-independent logic and the browser. The browser implements these
//! (`fetch`, IndexedDB, `Date.now`); tests use the mocks below.

use std::cell::RefCell;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    /// Path including query, relative to the server root, e.g. `/api/assets/ID/thumbnail?size=preview`.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, body: &str) -> Self {
        Self { status, headers: vec![("content-type".into(), "application/json".into())], body: body.as_bytes().to_vec() }
    }
}

#[derive(Debug, Clone)]
pub enum TransportError {
    /// Network failure (offline, DNS, aborted).
    Network(String),
}

/// Single-threaded on purpose: wasm futures are not `Send`.
#[allow(async_fn_in_trait)]
pub trait HttpTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Milliseconds since the Unix epoch.
pub trait Clock {
    fn now_ms(&self) -> i64;
}

/// Replays queued responses in order and records every request.
#[derive(Default)]
pub struct MockTransport {
    responses: RefCell<VecDeque<Result<HttpResponse, TransportError>>>,
    pub requests: RefCell<Vec<HttpRequest>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, response: HttpResponse) -> &Self {
        self.responses.borrow_mut().push_back(Ok(response));
        self
    }

    pub fn push_error(&self, error: TransportError) -> &Self {
        self.responses.borrow_mut().push_back(Err(error));
        self
    }
}

impl HttpTransport for MockTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.requests.borrow_mut().push(request);
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| Err(TransportError::Network("MockTransport: no response queued".into())))
    }
}

pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_replays_in_order_and_records() {
        let t = MockTransport::new();
        t.push(HttpResponse::json(200, "{\"res\":\"pong\"}"));
        let req = HttpRequest { method: Method::Get, path: "/api/server/ping".into(), headers: vec![], body: None };
        let r = futures_lite::future::block_on(t.send(req)).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(t.requests.borrow()[0].path, "/api/server/ping");
        let again = HttpRequest { method: Method::Get, path: "/x".into(), headers: vec![], body: None };
        assert!(futures_lite::future::block_on(t.send(again)).is_err());
    }
}
