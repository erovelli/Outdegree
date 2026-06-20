//! canvas2d renderer (§7.7): node radius ∝ √visits (boundary-source nodes stay
//! visible via MIN_R), fill = dominant provenance, edge width ∝ weight, edge
//! color = dominant kind. Pan/zoom via the [`Camera`].

use crate::layout::Pos;
use crate::model::GraphProjection;
use std::collections::HashMap;
use std::f64::consts::PI;
use web_sys::CanvasRenderingContext2d;

const MIN_R: f64 = 4.0;
const R_SCALE: f64 = 3.0;
const BG: &str = "#11151c";

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
        }
    }
}

impl Camera {
    fn project(&self, p: &Pos, w: f64, h: f64) -> (f64, f64) {
        (
            p.x as f64 * self.scale + self.x + w / 2.0,
            p.y as f64 * self.scale + self.y + h / 2.0,
        )
    }
}

fn set_fill(ctx: &CanvasRenderingContext2d, c: &str) {
    ctx.set_fill_style_str(c);
}
fn set_stroke(ctx: &CanvasRenderingContext2d, c: &str) {
    ctx.set_stroke_style_str(c);
}

/// Draw the full graph. `hover` is an optional node key to label.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    w: f64,
    h: f64,
    proj: &GraphProjection,
    pos: &HashMap<String, Pos>,
    cam: &Camera,
    hover: Option<&str>,
) {
    set_fill(ctx, BG);
    ctx.fill_rect(0.0, 0.0, w, h);

    // edges
    for e in &proj.edges {
        if let (Some(a), Some(b)) = (pos.get(&e.from), pos.get(&e.to)) {
            let (ax, ay) = cam.project(a, w, h);
            let (bx, by) = cam.project(b, w, h);
            set_stroke(ctx, e.kinds.dominant().color());
            ctx.set_line_width((1.0 + (e.weight as f64).ln_1p()).min(8.0));
            ctx.set_global_alpha(0.55);
            ctx.begin_path();
            ctx.move_to(ax, ay);
            ctx.line_to(bx, by);
            ctx.stroke();
        }
    }
    ctx.set_global_alpha(1.0);

    // nodes
    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            let (x, y) = cam.project(p, w, h);
            let r = MIN_R.max(R_SCALE * (n.visits as f64).sqrt()) * cam.scale.max(0.3);
            set_fill(ctx, n.prov.dominant().color());
            ctx.begin_path();
            let _ = ctx.arc(x, y, r, 0.0, PI * 2.0);
            ctx.fill();
        }
    }

    // hover label
    if let Some(key) = hover {
        if let Some(p) = pos.get(key) {
            let (x, y) = cam.project(p, w, h);
            set_fill(ctx, "#e6edf3");
            ctx.set_font("12px system-ui, sans-serif");
            let _ = ctx.fill_text(key, x + 8.0, y - 8.0);
        }
    }
}

/// Hit-test: the topmost node whose disc contains screen point `(sx, sy)`.
pub fn hit_test(
    sx: f64,
    sy: f64,
    w: f64,
    h: f64,
    proj: &GraphProjection,
    pos: &HashMap<String, Pos>,
    cam: &Camera,
) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            let (x, y) = cam.project(p, w, h);
            let r = MIN_R.max(R_SCALE * (n.visits as f64).sqrt()) * cam.scale.max(0.3);
            let d2 = (x - sx).powi(2) + (y - sy).powi(2);
            if d2 <= r * r {
                let better = best.as_ref().map(|(_, bd)| d2 < *bd).unwrap_or(true);
                if better {
                    best = Some((n.key.clone(), d2));
                }
            }
        }
    }
    best.map(|(k, _)| k)
}
