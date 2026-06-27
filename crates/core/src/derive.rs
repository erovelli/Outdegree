//! The read-time derivation pass (§7.3).
//!
//! Runs in **strict global id order** over the unified `events` stream, routing
//! each event to its tab and emitting node-visit / edge deltas + session records
//! into the rollup accumulator. All state lives in [`DeriveState`] so the pass is
//! resumable and `fold == recompute` (§7.4).

use crate::interpret::{classify, node_key};
use crate::model::{EdgeKind, Event, Granularity, Provenance};
use crate::rollup::{
    utc_date, Acc, Buffered, DeriveState, OpenSession, PendingOrigin, TabState, IDLE_GAP_MS,
    REDIRECT_WINDOW_MS,
};

/// Rollups are stored at hostname granularity; eTLD+1 is regrouped at merge (§4.3).
const GRAN: Granularity = Granularity::Hostname;

/// Whether `forward_back` navigations emit a node visit. Off by default
/// (decision #8); they always advance tab position regardless.
const FORWARD_BACK_NODE_VISIT: bool = false;

/// Dispatch one event in global order (§7.3).
pub fn step(state: &mut DeriveState, acc: &mut Acc, ev: &Event) {
    match ev {
        Event::Start { ts, .. } => on_start(state, acc, *ts),
        Event::Close { ts, tab_id, .. } => on_close(state, acc, *tab_id as i64, *ts),
        Event::Link {
            new_tab_id,
            source_tab_id,
            ..
        } => on_link(state, *new_tab_id as i64, *source_tab_id as i64),
        Event::Nav {
            id,
            ts,
            tab_id,
            window_id,
            to_url,
            transition_type,
            qualifiers,
        } => on_nav(
            state,
            acc,
            *id,
            *ts,
            *tab_id as i64,
            *window_id as i64,
            to_url,
            transition_type,
            qualifiers,
        ),
    }
}

/// `Start`: flush every tab's buffer, then clear all per-tab state and close all
/// sessions. Handles browser-restart tabId reuse — stale chains cannot survive a
/// restart (§7.3).
fn on_start(state: &mut DeriveState, acc: &mut Acc, ts: f64) {
    let mut tab_ids: Vec<i64> = state.tabs.keys().copied().collect();
    tab_ids.sort_unstable();
    for t in tab_ids {
        if let Some(b) = state.tabs.get_mut(&t).and_then(|ts| ts.buffer.take()) {
            // The restart ends each open page's dwell.
            finalize(acc, &b, ts);
        }
    }
    state.tabs.clear();
    state.pending_origin.clear();
    // Sorted so multi-session close on restart emits in a deterministic order
    // (required for the fold == recompute invariant).
    let mut wins: Vec<i64> = state.open_sessions.keys().copied().collect();
    wins.sort_unstable();
    for w in wins {
        if let Some(s) = state.open_sessions.remove(&w) {
            acc.emit_session(s.to_record());
        }
    }
}

/// `Close{tabId}`: flush that tab's buffer, then evict its state precisely
/// (bounds growth, prevents within-session tabId-reuse mis-chaining) (§7.3).
fn on_close(state: &mut DeriveState, acc: &mut Acc, tab_id: i64, close_ts: f64) {
    if let Some(mut ts) = state.tabs.remove(&tab_id) {
        if let Some(b) = ts.buffer.take() {
            // Closing the tab ends the open page's dwell.
            finalize(acc, &b, close_ts);
        }
    }
    state.pending_origin.remove(&tab_id);
}

/// `Link{newTabId, sourceTabId}`: snapshot the source's *current* page as the
/// child's pending origin (this is why global order matters), then reset the
/// child tab (§7.3).
///
/// The current page is the source tab's **buffered** Nav (the redirect-lookahead
/// holds each load one step before it commits to `last_url`); only when there is
/// no buffer (e.g. just after a forward/back) does `last_url` reflect what the
/// user is looking at. Using `last_url` unconditionally would attribute the new
/// tab to the page *before* the one the link was clicked from — or to nothing at
/// all when the source page was that tab's first navigation.
fn on_link(state: &mut DeriveState, new_tab_id: i64, source_tab_id: i64) {
    let (url, prov) = match state.tabs.get(&source_tab_id) {
        Some(ts) => match &ts.buffer {
            Some(b) => (Some(b.to_url.clone()), b.prov),
            None => (ts.last_url.clone(), ts.last_prov),
        },
        None => (None, Provenance::Other),
    };
    state
        .pending_origin
        .insert(new_tab_id, PendingOrigin { url, prov });
    state.tabs.insert(new_tab_id, TabState::default());
}

