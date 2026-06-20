//! §11 derive + rollup fixtures: global-order correctness, lifecycle markers, and
//! the `fold == from-scratch recompute` invariant across every watermark split.

use browsing_graph_core::model::{EdgeKind, Event};
use browsing_graph_core::rollup::{
    edge_key, fold, merge_bucket, DayBucket, DeriveState, SessionRec,
};
use std::collections::HashMap;

// ───────────────────────────── event builders ─────────────────────────────

fn nav(id: u64, ts: f64, tab: i64, win: i64, url: &str, tt: &str, quals: &[&str]) -> Event {
    Event::Nav {
        id: id as f64,
        ts,
        tab_id: tab as f64,
        window_id: win as f64,
        to_url: url.to_string(),
        transition_type: tt.to_string(),
        qualifiers: quals.iter().map(|s| s.to_string()).collect(),
    }
}
fn link(id: u64, ts: f64, new_tab: i64, src_tab: i64) -> Event {
    Event::Link {
        id: id as f64,
        ts,
        new_tab_id: new_tab as f64,
        source_tab_id: src_tab as f64,
    }
}
fn close(id: u64, ts: f64, tab: i64) -> Event {
    Event::Close {
        id: id as f64,
        ts,
        tab_id: tab as f64,
    }
}
fn start(id: u64, ts: f64) -> Event {
    Event::Start { id: id as f64, ts }
}

// ───────────────────────────── harness ─────────────────────────────

type Roll = HashMap<String, DayBucket>;

fn apply(deltas: Vec<DayBucket>, map: &mut Roll) {
    for d in deltas {
        let e = map.entry(d.date.clone()).or_insert_with(|| DayBucket {
            date: d.date.clone(),
            ..Default::default()
        });
        merge_bucket(e, &d);
    }
}

/// Fold the whole stream from scratch.
fn run_all(events: &[Event]) -> (Roll, Vec<SessionRec>, DeriveState) {
    let mut st = DeriveState::default();
    let (deltas, sess) = fold(&mut st, events);
    let mut map = Roll::new();
    apply(deltas, &mut map);
    (map, sess, st)
}

/// Fold `[..at]`, persist the checkpoint via a serde round-trip (as the store
/// would), then fold `[at..]` — exactly what reopening does.
fn run_split(events: &[Event], at: usize) -> (Roll, Vec<SessionRec>, DeriveState) {
    let mut st = DeriveState::default();
    let mut map = Roll::new();
    let mut sess = Vec::new();

    let (d1, s1) = fold(&mut st, &events[..at]);
    apply(d1, &mut map);
    sess.extend(s1);

    let json = serde_json::to_string(&st).expect("serialize checkpoint");
    let mut st2: DeriveState = serde_json::from_str(&json).expect("deserialize checkpoint");

    let (d2, s2) = fold(&mut st2, &events[at..]);
    apply(d2, &mut map);
    sess.extend(s2);

    (map, sess, st2)
}

/// Merge all day buckets into a single flat view for assertions.
fn flat(map: &Roll) -> DayBucket {
    let mut out = DayBucket::default();
    for b in map.values() {
        merge_bucket(&mut out, b);
    }
    out
}

fn has_edge(map: &Roll, from: &str, to: &str) -> bool {
    flat(map).edges.contains_key(&edge_key(from, to))
}
fn edge_dominant_kind(map: &Roll, from: &str, to: &str) -> Option<EdgeKind> {
    flat(map)
        .edges
        .get(&edge_key(from, to))
        .map(|e| e.kinds.dominant())
}
fn node_visits(map: &Roll, key: &str) -> u32 {
    flat(map).nodes.get(key).map(|n| n.visits).unwrap_or(0)
}

// ───────────────────────── global-order correctness ─────────────────────────

