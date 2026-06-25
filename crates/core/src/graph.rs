//! Graph construction + analysis (§7.6), all pure.
//!
//! * [`build`] — projection → `petgraph::DiGraph`.
//! * [`louvain`] — community detection on the **symmetrized** (undirected) graph.
//! * [`hubs`] — top nodes by weighted degree.
//! * [`top_edges`] — heaviest edges.
//! * [`frequent_sequences`] — PrefixSpan over per-tab linked chains.

use crate::model::{EdgeAgg, GraphProjection, NodeAgg};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use std::collections::HashMap;

/// Build a directed graph from a projection. Edges whose endpoints are not in the
/// node set are skipped.
pub fn build(p: &GraphProjection) -> DiGraph<NodeAgg, EdgeAgg> {
    let mut g = DiGraph::new();
    let mut idx: HashMap<&str, NodeIndex> = HashMap::new();
    for n in &p.nodes {
        let i = g.add_node(n.clone());
        idx.insert(n.key.as_str(), i);
    }
    for e in &p.edges {
        if let (Some(&a), Some(&b)) = (idx.get(e.from.as_str()), idx.get(e.to.as_str())) {
            g.add_edge(a, b, e.clone());
        }
    }
    g
}

/// Top `n` hub nodes by weighted degree (in + out edge weight). Returns
/// `(key, degree)` descending.
pub fn hubs(g: &DiGraph<NodeAgg, EdgeAgg>, n: usize) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = g
        .node_indices()
        .map(|i| {
            let deg: u32 = g
                .edges_directed(i, Direction::Outgoing)
                .chain(g.edges_directed(i, Direction::Incoming))
                .map(|e| e.weight().weight)
                .sum();
            (g[i].key.clone(), deg)
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

/// A ranked `(host, weight)` list (descending), as returned by the hub queries.
pub type RankedHosts = Vec<(String, u32)>;

/// Directional hubs: the top `n` **launch pads** (by outbound edge weight — where
/// journeys start) and top `n` **destinations** (by inbound edge weight — where
/// journeys end). Unlike [`hubs`], which sums both directions into one symmetric
/// degree, this surfaces the asymmetry a single column can't express: google.com
/// is a launch pad (high out, low in); a checkout or doc page is a sink (high in,
/// ~0 out). Nodes with zero weight in the relevant direction are omitted.
pub fn directional_hubs(g: &DiGraph<NodeAgg, EdgeAgg>, n: usize) -> (RankedHosts, RankedHosts) {
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut inb: Vec<(String, u32)> = Vec::new();
    for i in g.node_indices() {
        let o: u32 = g
            .edges_directed(i, Direction::Outgoing)
            .map(|e| e.weight().weight)
            .sum();
        let n_in: u32 = g
            .edges_directed(i, Direction::Incoming)
            .map(|e| e.weight().weight)
            .sum();
        if o > 0 {
            out.push((g[i].key.clone(), o));
        }
        if n_in > 0 {
            inb.push((g[i].key.clone(), n_in));
        }
    }
    let by_weight = |v: &mut Vec<(String, u32)>| {
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(n);
    };
    by_weight(&mut out);
    by_weight(&mut inb);
    (out, inb)
}

/// Heaviest `n` edges by weight.
pub fn top_edges(p: &GraphProjection, n: usize) -> Vec<EdgeAgg> {
    let mut e = p.edges.clone();
    e.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });
    e.truncate(n);
    e
}