/// `Nav`: session update → classify → redirect-collapse lookahead → forward_back
/// → buffer with computed origin (§7.3).
#[allow(clippy::too_many_arguments)]
fn on_nav(
    state: &mut DeriveState,
    acc: &mut Acc,
    id: f64,
    ts: f64,
    tab_id: i64,
    window_id: i64,
    to_url: &str,
    transition_type: &str,
    qualifiers: &[String],
) {
    // Step 1 — session update (counts all navs, including reload/other activity).
    update_session(state, acc, id, ts, window_id, to_url);

    // Step 2 — classify; reload/other change no node/edge/per-tab state.
    let prov = classify(transition_type);
    if prov.is_ignored() {
        return;
    }

    let has_client_redirect = qualifiers.iter().any(|q| q == "client_redirect");
    let has_forward_back = qualifiers.iter().any(|q| q == "forward_back");

    // Step 3 — redirect-collapse via one-event lookahead.
    {
        let tab = state.tabs.entry(tab_id).or_default();
        if let Some(b) = &tab.buffer {
            if has_client_redirect && (ts - b.ts) < REDIRECT_WINDOW_MS {
                // This Nav continues the buffered redirect burst. Discard the
                // buffered emission and carry the original origin forward; do not
                // advance last_url.
                let origin_url = b.origin_url.clone();
                let origin_prov = b.origin_prov;
                tab.buffer = Some(Buffered {
                    origin_url,
                    origin_prov,
                    to_url: to_url.to_string(),
                    prov,
                    ts,
                });
                return;
            }
        }
    }

    // Not a redirect continuation: finalize the buffered (now-stable) Nav. This
    // Nav's ts is when the user left that page, so it sets the buffered page's dwell.
    if let Some(b) = state.tabs.entry(tab_id).or_default().buffer.take() {
        finalize(acc, &b, ts);
        let tab = state.tabs.get_mut(&tab_id).expect("tab present");
        tab.last_url = Some(b.to_url);
        tab.last_prov = b.prov;
    }

    // Step 4 — forward_back: no edge; advance position; do not buffer (§7.3,
    // decision #8). Node-visit off by default.
    if has_forward_back {
        if FORWARD_BACK_NODE_VISIT {
            if let Some(k) = node_key(to_url, GRAN) {
                acc.node(&utc_date(ts), &k, prov, 0);
            }
        }
        let tab = state.tabs.entry(tab_id).or_default();
        tab.last_url = Some(to_url.to_string());
        tab.last_prov = prov;
        return;
    }

    // Compute this Nav's origin: a consumed new-tab snapshot, else the tab's
    // current page. Then buffer it for one event of redirect lookahead.
    let (origin_url, origin_prov) = match state.pending_origin.remove(&tab_id) {
        Some(po) => (po.url, po.prov),
        None => {
            let tab = state.tabs.entry(tab_id).or_default();
            (tab.last_url.clone(), tab.last_prov)
        }
    };
    let tab = state.tabs.entry(tab_id).or_default();
    tab.buffer = Some(Buffered {
        origin_url,
        origin_prov,
        to_url: to_url.to_string(),
        prov,
        ts,
    });
}

/// Emit a buffered Nav's node visit + (if applicable) edge (§7.3 Finalize).
///
/// Date is the buffered nav's own ts (so the visit lands in the correct UTC
/// bucket regardless of when it is confirmed). `departure_ts` is when the user
/// left this page (the next nav / close / restart); the gap back to the page's
/// arrival (`b.ts`) is its dwell, clamped at 0 and capped at the idle gap so an
/// overnight-idle tab doesn't claim hours of attention.
fn finalize(acc: &mut Acc, b: &Buffered, departure_ts: f64) {
    let Some(to_key) = node_key(&b.to_url, GRAN) else {
        return; // non-http(s) landing: no node, no edge.
    };
    let date = utc_date(b.ts);
    let dwell_ms = (departure_ts - b.ts).clamp(0.0, IDLE_GAP_MS) as u64;
    acc.node(&date, &to_key, b.prov, dwell_ms);

    if b.prov.is_edge() {
        if let Some(from_key) = b.origin_url.as_deref().and_then(|u| node_key(u, GRAN)) {
            if from_key != to_key {
                // Self-loop drop at this granularity (decision #6); raw retained.
                let kind = if b.origin_prov == Provenance::SearchOrigin {
                    EdgeKind::SearchLink
                } else if b.prov == Provenance::Form {
                    EdgeKind::Form
                } else {
                    EdgeKind::Link
                };
                acc.edge(&date, &from_key, &to_key, kind);
            }
        }
    }
}

/// Flush every tab's pending buffer (the page currently open in that tab, not yet
/// confirmed by a following event) into provisional node/edge deltas.
///
/// These are **display-only** — never persisted, because a buffered nav can still
/// collapse as a redirect — so the graph/tables show the page you are on *now*,
/// not just the page you last navigated away from. Mirrors [`finalize`]; the
/// caller appends these to the read buckets before projecting (the projection
/// sums buckets by date, so duplicates merge harmlessly).
pub fn provisional_buckets(state: &DeriveState) -> Vec<crate::rollup::DayBucketDelta> {
    let mut acc = Acc::default();
    let mut tabs: Vec<i64> = state.tabs.keys().copied().collect();
    tabs.sort_unstable(); // deterministic
    for t in tabs {
        if let Some(b) = state.tabs.get(&t).and_then(|ts| ts.buffer.as_ref()) {
            // The page is still open (no departure yet), so its provisional dwell
            // is 0; it fills in once a real following event finalizes the buffer.
            finalize(&mut acc, b, b.ts);
        }
    }
    acc.days.into_values().collect()
}

/// Per-window session tracking (§7.3 step 1). `clamp0` maps a backward clock jump
/// to 0 so it never spuriously splits a session.
fn update_session(
    state: &mut DeriveState,
    acc: &mut Acc,
    id: f64,
    ts: f64,
    window_id: i64,
    to_url: &str,
) {
    let key = node_key(to_url, GRAN);
    match state.open_sessions.get_mut(&window_id) {
        None => {
            state
                .open_sessions
                .insert(window_id, OpenSession::open(id, window_id, ts, key));
        }
        Some(s) => {
            let dt = (ts - s.last_ts).max(0.0); // clamp0
            if dt > IDLE_GAP_MS {
                let closed = state.open_sessions.remove(&window_id).expect("present");
                acc.emit_session(closed.to_record());
                state
                    .open_sessions
                    .insert(window_id, OpenSession::open(id, window_id, ts, key));
            } else {
                s.last_ts = ts;
                s.nav_count += 1;
                s.end_id = id;
                if let Some(k) = key {
                    s.bump_host(k);
                }
            }
        }
    }
}