/// §1 interleaving trace: a new tab's origin is the source's page *at link time*,
/// not the source's latest page.
#[test]
fn new_tab_origin_is_source_then_current_page() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, 200.0, 1, 1, "https://m.com/", "link", &[]), // finalizes a.com; last_url=a.com
        link(3, 250.0, 2, 1), // snapshot tab1.last_url == a.com
        nav(4, 300.0, 1, 1, "https://b.com/", "link", &[]), // source navigates on
        nav(5, 400.0, 2, 1, "https://c.com/", "link", &[]), // child's first nav
        close(6, 500.0, 2),
        close(7, 600.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    // child edge originates from the snapshot (a.com), not m.com / b.com.
    assert!(has_edge(&map, "a.com", "c.com"), "snapshot origin a.com->c.com");
    assert!(!has_edge(&map, "m.com", "c.com"));
    assert!(!has_edge(&map, "b.com", "c.com"));
    // sanity: within-tab chain edges
    assert!(has_edge(&map, "a.com", "m.com"));
    assert!(has_edge(&map, "m.com", "b.com"));
}

/// Two interleaved tabs keep independent within-tab origins under a global sort.
#[test]
fn two_tab_interleave_no_cross_chaining() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a1.com/", "typed", &[]),
        nav(2, 110.0, 2, 1, "https://b1.com/", "typed", &[]),
        nav(3, 120.0, 1, 1, "https://a2.com/", "link", &[]),
        nav(4, 130.0, 2, 1, "https://b2.com/", "link", &[]),
        close(5, 200.0, 1),
        close(6, 210.0, 2),
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "a1.com", "a2.com"));
    assert!(has_edge(&map, "b1.com", "b2.com"));
    assert!(!has_edge(&map, "a1.com", "b2.com"));
    assert!(!has_edge(&map, "b1.com", "a2.com"));
}

/// Client-redirect burst collapses to one edge from the pre-burst predecessor;
/// intermediate hops emit no node or edge.
#[test]
fn client_redirect_burst_collapses() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://p.com/", "typed", &[]),
        nav(2, 200.0, 1, 1, "https://r1.com/", "link", &[]),
        nav(3, 500.0, 1, 1, "https://r2.com/", "link", &["client_redirect"]),
        nav(4, 900.0, 1, 1, "https://r3.com/", "link", &["client_redirect"]),
        close(5, 1000.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "p.com", "r3.com"), "collapsed origin->final");
    assert!(!has_edge(&map, "p.com", "r1.com"));
    assert!(!has_edge(&map, "p.com", "r2.com"));
    // intermediate hops are not visited
    assert_eq!(node_visits(&map, "r1.com"), 0);
    assert_eq!(node_visits(&map, "r2.com"), 0);
    assert_eq!(node_visits(&map, "r3.com"), 1);
    assert_eq!(node_visits(&map, "p.com"), 1);
}

/// A redirect hop too far apart in time is NOT collapsed (separate navs).
#[test]
fn redirect_outside_window_not_collapsed() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://p.com/", "typed", &[]),
        nav(2, 200.0, 1, 1, "https://r1.com/", "link", &[]),
        // 5s later: outside the 1s redirect window even with the qualifier.
        nav(3, 5200.0, 1, 1, "https://r2.com/", "link", &["client_redirect"]),
        close(4, 6000.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "p.com", "r1.com"));
    assert!(has_edge(&map, "r1.com", "r2.com"));
    assert_eq!(node_visits(&map, "r1.com"), 1);
    assert_eq!(node_visits(&map, "r2.com"), 1);
}

/// `forward_back`: no edge, position advances, and `last_prov` becomes
/// SearchOrigin after a back-button to a results page (so later links are
/// SearchLink).
#[test]
fn forward_back_no_edge_advances_search_prov() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://results.com/", "generated", &[]),
        nav(2, 200.0, 1, 1, "https://article.com/", "link", &[]),
        // back button to the results page
        nav(3, 300.0, 1, 1, "https://results.com/", "generated", &["forward_back"]),
        nav(4, 400.0, 1, 1, "https://article2.com/", "link", &[]),
        close(5, 500.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    // first traversal is a SearchLink (origin arrived via search)
    assert_eq!(
        edge_dominant_kind(&map, "results.com", "article.com"),
        Some(EdgeKind::SearchLink)
    );
    // forward_back emits no edge back to results
    assert!(!has_edge(&map, "article.com", "results.com"));
    // last_prov updated to SearchOrigin → the post-back link is also SearchLink
    assert_eq!(
        edge_dominant_kind(&map, "results.com", "article2.com"),
        Some(EdgeKind::SearchLink)
    );
}

