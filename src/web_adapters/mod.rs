//! Browser implementations of the `logic::ports` traits: `fetch` transport and IndexedDB persistence.
//! Everything here is wasm-only; the platform-independent logic stays in `crate::logic`.

pub mod fetch;
pub mod idb;
pub mod selftest;
