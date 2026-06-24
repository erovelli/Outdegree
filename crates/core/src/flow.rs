//! Sankey flow layout (pure + testable): aggregate per-tab host chains into a
//! weighted, layered flow graph. The SVG rendering lives in `ui::sankey`; here we
//! only compute the columns (layers), per-node throughput, and link weights.
//!
//! Browsing flows are full of cycles (`google → site → google`), so we detect
//! back edges with a DFS and layer only the resulting DAG — cycles still draw as
//! ribbons but can't blow the column count up.

use crate::interpret::{classify, node_key};
use crate::model::{EdgeKind, Event, Granularity, KindBreakdown, ProvBreakdown, Provenance, Shape};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// A host in the flow, assigned to a column (`layer`) and sized by `throughput`
/// (the larger of its total inbound / outbound transition weight). `prov` is the
/// host's dominant (display-folded) provenance — how it tended to be reached —
/// and colors/glyphs its bar; it is `Other` for the non-session builders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowNode {
    pub key: String,
    pub layer: usize,
    pub throughput: u32,
    pub prov: Provenance,
}

/// A weighted transition `from → to` (indices into [`FlowGraph::nodes`]). `kind`
/// is the dominant edge kind of the hop (colors the ribbon); `Link` by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowLink {
    pub from: usize,
    pub to: usize,
    pub weight: u32,
    pub kind: EdgeKind,
}

/// A laid-out flow graph: nodes (in column order) + links + the column count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowGraph {
    pub nodes: Vec<FlowNode>,
    pub links: Vec<FlowLink>,
    pub layers: usize,
}

/// Build a layered flow graph from per-tab host chains (consecutive duplicates
/// already collapsed). Each chain's first host is a session start; consecutive
/// hosts are transitions. For session flow that spans tabs (links opened in new
/// tabs), use [`build_transitions`] with explicit cross-tab bridges instead.
pub fn build(chains: &[Vec<String>]) -> FlowGraph {
    let mut transitions: Vec<(String, String)> = Vec::new();
    let mut starts: Vec<String> = Vec::new();
    for chain in chains {
        if let Some(first) = chain.first() {
            starts.push(first.clone());
        }
        for w in chain.windows(2) {
            transitions.push((w[0].clone(), w[1].clone()));
        }
    }
    build_transitions(&transitions, &starts)
}

fn intern(h: &str, index: &mut HashMap<String, usize>, keys: &mut Vec<String>) -> usize {
    if let Some(&i) = index.get(h) {
        return i;
    }
    let i = keys.len();
    keys.push(h.to_string());
    index.insert(h.to_string(), i);
    i
}

/// Build a layered flow graph from explicit directed transitions plus the set of
/// session-start hosts (pinned to the leftmost column). `starts` hosts with no
/// transitions still appear as nodes. This is the cross-tab-aware entry point:
/// a link opened in a new tab is just a transition `source-host → new-tab-host`.
pub fn build_transitions(transitions: &[(String, String)], starts: &[String]) -> FlowGraph {
    build_flow(transitions, starts, &[])
}

