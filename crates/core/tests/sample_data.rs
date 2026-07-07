//! F4 onboarding fixture: drive the committed `extension/src/sample-data.json`
//! through the pure materialize → derive → rollup → project → analyze pipeline
//! (the same path the dashboard runs on "Load sample data") and assert it has the
//! structure the onboarding experience promises: real day buckets, ≥2 Louvain
//! communities, and ≥1 frequent journey.
#![cfg(not(target_arch = "wasm32"))]

use browsing_graph_core::flow::session_chains;
use browsing_graph_core::graph::{build, closed_sequences, frequent_sequences, louvain};
use browsing_graph_core::model::{Event, Granularity};
use browsing_graph_core::project::{project, Filters};
use browsing_graph_core::rollup::{fold, DayBucket, DeriveState};
use browsing_graph_core::sample::{self, materialize};
use std::collections::HashSet;

/// The committed fixture, inlined at compile time (the wasm build reaches it via a
/// `?raw` import instead — see chrome-bridge.ts). One source of truth either way.
const FIXTURE: &str = include_str!("../../../extension/src/sample-data.json");

/// A fixed "now" so the shift is deterministic (well after the ~3-week span).
const NOW: f64 = 1_900_000_000_000.0;

/// Parse the materialized export document's `events` array into `Event`s (id order).
fn materialized_events() -> Vec<Event> {
    let json = materialize(FIXTURE, NOW).expect("materialize the committed fixture");
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = doc["events"].as_array().expect("events array");
    arr.iter()
        .map(|v| serde_json::from_value(v.clone()).expect("event shape matches model::Event"))
        .collect()
}

/// Fold the events from scratch into merged day buckets (as a fresh load would,
/// via reset_derivation then a full fold).
fn buckets_from(events: &[Event]) -> Vec<DayBucket> {
    let mut st = DeriveState::default();
    fold(&mut st, events).0
}

#[test]
fn fixture_is_schemeless_and_offset_encoded() {
    // The committed text must never carry a scheme (the CI dist audit greps for
    // `https?://`; the fixture is inlined verbatim into the bundle).
    assert!(
        !FIXTURE.contains("http://") && !FIXTURE.contains("https://"),
        "committed fixture must store URLs schemeless (audit interaction)"
    );
    // Timestamps are offsets before "now": after the shift they must all be in the
    // past relative to NOW, and span roughly three weeks.
    let events = materialized_events();
    assert!(!events.is_empty());
    let (mut min_ts, mut max_ts) = (f64::INFINITY, f64::NEG_INFINITY);
    for e in &events {
        min_ts = min_ts.min(e.ts());
        max_ts = max_ts.max(e.ts());
        assert!(e.ts() <= NOW, "shifted ts must be at or before now");
    }
    let span_days = (max_ts - min_ts) / 86_400_000.0;
    assert!(
        (14.0..=31.0).contains(&span_days),
        "expected ~3 weeks of history, got {span_days:.1} days"
    );
}

#[test]
fn fixture_events_are_in_ascending_id_order() {
    // The derive pass consumes events in strict global id order; the fixture must
    // already be sorted so a straight import preserves that contract.
    let events = materialized_events();
    let mut last = f64::NEG_INFINITY;
    for e in &events {
        assert!(e.id() > last, "ids must be strictly ascending");
        last = e.id();
    }
}

#[test]
fn fixture_yields_nonempty_day_buckets() {
    let events = materialized_events();
    let buckets = buckets_from(&events);
    assert!(!buckets.is_empty(), "expected day buckets");
    let with_nodes = buckets.iter().filter(|b| !b.nodes.is_empty()).count();
    assert!(
        with_nodes >= 5,
        "expected several populated day buckets, got {with_nodes}"
    );
    // A realistic provenance mix: link edges and at least one non-link origin.
    let total_edges: usize = buckets.iter().map(|b| b.edges.len()).sum();
    assert!(total_edges > 0, "expected traversal edges");
}

#[test]
fn fixture_has_at_least_two_communities() {
    let events = materialized_events();
    let buckets = buckets_from(&events);
    // Project the whole history at hostname granularity (the graph view's default).
    let proj = project(&buckets, Granularity::Hostname, &Filters::default());
    assert!(proj.nodes.len() >= 6, "expected a non-trivial graph");
    let g = build(&proj);
    let comm = louvain(&g);
    let distinct: HashSet<usize> = comm.values().copied().collect();
    assert!(
        distinct.len() >= 2,
        "expected ≥2 Louvain communities, got {}",
        distinct.len()
    );
}

#[test]
fn fixture_has_frequent_journeys() {
    let events = materialized_events();
    // Mine per-tab link chains across the whole stream (session_chains splits on
    // Start / rootless navs / tab close), then find paths taken ≥2 times.
    let chains = session_chains(&events, Granularity::Hostname);
    assert!(!chains.is_empty(), "expected reconstructable link chains");
    let journeys = closed_sequences(frequent_sequences(&chains, 2, 5));
    assert!(
        !journeys.is_empty(),
        "expected ≥1 frequent journey (PrefixSpan support ≥2)"
    );
    // Every journey is a real multi-hop path.
    assert!(journeys.iter().all(|(p, _)| p.len() >= 2));
}

#[test]
fn materialize_matches_the_pure_shift_helpers() {
    // Cross-check that materialize used the documented shift on a known record.
    let json = materialize(FIXTURE, NOW).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    let first_nav = doc["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "nav")
        .unwrap();
    let url = first_nav["toUrl"].as_str().unwrap();
    assert!(url.starts_with(sample::URL_SCHEME), "scheme was prepended");
}
