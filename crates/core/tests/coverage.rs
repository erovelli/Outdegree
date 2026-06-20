//! Additional coverage for previously-untested branches: the `provenance_in`
//! filter, Louvain edge cases, and host/registrable corners.

use browsing_graph_core::graph::{build, louvain, louvain_communities};
use browsing_graph_core::interpret::{host, node_key, registrable};
use browsing_graph_core::model::{
    EdgeAgg, Granularity, GraphProjection, KindBreakdown, NodeAgg, ProvBreakdown, Provenance,
};
use browsing_graph_core::project::{origination, project, Filters};
use browsing_graph_core::rollup::{DayBucket, NodeStat};
use std::collections::HashMap;

// ───────────────────────────── interpret corners ─────────────────────────────

#[test]
fn host_strips_port_and_handles_ip() {
    assert_eq!(
        host("https://example.com:8443/p?q=1"),
        Some("example.com".into())
    );
    assert_eq!(host("http://192.168.1.1:8080/"), Some("192.168.1.1".into()));
    assert_eq!(host("http://[::1]:3000/"), Some("[::1]".into()));
}

#[test]
fn registrable_trailing_dot_and_deep_subdomains() {
    assert_eq!(registrable("example.com."), "example.com");
    assert_eq!(registrable("a.b.c.example.co.uk"), "example.co.uk");
    assert_eq!(
        node_key("https://a.b.c.example.co.uk/x", Granularity::Registrable),
        Some("example.co.uk".into())
    );
}

#[test]
fn host_punycodes_idn() {
    // url applies IDNA; the unicode host becomes its punycode form.
    let h = host("https://münchen.example/").unwrap();
    assert!(h.starts_with("xn--"), "expected punycode host, got {h}");
}

// ───────────────────────────── provenance_in filter ─────────────────────────────

fn node_stat(prov: Provenance, visits: u32) -> NodeStat {
    let mut p = ProvBreakdown::default();
    for _ in 0..visits {
        p.add(prov);
    }
    NodeStat { visits, prov: p }
}

#[test]
fn provenance_in_keeps_only_allowed_dominant() {
    let mut b = DayBucket {
        date: "2021-01-01".into(),
        ..Default::default()
    };
    b.nodes
        .insert("link.com".into(), node_stat(Provenance::Link, 3));
    b.nodes
        .insert("typed.com".into(), node_stat(Provenance::TypedUrl, 3));
    // an edge from a kept node to a filtered node must be pruned
    b.edges.insert(
        browsing_graph_core::rollup::edge_key("link.com", "typed.com"),
        browsing_graph_core::rollup::EdgeStat {
            weight: 2,
            kinds: KindBreakdown::default(),
        },
    );

    let filters = Filters {
        provenance_in: Some(vec![Provenance::Link]),
        ..Default::default()
    };
    let g = project(&[b], Granularity::Hostname, &filters);
    assert_eq!(g.nodes.len(), 1);
    assert_eq!(g.nodes[0].key, "link.com");
    assert_eq!(g.edges.len(), 0, "edge to filtered node pruned");
}

#[test]
fn origination_sums_provenance_across_buckets() {
    let mut b1 = DayBucket {
        date: "2021-01-01".into(),
        ..Default::default()
    };
    b1.nodes.insert("a".into(), node_stat(Provenance::Link, 2));
    let mut b2 = DayBucket {
        date: "2021-01-02".into(),
        ..Default::default()
    };
    b2.nodes.insert("a".into(), node_stat(Provenance::Link, 3));
    b2.nodes
        .insert("b".into(), node_stat(Provenance::TypedUrl, 1));
    let p = origination(&[b1, b2]);
    assert_eq!(p.link, 5);
    assert_eq!(p.typed_url, 1);
}

// ───────────────────────────── Louvain edge cases ─────────────────────────────

fn node(key: &str) -> NodeAgg {
    NodeAgg {
        key: key.into(),
        visits: 1,
        prov: ProvBreakdown::default(),
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
fn louvain_empty_graph() {
    let g = build(&GraphProjection::default());
    assert!(louvain(&g).is_empty());
    assert!(louvain_communities(0, &[]).is_empty());
}

#[test]
fn louvain_single_node() {
    let p = GraphProjection {
        nodes: vec![node("solo")],
        edges: vec![],
    };
    let g = build(&p);
    let comm = louvain(&g);
    assert_eq!(comm.len(), 1);
}

#[test]
fn louvain_isolated_nodes_are_separate_communities() {
    // three nodes, no edges -> three singleton communities
    let comm = louvain_communities(3, &[]);
    let distinct: std::collections::HashSet<usize> = comm.iter().copied().collect();
    assert_eq!(distinct.len(), 3);
}

#[test]
fn louvain_directed_pair_groups_symmetrized() {
    // a -> b (directed, heavy), c isolated. Symmetrization joins a,b.
    let p = GraphProjection {
        nodes: vec![node("a"), node("b"), node("c")],
        edges: vec![edge("a", "b", 10)],
    };
    let g = build(&p);
    let comm = louvain(&g);
    let mut by_comm: HashMap<usize, Vec<String>> = HashMap::new();
    for (ix, c) in &comm {
        by_comm.entry(*c).or_default().push(g[*ix].key.clone());
    }
    assert_eq!(by_comm.len(), 2, "expected {{a,b}} and {{c}}: {by_comm:?}");
    // a and b share a community
    let a = comm
        .iter()
        .find_map(|(ix, c)| (g[*ix].key == "a").then_some(*c))
        .unwrap();
    let b = comm
        .iter()
        .find_map(|(ix, c)| (g[*ix].key == "b").then_some(*c))
        .unwrap();
    assert_eq!(a, b);
}
