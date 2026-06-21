//! Browsing Graph core — pure read-time derivation + projection + graph analysis,
//! plus a thin WASM shell (store / render / ui / bridge) for the dashboard page.
//!
//! Architecture: WASM never runs in the service worker; all Rust lives on the
//! dashboard page (§1). The modules below marked pure compile and run under
//! `cargo test`; only `store` / `render` / `ui` / `bridge` are WASM-only.

// ── Pure core (runs under `cargo test`) ────────────────────────────────────────
pub mod derive;
pub mod flow;
pub mod graph;
pub mod interpret;
pub mod layout;
pub mod model;
pub mod project;
pub mod rollup;

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
