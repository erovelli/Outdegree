//! Pure SVG serialization of the browsing graph for the dashboard's
//! "Export graph (SVG)" action. Mirrors the canvas2d renderer's visual language
//! (pure-black backdrop, provenance-colored node markers by shape, edge-kind
//! colored directional links) but as a standalone, fit-to-page vector file —
//! independent of the live camera's pan/zoom.
//!
//! Kept pure (no web_sys) so the framing math and XML escaping are unit-tested
//! under `cargo test`. Colors are inlined as `oklch(...)` exactly as the canvas
//! uses them, so the export carries the same single data-color spectrum and no
//! external stylesheet is needed.

use crate::layout::Pos;
use crate::model::GraphProjection;
use std::collections::HashMap;

const BG: &str = "#000000";
const LABEL: &str = "#8a8a90";
const MONO: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

/// Node disc radius for the export: `max(8, 2.6·√visits)·scale`, clamped — the
/// same shape law the canvas uses, so the busiest node reads as the largest.
fn radius(visits: u32, scale: f64) -> f64 {
    ((8.0_f64).max(2.6 * (visits as f64).sqrt()) * scale).clamp(2.0, 64.0)
}

/// XML-escape a string for embedding in attribute values / text nodes.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Truncate to `max` chars with an ellipsis, matching the canvas label clip.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Framing transform: screen = world·scale + (ox, oy). Frames all finite node
/// positions into the central 80%×80% of a `w`×`h` page (mirrors canvas2d::fit).
struct Fit {
    scale: f64,
    ox: f64,
    oy: f64,
}

impl Fit {
    fn of(proj: &GraphProjection, pos: &HashMap<String, Pos>, w: f64, h: f64) -> Fit {
        let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
        let (mut maxx, mut maxy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut any = false;
        for n in &proj.nodes {
            if let Some(p) = pos.get(&n.key) {
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
            return Fit {
                scale: 1.0,
                ox: w / 2.0,
                oy: h / 2.0,
            };
        }
        let bw = (maxx - minx).max(1.0);
        let bh = (maxy - miny).max(1.0);
        let scale = ((w * 0.8) / bw).min((h * 0.8) / bh).clamp(0.05, 3.0);
        let scale = if scale.is_finite() { scale } else { 1.0 };
        let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
        Fit {
            scale,
            ox: -cx * scale + w / 2.0,
            oy: -cy * scale + h / 2.0,
        }
    }

    fn project(&self, p: &Pos) -> (f64, f64) {
        (
            p.x as f64 * self.scale + self.ox,
            p.y as f64 * self.scale + self.oy,
        )
    }
}

/// Serialize `proj` (laid out by `pos`) as a standalone SVG document sized
/// `w`×`h`, fit-to-page. Node markers carry their provenance shape + color and
/// edges their dominant-kind color (search links dashed), with directional
/// arrowheads — the same encoding as the canvas, frozen as vectors.
pub fn graph_svg(proj: &GraphProjection, pos: &HashMap<String, Pos>, w: f64, h: f64) -> String {
    let f = Fit::of(proj, pos, w, h);
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" \
         viewBox=\"0 0 {w:.0} {h:.0}\">\n"
    ));
    out.push_str(&format!(
        "<rect width=\"{w:.0}\" height=\"{h:.0}\" fill=\"{BG}\"/>\n"
    ));

