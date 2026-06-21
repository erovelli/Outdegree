//! canvas2d renderer (§7.7) in the Palantir-AIP visual language from the design
//! handoff: pure-black canvas with a panning dot grid, a center vignette, a glow
//! halo on the busiest hub, monochrome chrome, and the single blue→red OKLCH
//! data spectrum used *only* to encode nodes/edges.
//!
//! Node radius ∝ √visits (boundary-source nodes stay visible via the `8` floor),
//! fill = dominant provenance, edges colored by dominant kind (search links
//! dashed), each node ringed by a 2px black "moat". Pan/zoom via the [`Camera`].

use crate::layout::Pos;
use crate::model::{GraphProjection, Provenance};
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

// ── chrome (monochrome) ──────────────────────────────────────────────────────
const BG: &str = "#000000";
const DOT_GRID: &str = "#161619";
const GRID: f64 = 26.0;
const LABEL: &str = "#8a8a90";
const VIGNETTE: &str = "oklch(0.22 0.03 290 / 0.28)";
/// Fixed blue stop of the spectrum, used for the busiest-hub glow + orbit ring.
const HUB_GLOW: &str = "oklch(0.64 0.205 264)";
const RETICLE: &str = "#e8e8ea";
const CONNECTOR: &str = "#3a3a40";
const CALLOUT_FILL: &str = "oklch(0.12 0.004 285 / 0.92)";
const CALLOUT_BORDER: &str = "#2c2c30";
const CALLOUT_HOST: &str = "#f4f4f5";

const MONO: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

/// Node disc radius in screen pixels: `max(8, 2.6·√visits)` (design units),
/// scaled by zoom and clamped so nodes never vanish or dominate.
fn radius(visits: u32, scale: f64) -> f64 {
    ((8.0_f64).max(2.6 * (visits as f64).sqrt()) * scale).clamp(2.0, 64.0)
}

/// Label font size in screen pixels — scales with zoom so labels shrink when
/// zoomed out (less clutter) and grow when zoomed in.
fn label_px(scale: f64) -> f64 {
    (13.0 * scale).clamp(8.0, 22.0)
}

/// Human label for the inspect callout's sub-line.
fn prov_label(p: Provenance) -> &'static str {
    match p {
        Provenance::Link => "Link",
        Provenance::Form => "Form",
        Provenance::TypedUrl => "Typed URL",
        Provenance::SearchOrigin => "Search",
        Provenance::Bookmark => "Bookmark",
        Provenance::Start => "Start",
        Provenance::Reload => "Reload",
        Provenance::Other => "Other",
    }
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
    // Leave 10% padding on each edge so the floating chrome (legend, toolbars,
    // readout) doesn't cover the network — the graph fits the central 80%×80%.
    let bw = (maxx - minx).max(1.0);
    let bh = (maxy - miny).max(1.0);
    let usable_w = (w * 0.8).max(1.0);
    let usable_h = (h * 0.8).max(1.0);
    let scale = (usable_w / bw).min(usable_h / bh).clamp(0.05, 3.0);
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
fn set_dash(ctx: &CanvasRenderingContext2d, segs: &[f64]) {
    let arr = js_sys::Array::new();
    for s in segs {
        arr.push(&JsValue::from_f64(*s));
    }
    let _ = ctx.set_line_dash(arr.as_ref());
}

/// Pure-black fill, a dot grid that pans/scales with the camera, and a soft
/// center vignette for depth.
fn draw_backdrop(ctx: &CanvasRenderingContext2d, w: f64, h: f64, cam: &Camera) {
    set_fill(ctx, BG);
    ctx.fill_rect(0.0, 0.0, w, h);

    // Dot grid: world-space lattice projected to screen, so it drifts under a pan
    // and spreads under a zoom. Skip when the spacing collapses (too dense).
    let step = GRID * cam.scale;
    if step >= 9.0 {
        let ox = cam.x + w / 2.0; // screen x of world origin
        let oy = cam.y + h / 2.0;
        set_fill(ctx, DOT_GRID);
        let mut x = ox.rem_euclid(step) - step;
        while x < w + step {
            let mut y = oy.rem_euclid(step) - step;
            while y < h + step {
                ctx.begin_path();
                let _ = ctx.arc(x, y, 1.0, 0.0, PI * 2.0);
                ctx.fill();
                y += step;
            }
            x += step;
        }
    }

    // Center vignette (oklch lift fading to transparent).
    if let Ok(g) =
        ctx.create_radial_gradient(0.52 * w, 0.46 * h, 0.0, 0.52 * w, 0.46 * h, 0.62 * w.max(h))
    {
        let _ = g.add_color_stop(0.0, VIGNETTE);
        let _ = g.add_color_stop(0.7, "transparent");
        ctx.set_fill_style_canvas_gradient(&g);
        ctx.fill_rect(0.0, 0.0, w, h);
    }
}

