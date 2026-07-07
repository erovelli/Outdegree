//! Incremental rollup state and the `fold` entry point (§7.4).
//!
//! `DeriveState` is the **complete checkpoint** (§7.4): persisting it lets an
//! incremental fold over `id > watermark` produce results bit-identical to a
//! from-scratch recompute over all events. `fold` drives the per-event read-time
//! pass in [`crate::derive`] and accumulates UTC-day bucket deltas + closed
//! session records.

use crate::model::{KindBreakdown, ProvBreakdown, Provenance};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Session idle-gap (decision #2): 30 minutes, per-window.
pub const IDLE_GAP_MS: f64 = 1_800_000.0;
/// Redirect-collapse lookahead window (§7.3); tune at M0.
pub const REDIRECT_WINDOW_MS: f64 = 1_000.0;

// ───────────────────────────── DeriveState (§7.4) ─────────────────────────────

/// The persisted checkpoint. Every field is justified against a derive read that
/// consumes it across the watermark boundary (§7.4 table).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeriveState {
    /// Incremental cursor lower bound (last processed event id).
    pub watermark: f64,
    /// Per-tab origin chain + redirect buffer, keyed by tabId.
    pub tabs: HashMap<i64, TabState>,
    /// New-tab origin snapshots awaiting the child's first Nav, keyed by newTabId.
    pub pending_origin: HashMap<i64, PendingOrigin>,
    /// Open sessions keyed by windowId.
    pub open_sessions: HashMap<i64, OpenSession>,
    /// Foreground attribution (§F7): the currently focused window, or `-1` when the
    /// browser itself is blurred (`WINDOW_ID_NONE`). `None` until the first
    /// window-focus event. Checkpointed so an interval that spans a watermark split
    /// is credited to the same window it would be in a from-scratch recompute.
    #[serde(default)]
    pub focused_window: Option<i64>,
    /// The active tab per window (§F7), keyed by windowId. Set by focus (tab
    /// activation) events; a window's entry is dropped when its active tab Closes.
    #[serde(default)]
    pub active_tab: HashMap<i64, i64>,
    /// The timestamp of the last processed event, of any kind (§F7). The lower
    /// bound of the interval the *next* event credits foreground time for.
    /// Checkpointed so crediting is bit-identical across a watermark split — the
    /// load-bearing piece that makes fold == recompute hold for fg attribution.
    #[serde(default)]
    pub last_event_ts: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TabState {
    pub last_url: Option<String>,
    pub last_prov: Provenance,
    pub buffer: Option<Buffered>,
}

impl Default for TabState {
    fn default() -> Self {
        TabState {
            last_url: None,
            last_prov: Provenance::Other,
            buffer: None,
        }
    }
}

/// A Nav held for one event of redirect lookahead (§7.3). `ts` is the page's
/// arrival time, used both to gate the redirect window and to derive dwell (the
/// gap to the event that finalizes this buffer).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Buffered {
    pub origin_url: Option<String>,
    pub origin_prov: Provenance,
    pub to_url: String,
    pub prov: Provenance,
    pub ts: f64,
}

/// Snapshot of a source tab's current page at child-spawn time (§7.3).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingOrigin {
    pub url: Option<String>,
    pub prov: Provenance,
}

/// An accumulating, not-yet-closed session (§7.4). `end_id` is tracked so the
/// emitted [`SessionRec`] can carry `endId` for the Sankey id-range cursor (§4.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenSession {
    pub id: f64,
    pub window_id: i64,
    pub start_ts: f64,
    pub start_id: f64,
    pub last_ts: f64,
    pub end_id: f64,
    pub nav_count: u32,
    pub host_counts: HashMap<String, u32>,
}

/// Upper bound on the number of distinct hosts tracked per open session. A
/// marathon session (left open for days, touching thousands of hosts) would
/// otherwise grow `host_counts` — and therefore the persisted `DeriveState`
/// checkpoint — without bound. 64 is far above the 5 surfaced in `to_record`, so
/// the displayed top-hosts are unaffected in practice; only the long tail of
/// once-seen hosts in an unusually broad session is dropped.
pub const SESSION_HOST_CAP: usize = 64;

