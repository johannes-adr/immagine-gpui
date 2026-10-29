//! Owner: A1. Swift source: `../immagine/Immagine/Support/JSONCoding.swift`.
//! Spec: `../immagine/ImmagineTests/` -> CoreTests (JSON*).
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::json` is green.

//! Shared JSON configuration. Swift's `JSONCoding` only fixes the key strategy (`useDefaultKeys`)
//! and holds no formatters, so this is serde_json with its defaults. Dates stay `String` in the
//! models and go through `iso_date::parse` where needed.

use serde::{de::DeserializeOwned, Serialize};

/// Decodes a JSON document. Field names are used as written (no key conversion).
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    serde_json::from_slice(bytes)
}

/// Encodes a value as compact JSON.
pub fn encode<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(value)
}