/// Glow halo + dashed orbit ring on the busiest node (max visits).
fn draw_hub(
    ctx: &CanvasRenderingContext2d,
    w: f64,
    h: f64,
    proj: &GraphProjection,
    pos: &HashMap<String, Pos>,
    cam: &Camera,
) {
    let Some(hub) = proj.nodes.iter().max_by_key(|n| n.visits) else {
        return;
    };
    let Some(p) = pos.get(&hub.key) else { return };
    let (x, y) = cam.project(p, w, h);
    let r = radius(hub.visits, cam.scale);
    let glow = (r * 5.0).clamp(48.0, 280.0);
    if let Ok(g) = ctx.create_radial_gradient(x, y, 0.0, x, y, glow) {
        let _ = g.add_color_stop(0.0, "oklch(0.64 0.205 264 / 0.35)");
        let _ = g.add_color_stop(1.0, "transparent");
        ctx.set_fill_style_canvas_gradient(&g);
        ctx.begin_path();
        let _ = ctx.arc(x, y, glow, 0.0, PI * 2.0);
        ctx.fill();
    }
    // thin dashed orbit ring
    let ring = (r * 2.4).clamp(26.0, 140.0);
    set_stroke(ctx, HUB_GLOW);
    ctx.set_global_alpha(0.35);
    ctx.set_line_width(1.0);
    set_dash(ctx, &[2.0, 7.0]);
    ctx.begin_path();
    let _ = ctx.arc(x, y, ring, 0.0, PI * 2.0);
    ctx.stroke();
    set_dash(ctx, &[]);
    ctx.set_global_alpha(1.0);
}

