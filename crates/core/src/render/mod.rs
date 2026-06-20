//! Rendering backends (§7.7). canvas2d is the v1 renderer; wgpu is an optional
//! escape hatch behind `feature = "webgpu"`.

pub mod canvas2d;

#[cfg(feature = "webgpu")]
pub mod wgpu;
