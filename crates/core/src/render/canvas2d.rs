//! canvas2d renderer (§7.7): node radius ∝ √visits (boundary-source nodes stay
//! visible via MIN_R), fill = dominant provenance, edge width ∝ weight, edge
//! color = dominant kind. Pan/zoom via the [`Camera`].

use crate::layout::Pos;
use crate::model::GraphProjection;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use web_sys::CanvasRenderingContext2d;

const MIN_R: f64 = 4.0;
const R_SCALE: f64 = 3.0;
const BG: &str = "#11151c";
const LABEL: &str = "#cfd8e3";

/// Node disc radius in screen pixels at the given camera.
fn radius(visits: u32, scale: f64) -> f64 {
    MIN_R.max(R_SCALE * (visits as f64).sqrt()) * scale.clamp(0.4, 2.0)
}

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

/// Compute a camera that frames all of `proj`'s laid-out nodes within a `w`×`h`
/// canvas (with padding). Without this, a sparse/edgeless layout spreads nodes
/// off-screen and the graph looks blank.
pub fn fit(proj: &GraphProjection, pos: &HashMap<String, Pos>, w: f64, h: f64) -> Camera {
    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    let (mut maxx, mut maxy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            // Skip non-finite positions so one stray NaN can't blank the view.
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            any = true;
            minx = minx.min(p.x as f64);
            miny = miny.min(p.y as f64);
            maxx = maxx.max(p.x as f64);
            maxy = maxy.max(p.y as f64);
        }
    }
    if !any {
        return Camera::default();
    }
    let pad = 80.0;
    let bw = (maxx - minx).max(1.0);
    let bh = (maxy - miny).max(1.0);
    let scale = ((w - pad) / bw).min((h - pad) / bh).clamp(0.05, 3.0);
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    // screen = pos*scale + cam + size/2; center the bbox at the canvas center.
    Camera {
        x: -cx * scale,
        y: -cy * scale,
        scale,
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
    focus: Option<&str>,
) {
    set_fill(ctx, BG);
    ctx.fill_rect(0.0, 0.0, w, h);

    // When a node is focused (hovered), highlight it + its neighbors and dim the
    // rest — the Obsidian-style "spotlight" behavior.
    let highlight: Option<HashSet<&str>> = focus.map(|f| {
        let mut s = HashSet::new();
        s.insert(f);
        for e in &proj.edges {
            if e.from == f {
                s.insert(e.to.as_str());
            }
            if e.to == f {
                s.insert(e.from.as_str());
            }
        }
        s
    });
    let lit = |key: &str| highlight.as_ref().map(|s| s.contains(key)).unwrap_or(true);

    // edges
    for e in &proj.edges {
        if let (Some(a), Some(b)) = (pos.get(&e.from), pos.get(&e.to)) {
            let (ax, ay) = cam.project(a, w, h);
            let (bx, by) = cam.project(b, w, h);
            let touches = focus.map(|f| e.from == f || e.to == f).unwrap_or(true);
            set_stroke(ctx, e.kinds.dominant().color());
            ctx.set_line_width((1.0 + (e.weight as f64).ln_1p()).min(8.0));
            ctx.set_global_alpha(if touches { 0.7 } else { 0.07 });
            ctx.begin_path();
            ctx.move_to(ax, ay);
            ctx.line_to(bx, by);
            ctx.stroke();
        }
    }
    ctx.set_global_alpha(1.0);

    // Show labels when the graph is small or zoomed in (else they clutter), and
    // always for highlighted nodes.
    let show_labels = proj.nodes.len() <= 150 || cam.scale >= 1.0;
    ctx.set_text_align("center");
    ctx.set_font("12px system-ui, -apple-system, sans-serif");

    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            let (x, y) = cam.project(p, w, h);
            let r = radius(n.visits, cam.scale);
            let hot = lit(&n.key);
            ctx.set_global_alpha(if hot { 1.0 } else { 0.2 });
            set_fill(ctx, n.prov.dominant().color());
            ctx.begin_path();
            let _ = ctx.arc(x, y, r, 0.0, PI * 2.0);
            ctx.fill();

            let label_this = if focus.is_some() { hot } else { show_labels };
            if label_this {
                ctx.set_global_alpha(if hot { 1.0 } else { 0.5 });
                set_fill(ctx, LABEL);
                let _ = ctx.fill_text(&n.key, x, y + r + 12.0);
            }
        }
    }
    ctx.set_global_alpha(1.0);
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
            // a little slop so small nodes are still easy to hover
            let r = radius(n.visits, cam.scale).max(6.0);
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