/// Draw the full graph. `hover` labels + spotlights a node; `selected` (the
/// drilled-down focus) wears the reticle bracket.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    w: f64,
    h: f64,
    proj: &GraphProjection,
    pos: &HashMap<String, Pos>,
    cam: &Camera,
    hover: Option<&str>,
    selected: Option<&str>,
) {
    draw_backdrop(ctx, w, h, cam);
    draw_hub(ctx, w, h, proj, pos, cam);

    // When a node is hovered, light it + its neighbors and dim the rest — the
    // Obsidian/Palantir "spotlight" behavior.
    let highlight: Option<HashSet<&str>> = hover.map(|f| {
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

    // ── edges (directional: arrowhead at the target end) ─────────────────────
    let vis: HashMap<&str, u32> = proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.visits))
        .collect();
    let ew = (1.2 * cam.scale).clamp(0.6, 4.0);
    ctx.set_line_width(ew);
    for e in &proj.edges {
        if let (Some(a), Some(b)) = (pos.get(&e.from), pos.get(&e.to)) {
            let (ax, ay) = cam.project(a, w, h);
            let (bx, by) = cam.project(b, w, h);
            let kind = e.kinds.dominant();
            let touches = hover.map(|f| e.from == f || e.to == f).unwrap_or(true);
            let dashed = matches!(kind, crate::model::EdgeKind::SearchLink);
            let base = if dashed { 0.5 } else { 0.34 };
            set_stroke(ctx, kind.color());
            set_fill(ctx, kind.color());

            // Stop the shaft at the target node's rim so the arrow reads cleanly.
            let dx = bx - ax;
            let dy = by - ay;
            let len = (dx * dx + dy * dy).sqrt();
            let tr = vis
                .get(e.to.as_str())
                .map(|v| radius(*v, cam.scale))
                .unwrap_or(4.0);
            let (tipx, tipy) = if len > 1.0 {
                (bx - dx / len * (tr + 1.5), by - dy / len * (tr + 1.5))
            } else {
                (bx, by)
            };

            ctx.set_global_alpha(if touches { base } else { 0.06 });
            set_dash(ctx, if dashed { &[5.0, 5.0] } else { &[] });
            ctx.begin_path();
            ctx.move_to(ax, ay);
            ctx.line_to(tipx, tipy);
            ctx.stroke();

            // arrowhead (solid, slightly brighter so direction is legible)
            if len > tr + 4.0 {
                let (ux, uy) = (dx / len, dy / len);
                let (px, py) = (-uy, ux);
                let ah = (5.0 * cam.scale).clamp(4.0, 9.0);
                let (bxh, byh) = (tipx - ux * ah, tipy - uy * ah);
                set_dash(ctx, &[]);
                ctx.set_global_alpha(if touches { (base + 0.3).min(0.9) } else { 0.08 });
                ctx.begin_path();
                ctx.move_to(tipx, tipy);
                ctx.line_to(bxh + px * ah * 0.55, byh + py * ah * 0.55);
                ctx.line_to(bxh - px * ah * 0.55, byh - py * ah * 0.55);
                ctx.close_path();
                ctx.fill();
            }
        }
    }
    set_dash(ctx, &[]);
    ctx.set_global_alpha(1.0);

    // ── nodes + labels ───────────────────────────────────────────────────────
    let fs = label_px(cam.scale);
    let moat = (2.0 * cam.scale).clamp(1.0, 4.0);
    ctx.set_text_align("center");
    ctx.set_text_baseline("alphabetic");
    ctx.set_font(&format!("{fs:.0}px {MONO}"));
    set_stroke(ctx, BG);
    ctx.set_line_width(moat);

    for n in &proj.nodes {
        if let Some(p) = pos.get(&n.key) {
            let (x, y) = cam.project(p, w, h);
            let r = radius(n.visits, cam.scale);
            let hot = lit(&n.key);
            ctx.set_global_alpha(if hot { 1.0 } else { 0.22 });
            // Fill = dominant provenance; shape = same provenance (a CVD-safe
            // redundant channel). 2px black moat so markers read over the edges.
            let prov = n.prov.dominant().display();
            set_fill(ctx, prov.color());
            trace_marker(ctx, prov.shape(), x, y, r);
            ctx.fill();
            ctx.stroke();

            // Every node is labeled, always (dimmed when another node is hovered).
            ctx.set_global_alpha(if hot { 1.0 } else { 0.35 });
            set_fill(ctx, LABEL);
            let _ = ctx.fill_text(&n.key, x, y + r + fs + 2.0);
        }
    }
    ctx.set_global_alpha(1.0);

    // ── selection reticle + hover callout ────────────────────────────────────
    if let Some(sel) = selected {
        if let Some(node) = proj.nodes.iter().find(|n| n.key == sel) {
            if let Some(p) = pos.get(sel) {
                let (x, y) = cam.project(p, w, h);
                draw_reticle(ctx, x, y, radius(node.visits, cam.scale));
            }
        }
    }
    if let Some(hk) = hover {
        if let Some(node) = proj.nodes.iter().find(|n| n.key == hk) {
            if let Some(p) = pos.get(hk) {
                let (x, y) = cam.project(p, w, h);
                draw_callout(ctx, w, h, x, y, radius(node.visits, cam.scale), node);
            }
        }
    }
}

