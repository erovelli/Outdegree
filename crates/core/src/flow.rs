//! Sankey flow layout (pure + testable): aggregate per-tab host chains into a
//! weighted, layered flow graph. The SVG rendering lives in `ui::sankey`; here we
//! only compute the columns (layers), per-node throughput, and link weights.
//!
//! Browsing flows are full of cycles (`google → site → google`), so we detect
//! back edges with a DFS and layer only the resulting DAG — cycles still draw as
//! ribbons but can't blow the column count up.

use std::collections::{HashMap, HashSet, VecDeque};

/// A host in the flow, assigned to a column (`layer`) and sized by `throughput`
/// (the larger of its total inbound / outbound transition weight).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowNode {
    pub key: String,
    pub layer: usize,
    pub throughput: u32,
}

/// A weighted transition `from → to` (indices into [`FlowGraph::nodes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowLink {
    pub from: usize,
    pub to: usize,
    pub weight: u32,
}

/// A laid-out flow graph: nodes (in column order) + links + the column count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowGraph {
    pub nodes: Vec<FlowNode>,
    pub links: Vec<FlowLink>,
    pub layers: usize,
}

/// Build a layered flow graph from per-tab host chains (consecutive duplicates
/// already collapsed by the caller). Transitions are aggregated within a chain.
pub fn build(chains: &[Vec<String>]) -> FlowGraph {
    // Index hosts by first appearance; aggregate consecutive transitions.
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut keys: Vec<String> = Vec::new();
    let mut link_w: HashMap<(usize, usize), u32> = HashMap::new();

    for chain in chains {
        let mut prev: Option<usize> = None;
        for host in chain {
            let i = *index.entry(host.clone()).or_insert_with(|| {
                keys.push(host.clone());
                keys.len() - 1
            });
            if let Some(p) = prev {
                if p != i {
                    *link_w.entry((p, i)).or_insert(0) += 1;
                }
            }
            prev = Some(i);
        }
    }

    let n = keys.len();
    if n == 0 {
        return FlowGraph::default();
    }

    let mut out_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_sum = vec![0u32; n];
    let mut out_sum = vec![0u32; n];
    for (&(u, v), &w) in &link_w {
        out_adj[u].push(v);
        out_sum[u] += w;
        in_sum[v] += w;
    }
    for a in out_adj.iter_mut() {
        a.sort_unstable(); // deterministic traversal
    }

    // Back-edge detection (iterative 3-color DFS): an edge into a node currently
    // on the stack (gray) closes a cycle.
    let mut color = vec![0u8; n]; // 0 white, 1 gray, 2 black
    let mut back: HashSet<(usize, usize)> = HashSet::new();
    for start in 0..n {
        if color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(u, ai)) = stack.last() {
            if ai < out_adj[u].len() {
                let v = out_adj[u][ai];
                stack.last_mut().unwrap().1 += 1;
                match color[v] {
                    0 => {
                        color[v] = 1;
                        stack.push((v, 0));
                    }
                    1 => {
                        back.insert((u, v));
                    }
                    _ => {}
                }
            } else {
                color[u] = 2;
                stack.pop();
            }
        }
    }

    // Session entry hosts (the first host of each tab chain) are pinned to the
    // leftmost column so the diagram reads as "where the session started → where
    // it went". Their incoming edges still draw (as ribbons curving back), they
    // just don't push the start host rightward.
    let starts: HashSet<usize> = chains
        .iter()
        .filter_map(|c| c.first())
        .filter_map(|h| index.get(h).copied())
        .collect();

    // Longest-path layering on the forward DAG (Kahn topological order). Edges
    // into a start host are excluded from layering so starts stay at column 0.
    let mut fwd: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(u, v) in link_w.keys() {
        if back.contains(&(u, v)) || starts.contains(&v) {
            continue;
        }
        fwd[u].push(v);
        indeg[v] += 1;
    }
    let mut layer = vec![0usize; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    while let Some(u) = queue.pop_front() {
        for &v in &fwd[u] {
            if layer[u] + 1 > layer[v] {
                layer[v] = layer[u] + 1;
            }
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }

    let layers = layer.iter().copied().max().unwrap_or(0) + 1;
    let nodes = keys
        .into_iter()
        .enumerate()
        .map(|(i, key)| FlowNode {
            key,
            layer: layer[i],
            throughput: in_sum[i].max(out_sum[i]).max(1),
        })
        .collect();
    let mut links: Vec<FlowLink> = link_w
        .into_iter()
        .map(|((from, to), weight)| FlowLink { from, to, weight })
        .collect();
    links.sort_by_key(|l| (l.from, l.to));

    FlowGraph {
        nodes,
        links,
        layers,
    }
}

/// Lay out a flow graph and emit an inline SVG Sankey: hosts are column bars,
/// transitions are bezier ribbons whose thickness ∝ weight. Pure (geometry +
/// string building) so it is testable; the caller just sets it as innerHTML.
pub fn render_svg(fg: &FlowGraph, vw: f64) -> String {
    let n = fg.nodes.len();
    if n == 0 {
        return "<div class=\"bg-empty\">No navigations in this session.</div>".into();
    }

    let node_w = 13.0;
    let pad_l = 18.0;
    let pad_r = 180.0; // label room on the right
    let pad_v = 24.0;
    let gap = 10.0;
    let min_h = 4.0;
    let base_h = 480.0_f64;
    let cols = fg.layers.max(1);

    // Group nodes into columns; biggest throughput toward the top of each.
    let mut by_layer: Vec<Vec<usize>> = vec![Vec::new(); cols];
    for (i, nd) in fg.nodes.iter().enumerate() {
        by_layer[nd.layer.min(cols - 1)].push(i);
    }
    for col in by_layer.iter_mut() {
        col.sort_by(|&a, &b| {
            fg.nodes[b]
                .throughput
                .cmp(&fg.nodes[a].throughput)
                .then_with(|| fg.nodes[a].key.cmp(&fg.nodes[b].key))
        });
    }

    // Pick a weight→pixels scale so the densest column fits the target height.
    let mut vscale = f64::INFINITY;
    for col in &by_layer {
        let sum: u32 = col.iter().map(|&i| fg.nodes[i].throughput).sum();
        if sum == 0 {
            continue;
        }
        let avail = (base_h - 2.0 * pad_v - (col.len() as f64 - 1.0) * gap).max(40.0);
        vscale = vscale.min(avail / sum as f64);
    }
    if !vscale.is_finite() {
        vscale = 1.0;
    }
    vscale = vscale.clamp(0.3, 10.0);

    let col_x = |l: usize| -> f64 {
        if cols > 1 {
            pad_l + l as f64 * (vw - pad_l - pad_r - node_w) / ((cols - 1) as f64)
        } else {
            pad_l
        }
    };

    // Place nodes; track the tallest column so the SVG can grow (and scroll).
    let mut nx = vec![0.0; n];
    let mut ny = vec![0.0; n];
    let mut nh = vec![0.0; n];
    let mut svg_h = base_h;
    for (l, col) in by_layer.iter().enumerate() {
        let heights: Vec<f64> = col
            .iter()
            .map(|&i| (fg.nodes[i].throughput as f64 * vscale).max(min_h))
            .collect();
        let col_h: f64 = heights.iter().sum::<f64>() + col.len().saturating_sub(1) as f64 * gap;
        svg_h = svg_h.max(col_h + 2.0 * pad_v);
        let mut y = pad_v + ((base_h - 2.0 * pad_v - col_h).max(0.0)) / 2.0;
        for (k, &i) in col.iter().enumerate() {
            nx[i] = col_x(l);
            ny[i] = y;
            nh[i] = heights[k];
            y += heights[k] + gap;
        }
    }

    // Stack link endpoints: outgoing along each source's right edge (ordered by
    // target y), incoming along each target's left edge (ordered by source y).
    let mut out_links: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_links: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (li, l) in fg.links.iter().enumerate() {
        out_links[l.from].push(li);
        in_links[l.to].push(li);
    }
    let nlinks = fg.links.len();
    let thick: Vec<f64> = fg
        .links
        .iter()
        .map(|l| (l.weight as f64 * vscale).max(1.0))
        .collect();
    let mut s_y0 = vec![0.0; nlinks];
    let mut t_y0 = vec![0.0; nlinks];
    let cmp_y = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
    for u in 0..n {
        let mut outs = out_links[u].clone();
        outs.sort_by(|&a, &b| cmp_y(ny[fg.links[a].to], ny[fg.links[b].to]));
        let mut off = ny[u];
        for li in outs {
            s_y0[li] = off;
            off += thick[li];
        }
        let mut ins = in_links[u].clone();
        ins.sort_by(|&a, &b| cmp_y(ny[fg.links[a].from], ny[fg.links[b].from]));
        let mut off = ny[u];
        for li in ins {
            t_y0[li] = off;
            off += thick[li];
        }
    }

    // Emit SVG: ribbons first (behind), then node bars + labels.
    let stops = [264.0, 288.0, 312.0, 340.0, 8.0, 30.0];
    let mut s = String::with_capacity(256 + nlinks * 200 + n * 120);
    s.push_str(&format!(
        "<svg class=\"sankey\" width=\"{vw:.0}\" height=\"{svg_h:.0}\" viewBox=\"0 0 {vw:.0} {svg_h:.0}\">"
    ));
    for (li, l) in fg.links.iter().enumerate() {
        let x0 = nx[l.from] + node_w;
        let x1 = nx[l.to];
        let (sy0, sy1) = (s_y0[li], s_y0[li] + thick[li]);
        let (ty0, ty1) = (t_y0[li], t_y0[li] + thick[li]);
        let cx = (x0 + x1) / 2.0;
        let hue = stops[l.from % stops.len()];
        s.push_str(&format!(
            "<path d=\"M{x0:.1} {sy0:.1} C{cx:.1} {sy0:.1} {cx:.1} {ty0:.1} {x1:.1} {ty0:.1} \
             L{x1:.1} {ty1:.1} C{cx:.1} {ty1:.1} {cx:.1} {sy1:.1} {x0:.1} {sy1:.1} Z\" \
             fill=\"oklch(0.64 0.17 {hue} / 0.34)\"/>"
        ));
    }
    for (i, nd) in fg.nodes.iter().enumerate() {
        s.push_str(&format!(
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{node_w:.1}\" height=\"{:.1}\" rx=\"2\" fill=\"#9a9aa0\"/>",
            nx[i], ny[i], nh[i]
        ));
        s.push_str(&format!(
            "<text class=\"sankey-label\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            nx[i] + node_w + 6.0,
            ny[i] + nh[i] / 2.0,
            esc_svg(&clip(&nd.key, 26))
        ));
    }
    s.push_str("</svg>");
    s
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

/// Minimal XML-text escape for host labels embedded in SVG.
fn esc_svg(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(hosts: &[&str]) -> Vec<String> {
        hosts.iter().map(|s| s.to_string()).collect()
    }
    fn layer_of(g: &FlowGraph, k: &str) -> usize {
        g.nodes.iter().find(|n| n.key == k).unwrap().layer
    }
    fn weight(g: &FlowGraph, f: &str, t: &str) -> Option<u32> {
        g.links
            .iter()
            .find(|l| g.nodes[l.from].key == f && g.nodes[l.to].key == t)
            .map(|l| l.weight)
    }

    #[test]
    fn aggregates_transitions_and_layers_a_path() {
        let g = build(&[chain(&["a", "b", "c"]), chain(&["a", "b"])]);
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(layer_of(&g, "a"), 0);
        assert_eq!(layer_of(&g, "b"), 1);
        assert_eq!(layer_of(&g, "c"), 2);
        assert_eq!(weight(&g, "a", "b"), Some(2));
        assert_eq!(weight(&g, "b", "c"), Some(1));
        assert_eq!(g.layers, 3);
    }

    #[test]
    fn throughput_is_max_of_in_and_out() {
        // hub: out to x and y (out_sum 2), in from s (in_sum 1) → throughput 2.
        let g = build(&[chain(&["s", "hub", "x"]), chain(&["hub", "y"])]);
        let hub = g.nodes.iter().find(|n| n.key == "hub").unwrap();
        assert_eq!(hub.throughput, 2);
    }

    #[test]
    fn cycles_do_not_blow_up_the_column_count() {
        let g = build(&[chain(&["a", "b", "a", "b"])]);
        // a→b forward, b→a back edge; both links kept, but only 2 columns.
        assert_eq!(g.layers, 2);
        assert_eq!(layer_of(&g, "a"), 0);
        assert_eq!(layer_of(&g, "b"), 1);
        assert_eq!(g.links.len(), 2);
        assert_eq!(weight(&g, "a", "b"), Some(2));
        assert_eq!(weight(&g, "b", "a"), Some(1));
    }

    #[test]
    fn session_start_hosts_are_pinned_to_the_left() {
        // `hub` starts tab 0, but in tab 1 it's navigated to from `b`. It must
        // still sit in column 0 (a session entry point), not get pushed right.
        let g = build(&[chain(&["hub", "a"]), chain(&["b", "hub", "c"])]);
        assert_eq!(layer_of(&g, "hub"), 0);
        assert_eq!(layer_of(&g, "b"), 0);
        assert_eq!(layer_of(&g, "a"), 1);
        assert_eq!(layer_of(&g, "c"), 1);
        // the revisit b→hub is still recorded as a link
        assert_eq!(weight(&g, "b", "hub"), Some(1));
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(build(&[]), FlowGraph::default());
        assert_eq!(build(&[chain(&["solo"])]).nodes.len(), 1);
        assert_eq!(build(&[chain(&["solo"])]).links.len(), 0);
    }

    #[test]
    fn render_svg_is_wellformed_with_one_rect_per_node_and_path_per_link() {
        let g = build(&[chain(&["a", "b", "c"]), chain(&["a", "c"])]);
        let svg = render_svg(&g, 900.0);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert_eq!(svg.matches("<rect").count(), g.nodes.len()); // 3
        assert_eq!(svg.matches("<path").count(), g.links.len()); // a→b, b→c, a→c
        assert!(svg.contains(">a<") && svg.contains(">c<"));
    }

    #[test]
    fn render_svg_escapes_and_handles_empty() {
        assert!(render_svg(&FlowGraph::default(), 900.0).contains("bg-empty"));
        let g = build(&[chain(&["x&<y", "z"])]);
        assert!(render_svg(&g, 900.0).contains("x&amp;&lt;y"));
    }
}
