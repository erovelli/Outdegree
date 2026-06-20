//! Property-based verification of the central invariant: an incremental fold
//! (split + checkpoint round-trip) equals a from-scratch recompute, for randomly
//! generated event streams, at **every** watermark split.
#![cfg(not(target_arch = "wasm32"))]

use browsing_graph_core::model::Event;
use browsing_graph_core::rollup::{fold, merge_bucket, DayBucket, DeriveState, SessionRec};
use proptest::prelude::*;
use std::collections::HashMap;

// ── harness (mirrors tests/derive_rollup.rs) ────────────────────────────────────

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

fn run_all(events: &[Event]) -> (Roll, Vec<SessionRec>, DeriveState) {
    let mut st = DeriveState::default();
    let (deltas, sess) = fold(&mut st, events);
    let mut map = Roll::new();
    apply(deltas, &mut map);
    (map, sess, st)
}

fn run_split(events: &[Event], at: usize) -> (Roll, Vec<SessionRec>, DeriveState) {
    let mut st = DeriveState::default();
    let mut map = Roll::new();
    let mut sess = Vec::new();

    let (d1, s1) = fold(&mut st, &events[..at]);
    apply(d1, &mut map);
    sess.extend(s1);

    // checkpoint round-trip, exactly as the store persists it
    let json = serde_json::to_string(&st).unwrap();
    let mut st2: DeriveState = serde_json::from_str(&json).unwrap();

    let (d2, s2) = fold(&mut st2, &events[at..]);
    apply(d2, &mut map);
    sess.extend(s2);
    (map, sess, st2)
}

// ── random event-stream generation ──────────────────────────────────────────────

const URLS: [&str; 6] = [
    "https://a.com/",
    "https://b.com/",
    "https://c.com/",
    "https://a.com/page2",
    "https://sub.b.com/",
    "chrome://newtab", // non-http(s): exercises the dropped-node paths
];
const TTS: [&str; 6] = [
    "link",
    "typed",
    "generated",
    "form_submit",
    "reload",
    "start_page",
];

#[derive(Clone, Debug)]
enum Spec {
    Nav {
        tab: i64,
        win: i64,
        url: usize,
        tt: usize,
        quals: u8,
    },
    Link {
        new_tab: i64,
        src_tab: i64,
    },
    Close {
        tab: i64,
    },
    Start,
}

fn spec() -> impl Strategy<Value = Spec> {
    prop_oneof![
        6 => (1i64..5, 1i64..3, 0usize..6, 0usize..6, prop_oneof![8 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)])
            .prop_map(|(tab, win, url, tt, quals)| Spec::Nav { tab, win, url, tt, quals }),
        2 => (1i64..5, 1i64..5).prop_map(|(new_tab, src_tab)| Spec::Link { new_tab, src_tab }),
        2 => (1i64..5).prop_map(|tab| Spec::Close { tab }),
        1 => Just(Spec::Start),
    ]
}

/// Build a valid stream: sequential ids, and timestamps that occasionally jump
/// backward (exercising clamp0) and occasionally exceed the idle gap (sessions).
fn build(specs: Vec<(Spec, i64)>) -> Vec<Event> {
    let mut ts: i64 = 1_600_000_000_000;
    let floor: i64 = 1_000_000_000_000;
    let mut out = Vec::with_capacity(specs.len());
    for (i, (s, gap)) in specs.into_iter().enumerate() {
        ts = (ts + gap).max(floor);
        let id = (i + 1) as f64;
        let tsf = ts as f64;
        let ev = match s {
            Spec::Nav {
                tab,
                win,
                url,
                tt,
                quals,
            } => {
                let mut q = Vec::new();
                if quals & 1 != 0 {
                    q.push("client_redirect".to_string());
                }
                if quals & 2 != 0 {
                    q.push("forward_back".to_string());
                }
                Event::Nav {
                    id,
                    ts: tsf,
                    tab_id: tab as f64,
                    window_id: win as f64,
                    to_url: URLS[url].to_string(),
                    transition_type: TTS[tt].to_string(),
                    qualifiers: q,
                }
            }
            Spec::Link { new_tab, src_tab } => Event::Link {
                id,
                ts: tsf,
                new_tab_id: new_tab as f64,
                source_tab_id: src_tab as f64,
            },
            Spec::Close { tab } => Event::Close {
                id,
                ts: tsf,
                tab_id: tab as f64,
            },
            Spec::Start => Event::Start { id, ts: tsf },
        };
        out.push(ev);
    }
    out
}

fn stream() -> impl Strategy<Value = Vec<Event>> {
    // gap can be negative (backward clock) or exceed the 30-min idle gap.
    proptest::collection::vec((spec(), -200_000i64..4_000_001i64), 0..40).prop_map(build)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    /// Incremental == recompute at every split, for any random stream.
    #[test]
    fn fold_equals_recompute_at_every_split(events in stream()) {
        let (m_all, s_all, st_all) = run_all(&events);
        let st_all_v = serde_json::to_value(&st_all).unwrap();
        for at in 0..=events.len() {
            let (m_sp, s_sp, st_sp) = run_split(&events, at);
            prop_assert_eq!(&m_all, &m_sp, "rollup mismatch at split {}", at);
            prop_assert_eq!(&s_all, &s_sp, "sessions mismatch at split {}", at);
            prop_assert_eq!(
                &st_all_v,
                &serde_json::to_value(&st_sp).unwrap(),
                "checkpoint mismatch at split {}",
                at
            );
        }
    }
}