impl OpenSession {
    pub(crate) fn open(id: f64, window_id: i64, ts: f64, first_key: Option<String>) -> Self {
        let mut s = OpenSession {
            id,
            window_id,
            start_ts: ts,
            start_id: id,
            last_ts: ts,
            end_id: id,
            nav_count: 1,
            host_counts: HashMap::new(),
        };
        if let Some(k) = first_key {
            s.bump_host(k);
        }
        s
    }

    /// Record one more visit to `key` within this session, keeping `host_counts`
    /// bounded to [`SESSION_HOST_CAP`] distinct hosts. An already-tracked host
    /// always increments (exact count retained); a brand-new host is added only
    /// while there's room, and dropped once the cap is reached. Applied per-event
    /// in global id order, so the result is identical whether the session is built
    /// by an incremental fold or a from-scratch recompute (the fold == recompute
    /// invariant, §11).
    pub(crate) fn bump_host(&mut self, key: String) {
        if let Some(c) = self.host_counts.get_mut(&key) {
            *c += 1;
        } else if self.host_counts.len() < SESSION_HOST_CAP {
            self.host_counts.insert(key, 1);
        }
    }

    /// Materialize the persisted session record on close.
    pub fn to_record(&self) -> SessionRec {
        let mut hosts: Vec<(String, u32)> = self
            .host_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        // Deterministic ordering: count desc, then key asc.
        hosts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hosts.truncate(5);
        SessionRec {
            id: self.id,
            window_id: self.window_id,
            start_ts: self.start_ts,
            end_ts: self.last_ts,
            start_id: self.start_id,
            end_id: self.end_id,
            nav_count: self.nav_count,
            top_hosts: hosts,
        }
    }
}

/// Provisional view of an open session for the picker (§4.4, §7.7).
impl OpenSession {
    pub fn provisional_record(&self) -> SessionRec {
        self.to_record()
    }
}

// ────────────────────────── Derived outputs (§4.3, §4.4) ──────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeStat {
    pub visits: u32,
    pub prov: ProvBreakdown,
    /// Total active-page time across the visits in this bucket, in milliseconds.
    /// `#[serde(default)]` so rollups written before dwell tracking still load.
    #[serde(default)]
    pub dwell_ms: u64,
    /// Total *foreground* time attributed to this host in this bucket, in
    /// milliseconds (§F7): the share of inter-event intervals during which this
    /// host was loaded in the focused window's active tab, capped at the idle gap.
    /// `#[serde(default)]` so pre-focus rollups still load (they read as `0`).
    #[serde(default)]
    pub fg_dwell_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeStat {
    pub weight: u32,
    pub kinds: KindBreakdown,
}

/// A UTC-day aggregate bucket — both the stored `rollup_days` value (§4.3) and the
/// shape of a fold delta. Edges are keyed `"{from}\u{0}{to}"` to match the JS
/// store contract.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DayBucket {
    pub date: String,
    #[serde(rename = "maxId")]
    pub max_id: i64,
    pub nodes: HashMap<String, NodeStat>,
    pub edges: HashMap<String, EdgeStat>,
    /// Whether any focus/window-focus event fell on this UTC day (§F7). Drives the
    /// dwell UI: a window with signal shows foreground time; one without shows the
    /// gap-based estimate (prefixed "≈"). `#[serde(default)]` so pre-focus buckets
    /// read as `false` ("no focus data").
    #[serde(default, rename = "hasFocusSignal")]
    pub has_focus_signal: bool,
    /// Visits per **UTC hour** of this UTC day (§F9): index `h` counts the visit
    /// events whose timestamp fell in `[h:00, h+1:00)` UTC. Incremented in lockstep
    /// with the per-host [`NodeStat::visits`] — same [`Acc::node`] site, same
    /// timestamp — so `visits_by_hour.iter().sum()` always equals this day's total
    /// node visits. The window-level 7×24 Rhythm heatmap (`ui/tables.rs`) projects
    /// these UTC histograms to local weekday/hour at render time. `#[serde(default)]`
    /// so pre-F9 buckets deserialize (they read as all-zero; the
    /// `DERIVED_SCHEMA_VERSION` bump rebuilds them from raw anyway, but mixed-vintage
    /// exports/imports must not error).
    #[serde(default, rename = "visitsByHour")]
    pub visits_by_hour: [u32; 24],
}

/// A fold delta is shaped exactly like a stored bucket.
pub type DayBucketDelta = DayBucket;

