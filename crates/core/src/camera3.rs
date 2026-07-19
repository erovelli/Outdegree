//! Pure orbit-camera math for the graph view's 3-D perspective toggle.
//!
//! The 3-D mode reuses the canvas2d renderer: each [`Pos3`] is perspective-
//! projected to a screen point plus a per-node scale factor (nearer → larger),
//! and the draw pass paints back-to-front. Keeping the projection math here (not
//! in the wasm-only render shell) makes it deterministic and testable under
//! `cargo test`, like [`crate::layout`].

use crate::layout::Pos3;
use crate::model::GraphProjection;
use std::collections::HashMap;

/// Orbit pitch is clamped to ±this (radians, ≈ ±83°) so the camera can never
/// flip over the pole mid-drag (the gimbal somersault).
pub const PITCH_MAX: f64 = 1.45;

/// Per-node screen-scale clamp bounds — the 3-D analogue of the 2-D camera's
/// `scale` clamp. `fit` normalizes `scale` to frame the cloud, so these bounds
/// are relative zoom factors around that framing.
pub const SCALE_MIN: f64 = 0.02;
pub const SCALE_MAX: f64 = 8.0;

/// Nearest allowed depth, as a fraction of the orbit distance: points that would
/// cross behind the camera clamp here instead of exploding through the divide.
const NEAR: f64 = 0.1;

/// Orbit camera for the 3-D perspective: the eye circles `target` at `dist`,
/// oriented by `yaw` (about the vertical axis) then `pitch` (about the screen-
/// horizontal axis). `scale` is the screen scale at depth == `dist`, so zoom has
/// the same feel (and clamping role) as the 2-D camera's `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3 {
    pub yaw: f64,
    pub pitch: f64,
    pub dist: f64,
    pub scale: f64,
    pub target: (f64, f64, f64),
}

impl Default for Camera3 {
    fn default() -> Self {
        Camera3 {
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            dist: 1000.0,
            scale: 1.0,
            target: (0.0, 0.0, 0.0),
        }
    }
}

/// The canonical fit orientation: a gentle three-quarter view, so entering 3-D
/// reads as depth immediately (a head-on yaw/pitch of 0 would look flat 2-D).
pub const DEFAULT_YAW: f64 = 0.55;
pub const DEFAULT_PITCH: f64 = 0.30;

impl Camera3 {
    /// Project a world point to `(screen_x, screen_y, node_scale, depth)` on a
    /// `w`×`h` canvas. `node_scale` is the perspective-corrected screen scale at
    /// that point (multiply radii/labels by it); `depth` orders the painter's
    /// back-to-front pass (larger = farther).
    pub fn project(&self, p: &Pos3, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let x = p.x as f64 - self.target.0;
        let y = p.y as f64 - self.target.1;
        let z = p.z as f64 - self.target.2;
        // yaw about the vertical axis…
        let (sy, cy) = self.yaw.sin_cos();
        let x1 = x * cy + z * sy;
        let z1 = -x * sy + z * cy;
        // …then pitch about the screen-horizontal axis.
        let (sp, cp) = self.pitch.sin_cos();
        let y2 = y * cp - z1 * sp;
        let z2 = y * sp + z1 * cp;
        // Perspective divide, near-clamped so a point can't cross the eye.
        let depth = (self.dist + z2).max(self.dist * NEAR);
        let s = self.scale * self.dist / depth;
        (x1 * s + w / 2.0, y2 * s + h / 2.0, s, depth)
    }

    /// Multiply the zoom (screen scale), clamped like the 2-D camera's zoom.
    pub fn zoom(&mut self, factor: f64) {
        self.scale = (self.scale * factor).clamp(SCALE_MIN, SCALE_MAX);
    }

    /// Orbit by screen-space deltas (radians), clamping pitch short of the poles.
    pub fn orbit(&mut self, dyaw: f64, dpitch: f64) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-PITCH_MAX, PITCH_MAX);
    }
}

