//! Range projection (§7.5): merge UTC-day buckets, optionally regroup to eTLD+1,
//! and apply display filters. No raw-event read.

use crate::interpret::registrable;
use crate::model::{EdgeAgg, Granularity, GraphProjection, NodeAgg, ProvBreakdown, Provenance};
use crate::rollup::{split_edge_key, DayBucket, EdgeStat, NodeStat};
use std::collections::{HashMap, HashSet};

/// Display filters applied to the merged projection (§7.5).
#[derive(Clone, Debug, Default)]
pub struct Filters {
    pub min_visits: u32,
    pub hide_search_hubs: bool,
    pub provenance_in: Option<Vec<Provenance>>,
}

/// Merge the buckets spanning a range, regroup to the requested granularity, and
/// filter (§7.5).
pub fn project(buckets: &[DayBucket], gran: Granularity, filters: &Filters) -> GraphProjection {
    // 1. Merge buckets at stored (hostname) granularity.
    let mut nodes: HashMap<String, NodeStat> = HashMap::new();
    let mut edges: HashMap<(String, String), EdgeStat> = HashMap::new();
    for b in buckets {
        for (k, n) in &b.nodes {
            let e = nodes.entry(k.clone()).or_default();
            e.visits += n.visits;
            e.prov.merge(&n.prov);
        }
        for (k, ed) in &b.edges {
            if let Some((f, t)) = split_edge_key(k) {
                let e = edges.entry((f.to_string(), t.to_string())).or_default();
                e.weight += ed.weight;
                e.kinds.merge(&ed.kinds);
            }
        }
    }

    // 2. Regroup to eTLD+1 if requested; new self-loops are dropped (decision #6).
    if gran == Granularity::Registrable {
        let mut rn: HashMap<String, NodeStat> = HashMap::new();
        for (k, n) in nodes {
            let e = rn.entry(registrable(&k)).or_default();
            e.visits += n.visits;
            e.prov.merge(&n.prov);
        }
        nodes = rn;

        let mut re: HashMap<(String, String), EdgeStat> = HashMap::new();
        for ((f, t), ed) in edges {
            let (rf, rt) = (registrable(&f), registrable(&t));
            if rf == rt {
                continue; // self-loop in domain view
            }
            let e = re.entry((rf, rt)).or_default();
            e.weight += ed.weight;
            e.kinds.merge(&ed.kinds);
        }
        edges = re;
    }

    // 3. Apply display filters on the merged lists.
    let keep: HashSet<String> = nodes
        .iter()
        .filter(|(_, n)| {
            if n.visits < filters.min_visits {
                return false;
            }
            if filters.hide_search_hubs && n.prov.dominant() == Provenance::SearchOrigin {
                return false;
            }
            if let Some(allow) = &filters.provenance_in {
                if !allow.contains(&n.prov.dominant()) {
                    return false;
                }
            }
            true
        })
        .map(|(k, _)| k.clone())
        .collect();

    let mut node_vec: Vec<NodeAgg> = nodes
        .iter()
        .filter(|(k, _)| keep.contains(*k))
        .map(|(k, n)| NodeAgg {
            key: k.clone(),
            visits: n.visits,
            prov: n.prov,
        })
        .collect();
    node_vec.sort_by(|a, b| b.visits.cmp(&a.visits).then_with(|| a.key.cmp(&b.key)));

    let mut edge_vec: Vec<EdgeAgg> = edges
        .iter()
        .filter(|((f, t), _)| keep.contains(f) && keep.contains(t))
        .map(|((f, t), ed)| EdgeAgg {
            from: f.clone(),
            to: t.clone(),
            weight: ed.weight,
            kinds: ed.kinds,
        })
        .collect();
    edge_vec.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });

    GraphProjection {
        nodes: node_vec,
        edges: edge_vec,
    }
}

