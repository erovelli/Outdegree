//! Optional wgpu renderer (§7.7) — an escape hatch enabled only if a real profile
//! needs GPU acceleration (decision: canvas2d is sufficient for v1 because host
//! count, not event volume, bounds the rendered node set). Behind
//! `feature = "webgpu"`; intentionally a stub until a measured need arises.

/// Placeholder entry point for a future wgpu pipeline.
pub fn available() -> bool {
    false
}