/// Compute a camera that frames all of `proj`'s laid-out nodes within a `w`×`h`
/// canvas from the canonical three-quarter orientation — the 3-D analogue of
/// [`crate::render::canvas2d::fit`]'s job. Deterministic; ignores non-finite
/// positions so one stray NaN can't blank the view.
pub fn fit(proj: &GraphProjection, pos: &HashMap<String, Pos3>, w: f64, h: f64) -> Camera3 {
    // Centroid + bounding radius of the finite positions.
    let mut pts: Vec<(f64, f64, f64)> = Vec::new();
    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            if p.x.is_finite() && p.y.is_finite() && p.z.is_finite() {
                pts.push((p.x as f64, p.y as f64, p.z as f64));
            }
        }
    }
    if pts.is_empty() {
        return Camera3::default();
    }
    let inv = 1.0 / pts.len() as f64;
    let (mut cx, mut cy, mut cz) = (0.0, 0.0, 0.0);
    for &(x, y, z) in &pts {
        cx += x * inv;
        cy += y * inv;
        cz += z * inv;
    }
    let r = pts
        .iter()
        .map(|&(x, y, z)| ((x - cx).powi(2) + (y - cy).powi(2) + (z - cz).powi(2)).sqrt())
        .fold(0.0f64, f64::max)
        .max(1.0);

    // Eye at 2.5 radii: close enough for real perspective, far enough that the
    // whole cloud stays in front of the near plane while orbiting.
    let dist = 2.5 * r;
    // Frame the bounding sphere into 80% of the half-canvas (the same padding the
    // 2-D fit leaves for the floating chrome). A sphere point at angle θ projects
    // to lateral r·sinθ·scale·dist/(dist−r·cosθ); with dist = 2.5·r that peaks at
    // cosθ = 0.4, ≈1.09× the center-plane extent r·scale — hence 0.8/1.09 ≈ 0.73.
    let half = (w.min(h) / 2.0).max(1.0);
    let scale = 0.73 * half / r;
    let scale = if scale.is_finite() {
        scale.clamp(SCALE_MIN, SCALE_MAX)
    } else {
        1.0
    };
    Camera3 {
        yaw: DEFAULT_YAW,
        pitch: DEFAULT_PITCH,
        dist,
        scale,
        target: (cx, cy, cz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeAgg;

    fn proj_of(keys: &[&str]) -> GraphProjection {
        GraphProjection {
            nodes: keys
                .iter()
                .map(|k| NodeAgg {
                    key: k.to_string(),
                    ..Default::default()
                })
                .collect(),
            edges: Vec::new(),
        }
    }

    fn pos_of(entries: &[(&str, f32, f32, f32)]) -> HashMap<String, Pos3> {
        entries
            .iter()
            .map(|&(k, x, y, z)| (k.to_string(), Pos3 { x, y, z }))
            .collect()
    }

    #[test]
    fn fit_centers_the_centroid_on_the_canvas() {
        let proj = proj_of(&["a", "b"]);
        let pos = pos_of(&[("a", -100.0, -50.0, -20.0), ("b", 100.0, 50.0, 20.0)]);
        let cam = fit(&proj, &pos, 800.0, 600.0);
        // The centroid is the orbit target, so it projects to the exact center at
        // any orientation.
        let center = Pos3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let (sx, sy, _, _) = cam.project(&center, 800.0, 600.0);
        assert!((sx - 400.0).abs() < 1e-6 && (sy - 300.0).abs() < 1e-6);
    }

    #[test]
    fn fit_keeps_all_nodes_on_canvas() {
        let proj = proj_of(&["a", "b", "c", "d"]);
        let pos = pos_of(&[
            ("a", -900.0, 0.0, 0.0),
            ("b", 900.0, 0.0, 0.0),
            ("c", 0.0, -700.0, 500.0),
            ("d", 0.0, 700.0, -500.0),
        ]);
        let (w, h) = (800.0, 600.0);
        let cam = fit(&proj, &pos, w, h);
        for p in pos.values() {
            let (sx, sy, s, depth) = cam.project(p, w, h);
            assert!(sx.is_finite() && sy.is_finite() && s > 0.0 && depth > 0.0);
            assert!((0.0..=w).contains(&sx), "x off-canvas: {sx}");
            assert!((0.0..=h).contains(&sy), "y off-canvas: {sy}");
        }
    }

    #[test]
    fn nearer_points_project_larger() {
        let cam = Camera3 {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera3::default()
        };
        let near = Pos3 {
            x: 0.0,
            y: 0.0,
            z: -300.0,
        };
        let far = Pos3 {
            x: 0.0,
            y: 0.0,
            z: 300.0,
        };
        let (.., s_near, d_near) = cam.project(&near, 800.0, 600.0);
        let (.., s_far, d_far) = cam.project(&far, 800.0, 600.0);
        assert!(s_near > s_far, "perspective must shrink with depth");
        assert!(d_near < d_far, "depth must grow away from the camera");
    }

    #[test]
    fn near_plane_clamps_points_behind_the_eye() {
        let cam = Camera3 {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera3::default()
        };
        // z = −2·dist would put the point behind the camera; the near clamp keeps
        // the projection finite (and the scale bounded).
        let behind = Pos3 {
            x: 10.0,
            y: 10.0,
            z: -2000.0,
        };
        let (sx, sy, s, depth) = cam.project(&behind, 800.0, 600.0);
        assert!(sx.is_finite() && sy.is_finite() && s.is_finite());
        assert!(depth > 0.0);
    }

    #[test]
    fn orbit_clamps_pitch_and_zoom_clamps_scale() {
        let mut cam = Camera3::default();
        cam.orbit(0.0, 10.0);
        assert!((cam.pitch - PITCH_MAX).abs() < 1e-9);
        cam.orbit(0.0, -20.0);
        assert!((cam.pitch + PITCH_MAX).abs() < 1e-9);
        cam.zoom(1e9);
        assert!((cam.scale - SCALE_MAX).abs() < 1e-9);
        cam.zoom(1e-12);
        assert!((cam.scale - SCALE_MIN).abs() < 1e-9);
    }

    #[test]
    fn fit_on_empty_or_nan_positions_is_default() {
        let proj = proj_of(&["a"]);
        assert_eq!(
            fit(&proj, &HashMap::new(), 800.0, 600.0),
            Camera3::default()
        );
        let pos = pos_of(&[("a", f32::NAN, 0.0, 0.0)]);
        assert_eq!(fit(&proj, &pos, 800.0, 600.0), Camera3::default());
    }
}
