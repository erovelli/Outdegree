//! Pure helpers for the node inspector (§F8): a host's most-visited pages within
//! the displayed window, and its heaviest in/out graph connections.
//!
//! Kept pure (grouping / ordering / capping and edge selection) so they run under
//! `cargo test`; the wasm UI only supplies the bounded event slice / projection
//! and renders the result. The store enforces the read bound (the 20k-event cap);
//! this module never reads storage.

use crate::interpret;
use crate::model::{Event, Granularity, GraphProjection};
use std::collections::HashMap;
use url::Url;

/// One most-visited page within a host: a display path (`"/path?trimmed-query"`)
/// and how many times it was navigated to across the scanned events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageVisit {
    /// Path plus a length-capped query string (no scheme/host — those are the
    /// panel's title). Root renders as `"/"`.
    pub path_query: String,
    pub visits: u32,
}

/// The longest query string kept on a page row before it is truncated with `…`, so
/// a giant tracking query can't blow up an inspector row. Measured in characters.
const MAX_QUERY_CHARS: usize = 48;

/// Aggregate the most-visited pages of `host` from a slice of events (typically the
/// bounded window scan from `store`). Only `http(s)` `Nav` events whose node key at
/// `gran` equals `host` are counted — everything else (other hosts, non-nav,
/// non-http) is ignored, so the caller can pass an unfiltered window slice. Pages
/// are grouped by path + (truncated) query, counted, ordered by visits desc then
/// path asc (a total order → deterministic), and truncated to `top`.
pub fn top_pages(events: &[Event], host: &str, gran: Granularity, top: usize) -> Vec<PageVisit> {
    let mut agg: HashMap<String, u32> = HashMap::new();
    for ev in events {
        let Event::Nav { to_url, .. } = ev else {
            continue;
        };
        if interpret::node_key(to_url, gran).as_deref() != Some(host) {
            continue;
        }
        if let Some(pq) = page_path_query(to_url) {
            *agg.entry(pq).or_insert(0) += 1;
        }
    }
    let mut out: Vec<PageVisit> = agg
        .into_iter()
        .map(|(path_query, visits)| PageVisit { path_query, visits })
        .collect();
    out.sort_by(|a, b| {
        b.visits
            .cmp(&a.visits)
            .then_with(|| a.path_query.cmp(&b.path_query))
    });
    out.truncate(top);
    out
}

/// The display `"path + truncated query"` for a URL: the path, plus `?` and the
/// query truncated to [`MAX_QUERY_CHARS`]. `None` for an unparseable / non-`http(s)`
/// URL. An empty path renders as `"/"`.
fn page_path_query(raw: &str) -> Option<String> {
    let u = Url::parse(raw).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let path = match u.path() {
        "" => "/",
        p => p,
    };
    match u.query() {
        Some(q) if !q.is_empty() => Some(format!("{path}?{}", truncate_chars(q, MAX_QUERY_CHARS))),
        _ => Some(path.to_string()),
    }
}

/// Truncate `s` to at most `max` characters on a char boundary, appending `…` when
/// it was cut (so a multi-byte query never panics a byte-index slice).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// A host's top graph connections in the current projection: the sites visits to it
/// most often *came from* (in-edges) and the sites it most often *went to*
/// (out-edges), each `(host, weight)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeConnections {
    /// `(source host, weight)` — where visits to the node came from — heaviest first.
    pub came_from: Vec<(String, u32)>,
    /// `(destination host, weight)` — where the node led to — heaviest first.
    pub went_to: Vec<(String, u32)>,
}

