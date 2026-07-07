//! Outdegree core — pure read-time derivation + projection + graph analysis,
//! plus a thin WASM shell (store / render / ui / bridge) for the dashboard page.
//!
//! Architecture: WASM never runs in the service worker; all Rust lives on the
//! dashboard page (§1). The modules below marked pure compile and run under
//! `cargo test`; only `store` / `render` / `ui` / `bridge` are WASM-only.

// The pure analysis core uses no `unsafe`; forbid it on the native build — which
// is exactly the surface `cargo test` and the pure-core clippy leg cover. The
// wasm shell can't forbid it (wasm-bindgen's generated glue is `unsafe`), so the
// guard is native-only by design.
#![cfg_attr(not(target_arch = "wasm32"), forbid(unsafe_code))]

// ── Pure core (runs under `cargo test`) ────────────────────────────────────────
pub mod derive;
pub mod export;
pub mod flow;
pub mod graph;
pub mod inspect;
pub mod interpret;
pub mod layout;
pub mod model;
pub mod project;
pub mod rollup;
pub mod sample;
pub mod search;
pub mod stewardship;
pub mod svg;
pub mod ui_prefs;
pub mod views;

// ── WASM-only shell (dashboard page) ───────────────────────────────────────────
#[cfg(target_arch = "wasm32")]
pub mod bridge;
#[cfg(target_arch = "wasm32")]
pub mod render;
#[cfg(target_arch = "wasm32")]
pub mod store;
#[cfg(target_arch = "wasm32")]
pub mod ui;

#[cfg(target_arch = "wasm32")]
pub use bridge::mount;