/// Total provenance breakdown across a range — the "origination" view (§M2).
pub fn origination(buckets: &[DayBucket]) -> ProvBreakdown {
    let mut p = ProvBreakdown::default();
    for b in buckets {
        for n in b.nodes.values() {
            p.merge(&n.prov);
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KindBreakdown;
    use crate::rollup::edge_key;

    fn bucket(date: &str) -> DayBucket {
        DayBucket {
            date: date.into(),
            ..Default::default()
        }
    }

    #[test]
    fn bucket_merge_sums_visits_and_weights() {
        let mut b1 = bucket("2021-01-01");
        b1.nodes.insert(
            "a.com".into(),
            NodeStat {
                visits: 2,
                prov: ProvBreakdown {
                    link: 2,
                    ..Default::default()
                },
            },
        );
        // every edge endpoint also has a node visit (as real rollups produce).
        b1.nodes.insert(
            "b.com".into(),
            NodeStat {
                visits: 1,
                ..Default::default()
            },
        );
        b1.edges.insert(
            edge_key("a.com", "b.com"),
            EdgeStat {
                weight: 1,
                kinds: KindBreakdown {
                    link: 1,
                    ..Default::default()
                },
            },
        );
        let mut b2 = bucket("2021-01-02");
        b2.nodes.insert(
            "a.com".into(),
            NodeStat {
                visits: 3,
                prov: ProvBreakdown {
                    link: 3,
                    ..Default::default()
                },
            },
        );
        b2.edges.insert(
            edge_key("a.com", "b.com"),
            EdgeStat {
                weight: 4,
                kinds: KindBreakdown {
                    link: 4,
                    ..Default::default()
                },
            },
        );

        let g = project(&[b1, b2], Granularity::Hostname, &Filters::default());
        assert_eq!(g.nodes.len(), 2); // a.com + b.com
        let a = g.nodes.iter().find(|n| n.key == "a.com").unwrap();
        assert_eq!(a.visits, 5);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].weight, 5);
    }

    #[test]
    fn registrable_regroup_drops_self_loops() {
        // gist.github.com -> github.com and an edge gist.github.com -> raw.github.com
        let mut b = bucket("2021-01-01");
        for h in ["gist.github.com", "raw.github.com", "other.org"] {
            b.nodes.insert(
                h.into(),
                NodeStat {
                    visits: 1,
                    ..Default::default()
                },
            );
        }
        b.edges.insert(
            edge_key("gist.github.com", "raw.github.com"),
            EdgeStat {
                weight: 2,
                ..Default::default()
            },
        );
        b.edges.insert(
            edge_key("gist.github.com", "other.org"),
            EdgeStat {
                weight: 1,
                ..Default::default()
            },
        );

        let g = project(&[b], Granularity::Registrable, &Filters::default());
        // github.com + other.org
        assert_eq!(g.nodes.len(), 2);
        assert!(g
            .nodes
            .iter()
            .any(|n| n.key == "github.com" && n.visits == 2));
        // The github->github edge is a self-loop and dropped; github->other.org survives.
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].from, "github.com");
        assert_eq!(g.edges[0].to, "other.org");
    }

    #[test]
    fn min_visits_filter_prunes_nodes_and_dangling_edges() {
        let mut b = bucket("2021-01-01");
        b.nodes.insert(
            "big.com".into(),
            NodeStat {
                visits: 10,
                ..Default::default()
            },
        );
        b.nodes.insert(
            "small.com".into(),
            NodeStat {
                visits: 1,
                ..Default::default()
            },
        );
        b.edges.insert(
            edge_key("big.com", "small.com"),
            EdgeStat {
                weight: 5,
                ..Default::default()
            },
        );
        let filters = Filters {
            min_visits: 5,
            ..Default::default()
        };
        let g = project(&[b], Granularity::Hostname, &filters);
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].key, "big.com");
        assert_eq!(g.edges.len(), 0); // edge to pruned node removed
    }

    #[test]
    fn hide_search_hubs_removes_search_dominant_nodes() {
        let mut b = bucket("2021-01-01");
        b.nodes.insert(
            "google.com".into(),
            NodeStat {
                visits: 9,
                prov: ProvBreakdown {
                    search_origin: 9,
                    ..Default::default()
                },
            },
        );
        b.nodes.insert(
            "wiki.org".into(),
            NodeStat {
                visits: 4,
                prov: ProvBreakdown {
                    link: 4,
                    ..Default::default()
                },
            },
        );
        let filters = Filters {
            hide_search_hubs: true,
            ..Default::default()
        };
        let g = project(&[b], Granularity::Hostname, &filters);
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].key, "wiki.org");
    }
}