/// Core builder. `pinned_starts` are interned and pinned to column 0 (their
/// inbound links curve back). `detached_starts` are added as *separate* column-0
/// "entry point" nodes that are **not** merged with the host's chain node — so a
/// host reached both mid-chain (`A → B → C`) and on its own (a bookmark to `B`)
/// keeps the chain intact and shows the bookmark as a standalone `B` start,
/// instead of yanking the chain's `B` back to the left. Detached starts are
/// aggregated per host (count → throughput).
fn build_flow(
    transitions: &[(String, String)],
    pinned_starts: &[String],
    detached_starts: &[String],
) -> FlowGraph {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut keys: Vec<String> = Vec::new();
    let mut link_w: HashMap<(usize, usize), u32> = HashMap::new();
    for (from, to) in transitions {
        let fi = intern(from, &mut index, &mut keys);
        let ti = intern(to, &mut index, &mut keys);
        if fi != ti {
            *link_w.entry((fi, ti)).or_insert(0) += 1;
        }
    }
    // Session entry hosts are pinned to the leftmost column so the diagram reads
    // as "where the session started → where it went". Intern them so isolated
    // starts (a tab with a single nav) still appear.
    let start_set: HashSet<usize> = pinned_starts
        .iter()
        .map(|h| intern(h, &mut index, &mut keys))
        .collect();

    let n = keys.len();
    if n == 0 && detached_starts.is_empty() {
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

    // Longest-path layering on the forward DAG (Kahn topological order). Edges
    // into a start host are excluded from layering so starts stay at column 0
    // (their incoming ribbons still draw, just curving back).
    let mut fwd: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(u, v) in link_w.keys() {
        if back.contains(&(u, v)) || start_set.contains(&v) {
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
    let mut nodes: Vec<FlowNode> = keys
        .into_iter()
        .enumerate()
        .map(|(i, key)| FlowNode {
            key,
            layer: layer[i],
            throughput: in_sum[i].max(out_sum[i]).max(1),
            prov: Provenance::Other,
        })
        .collect();
    // Standalone entry points (e.g. a bookmark to a mid-chain host): one extra
    // column-0 node per host, sized by how many times it was entered. They carry
    // no links, so they read as lone start bars beside the pinned starts.
    let mut detached: BTreeMap<&str, u32> = BTreeMap::new();
    for h in detached_starts {
        *detached.entry(h.as_str()).or_insert(0) += 1;
    }
    for (key, count) in detached {
        // A detached host is always a link target, so it already has a chain node.
        // Only keep a *separate* column-0 entry bar when that chain node is
        // mid-chain (layer ≥ 1); if it already sits at column 0 (e.g. it lies on a
        // cycle, whose inbound back-edge is excluded from layering), a second bar
        // is pure duplication — fold the entry count into the chain node instead.
        if let Some(&idx) = index.get(key) {
            if layer[idx] == 0 {
                nodes[idx].throughput = nodes[idx].throughput.max(count);
                continue;
            }
        }
        nodes.push(FlowNode {
            key: key.to_string(),
            layer: 0,
            throughput: count.max(1),
            prov: Provenance::Other,
        });
    }
    let mut links: Vec<FlowLink> = link_w
        .into_iter()
        .map(|((from, to), weight)| FlowLink {
            from,
            to,
            weight,
            kind: EdgeKind::Link,
        })
        .collect();
    links.sort_by_key(|l| (l.from, l.to));

    FlowGraph {
        nodes,
        links,
        layers,
    }
}

/// Build the session flow directly from the session's raw `events`, honoring how
/// each page was reached (§7.2):
///
/// - **Link / Form** navigations chain — they add a transition from the tab's
///   current page (or, for a link opened in a new tab, the source page).
/// - **Typed-URL / bookmark / search / start** navigations are *rootless*: they
///   begin a **new flow** (a fresh left-column start), not an edge from wherever
///   you happened to be. So jumping to a bookmark or pasting a URL while on site
///   B does not draw a `B → …` ribbon — it starts its own web. And if that landing
///   host is *also* a real mid-chain node (`A → B → C` and you bookmark `B`), the
///   chain stays intact and the re-entry draws as its own standalone `B` start,
///   instead of pulling the chain's `B` back to column 0.
/// - **Reload / other** change nothing.
///
/// This is the cross-tab-aware, provenance-aware entry point used by the Sankey.
pub fn from_session_events(events: &[Event], gran: Granularity) -> FlowGraph {
    use crate::rollup::REDIRECT_WINDOW_MS;

    /// A nav held one event for redirect lookahead, mirroring `derive::Buffered`.
    struct Buf {
        origin: Option<(String, Provenance)>,
        to: String,
        prov: Provenance,
        ts: f64,
    }

    let mut current: HashMap<i64, (String, Provenance)> = HashMap::new(); // tab → confirmed page
    let mut buffer: HashMap<i64, Buf> = HashMap::new(); // tab → pending (lookahead) nav
    let mut pending: HashMap<i64, (String, Provenance)> = HashMap::new(); // new tab → spawn origin
    let mut transitions: Vec<(String, String)> = Vec::new();
    let mut starts: Vec<String> = Vec::new();
    // Display provenance per host (how it was reached) and dominant edge kind per
    // hop — attached to the built graph so bars/ribbons can be colored.
    let mut host_prov: HashMap<String, ProvBreakdown> = HashMap::new();
    let mut trans_kind: HashMap<(String, String), KindBreakdown> = HashMap::new();

    // Flush a stable buffered nav: record its provenance, then emit either a
    // transition (a real link/form hop) or a column-0 start. Mirrors
    // `derive::finalize`'s edge/kind rules so the Sankey agrees with the graph.
    let flush = |buf: Buf,
                 current: &mut HashMap<i64, (String, Provenance)>,
                 host_prov: &mut HashMap<String, ProvBreakdown>,
                 trans_kind: &mut HashMap<(String, String), KindBreakdown>,
                 transitions: &mut Vec<(String, String)>,
                 starts: &mut Vec<String>,
                 t: i64| {
        host_prov.entry(buf.to.clone()).or_default().add(buf.prov);
        if buf.prov.is_edge() {
            if let Some((o, o_prov)) = &buf.origin {
                if *o != buf.to {
                    let kind = if *o_prov == Provenance::SearchOrigin {
                        EdgeKind::SearchLink
                    } else if buf.prov == Provenance::Form {
                        EdgeKind::Form
                    } else {
                        EdgeKind::Link
                    };
                    trans_kind
                        .entry((o.clone(), buf.to.clone()))
                        .or_default()
                        .add(kind);
                    transitions.push((o.clone(), buf.to.clone()));
                    current.insert(t, (buf.to.clone(), buf.prov));
                    return;
                }
            }
        }
        starts.push(buf.to.clone());
        current.insert(t, (buf.to, buf.prov));
    };

    for ev in events {
        match ev {
            Event::Link {
                new_tab_id,
                source_tab_id,
                ..
            } => {
                let src = *source_tab_id as i64;
                // The spawn origin is the source's *buffered* page (what's on screen
                // now), falling back to its confirmed page — exactly `derive::on_link`.
                let origin = buffer
                    .get(&src)
                    .map(|b| (b.to.clone(), b.prov))
                    .or_else(|| current.get(&src).cloned());
                if let Some(o) = origin {
                    pending.insert(*new_tab_id as i64, o);
                }
            }
            Event::Close { tab_id, .. } => {
                let t = *tab_id as i64;
                if let Some(buf) = buffer.remove(&t) {
                    flush(
                        buf,
                        &mut current,
                        &mut host_prov,
                        &mut trans_kind,
                        &mut transitions,
                        &mut starts,
                        t,
                    );
                }
            }
            Event::Start { .. } => {
                let mut tabs: Vec<i64> = buffer.keys().copied().collect();
                tabs.sort_unstable(); // deterministic flush order
                for t in tabs {
                    if let Some(buf) = buffer.remove(&t) {
                        flush(
                            buf,
                            &mut current,
                            &mut host_prov,
                            &mut trans_kind,
                            &mut transitions,
                            &mut starts,
                            t,
                        );
                    }
                }
            }
            Event::Nav {
                tab_id,
                to_url,
                transition_type,
                qualifiers,
                ts,
                ..
            } => {
                let t = *tab_id as i64;
                let Some(host) = node_key(to_url, gran) else {
                    continue;
                };
                let prov = classify(transition_type);
                if prov.is_ignored() {
                    continue; // reload / other: no flow change
                }
                let has_redirect = qualifiers.iter().any(|q| q == "client_redirect");
                let has_fb = qualifiers.iter().any(|q| q == "forward_back");

                // Redirect continuation: extend the burst in place (carry the
                // origin, advance the landing) without emitting an intermediate hop,
                // so A→r1→r2→B collapses to a single A→B ribbon — like the graph.
                if has_redirect {
                    if let Some(buf) = buffer.get(&t) {
                        if (*ts - buf.ts) < REDIRECT_WINDOW_MS {
                            let origin = buf.origin.clone();
                            buffer.insert(
                                t,
                                Buf {
                                    origin,
                                    to: host,
                                    prov,
                                    ts: *ts,
                                },
                            );
                            continue;
                        }
                    }
                }

                // Not a redirect continuation: finalize the buffered (now-stable) nav.
                if let Some(buf) = buffer.remove(&t) {
                    flush(
                        buf,
                        &mut current,
                        &mut host_prov,
                        &mut trans_kind,
                        &mut transitions,
                        &mut starts,
                        t,
                    );
                }

                // forward_back: advance position only — no bar, no ribbon (it's a
                // back-button move, not a fresh traversal). Matches `derive`.
                if has_fb {
                    current.insert(t, (host, prov));
                    continue;
                }

                // Collapse consecutive duplicates of the current page.
                if current.get(&t).map(|(h, _)| h.as_str()) == Some(host.as_str()) {
                    continue;
                }

                // Compute this nav's origin and buffer it for one step of lookahead.
                let origin = if prov.is_edge() {
                    pending.remove(&t).or_else(|| current.get(&t).cloned())
                } else {
                    // Rootless (typed / bookmark / search / start): a fresh origin,
                    // not an edge from the previous page nor a consumed spawn link.
                    pending.remove(&t);
                    None
                };
                buffer.insert(
                    t,
                    Buf {
                        origin,
                        to: host,
                        prov,
                        ts: *ts,
                    },
                );
            }
        }
    }
    // Flush every still-open page in deterministic tab order.
    let mut tabs: Vec<i64> = buffer.keys().copied().collect();
    tabs.sort_unstable();
    for t in tabs {
        if let Some(buf) = buffer.remove(&t) {
            flush(
                buf,
                &mut current,
                &mut host_prov,
                &mut trans_kind,
                &mut transitions,
                &mut starts,
                t,
            );
        }
    }
    // A rootless start whose host is *also* a link target (a real mid-chain node)
    // becomes a standalone entry node, so re-entering it (e.g. a bookmark) doesn't
    // drag the chain's node back to column 0 — the chain stays intact and the
    // re-entry shows as its own start. Pure starts stay pinned as before.
    let targets: HashSet<&str> = transitions.iter().map(|(_, t)| t.as_str()).collect();
    let (detached, pinned): (Vec<String>, Vec<String>) = starts
        .into_iter()
        .partition(|h| targets.contains(h.as_str()));
    let mut fg = build_flow(&transitions, &pinned, &detached);

    // Attach the display provenance to each bar and the dominant kind to each hop.
    for node in &mut fg.nodes {
        if let Some(b) = host_prov.get(&node.key) {
            node.prov = b.dominant().display();
        }
    }
    let keys: Vec<String> = fg.nodes.iter().map(|n| n.key.clone()).collect();
    for link in &mut fg.links {
        if let Some(b) = trans_kind.get(&(keys[link.from].clone(), keys[link.to].clone())) {
            link.kind = b.dominant();
        }
    }
    fg
}

/// Reconstruct the per-tab **link/form chains** from a session's raw events, for
/// frequent-journey mining ([`crate::graph::frequent_sequences`]). Each chain is
/// an ordered host sequence following natural traversal; a rootless nav
/// (typed/bookmark/search/start) ends the current chain and begins a new one, so
/// chains never fabricate a cross-task A→B→C. Redirect bursts collapse onto the
/// landing host and forward_back moves are skipped, matching the Sankey/graph.
/// Only chains of length ≥ 2 (an actual journey) are returned.
pub fn session_chains(events: &[Event], gran: Granularity) -> Vec<Vec<String>> {
    use crate::rollup::REDIRECT_WINDOW_MS;
    let mut cur: HashMap<i64, Vec<String>> = HashMap::new();
    let mut pending: HashMap<i64, String> = HashMap::new();
    let mut last_ts: HashMap<i64, f64> = HashMap::new();
    let mut out: Vec<Vec<String>> = Vec::new();

    let flush = |cur: &mut HashMap<i64, Vec<String>>, out: &mut Vec<Vec<String>>, t: i64| {
        if let Some(c) = cur.remove(&t) {
            if c.len() >= 2 {
                out.push(c);
            }
        }
    };

    for ev in events {
        match ev {
            Event::Link {
                new_tab_id,
                source_tab_id,
                ..
            } => {
                let src = *source_tab_id as i64;
                if let Some(last) = cur.get(&src).and_then(|c| c.last()) {
                    pending.insert(*new_tab_id as i64, last.clone());
                }
            }
            Event::Close { tab_id, .. } => flush(&mut cur, &mut out, *tab_id as i64),
            Event::Start { .. } => {
                let mut tabs: Vec<i64> = cur.keys().copied().collect();
                tabs.sort_unstable();
                for t in tabs {
                    flush(&mut cur, &mut out, t);
                }
            }
            Event::Nav {
                tab_id,
                to_url,
                transition_type,
                qualifiers,
                ts,
                ..
            } => {
                let t = *tab_id as i64;
                let Some(host) = node_key(to_url, gran) else {
                    continue;
                };
                let prov = classify(transition_type);
                if prov.is_ignored() {
                    continue;
                }
                let recent = last_ts.get(&t).map(|p| (*ts - p) < REDIRECT_WINDOW_MS);
                last_ts.insert(t, *ts);
                if qualifiers.iter().any(|q| q == "client_redirect") && recent == Some(true) {
                    // Collapse the redirect onto the landing host in place.
                    if let Some(c) = cur.get_mut(&t) {
                        if let Some(last) = c.last_mut() {
                            *last = host;
                        } else {
                            c.push(host);
                        }
                    }
                    continue;
                }
                if qualifiers.iter().any(|q| q == "forward_back") {
                    continue; // position move; don't grow the chain
                }
                if cur.get(&t).and_then(|c| c.last()) == Some(&host) {
                    continue; // consecutive duplicate
                }
                if prov.is_edge() {
                    let chain = cur.entry(t).or_default();
                    if chain.is_empty() {
                        if let Some(o) = pending.remove(&t) {
                            chain.push(o);
                        }
                    } else {
                        pending.remove(&t);
                    }
                    chain.push(host);
                } else {
                    // Rootless: end the current chain, begin a fresh one.
                    pending.remove(&t);
                    flush(&mut cur, &mut out, t);
                    cur.insert(t, vec![host]);
                }
            }
        }
    }
    let mut tabs: Vec<i64> = cur.keys().copied().collect();
    tabs.sort_unstable();
    for t in tabs {
        flush(&mut cur, &mut out, t);
    }
    out
}

/// Partition a flow into the sub-graph that actually flows (nodes that are an
/// endpoint of at least one ribbon) and the **orphans** — column-0 hosts you
/// reached directly (typed/bookmark/search) that link to nothing. Returns the
/// participating sub-graph (re-indexed, ready for [`render_svg`]) plus the orphan
/// `(host, throughput)` list sorted by throughput desc. This keeps the Sankey
/// from being mostly empty column-0 bars, surfacing direct visits as a side list.
pub fn split_orphans(fg: &FlowGraph) -> (FlowGraph, Vec<(String, u32)>) {
    let n = fg.nodes.len();
    let mut touched = vec![false; n];
    for l in &fg.links {
        touched[l.from] = true;
        touched[l.to] = true;
    }
    // Re-index the participating nodes; old index → new index.
    let mut remap: Vec<Option<usize>> = vec![None; n];
    let mut nodes: Vec<FlowNode> = Vec::new();
    for (i, t) in touched.iter().enumerate() {
        if *t {
            remap[i] = Some(nodes.len());
            nodes.push(fg.nodes[i].clone());
        }
    }
    let links: Vec<FlowLink> = fg
        .links
        .iter()
        .filter_map(|l| {
            Some(FlowLink {
                from: remap[l.from]?,
                to: remap[l.to]?,
                weight: l.weight,
                kind: l.kind,
            })
        })
        .collect();
    let layers = nodes
        .iter()
        .map(|n| n.layer)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    let mut orphans: Vec<(String, u32)> = fg
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !touched[*i])
        .map(|(_, nd)| (nd.key.clone(), nd.throughput))
        .collect();
    orphans.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    (
        FlowGraph {
            nodes,
            links,
            layers,
        },
        orphans,
    )
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

    // Per-column bar heights, and the tallest column → the SVG sizes to its real
    // content (floored modestly) instead of always reserving a fixed 480px box, so
    // a sparse session no longer floats in a sea of empty canvas.
    let col_heights: Vec<Vec<f64>> = by_layer
        .iter()
        .map(|col| {
            col.iter()
                .map(|&i| (fg.nodes[i].throughput as f64 * vscale).max(min_h))
                .collect()
        })
        .collect();
    let col_total = |hs: &[f64]| hs.iter().sum::<f64>() + hs.len().saturating_sub(1) as f64 * gap;
    let content_h = col_heights
        .iter()
        .map(|hs| col_total(hs))
        .fold(0.0_f64, f64::max);
    let svg_h = (content_h + 2.0 * pad_v).clamp(160.0, base_h.max(content_h + 2.0 * pad_v));

    // Place nodes, vertically centering each column within the actual svg height.
    let mut nx = vec![0.0; n];
    let mut ny = vec![0.0; n];
    let mut nh = vec![0.0; n];
    for (l, col) in by_layer.iter().enumerate() {
        let heights = &col_heights[l];
        let col_h = col_total(heights);
        let mut y = pad_v + ((svg_h - 2.0 * pad_v - col_h).max(0.0)) / 2.0;
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

    // Emit SVG: ribbons first (behind), then node bars + glyph + labels.
    let mut s = String::with_capacity(256 + nlinks * 200 + n * 160);
    s.push_str(&format!(
        "<svg class=\"sankey\" width=\"{vw:.0}\" height=\"{svg_h:.0}\" viewBox=\"0 0 {vw:.0} {svg_h:.0}\">"
    ));
    for (li, l) in fg.links.iter().enumerate() {
        let x0 = nx[l.from] + node_w;
        let x1 = nx[l.to];
        let (sy0, sy1) = (s_y0[li], s_y0[li] + thick[li]);
        let (ty0, ty1) = (t_y0[li], t_y0[li] + thick[li]);
        let cx = (x0 + x1) / 2.0;
        // Ribbon color = the hop's edge kind, matching the graph's edge palette.
        let hue = match l.kind {
            EdgeKind::SearchLink => 264.0,
            EdgeKind::Link => 288.0,
            EdgeKind::Form => 8.0,
        };
        // A <title> gives a native hover tooltip (from → to ×weight) with no JS,
        // and the `.sankey-ribbon` class lets CSS brighten the hovered hop.
        let times = if l.weight == 1 { "time" } else { "times" };
        let tip = format!(
            "{} → {} · {} {times}",
            esc_svg(&fg.nodes[l.from].key),
            esc_svg(&fg.nodes[l.to].key),
            l.weight
        );
        s.push_str(&format!(
            "<path class=\"sankey-ribbon\" d=\"M{x0:.1} {sy0:.1} C{cx:.1} {sy0:.1} {cx:.1} {ty0:.1} {x1:.1} {ty0:.1} \
             L{x1:.1} {ty1:.1} C{cx:.1} {ty1:.1} {cx:.1} {sy1:.1} {x0:.1} {sy1:.1} Z\" \
             fill=\"oklch(0.6 0.14 {hue} / 0.4)\"><title>{tip}</title></path>"
        ));
    }
    for (i, nd) in fg.nodes.iter().enumerate() {
        // Bar = provenance color; a provenance glyph + label sit to its right.
        let prov = nd.prov;
        let hops = if nd.throughput == 1 { "hop" } else { "hops" };
        let bar_tip = format!("{} · {} {hops}", esc_svg(&nd.key), nd.throughput);
        s.push_str(&format!(
            "<rect class=\"sankey-bar\" x=\"{:.1}\" y=\"{:.1}\" width=\"{node_w:.1}\" height=\"{:.1}\" rx=\"2\" fill=\"{}\"><title>{bar_tip}</title></rect>",
            nx[i], ny[i], nh[i], prov.color()
        ));
        let gx = nx[i] + node_w + 9.0;
        let gy = ny[i] + nh[i] / 2.0;
        s.push_str(&marker_svg(prov.shape(), gx, gy, 4.5, prov.color()));
        s.push_str(&format!(
            "<text class=\"sankey-label\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            gx + 9.0,
            gy,
            esc_svg(&clip(&nd.key, 24))
        ));
    }
    s.push_str("</svg>");
    s
}

/// A small provenance marker as inline SVG (`<circle>` or `<polygon>`) so node
/// bars carry the CVD-safe shape channel beside their color.
fn marker_svg(shape: Shape, cx: f64, cy: f64, r: f64, fill: &str) -> String {
    match shape.points(cx, cy, r) {
        None => format!("<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"{fill}\"/>"),
        Some(pts) => {
            let mut p = String::with_capacity(pts.len() * 12);
            for (x, y) in pts {
                p.push_str(&format!("{x:.1},{y:.1} "));
            }
            format!("<polygon points=\"{}\" fill=\"{fill}\"/>", p.trim_end())
        }
    }
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
    fn has(g: &FlowGraph, f: &str, t: &str) -> bool {
        weight(g, f, t).is_some()
    }
    fn nav(id: u64, tab: i64, url: &str, tt: &str) -> Event {
        Event::Nav {
            id: id as f64,
            ts: id as f64,
            tab_id: tab as f64,
            window_id: 1.0,
            to_url: url.to_string(),
            transition_type: tt.to_string(),
            qualifiers: vec![],
        }
    }
    fn nav_q(id: u64, ts: f64, tab: i64, url: &str, tt: &str, quals: &[&str]) -> Event {
        Event::Nav {
            id: id as f64,
            ts,
            tab_id: tab as f64,
            window_id: 1.0,
            to_url: url.to_string(),
            transition_type: tt.to_string(),
            qualifiers: quals.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn rootless_navs_start_a_new_flow_not_an_edge() {
        // Start at A (typed), click to B (link), then *type* C, then click to D.
        // Typing C must NOT draw B→C; C is a fresh start, and C→D chains from it.
        let events = vec![
            nav(1, 1, "https://a.com/", "typed"),
            nav(2, 1, "https://b.com/", "link"),
            nav(3, 1, "https://c.com/", "typed"),
            nav(4, 1, "https://d.com/", "link"),
            nav(5, 1, "https://e.com/", "auto_bookmark"), // a bookmark is also rootless
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        assert!(has(&g, "a.com", "b.com"));
        assert!(has(&g, "c.com", "d.com"));
        assert!(
            !has(&g, "b.com", "c.com"),
            "typed URL must not chain from B"
        );
        assert!(!has(&g, "d.com", "e.com"), "bookmark must not chain from D");
        // Every rootless landing is a leftmost start.
        assert_eq!(layer_of(&g, "a.com"), 0);
        assert_eq!(layer_of(&g, "c.com"), 0);
        assert_eq!(layer_of(&g, "e.com"), 0);
    }

    #[test]
    fn reentering_a_mid_chain_host_keeps_the_chain_and_adds_a_standalone_start() {
        // A → B → C by link, then a bookmark back to B. The chain must stay
        // A(0) → B(1) → C(2), and B must *also* appear as its own column-0 start
        // (the bookmark) rather than dragging the chain's B back to the left.
        let events = vec![
            nav(1, 1, "https://a.com/", "typed"),
            nav(2, 1, "https://b.com/", "link"),
            nav(3, 1, "https://c.com/", "link"),
            nav(4, 1, "https://b.com/", "auto_bookmark"),
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        // chain intact
        assert!(has(&g, "a.com", "b.com"));
        assert!(has(&g, "b.com", "c.com"));
        assert_eq!(g.links.len(), 2, "bookmark must not add an edge");
        assert_eq!(layer_of(&g, "a.com"), 0);
        assert_eq!(layer_of(&g, "c.com"), 2, "chain still reaches column 2");
        // b.com appears twice: the chain node (layer 1) and a standalone start (0)
        let b_layers: Vec<usize> = g
            .nodes
            .iter()
            .filter(|n| n.key == "b.com")
            .map(|n| n.layer)
            .collect();
        assert_eq!(
            b_layers.len(),
            2,
            "expected a chain B and a standalone start B"
        );
        assert!(b_layers.contains(&1), "chain B stays mid-layer");
        assert!(b_layers.contains(&0), "bookmark B is a column-0 start");
    }

    #[test]
    fn repeated_reentry_aggregates_into_one_standalone_start() {
        // Bookmark B twice while A → B → C exists: one standalone B start, sized 2.
        let events = vec![
            nav(1, 1, "https://a.com/", "typed"),
            nav(2, 1, "https://b.com/", "link"),
            nav(3, 1, "https://c.com/", "link"),
            nav(4, 1, "https://b.com/", "auto_bookmark"),
            nav(5, 1, "https://x.com/", "typed"), // move away so the next B re-enters
            nav(6, 1, "https://b.com/", "auto_bookmark"),
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        let standalone = g
            .nodes
            .iter()
            .filter(|n| n.key == "b.com" && n.layer == 0)
            .collect::<Vec<_>>();
        assert_eq!(standalone.len(), 1, "re-entries collapse to one start node");
        assert_eq!(standalone[0].throughput, 2, "sized by entry count");
    }

    #[test]
    fn directly_entered_host_on_a_cycle_is_not_duplicated_at_column_0() {
        // Open gh.com directly (start_page = External), then gh.com → site → gh.com.
        // gh.com is both a session entry *and* a link target, but its chain node
        // sits at column 0 (the inbound edge is a cycle back-edge, excluded from
        // layering), so it must render as a single bar — not a duplicate column-0
        // entry beside it (the regression from the detached-start heuristic).
        let events = vec![
            nav(1, 1, "https://gh.com/", "start_page"),
            nav(1, 1, "https://site.com/", "link"),
            nav(1, 1, "https://gh.com/", "link"),
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        let gh_count = g.nodes.iter().filter(|n| n.key == "gh.com").count();
        assert_eq!(
            gh_count, 1,
            "a directly-entered cyclic host must not duplicate"
        );
        assert_eq!(layer_of(&g, "gh.com"), 0);
        assert!(has(&g, "gh.com", "site.com"));
        assert!(has(&g, "site.com", "gh.com"));
    }

    #[test]
    fn link_navs_still_chain_and_reload_is_ignored() {
        let events = vec![
            nav(1, 1, "https://a.com/", "typed"),
            nav(2, 1, "https://b.com/", "link"),
            nav(3, 1, "https://b.com/", "reload"), // ignored: no change
            nav(4, 1, "https://c.com/", "form_submit"), // form chains like a link
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        assert!(has(&g, "a.com", "b.com"));
        assert!(has(&g, "b.com", "c.com"));
        assert_eq!(g.links.len(), 2);
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
    fn build_transitions_bridges_new_tab_links() {
        // linkedin (session start) opens two job boards in new tabs; one job
        // board navigates onward. The bridges make the boards non-isolated.
        let tr: Vec<(String, String)> = [
            ("linkedin.com", "jobs1.com"),
            ("linkedin.com", "jobs2.com"),
            ("jobs1.com", "apply.com"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let g = build_transitions(&tr, &["linkedin.com".to_string()]);
        assert_eq!(layer_of(&g, "linkedin.com"), 0);
        assert_eq!(layer_of(&g, "jobs1.com"), 1);
        assert_eq!(layer_of(&g, "jobs2.com"), 1);
        assert_eq!(layer_of(&g, "apply.com"), 2);
        assert_eq!(g.links.len(), 3);
        // jobs2 has an inbound bridge → throughput 1, not an isolated node
        assert_eq!(
            g.nodes
                .iter()
                .find(|n| n.key == "jobs2.com")
                .unwrap()
                .throughput,
            1
        );
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

    #[test]
    fn client_redirect_burst_collapses_to_one_ribbon() {
        // a (typed) → b (link), then b client-redirects to c within the window.
        // The Sankey must show a→c (the landing), never a→b or b→c, and b must not
        // appear as a node — matching how the graph collapses the burst.
        let events = vec![
            nav_q(1, 100.0, 1, "https://a.com/", "typed", &[]),
            nav_q(2, 200.0, 1, "https://b.com/", "link", &[]),
            nav_q(3, 300.0, 1, "https://c.com/", "link", &["client_redirect"]),
            nav_q(4, 5_000.0, 1, "https://d.com/", "link", &[]), // flushes the burst
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        assert!(has(&g, "a.com", "c.com"), "burst collapses a→c");
        assert!(has(&g, "c.com", "d.com"));
        assert!(!has(&g, "a.com", "b.com"), "intermediate hop suppressed");
        assert!(!has(&g, "b.com", "c.com"));
        assert!(
            !g.nodes.iter().any(|n| n.key == "b.com"),
            "redirect hop is not a node"
        );
    }

    #[test]
    fn forward_back_navs_do_not_fabricate_a_ribbon() {
        // a→b by link, then a back-button move to c, then c→d by link. The
        // back-nav must not draw b→c; c chains onward normally.
        let events = vec![
            nav_q(1, 100.0, 1, "https://a.com/", "typed", &[]),
            nav_q(2, 200.0, 1, "https://b.com/", "link", &[]),
            nav_q(3, 300.0, 1, "https://c.com/", "link", &["forward_back"]),
            nav_q(4, 400.0, 1, "https://d.com/", "link", &[]),
        ];
        let g = from_session_events(&events, Granularity::Hostname);
        assert!(has(&g, "a.com", "b.com"));
        assert!(has(&g, "c.com", "d.com"));
        assert!(!has(&g, "b.com", "c.com"), "back-button isn't a traversal");
    }

    #[test]
    fn session_chains_split_on_rootless_and_mine_journeys() {
        let events = vec![
            nav_q(1, 100.0, 1, "https://a.com/", "typed", &[]),
            nav_q(2, 200.0, 1, "https://b.com/", "link", &[]),
            nav_q(3, 300.0, 1, "https://c.com/", "link", &[]),
            nav_q(4, 400.0, 1, "https://d.com/", "typed", &[]), // rootless: new chain
            nav_q(5, 500.0, 1, "https://e.com/", "link", &[]),
        ];
        let chains = session_chains(&events, Granularity::Hostname);
        assert!(chains.contains(&vec!["a.com".into(), "b.com".into(), "c.com".into()]));
        assert!(chains.contains(&vec!["d.com".into(), "e.com".into()]));
        // A single rootless visit with no link is not a journey.
        let lone = session_chains(
            &[nav_q(1, 1.0, 1, "https://solo.com/", "typed", &[])],
            Granularity::Hostname,
        );
        assert!(lone.is_empty());
    }

    #[test]
    fn split_orphans_separates_direct_visits_from_the_flow() {
        // a→b is a real flow; c is a lone start (a direct visit linking nowhere).
        let g = build(&[chain(&["a", "b"]), chain(&["c"])]);
        let (flow, orphans) = split_orphans(&g);
        let keys: std::collections::HashSet<&str> =
            flow.nodes.iter().map(|n| n.key.as_str()).collect();
        assert_eq!(keys, ["a", "b"].into_iter().collect());
        assert_eq!(flow.links.len(), 1);
        assert!(has(&flow, "a", "b"), "remapped link survives");
        assert_eq!(orphans, vec![("c".to_string(), 1)]);
    }
}