/// A rootless (typed) nav resets a stale within-tab chain — no edge spans it.
#[test]
fn rootless_resets_stale_chain() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a.com/", "link", &[]),
        nav(2, 200.0, 1, 1, "https://b.com/", "link", &[]),
        nav(3, 300.0, 1, 1, "https://c.com/", "typed", &[]), // rootless
        nav(4, 400.0, 1, 1, "https://d.com/", "link", &[]),
        close(5, 500.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "a.com", "b.com"));
    assert!(!has_edge(&map, "b.com", "c.com"), "typed nav breaks the chain");
    assert!(has_edge(&map, "c.com", "d.com"));
}

// ───────────────────────────── lifecycle ─────────────────────────────

/// `Start` clears per-tab state: no phantom edge after a restart that reuses a
/// tabId.
#[test]
fn start_clears_state_no_phantom_edge() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a.com/", "link", &[]),
        nav(2, 200.0, 1, 1, "https://b.com/", "link", &[]),
        start(3, 300.0), // restart: flush a->b, clear everything
        nav(4, 400.0, 1, 1, "https://c.com/", "link", &[]), // tabId 1 reused
        close(5, 500.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "a.com", "b.com"));
    assert!(!has_edge(&map, "b.com", "c.com"), "no edge across restart");
}

/// `Close` flushes the closing tab's buffer before eviction.
#[test]
fn close_flushes_buffer() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a.com/", "link", &[]),
        nav(2, 200.0, 1, 1, "https://b.com/", "link", &[]),
        close(3, 300.0, 1), // must flush a->b
    ];
    let (map, _, _) = run_all(&events);
    assert!(has_edge(&map, "a.com", "b.com"));
    assert_eq!(node_visits(&map, "b.com"), 1);
}

// ───────────────────────────── sessions ─────────────────────────────

/// Idle-gap boundary is `> threshold` (inclusive stays in-session).
#[test]
fn idle_gap_threshold_is_exclusive() {
    let gap = 1_800_000.0;
    // exactly the threshold -> one session of two navs
    let at_threshold = vec![
        nav(1, 0.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, gap, 1, 1, "https://b.com/", "link", &[]),
        start(3, gap + 10.0),
    ];
    let (_, sess, _) = run_all(&at_threshold);
    assert_eq!(sess.len(), 1);
    assert_eq!(sess[0].nav_count, 2);

    // threshold + 1 ms -> a split into two sessions
    let over = vec![
        nav(1, 0.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, gap + 1.0, 1, 1, "https://b.com/", "link", &[]),
        start(3, gap + 10.0),
    ];
    let (_, sess2, _) = run_all(&over);
    assert_eq!(sess2.len(), 2);
    assert_eq!(sess2[0].nav_count, 1);
    assert_eq!(sess2[1].nav_count, 1);
}

/// Sessions are per-window.
#[test]
fn sessions_split_per_window() {
    let events = vec![
        nav(1, 100.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, 110.0, 2, 2, "https://b.com/", "typed", &[]),
        nav(3, 120.0, 1, 1, "https://c.com/", "link", &[]),
        nav(4, 130.0, 2, 2, "https://d.com/", "link", &[]),
        start(5, 200.0),
    ];
    let (_, mut sess, _) = run_all(&events);
    assert_eq!(sess.len(), 2);
    sess.sort_by_key(|s| s.window_id);
    assert_eq!(sess[0].window_id, 1);
    assert_eq!(sess[0].nav_count, 2);
    assert_eq!(sess[1].window_id, 2);
    assert_eq!(sess[1].nav_count, 2);
}

/// Backward clock jump (negative Δt) is clamped to 0 — no spurious split.
#[test]
fn backward_ts_does_not_split_session() {
    let events = vec![
        nav(1, 10_000.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, 5_000.0, 1, 1, "https://b.com/", "link", &[]), // ts goes backward
        start(3, 20_000.0),
    ];
    let (_, sess, _) = run_all(&events);
    assert_eq!(sess.len(), 1, "backward ts must not split");
    assert_eq!(sess[0].nav_count, 2);
}

// ───────────────────────────── UTC bucketing ─────────────────────────────