    // visits lookup so the edge shaft can stop at the target node's rim.
    let vis: HashMap<&str, u32> = proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.visits))
        .collect();

    // ── edges ────────────────────────────────────────────────────────────────
    out.push_str("<g stroke-linecap=\"round\">\n");
    for e in &proj.edges {
        let (Some(a), Some(b)) = (pos.get(&e.from), pos.get(&e.to)) else {
            continue;
        };
        let (ax, ay) = f.project(a);
        let (bx, by) = f.project(b);
        let kind = e.kinds.dominant();
        let color = kind.color();
        let dashed = matches!(kind, crate::model::EdgeKind::SearchLink);
        let lw = (1.2 * f.scale * (e.weight as f64).sqrt()).clamp(0.6, 6.0);
        // Stop the shaft at the target rim so the arrowhead reads cleanly.
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let tr = vis
            .get(e.to.as_str())
            .map(|v| radius(*v, f.scale))
            .unwrap_or(4.0);
        let (tipx, tipy) = if len > 1.0 {
            (bx - dx / len * (tr + 1.5), by - dy / len * (tr + 1.5))
        } else {
            (bx, by)
        };
        let dash = if dashed {
            " stroke-dasharray=\"5,5\""
        } else {
            ""
        };
        let opacity = if dashed { 0.5 } else { 0.34 };
        out.push_str(&format!(
            "<line x1=\"{ax:.1}\" y1=\"{ay:.1}\" x2=\"{tipx:.1}\" y2=\"{tipy:.1}\" \
             stroke=\"{color}\" stroke-width=\"{lw:.2}\" stroke-opacity=\"{opacity}\"{dash}/>\n"
        ));
        // Arrowhead (solid triangle at the target end).
        if len > tr + 4.0 {
            let (ux, uy) = (dx / len, dy / len);
            let (px, py) = (-uy, ux);
            let ah = (5.0 * f.scale).clamp(4.0, 10.0);
            let (bxh, byh) = (tipx - ux * ah, tipy - uy * ah);
            let p1 = (bxh + px * ah * 0.55, byh + py * ah * 0.55);
            let p2 = (bxh - px * ah * 0.55, byh - py * ah * 0.55);
            out.push_str(&format!(
                "<polygon points=\"{tipx:.1},{tipy:.1} {:.1},{:.1} {:.1},{:.1}\" \
                 fill=\"{color}\" fill-opacity=\"0.7\"/>\n",
                p1.0, p1.1, p2.0, p2.1
            ));
        }
    }
    out.push_str("</g>\n");

    // ── nodes (provenance color + shape, black moat) ──────────────────────────
    out.push_str(&format!("<g stroke=\"{BG}\" stroke-width=\"2\">\n"));
    for n in &proj.nodes {
        let Some(p) = pos.get(&n.key) else { continue };
        let (x, y) = f.project(p);
        let r = radius(n.visits, f.scale);
        let prov = n.prov.dominant().display();
        let color = prov.color();
        match prov.shape().points(x, y, r) {
            None => out.push_str(&format!(
                "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{r:.1}\" fill=\"{color}\"/>\n"
            )),
            Some(pts) => {
                let points = pts
                    .iter()
                    .map(|(px, py)| format!("{px:.1},{py:.1}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push_str(&format!(
                    "<polygon points=\"{points}\" fill=\"{color}\"/>\n"
                ));
            }
        }
    }
    out.push_str("</g>\n");

    // ── labels (decluttered: busiest first, skip on overlap) ──────────────────
    let fs = (13.0 * f.scale).clamp(8.0, 22.0);
    out.push_str(&format!(
        "<g font-family=\"{MONO}\" font-size=\"{fs:.0}\" fill=\"{LABEL}\" \
         text-anchor=\"middle\">\n"
    ));
    let mut placed: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut order: Vec<usize> = (0..proj.nodes.len()).collect();
    order.sort_by(|&a, &b| {
        proj.nodes[b]
            .visits
            .cmp(&proj.nodes[a].visits)
            .then(proj.nodes[a].key.cmp(&proj.nodes[b].key))
    });
    for i in order {
        let n = &proj.nodes[i];
        let Some(p) = pos.get(&n.key) else { continue };
        let (x, y) = f.project(p);
        let r = radius(n.visits, f.scale);
        let label = clip(&n.key, 28);
        let tw = label.chars().count() as f64 * fs * 0.6;
        let cy = y + r + fs + 2.0;
        let (bx0, bx1, by0, by1) = (x - tw / 2.0, x + tw / 2.0, cy - fs, cy + 2.0);
        let overlaps = placed
            .iter()
            .any(|&(ox0, oy0, ox1, oy1)| bx0 < ox1 && bx1 > ox0 && by0 < oy1 && by1 > oy0);
        if overlaps {
            continue;
        }
        placed.push((bx0, by0, bx1, by1));
        out.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{cy:.1}\">{}</text>\n",
            esc(&label)
        ));
    }
    out.push_str("</g>\n");

    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Pos;
    use crate::model::{EdgeAgg, KindBreakdown, NodeAgg, ProvBreakdown};

    fn node(key: &str, visits: u32) -> NodeAgg {
        NodeAgg {
            key: key.into(),
            visits,
            prov: ProvBreakdown::default(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_graph_is_valid_svg() {
        let proj = GraphProjection::default();
        let pos = HashMap::new();
        let svg = graph_svg(&proj, &pos, 800.0, 600.0);
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("viewBox=\"0 0 800 600\""));
        assert!(svg.contains(&format!("fill=\"{BG}\"")));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn renders_nodes_edges_and_escaped_labels() {
        let prov = ProvBreakdown {
            link: 3, // dominant → Link → circle
            ..Default::default()
        };
        let proj = GraphProjection {
            nodes: vec![
                NodeAgg {
                    key: "a&b.com".into(),
                    visits: 9,
                    prov,
                    ..Default::default()
                },
                node("c.com", 1),
            ],
            edges: vec![EdgeAgg {
                from: "a&b.com".into(),
                to: "c.com".into(),
                weight: 4,
                kinds: KindBreakdown {
                    link: 4,
                    ..Default::default()
                },
            }],
        };
        let mut pos = HashMap::new();
        pos.insert("a&b.com".into(), Pos { x: -50.0, y: 0.0 });
        pos.insert("c.com".into(), Pos { x: 50.0, y: 20.0 });
        let svg = graph_svg(&proj, &pos, 800.0, 600.0);
        // An edge line + an arrowhead polygon.
        assert!(svg.contains("<line "));
        assert!(svg.contains("<polygon "));
        // The Link-dominant node is a circle; labels are XML-escaped.
        assert!(svg.contains("<circle "));
        assert!(svg.contains("a&amp;b.com"));
        assert!(!svg.contains("a&b.com")); // raw ampersand must not leak
        assert!(svg.contains("c.com"));
    }

    #[test]
    fn esc_covers_xml_metacharacters() {
        assert_eq!(esc("a&b"), "a&amp;b");
        assert_eq!(esc("<x>"), "&lt;x&gt;");
        assert_eq!(esc("\"q\""), "&quot;q&quot;");
        assert_eq!(esc("'a'"), "&apos;a&apos;");
    }
}