/// Where your searches went: group edges that carry any `search_link` traversal
/// (a hop *out of* a search-results page) by destination host, summing the
/// search-link counts. Uses the raw `search_link` channel rather than the edge's
/// `dominant()` kind, so a destination reached by both search and plain links
/// isn't dropped when links happen to outnumber searches. Returns `(dest, count)`
/// descending.
pub fn search_destinations(p: &GraphProjection, n: usize) -> Vec<(String, u32)> {
    let mut by_dest: HashMap<&str, u32> = HashMap::new();
    for e in &p.edges {
        if e.kinds.search_link > 0 {
            *by_dest.entry(e.to.as_str()).or_insert(0) += e.kinds.search_link;
        }
    }
    let mut v: Vec<(String, u32)> = by_dest
        .into_iter()
        .map(|(k, c)| (k.to_string(), c))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

// ───────────────────────────── Louvain (symmetrized) ─────────────────────────────

/// Community assignment via Louvain on the symmetrized graph (§7.6).
/// Directed weights `a->b` and `b->a` are summed into one undirected weight.
pub fn louvain(g: &DiGraph<NodeAgg, EdgeAgg>) -> HashMap<NodeIndex, usize> {
    let nodes: Vec<NodeIndex> = g.node_indices().collect();
    let n = nodes.len();
    let index_of: HashMap<NodeIndex, usize> =
        nodes.iter().enumerate().map(|(i, &ix)| (ix, i)).collect();

    let mut wedges: Vec<(usize, usize, f64)> = Vec::with_capacity(g.edge_count());
    for e in g.edge_references_all() {
        let a = index_of[&e.0];
        let b = index_of[&e.1];
        wedges.push((a, b, e.2 as f64));
    }

    let comm = louvain_communities(n, &wedges);
    nodes
        .iter()
        .enumerate()
        .map(|(i, &ix)| (ix, comm[i]))
        .collect()
}

/// Internal helper: list `(from_idx, to_idx, weight)` for every directed edge.
trait EdgeRefsAll {
    fn edge_references_all(&self) -> Vec<(NodeIndex, NodeIndex, u32)>;
}
impl EdgeRefsAll for DiGraph<NodeAgg, EdgeAgg> {
    fn edge_references_all(&self) -> Vec<(NodeIndex, NodeIndex, u32)> {
        self.edge_indices()
            .filter_map(|ei| {
                let (a, b) = self.edge_endpoints(ei)?;
                Some((a, b, self[ei].weight))
            })
            .collect()
    }
}

/// Louvain community detection on an undirected weighted graph given as a flat
/// edge list (duplicate / reverse pairs are summed; self-loops allowed).
pub fn louvain_communities(n: usize, weighted_edges: &[(usize, usize, f64)]) -> Vec<usize> {
    if n == 0 {
        return vec![];
    }
    // node_comm[orig] = current super-node the original node belongs to.
    let mut node_comm: Vec<usize> = (0..n).collect();
    let mut cur_n = n;
    let mut cur_edges: Vec<(usize, usize, f64)> = weighted_edges.to_vec();

    loop {
        let (adj, k, m2) = build_adj(cur_n, &cur_edges);
        if m2 == 0.0 {
            break; // no edges: every node its own community
        }
        let (comm_of, n_comm) = one_level(cur_n, &adj, &k, m2);
        // thread original nodes through this level's relabeling
        for c in node_comm.iter_mut() {
            *c = comm_of[*c];
        }
        if n_comm == cur_n {
            break; // converged (no merging)
        }
        cur_edges = aggregate(&cur_edges, &comm_of);
        cur_n = n_comm;
    }
    relabel(&node_comm)
}

type Adj = Vec<Vec<(usize, f64)>>;

fn build_adj(n: usize, edges: &[(usize, usize, f64)]) -> (Adj, Vec<f64>, f64) {
    let mut pair: HashMap<(usize, usize), f64> = HashMap::new();
    let mut selfloop = vec![0.0f64; n];
    for &(u, v, w) in edges {
        if u == v {
            selfloop[u] += w;
        } else {
            let key = if u < v { (u, v) } else { (v, u) };
            *pair.entry(key).or_insert(0.0) += w;
        }
    }
    let mut adj: Adj = vec![Vec::new(); n];
    let mut k = vec![0.0f64; n];
    for (&(a, b), &w) in &pair {
        adj[a].push((b, w));
        adj[b].push((a, w));
        k[a] += w;
        k[b] += w;
    }
    for (i, &s) in selfloop.iter().enumerate() {
        k[i] += 2.0 * s; // self-loops count twice toward degree
    }
    let m2: f64 = k.iter().sum();
    (adj, k, m2)
}

/// One level of local modularity optimization. Returns `(community_of_node,
/// n_communities)` with communities relabeled to `0..n_communities`.
fn one_level(n: usize, adj: &Adj, k: &[f64], m2: f64) -> (Vec<usize>, usize) {
    let mut comm: Vec<usize> = (0..n).collect();
    let mut sigma_tot: Vec<f64> = k.to_vec(); // singletons: Σ_tot[c] = k[c]

    let mut improved = true;
    while improved {
        improved = false;
        for i in 0..n {
            let ci = comm[i];
            // remove i from its community
            sigma_tot[ci] -= k[i];

            // weight from i into each neighboring community
            let mut neigh: HashMap<usize, f64> = HashMap::new();
            for &(j, w) in &adj[i] {
                *neigh.entry(comm[j]).or_insert(0.0) += w;
            }

            let ki = k[i];
            // baseline: staying in ci
            let mut best_c = ci;
            let mut best_gain = neigh.get(&ci).copied().unwrap_or(0.0) - sigma_tot[ci] * ki / m2;
            for (&c, &kin) in &neigh {
                let gain = kin - sigma_tot[c] * ki / m2;
                if gain > best_gain + 1e-12 || (gain > best_gain - 1e-12 && c < best_c) {
                    best_gain = gain;
                    best_c = c;
                }
            }

            sigma_tot[best_c] += ki;
            if best_c != ci {
                comm[i] = best_c;
                improved = true;
            } else {
                comm[i] = ci;
            }
        }
    }
    let relabeled = relabel(&comm);
    let n_comm = relabeled.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    (relabeled, n_comm)
}

fn aggregate(edges: &[(usize, usize, f64)], comm_of: &[usize]) -> Vec<(usize, usize, f64)> {
    let mut map: HashMap<(usize, usize), f64> = HashMap::new();
    for &(u, v, w) in edges {
        let (cu, cv) = (comm_of[u], comm_of[v]);
        let key = if cu <= cv { (cu, cv) } else { (cv, cu) };
        *map.entry(key).or_insert(0.0) += w;
    }
    map.into_iter().map(|((u, v), w)| (u, v, w)).collect()
}

/// Compact community labels to `0..k` in order of first appearance.
fn relabel(comm: &[usize]) -> Vec<usize> {
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut next = 0;
    comm.iter()
        .map(|&c| {
            *remap.entry(c).or_insert_with(|| {
                let v = next;
                next += 1;
                v
            })
        })
        .collect()
}

// ───────────────────────────── PrefixSpan ─────────────────────────────

/// Mine frequent sequential patterns (length 2..=`max_len`) from per-tab linked
/// chains (§7.6). Support = number of chains containing the pattern as a
/// subsequence. Mining per-tab chains (not interleaved windows) avoids
/// fabricating cross-task A→B→C.
pub fn frequent_sequences(
    chains: &[Vec<String>],
    min_support: u32,
    max_len: usize,
) -> Vec<(Vec<String>, u32)> {
    let mut out = Vec::new();
    if max_len == 0 {
        return out;
    }
    // projected db entries: (chain_index, start_pos)
    let db: Vec<(usize, usize)> = (0..chains.len()).map(|i| (i, 0)).collect();
    prefixspan(chains, &db, &mut Vec::new(), min_support, max_len, &mut out);
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn prefixspan(
    chains: &[Vec<String>],
    db: &[(usize, usize)],
    prefix: &mut Vec<String>,
    min_support: u32,
    max_len: usize,
    out: &mut Vec<(Vec<String>, u32)>,
) {
    if prefix.len() >= max_len {
        return;
    }
    // For each item, count supporting chains (once each) and build the projected db
    // anchored at the first occurrence per chain.
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut projected: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for &(ci, pos) in db {
        let seq = &chains[ci];
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (j, item) in seq.iter().enumerate().skip(pos) {
            if seen.insert(item.as_str()) {
                *counts.entry(item.clone()).or_insert(0) += 1;
                projected.entry(item.clone()).or_default().push((ci, j + 1));
            }
        }
    }
    let mut items: Vec<(String, u32)> = counts
        .into_iter()
        .filter(|(_, c)| *c >= min_support)
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));

    for (item, sup) in items {
        prefix.push(item.clone());
        if prefix.len() >= 2 {
            out.push((prefix.clone(), sup));
        }
        if let Some(proj) = projected.remove(&item) {
            prefixspan(chains, &proj, prefix, min_support, max_len, out);
        }
        prefix.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeAgg, GraphProjection, KindBreakdown, NodeAgg, ProvBreakdown};

    fn node(key: &str, visits: u32) -> NodeAgg {
        NodeAgg {
            key: key.into(),
            visits,
            prov: ProvBreakdown::default(),
            ..Default::default()
        }
    }
    fn edge(f: &str, t: &str, w: u32) -> EdgeAgg {
        EdgeAgg {
            from: f.into(),
            to: t.into(),
            weight: w,
            kinds: KindBreakdown::default(),
        }
    }

    #[test]
    fn top_edges_orders_by_weight() {
        let p = GraphProjection {
            nodes: vec![node("a", 1), node("b", 1), node("c", 1)],
            edges: vec![edge("a", "b", 1), edge("b", "c", 9), edge("a", "c", 5)],
        };
        let te = top_edges(&p, 2);
        assert_eq!(te.len(), 2);
        assert_eq!((te[0].from.as_str(), te[0].to.as_str()), ("b", "c"));
        assert_eq!((te[1].from.as_str(), te[1].to.as_str()), ("a", "c"));
    }

    #[test]
    fn louvain_two_cliques_two_communities() {
        // Two triangles a-b-c and x-y-z joined by a single weak edge c-x.
        let p = GraphProjection {
            nodes: vec![
                node("a", 1),
                node("b", 1),
                node("c", 1),
                node("x", 1),
                node("y", 1),
                node("z", 1),
            ],
            edges: vec![
                edge("a", "b", 10),
                edge("b", "c", 10),
                edge("a", "c", 10),
                edge("x", "y", 10),
                edge("y", "z", 10),
                edge("x", "z", 10),
                edge("c", "x", 1),
            ],
        };
        let g = build(&p);
        let comm = louvain(&g);
        // group keys by community
        let mut by_comm: HashMap<usize, Vec<String>> = HashMap::new();
        for (ix, c) in &comm {
            by_comm.entry(*c).or_default().push(g[*ix].key.clone());
        }
        assert_eq!(
            by_comm.len(),
            2,
            "expected two communities, got {by_comm:?}"
        );
        // each community holds exactly one clique
        for members in by_comm.values() {
            let set: std::collections::HashSet<&str> = members.iter().map(|s| s.as_str()).collect();
            assert!(
                set == ["a", "b", "c"].into_iter().collect()
                    || set == ["x", "y", "z"].into_iter().collect(),
                "unexpected community members: {members:?}"
            );
        }
    }

    #[test]
    fn hubs_by_weighted_degree() {
        let p = GraphProjection {
            nodes: vec![node("hub", 1), node("a", 1), node("b", 1)],
            edges: vec![edge("hub", "a", 5), edge("hub", "b", 5), edge("a", "b", 1)],
        };
        let g = build(&p);
        let h = hubs(&g, 1);
        assert_eq!(h[0].0, "hub");
        assert_eq!(h[0].1, 10);
    }

    #[test]
    fn search_destinations_use_the_search_link_channel_not_dominant() {
        use crate::model::KindBreakdown;
        // google->wiki is mostly plain links (dominant = Link) but has 2 search
        // hops; google->docs is purely search. Both must surface as destinations.
        let p = GraphProjection {
            nodes: vec![node("google", 1), node("wiki", 1), node("docs", 1)],
            edges: vec![
                EdgeAgg {
                    from: "google".into(),
                    to: "wiki".into(),
                    weight: 12,
                    kinds: KindBreakdown {
                        link: 10,
                        search_link: 2,
                        ..Default::default()
                    },
                },
                EdgeAgg {
                    from: "google".into(),
                    to: "docs".into(),
                    weight: 5,
                    kinds: KindBreakdown {
                        search_link: 5,
                        ..Default::default()
                    },
                },
            ],
        };
        let d = search_destinations(&p, 10);
        assert_eq!(d, vec![("docs".to_string(), 5), ("wiki".to_string(), 2)]);
    }

    #[test]
    fn directional_hubs_split_sources_from_sinks() {
        // launch is a pure source (out 8, in 0); sink is a pure sink (in 8, out 0).
        // hubs() would rank both identically at weighted degree 8.
        let p = GraphProjection {
            nodes: vec![node("launch", 1), node("sink", 1)],
            edges: vec![edge("launch", "sink", 8)],
        };
        let g = build(&p);
        let (pads, dests) = directional_hubs(&g, 10);
        assert_eq!(pads, vec![("launch".to_string(), 8)]);
        assert_eq!(dests, vec![("sink".to_string(), 8)]);
        // A pure sink never appears as a launch pad, and vice-versa.
        assert!(!pads.iter().any(|(k, _)| k == "sink"));
        assert!(!dests.iter().any(|(k, _)| k == "launch"));
    }

    #[test]
    fn frequent_sequences_per_tab_chains() {
        let chains = vec![
            vec!["a".into(), "b".into(), "c".into()],
            vec!["a".into(), "b".into(), "d".into()],
            vec!["a".into(), "b".into(), "c".into()],
        ];
        let seqs = frequent_sequences(&chains, 2, 4);
        // a->b appears in all 3; a->b->c in 2.
        assert!(seqs
            .iter()
            .any(|(p, s)| p == &vec!["a".to_string(), "b".to_string()] && *s == 3));
        assert!(seqs.iter().any(|(p, s)| p
            == &vec!["a".to_string(), "b".to_string(), "c".to_string()]
            && *s == 2));
        // a->b->d only has support 1, below min_support 2.
        assert!(!seqs
            .iter()
            .any(|(p, _)| p == &vec!["a".to_string(), "b".to_string(), "d".to_string()]));
    }
}