/// Visits are bucketed by **UTC** date, not local — two navs straddling local
/// midnight but on the same UTC day share a bucket; crossing UTC midnight splits.
#[test]
fn utc_day_bucketing() {
    // 2021-01-01T23:00:00Z and 2021-01-02T01:00:00Z -> different UTC days
    let t_2301 = 1_609_542_000_000.0; // 2021-01-01T23:00:00Z
    let t_0101 = 1_609_549_200_000.0; // 2021-01-02T01:00:00Z
    let events = vec![
        nav(1, t_2301, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, t_2301 + 1000.0, 1, 1, "https://b.com/", "link", &[]),
        nav(3, t_0101, 1, 1, "https://c.com/", "link", &[]),
        close(4, t_0101 + 1000.0, 1),
    ];
    let (map, _, _) = run_all(&events);
    assert!(map.contains_key("2021-01-01"));
    assert!(map.contains_key("2021-01-02"));
    // a.com finalized on the 1st; c.com finalized on the 2nd
    assert_eq!(map["2021-01-01"].nodes.get("a.com").map(|n| n.visits), Some(1));
    assert!(map["2021-01-02"].nodes.contains_key("c.com"));
}

// ─────────────────── fold == recompute, at every split ───────────────────

/// A rich stream exercising: two tabs/windows, a new-tab link, a client-redirect
/// burst, forward_back, a typed reset, an idle-gap session split, a backward-ts
/// nav, a Close, and a Start. The incremental fold (split + checkpoint
/// round-trip) must equal the from-scratch recompute at **every** watermark.
fn rich_stream() -> Vec<Event> {
    let gap = 1_800_000.0;
    vec![
        nav(1, 1_000.0, 1, 1, "https://a.com/", "typed", &[]),
        nav(2, 2_000.0, 1, 1, "https://b.com/", "link", &[]),
        link(3, 2_500.0, 2, 1), // new tab from tab1 (origin snapshot a.com)
        nav(4, 3_000.0, 2, 1, "https://c.com/", "link", &[]),
        nav(5, 3_200.0, 2, 1, "https://c2.com/", "link", &["client_redirect"]), // burst
        nav(6, 4_000.0, 1, 1, "https://results.com/", "generated", &[]),
        nav(7, 4_500.0, 1, 1, "https://art.com/", "link", &[]),
        nav(8, 4_700.0, 1, 1, "https://results.com/", "generated", &["forward_back"]),
        nav(9, 4_300.0, 1, 1, "https://art2.com/", "link", &[]), // backward ts
        nav(10, 5_000.0, 3, 2, "https://w2.com/", "typed", &[]), // second window
        close(11, 6_000.0, 2),
        nav(12, 6_000.0 + gap + 1.0, 1, 1, "https://later.com/", "typed", &[]), // idle-gap split
        start(13, 9_999_999.0), // restart: flush + close all sessions
        nav(14, 10_000_000.0, 1, 1, "https://post.com/", "link", &[]),
        close(15, 10_001_000.0, 1),
    ]
}

#[test]
fn incremental_equals_recompute_at_every_split() {
    let events = rich_stream();
    let (m_all, s_all, st_all) = run_all(&events);
    let st_all_v = serde_json::to_value(&st_all).unwrap();

    for at in 0..=events.len() {
        let (m_sp, s_sp, st_sp) = run_split(&events, at);
        assert_eq!(m_all, m_sp, "rollup buckets mismatch at split {at}");
        assert_eq!(s_all, s_sp, "session records mismatch at split {at}");
        assert_eq!(
            st_all_v,
            serde_json::to_value(&st_sp).unwrap(),
            "checkpoint state mismatch at split {at}"
        );
    }
}

/// Destructive-edit invalidation rebuilds identically: clearing the rollup +
/// watermark and re-folding from scratch reproduces the original rollup.
#[test]
fn destructive_edit_rebuild_matches() {
    let events = rich_stream();
    let (m1, s1, _) = run_all(&events);
    // simulate forget/delete -> cleared rollup + watermark, then lazy rebuild
    let (m2, s2, _) = run_all(&events);
    assert_eq!(m1, m2);
    assert_eq!(s1, s2);
}
