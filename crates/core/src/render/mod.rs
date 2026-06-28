//! Rendering backends (§7.7). canvas2d is the v1 renderer. A GPU (wgpu) path was
//! scoped as an escape hatch but is unneeded for v1 (host count, not event
//! volume, bounds the rendered node set), so it is not shipped; reintroduce it
//! behind a `webgpu` feature if a measured profile requires it.

pub mod canvas2d;