/// 40×40 corner-bracket reticle with four crosshair ticks — the "selected" mark.
fn draw_reticle(ctx: &CanvasRenderingContext2d, x: f64, y: f64, r: f64) {
    let hs = (r + 12.0).clamp(16.0, 60.0);
    let cl = hs * 0.45;
    set_stroke(ctx, RETICLE);
    ctx.set_global_alpha(0.85);
    ctx.set_line_width(1.0);
    set_dash(ctx, &[]);
    ctx.begin_path();
    // four L-shaped corners
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let cx = x + sx * hs;
        let cy = y + sy * hs;
        ctx.move_to(cx - sx * cl, cy);
        ctx.line_to(cx, cy);
        ctx.line_to(cx, cy - sy * cl);
    }
    // four short edge ticks pointing inward
    ctx.move_to(x, y - hs);
    ctx.line_to(x, y - hs + 5.0);
    ctx.move_to(x, y + hs);
    ctx.line_to(x, y + hs - 5.0);
    ctx.move_to(x - hs, y);
    ctx.line_to(x - hs + 5.0, y);
    ctx.move_to(x + hs, y);
    ctx.line_to(x + hs - 5.0, y);
    ctx.stroke();
    ctx.set_global_alpha(1.0);
}

/// Inspect callout pinned to a node by a thin connector line.
fn draw_callout(
    ctx: &CanvasRenderingContext2d,
    w: f64,
    h: f64,
    x: f64,
    y: f64,
    r: f64,
    node: &crate::model::NodeAgg,
) {
    const BW: f64 = 204.0;
    const BH: f64 = 56.0;
    // Prefer up-right of the node; flip left / clamp down to stay on-screen.
    let mut bx = x + r + 16.0;
    if bx + BW > w - 8.0 {
        bx = x - r - 16.0 - BW;
    }
    let by = (y - BH / 2.0).clamp(8.0, (h - BH - 8.0).max(8.0));

    // connector from the node edge to the callout's near side
    let near_x = if bx >= x { bx } else { bx + BW };
    set_stroke(ctx, CONNECTOR);
    ctx.set_line_width(1.0);
    set_dash(ctx, &[]);
    ctx.begin_path();
    ctx.move_to(x, y);
    ctx.line_to(near_x, by + BH / 2.0);
    ctx.stroke();

    rounded_rect(ctx, bx, by, BW, BH, 4.0);
    set_fill(ctx, CALLOUT_FILL);
    ctx.fill();
    set_stroke(ctx, CALLOUT_BORDER);
    ctx.set_line_width(1.0);
    ctx.stroke();

    let prov = node.prov.dominant().display();
    // provenance dot
    set_fill(ctx, prov.color());
    ctx.begin_path();
    let _ = ctx.arc(bx + 16.0, by + 22.0, 4.0, 0.0, PI * 2.0);
    ctx.fill();

    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    set_fill(ctx, CALLOUT_HOST);
    ctx.set_font(&format!("600 13px {MONO}"));
    let _ = ctx.fill_text(&clip(&node.key, 22), bx + 28.0, by + 22.0);
    set_fill(ctx, LABEL);
    ctx.set_font(&format!("11px {MONO}"));
    let sub = format!("{} visits · {}", node.visits, prov_label(prov));
    let _ = ctx.fill_text(&sub, bx + 16.0, by + 41.0);
    ctx.set_text_baseline("alphabetic");
}

/// Trace a provenance marker (path only — caller fills/strokes): a circle, or the
/// provenance's polygon when the shape isn't round.
fn trace_marker(
    ctx: &CanvasRenderingContext2d,
    shape: crate::model::Shape,
    x: f64,
    y: f64,
    r: f64,
) {
    ctx.begin_path();
    match shape.points(x, y, r) {
        None => {
            let _ = ctx.arc(x, y, r, 0.0, PI * 2.0);
        }
        Some(pts) => {
            if let Some(&(x0, y0)) = pts.first() {
                ctx.move_to(x0, y0);
                for &(px, py) in &pts[1..] {
                    ctx.line_to(px, py);
                }
                ctx.close_path();
            }
        }
    }
}

/// Trace a rounded rectangle (path only — caller fills/strokes).
fn rounded_rect(ctx: &CanvasRenderingContext2d, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.begin_path();
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);
    let _ = ctx.arc_to(x + w, y, x + w, y + r, r);
    ctx.line_to(x + w, y + h - r);
    let _ = ctx.arc_to(x + w, y + h, x + w - r, y + h, r);
    ctx.line_to(x + r, y + h);
    let _ = ctx.arc_to(x, y + h, x, y + h - r, r);
    ctx.line_to(x, y + r);
    let _ = ctx.arc_to(x, y, x + r, y, r);
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
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
