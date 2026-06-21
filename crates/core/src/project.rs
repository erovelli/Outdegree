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

/// Time window over the day-bucket history (design "Range" control). The window
/// is measured back from the most recent bucket present, so historical data is
/// still visible under the wider ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TimeRange {
    /// The most recent session ≈ the latest day with data.
    Session,
    Day,
    Week,
    Month,
    /// Default: effectively "all recent history" for a normal browsing record.
    #[default]
    Year,
}

impl TimeRange {
    /// Trailing window length in days (inclusive of the latest day).
    fn days(self) -> i64 {
        match self {
            TimeRange::Session | TimeRange::Day => 1,
            TimeRange::Week => 7,
            TimeRange::Month => 30,
            TimeRange::Year => 365,
        }
    }
}

/// Days since the Unix epoch for a `YYYY-MM-DD` UTC date (Howard Hinnant's
/// `days_from_civil`). Returns `None` for a malformed date.
fn day_number(date: &str) -> Option<i64> {
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// Keep only the buckets whose date falls within `range`'s trailing window of the
/// latest dated bucket. Buckets with unparseable dates (or when no date parses)
/// are passed through unchanged so a bad date can never blank the view.
pub fn select_window(buckets: &[DayBucket], range: TimeRange) -> Vec<DayBucket> {
    let latest = buckets.iter().filter_map(|b| day_number(&b.date)).max();
    let Some(latest) = latest else {
        return buckets.to_vec();
    };
    let cutoff = latest - (range.days() - 1);
    buckets
        .iter()
        .filter(|b| day_number(&b.date).map(|d| d >= cutoff).unwrap_or(true))
        .cloned()
        .collect()
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

/// Drill-down: the ego subgraph of `focus` — the node, its direct neighbors, and
/// the edges among that set (§M3).
pub fn ego(p: &GraphProjection, focus: &str) -> GraphProjection {
    let mut keep: HashSet<&str> = HashSet::new();
    keep.insert(focus);
    for e in &p.edges {
        if e.from == focus {
            keep.insert(e.to.as_str());
        }
        if e.to == focus {
            keep.insert(e.from.as_str());
        }
    }
    let nodes = p
        .nodes
        .iter()
        .filter(|n| keep.contains(n.key.as_str()))
        .cloned()
        .collect();
    let edges = p
        .edges
        .iter()
        .filter(|e| keep.contains(e.from.as_str()) && keep.contains(e.to.as_str()))
        .cloned()
        .collect();
    GraphProjection { nodes, edges }
}

/// Drill-down: the whole connected component containing `focus` — every node
/// reachable through edges (treated as undirected) and the edges among that set.
/// Unlike [`ego`] (1-hop), this is the node's *full* connected network (§M3).
pub fn component(p: &GraphProjection, focus: &str) -> GraphProjection {
    // Undirected adjacency over the projection's edges.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &p.edges {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        adj.entry(e.to.as_str()).or_default().push(e.from.as_str());
    }
    // Seed from the focus key as borrowed from `p` so every kept &str shares one
    // lifetime; BFS the component.
    let mut keep: HashSet<&str> = HashSet::new();
    if let Some(seed) = p
        .nodes
        .iter()
        .find(|n| n.key == focus)
        .map(|n| n.key.as_str())
    {
        keep.insert(seed);
        let mut stack = vec![seed];
        while let Some(u) = stack.pop() {
            if let Some(neighbors) = adj.get(u) {
                for &v in neighbors {
                    if keep.insert(v) {
                        stack.push(v);
                    }
                }
            }
        }
    }
    let nodes = p
        .nodes
        .iter()
        .filter(|n| keep.contains(n.key.as_str()))
        .cloned()
        .collect();
    let edges = p
        .edges
        .iter()
        .filter(|e| keep.contains(e.from.as_str()) && keep.contains(e.to.as_str()))
        .cloned()
        .collect();
    GraphProjection { nodes, edges }
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

    #[test]
    fn day_number_matches_known_epochs() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("1970-01-02"), Some(1));
        assert_eq!(day_number("2021-01-01"), Some(18628));
        assert!(day_number("not-a-date").is_none());
        // ordering across a month boundary
        assert!(day_number("2021-02-01") > day_number("2021-01-31"));
    }

    #[test]
    fn select_window_keeps_trailing_days_from_latest() {
        let bs = vec![
            bucket("2021-01-01"),
            bucket("2021-01-05"),
            bucket("2021-01-10"),
            bucket("2021-01-11"), // latest
        ];
        // Week = 7-day trailing window from 01-11 → cutoff 01-05.
        let w = select_window(&bs, TimeRange::Week);
        let dates: std::collections::HashSet<&str> = w.iter().map(|b| b.date.as_str()).collect();
        assert_eq!(dates, ["2021-01-05", "2021-01-10", "2021-01-11"].into());
        // Day = just the latest day.
        let d = select_window(&bs, TimeRange::Day);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].date, "2021-01-11");
        // Year keeps everything here.
        assert_eq!(select_window(&bs, TimeRange::Year).len(), 4);
    }

    #[test]
    fn select_window_passes_through_when_no_dates_parse() {
        let bs = vec![bucket("garbage"), bucket("also-bad")];
        assert_eq!(select_window(&bs, TimeRange::Day).len(), 2);
    }

    #[test]
    fn ego_returns_focus_plus_neighbors() {
        use crate::model::{EdgeAgg, NodeAgg};
        let n = |k: &str| NodeAgg {
            key: k.into(),
            visits: 1,
            prov: ProvBreakdown::default(),
        };
        let e = |f: &str, t: &str| EdgeAgg {
            from: f.into(),
            to: t.into(),
            weight: 1,
            kinds: crate::model::KindBreakdown::default(),
        };
        let p = GraphProjection {
            nodes: vec![n("hub"), n("a"), n("b"), n("far")],
            edges: vec![e("hub", "a"), e("b", "hub"), e("a", "far")],
        };
        let g = ego(&p, "hub");
        let keys: std::collections::HashSet<&str> =
            g.nodes.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["hub", "a", "b"].into_iter().collect());
        // only edges among the kept set survive (a->far is dropped)
        assert_eq!(g.edges.len(), 2);
        assert!(g.edges.iter().all(|x| x.to != "far" && x.from != "far"));
    }

    #[test]
    fn component_returns_the_whole_connected_network() {
        use crate::model::{EdgeAgg, NodeAgg};
        let n = |k: &str| NodeAgg {
            key: k.into(),
            visits: 1,
            prov: ProvBreakdown::default(),
        };
        let e = |f: &str, t: &str| EdgeAgg {
            from: f.into(),
            to: t.into(),
            weight: 1,
            kinds: crate::model::KindBreakdown::default(),
        };
        // Two components: a–b–c–d chain, and an isolated pair x–y.
        let p = GraphProjection {
            nodes: vec![n("a"), n("b"), n("c"), n("d"), n("x"), n("y")],
            edges: vec![e("a", "b"), e("b", "c"), e("c", "d"), e("x", "y")],
        };
        // Focusing `b` returns the *whole* chain (not just b's neighbors a,c).
        let g = component(&p, "b");
        let keys: std::collections::HashSet<&str> =
            g.nodes.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c", "d"].into_iter().collect());
        assert_eq!(g.edges.len(), 3);
        // The other component is excluded.
        assert!(!keys.contains("x") && !keys.contains("y"));
        // A node in the other component returns only its own pair.
        let gx = component(&p, "x");
        assert_eq!(gx.nodes.len(), 2);
    }
}