/// Join an edge endpoint pair into the storage key (§4.3).
pub fn edge_key(from: &str, to: &str) -> String {
    format!("{from}\u{0}{to}")
}

/// Split a stored edge key back into `(from, to)`.
pub fn split_edge_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('\u{0}')
}

/// Merge a delta bucket into an existing (stored) bucket in place. Used by the
/// store layer and by the §11 "incremental == recompute" tests.
pub fn merge_bucket(into: &mut DayBucket, delta: &DayBucket) {
    if into.date.is_empty() {
        into.date = delta.date.clone();
    }
    into.max_id = into.max_id.max(delta.max_id);
    into.has_focus_signal |= delta.has_focus_signal;
    for (h, c) in delta.visits_by_hour.iter().enumerate() {
        into.visits_by_hour[h] += c;
    }
    for (k, n) in &delta.nodes {
        let e = into.nodes.entry(k.clone()).or_default();
        e.visits += n.visits;
        e.dwell_ms += n.dwell_ms;
        e.fg_dwell_ms += n.fg_dwell_ms;
        e.prov.merge(&n.prov);
    }
    for (k, ed) in &delta.edges {
        let e = into.edges.entry(k.clone()).or_default();
        e.weight += ed.weight;
        e.kinds.merge(&ed.kinds);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRec {
    pub id: f64,
    pub window_id: i64,
    pub start_ts: f64,
    pub end_ts: f64,
    pub start_id: f64,
    pub end_id: f64,
    pub nav_count: u32,
    pub top_hosts: Vec<(String, u32)>,
}

// ───────────────────────────── Accumulator + fold ─────────────────────────────

/// Scratch accumulator the derive pass writes into (per fold).
#[derive(Default)]
pub struct Acc {
    pub days: HashMap<String, DayBucket>,
    pub sessions: Vec<SessionRec>,
    /// Event id currently being processed (sets bucket `maxId`).
    pub cur_id: f64,
}

impl Acc {
    fn bucket(&mut self, date: &str) -> &mut DayBucket {
        let cur = self.cur_id as i64;
        let b = self.days.entry(date.to_string()).or_default();
        if b.date.is_empty() {
            b.date = date.to_string();
        }
        b.max_id = b.max_id.max(cur);
        b
    }

    /// Record one node visit. `ts_ms` is the visit's own timestamp (the buffered
    /// nav's arrival time, §7.3), used both to derive nothing here directly and to
    /// bin the visit into its UTC hour (§F9): the per-hour counter is bumped at the
    /// *same site* as `visits`, so the two can never diverge — the Rhythm heatmap's
    /// total always matches the visit count. `date` must be `utc_date(ts_ms)`; the
    /// caller passes both so the hour and the day bucket stay in the same instant.
    pub fn node(&mut self, date: &str, key: &str, prov: Provenance, dwell_ms: u64, ts_ms: f64) {
        let hour = utc_hour(ts_ms) as usize;
        let b = self.bucket(date);
        b.visits_by_hour[hour] += 1;
        let n = b.nodes.entry(key.to_string()).or_default();
        n.visits += 1;
        n.dwell_ms += dwell_ms;
        n.prov.add(prov);
    }

    /// Credit `fg_ms` of foreground time to a host without recording a visit (§F7):
    /// foreground time accrues per inter-event interval, independently of the
    /// visit/dwell recorded when a page finalizes. Buckets by the interval-start
    /// date the caller passes.
    pub fn fg_credit(&mut self, date: &str, key: &str, fg_ms: u64) {
        let b = self.bucket(date);
        let n = b.nodes.entry(key.to_string()).or_default();
        n.fg_dwell_ms += fg_ms;
    }

    /// Mark a UTC day as carrying a focus signal (§F7): a focus/window-focus event
    /// fell on it, so the dwell UI trusts foreground time over the gap estimate.
    pub fn mark_focus_signal(&mut self, date: &str) {
        self.bucket(date).has_focus_signal = true;
    }

    pub fn edge(&mut self, date: &str, from: &str, to: &str, kind: crate::model::EdgeKind) {
        let k = edge_key(from, to);
        let b = self.bucket(date);
        let e = b.edges.entry(k).or_default();
        e.weight += 1;
        e.kinds.add(kind);
    }

    pub fn emit_session(&mut self, rec: SessionRec) {
        self.sessions.push(rec);
    }
}

/// Process `new_events` (already in ascending id order) into bucket deltas and
/// closed session records, mutating the checkpoint in place (§7.4).
///
/// Buffers and open sessions remain in `state` so they carry across folds; this
/// is what makes the incremental result equal a from-scratch recompute.
pub fn fold(
    state: &mut DeriveState,
    new_events: &[crate::model::Event],
) -> (Vec<DayBucketDelta>, Vec<SessionRec>) {
    let mut acc = Acc::default();
    for ev in new_events {
        // Forward compat (§F7): an unrecognized event kind from a newer log
        // deserializes to `Unknown`. Skip it entirely — no accumulation and no
        // watermark advance (a later known event carries the watermark past it) —
        // so an older dashboard degrades gracefully instead of erroring.
        if matches!(ev, crate::model::Event::Unknown) {
            continue;
        }
        acc.cur_id = ev.id();
        crate::derive::step(state, &mut acc, ev);
        state.watermark = ev.id();
    }
    let deltas = acc.days.into_values().collect();
    (deltas, acc.sessions)
}

/// Merge the unified `events` stream with the separate `spa` (history-state)
/// stream for the opt-in "in-app navigations" view (§4.2).
///
/// A **stable two-pointer merge** by `ts`: each input is already in its own id
/// order, and we interleave the two by timestamp while *never reordering two
/// records from the same stream*. So a backward clock jump inside `events`
/// (a later id with an earlier ts) keeps its id order — unlike a flat `(ts, id)`
/// sort, which would swap them. ts ties resolve to `events` first. The result is
/// folded from scratch (the watermark cursor only tracks the `events` sequence).
pub fn merge_streams(
    events: &[crate::model::Event],
    spa: &[crate::model::Event],
) -> Vec<crate::model::Event> {
    let mut out: Vec<crate::model::Event> = Vec::with_capacity(events.len() + spa.len());
    let (mut i, mut j) = (0, 0);
    while i < events.len() && j < spa.len() {
        // Take from `spa` only when it is strictly earlier; on a tie or when
        // `events` is earlier, take `events` (keeping the unified stream's order).
        if spa[j].ts() < events[i].ts() {
            out.push(spa[j].clone());
            j += 1;
        } else {
            out.push(events[i].clone());
            i += 1;
        }
    }
    out.extend(events[i..].iter().cloned());
    out.extend(spa[j..].iter().cloned());
    out
}

/// UTC `YYYY-MM-DD` for a millisecond epoch timestamp (decision #11).
///
/// Pure integer civil-date conversion (Howard Hinnant's algorithm) so no time
/// crate is needed and the pure core stays minimal. Display converts to local
/// time later (§7.5).
pub fn utc_date(ts_ms: f64) -> String {
    let days = (ts_ms / 86_400_000.0).floor() as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// UTC hour-of-day (`0..=23`) for a millisecond epoch timestamp (§F9) — the hour
/// the [`utc_date`] bucketing key places the same instant in. `rem_euclid` keeps
/// it correct for pre-epoch (negative) timestamps, so a clock-skewed event still
/// bins to a valid hour rather than panicking on an out-of-range index.
pub fn utc_hour(ts_ms: f64) -> u32 {
    let ms_in_day = ts_ms.rem_euclid(86_400_000.0);
    (ms_in_day / 3_600_000.0).floor() as u32
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Event;

    #[test]
    fn merge_streams_orders_by_ts() {
        let nav = |id: f64, ts: f64| Event::Nav {
            id,
            ts,
            tab_id: 1.0,
            window_id: 1.0,
            to_url: "https://a.com/".into(),
            transition_type: "link".into(),
            qualifiers: vec![],
        };
        let events = vec![nav(1.0, 100.0), nav(2.0, 300.0)];
        let spa = vec![nav(1.0, 200.0)]; // independent id sequence
        let merged = merge_streams(&events, &spa);
        let ts: Vec<f64> = merged.iter().map(|e| e.ts()).collect();
        assert_eq!(ts, vec![100.0, 200.0, 300.0]);
    }

    #[test]
    fn merge_streams_keeps_each_streams_internal_order() {
        // A backward clock jump *within* events (id 2 has an earlier ts than id 1)
        // must NOT reorder the two events — a flat (ts,id) sort would swap them.
        let nav = |id: f64, ts: f64| Event::Nav {
            id,
            ts,
            tab_id: 1.0,
            window_id: 1.0,
            to_url: "https://a.com/".into(),
            transition_type: "link".into(),
            qualifiers: vec![],
        };
        let events = vec![nav(1.0, 500.0), nav(2.0, 100.0)]; // id order, ts goes backward
        let spa = vec![nav(7.0, 300.0)];
        let merged = merge_streams(&events, &spa);
        let ts: Vec<f64> = merged.iter().map(|e| e.ts()).collect();
        // events keep their input (id) order: the ts-500 record precedes the ts-100
        // record even though that's "out of ts order" — a flat (ts,id) sort would
        // have swapped them. spa (ts 300) merely interleaves between streams.
        let p_first = ts.iter().position(|&t| t == 500.0).unwrap();
        let p_second = ts.iter().position(|&t| t == 100.0).unwrap();
        assert!(
            p_first < p_second,
            "events must keep id order across a backward ts jump, got {ts:?}"
        );
    }

    #[test]
    fn utc_date_known_epochs() {
        assert_eq!(utc_date(0.0), "1970-01-01");
        // 2021-01-01T00:00:00Z = 1609459200000 ms
        assert_eq!(utc_date(1_609_459_200_000.0), "2021-01-01");
        // 2021-01-01T23:59:59Z stays on the same UTC day
        assert_eq!(utc_date(1_609_459_200_000.0 + 86_399_000.0), "2021-01-01");
        // one second later rolls to the next UTC day
        assert_eq!(utc_date(1_609_459_200_000.0 + 86_400_000.0), "2021-01-02");
        // a known leap-year date: 2020-02-29
        assert_eq!(utc_date(1_582_934_400_000.0), "2020-02-29");
    }

    #[test]
    fn utc_hour_bins_including_midnight_boundary() {
        // 2021-01-01T00:00:00Z → hour 0; +59m59s stays hour 0; +1h rolls to hour 1.
        let midnight = 1_609_459_200_000.0;
        assert_eq!(utc_hour(midnight), 0);
        assert_eq!(utc_hour(midnight + 3_599_000.0), 0);
        assert_eq!(utc_hour(midnight + 3_600_000.0), 1);
        // Last hour of the day, then the next UTC midnight resets to hour 0.
        assert_eq!(utc_hour(midnight + 23.0 * 3_600_000.0), 23);
        assert_eq!(utc_hour(midnight + 24.0 * 3_600_000.0), 0);
        // The epoch itself and a pre-epoch (negative) instant both bin validly.
        assert_eq!(utc_hour(0.0), 0);
        assert_eq!(utc_hour(-3_600_000.0), 23, "one hour before the epoch");
    }

    #[test]
    fn merge_bucket_sums_hour_histograms() {
        let mut a = DayBucket {
            date: "2021-01-01".into(),
            ..Default::default()
        };
        a.visits_by_hour[9] = 2;
        a.visits_by_hour[14] = 1;
        let mut b = DayBucket {
            date: "2021-01-01".into(),
            ..Default::default()
        };
        b.visits_by_hour[9] = 3;
        b.visits_by_hour[22] = 4;
        merge_bucket(&mut a, &b);
        assert_eq!(a.visits_by_hour[9], 5);
        assert_eq!(a.visits_by_hour[14], 1);
        assert_eq!(a.visits_by_hour[22], 4);
        assert_eq!(a.visits_by_hour.iter().sum::<u32>(), 10);
    }

    #[test]
    fn merge_bucket_sums() {
        let mut a = DayBucket {
            date: "2021-01-01".into(),
            max_id: 5,
            ..Default::default()
        };
        a.nodes.insert(
            "x".into(),
            NodeStat {
                visits: 2,
                ..Default::default()
            },
        );
        let mut b = DayBucket {
            date: "2021-01-01".into(),
            max_id: 9,
            ..Default::default()
        };
        b.nodes.insert(
            "x".into(),
            NodeStat {
                visits: 3,
                ..Default::default()
            },
        );
        merge_bucket(&mut a, &b);
        assert_eq!(a.nodes["x"].visits, 5);
        assert_eq!(a.max_id, 9);
    }
}