/// Select `node`'s heaviest in- and out-edges from `proj`, up to `top` each, ordered
/// by weight desc then host name asc (a total order → deterministic). `proj` is the
/// currently projected graph (the focused component when drilled in), so a node's
/// full in/out neighbourhood is present.
pub fn node_connections(proj: &GraphProjection, node: &str, top: usize) -> NodeConnections {
    let by_weight =
        |a: &(String, u32), b: &(String, u32)| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0));
    let mut came_from: Vec<(String, u32)> = proj
        .edges
        .iter()
        .filter(|e| e.to == node)
        .map(|e| (e.from.clone(), e.weight))
        .collect();
    let mut went_to: Vec<(String, u32)> = proj
        .edges
        .iter()
        .filter(|e| e.from == node)
        .map(|e| (e.to.clone(), e.weight))
        .collect();
    came_from.sort_by(by_weight);
    went_to.sort_by(by_weight);
    came_from.truncate(top);
    went_to.truncate(top);
    NodeConnections { came_from, went_to }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeAgg, KindBreakdown, NodeAgg, ProvBreakdown};

    fn nav(id: f64, url: &str) -> Event {
        Event::Nav {
            id,
            ts: id,
            tab_id: 1.0,
            window_id: 1.0,
            to_url: url.to_string(),
            transition_type: "link".into(),
            qualifiers: vec![],
        }
    }

    #[test]
    fn top_pages_groups_orders_and_caps() {
        let events = vec![
            nav(1.0, "https://ex.com/a"),
            nav(2.0, "https://ex.com/a"), // /a twice
            nav(3.0, "https://ex.com/b?q=1&x=2"),
            nav(4.0, "https://ex.com/a?ref=z"), // distinct page (has a query)
            nav(5.0, "https://other.com/a"),    // different host → ignored
            nav(6.0, "chrome://settings/a"),    // non-http → ignored
            nav(7.0, "https://ex.com/"),
            nav(8.0, "https://ex.com/"),
            nav(9.0, "https://ex.com/"), // "/" three times → ranks first
        ];
        let top = top_pages(&events, "ex.com", Granularity::Hostname, 8);
        assert_eq!(
            top[0],
            PageVisit {
                path_query: "/".into(),
                visits: 3
            }
        );
        assert_eq!(
            top[1],
            PageVisit {
                path_query: "/a".into(),
                visits: 2
            }
        );
        // The remaining single-visit pages are ordered by path asc.
        assert_eq!(top[2].path_query, "/a?ref=z");
        assert_eq!(top[2].visits, 1);
        assert_eq!(top[3].path_query, "/b?q=1&x=2");
        // Only ex.com http(s) pages survive.
        assert_eq!(top.len(), 4);

        // `top` caps the returned rows.
        assert_eq!(
            top_pages(&events, "ex.com", Granularity::Hostname, 2).len(),
            2
        );
    }

    #[test]
    fn top_pages_folds_subdomains_at_registrable_gran() {
        let events = vec![
            nav(1.0, "https://a.ex.com/p"),
            nav(2.0, "https://b.ex.com/p"),
        ];
        // Both subdomains fold into ex.com and share the same path → one grouped row.
        let top = top_pages(&events, "ex.com", Granularity::Registrable, 8);
        assert_eq!(
            top,
            vec![PageVisit {
                path_query: "/p".into(),
                visits: 2
            }]
        );
    }

    #[test]
    fn top_pages_truncates_long_query() {
        let long = "v".repeat(120);
        let url = format!("https://ex.com/s?{long}");
        let top = top_pages(&[nav(1.0, &url)], "ex.com", Granularity::Hostname, 8);
        assert_eq!(top.len(), 1);
        assert!(top[0].path_query.starts_with("/s?"));
        assert!(top[0].path_query.ends_with('…'));
        assert!(top[0].path_query.chars().count() < url.chars().count());
    }

    fn edge(from: &str, to: &str, weight: u32) -> EdgeAgg {
        EdgeAgg {
            from: from.into(),
            to: to.into(),
            weight,
            kinds: KindBreakdown::default(),
        }
    }
    fn node(key: &str) -> NodeAgg {
        NodeAgg {
            key: key.into(),
            visits: 1,
            prov: ProvBreakdown::default(),
            dwell_ms: 0,
            fg_dwell_ms: 0,
        }
    }

    #[test]
    fn node_connections_selects_top5_in_and_out() {
        let proj = GraphProjection {
            nodes: vec![
                node("hub"),
                node("a"),
                node("b"),
                node("c"),
                node("d"),
                node("e"),
                node("f"),
                node("x"),
                node("y"),
            ],
            edges: vec![
                // in-edges (came from): weights 6,5,4,3,2,1 → the '1' drops at top-5.
                edge("a", "hub", 6),
                edge("b", "hub", 5),
                edge("c", "hub", 4),
                edge("d", "hub", 3),
                edge("e", "hub", 2),
                edge("f", "hub", 1),
                // out-edges (went to): a 9/9 tie broken by host name asc (x before y).
                edge("hub", "y", 9),
                edge("hub", "x", 9),
            ],
        };
        let c = node_connections(&proj, "hub", 5);
        assert_eq!(c.came_from.len(), 5);
        assert_eq!(c.came_from[0], ("a".into(), 6));
        assert_eq!(c.came_from[4], ("e".into(), 2));
        assert!(!c.came_from.iter().any(|(h, _)| h == "f")); // dropped by the cap
        assert_eq!(c.went_to, vec![("x".into(), 9), ("y".into(), 9)]);
    }
}
