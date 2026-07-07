//! Range projection (§7.5): merge UTC-day buckets, optionally regroup to eTLD+1,
//! and apply display filters. No raw-event read.

use crate::interpret::registrable;
use crate::model::{EdgeAgg, Granularity, GraphProjection, NodeAgg, ProvBreakdown, Provenance};
use crate::rollup::{split_edge_key, DayBucket, EdgeStat, NodeStat, SessionRec};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Display filters applied to the merged projection (§7.5).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    pub min_visits: u32,
    pub hide_search_hubs: bool,
    /// Drop degree-0 nodes (the typed/bookmark/search singletons that link to
    /// nothing), so the connected structure fills the frame instead of being
    /// zoomed out to fit a halo of isolated dots.
    pub hide_isolated: bool,
    pub provenance_in: Option<Vec<Provenance>>,
}

/// Time window over the day-bucket history (design "Range" control). The window
/// is measured back from the most recent bucket present, so historical data is
/// still visible under the wider ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// Inverse of [`day_number`]: the `(year, month, day)` UTC civil date for a
/// day-number (days from the Unix epoch). Howard Hinnant's `civil_from_days`.
/// Month is 1–12, day 1–31. Pure, so the window labels are unit-testable.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Weekday index (Sun = 0 … Sat = 6) of a day-number. Day 0 (1970-01-01) is a
/// Thursday (index 4).
fn weekday(z: i64) -> usize {
    (z % 7 + 4).rem_euclid(7) as usize
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// UTC day-number (days from the Unix epoch) for an epoch-millisecond timestamp —
/// the same bucketing key [`day_number`] yields for a `YYYY-MM-DD` string. Used to
/// map a session's `start_ts` onto the day-bucket window when anchoring to it.
pub fn day_of_ms(ms: f64) -> i64 {
    (ms / 86_400_000.0).floor() as i64
}

/// The latest dated bucket's day-number, or `None` when no date parses.
pub fn latest_day(buckets: &[DayBucket]) -> Option<i64> {
    buckets.iter().filter_map(|b| day_number(&b.date)).max()
}

/// The earliest dated bucket's day-number, or `None` when no date parses. The
/// floor the backward step ([`can_step_back`]) clamps against.
pub fn earliest_day(buckets: &[DayBucket]) -> Option<i64> {
    buckets.iter().filter_map(|b| day_number(&b.date)).min()
}

/// Resolve a window's end day-number: the explicit anchor end when set, else the
/// latest dated bucket (the live "trailing window ending at the latest data").
fn resolve_end(buckets: &[DayBucket], anchor_end: Option<i64>) -> Option<i64> {
    anchor_end.or_else(|| latest_day(buckets))
}

/// Keep only the buckets whose date falls within `range`'s window ending at
/// `anchor_end` (a day-number, or the latest dated bucket when `None`). The
/// window is `[end − (range − 1), end]` inclusive. Buckets with unparseable dates
/// (or when the end can't be resolved) are passed through unchanged so a bad date
/// can never blank the view. When `anchor_end` is `None` this is exactly the
/// original trailing-window behavior (nothing is ever after the latest bucket).
pub fn select_window(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
) -> Vec<DayBucket> {
    let Some(end) = resolve_end(buckets, anchor_end) else {
        return buckets.to_vec();
    };
    let cutoff = end - (range.days() - 1);
    buckets
        .iter()
        .filter(|b| {
            day_number(&b.date)
                .map(|d| d >= cutoff && d <= end)
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

/// Inclusive `[start_day, end_day]` UTC day-numbers of the window for `range`,
/// ending at `anchor_end` (or the latest dated bucket when `None`). `None` when
/// there are no dated buckets and no explicit anchor. Drives the window label and
/// the ‹/› step bookkeeping.
pub fn window_day_bounds(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
) -> Option<(i64, i64)> {
    let end = resolve_end(buckets, anchor_end)?;
    Some((end - (range.days() - 1), end))
}

/// Human label for the window `[start_day, end_day]` (UTC day-numbers) under
/// `range`: `Day`/`Session` → the single end day (`"Wed, Jul 1"`); the multi-day
/// ranges → a start–end span (`"Jun 23 – 29"`, `"Jun 30 – Jul 6"`, and — when the
/// span crosses a calendar year — `"Jul 7, 2024 – Jul 6, 2025"`). Rendered in UTC
/// to match the day-bucket boundaries the projection uses. Pure/testable.
pub fn window_label(range: TimeRange, start_day: i64, end_day: i64) -> String {
    let (_ey, em, ed) = civil_from_days(end_day);
    if matches!(range, TimeRange::Day | TimeRange::Session) {
        return format!(
            "{}, {} {}",
            WDAYS[weekday(end_day)],
            MONTHS[(em - 1) as usize],
            ed
        );
    }
    let (sy, sm, sd) = civil_from_days(start_day);
    let (ey, ..) = civil_from_days(end_day);
    if sy != ey {
        format!(
            "{} {}, {} – {} {}, {}",
            MONTHS[(sm - 1) as usize],
            sd,
            sy,
            MONTHS[(em - 1) as usize],
            ed,
            ey
        )
    } else if sm == em {
        format!("{} {} – {}", MONTHS[(sm - 1) as usize], sd, ed)
    } else {
        format!(
            "{} {} – {} {}",
            MONTHS[(sm - 1) as usize],
            sd,
            MONTHS[(em - 1) as usize],
            ed
        )
    }
}

/// Whether ‹ (step one range duration earlier) is allowed: the window it would
/// land on must still start on or after the earliest recorded day, so navigation
/// clamps before the first data instead of scrolling into an unbounded void.
/// Always false when there are no dated buckets.
pub fn can_step_back(buckets: &[DayBucket], range: TimeRange, anchor_end: Option<i64>) -> bool {
    let (Some(end), Some(earliest)) = (resolve_end(buckets, anchor_end), earliest_day(buckets))
    else {
        return false;
    };
    let days = range.days();
    let next_start = (end - days) - (days - 1);
    next_start >= earliest
}

/// Whether › (step one range duration later) is allowed: only when anchored in
/// the past (`anchor_end` set and strictly before the latest data). At the live
/// view there is nothing later to move toward, so it is disabled.
pub fn can_step_forward(buckets: &[DayBucket], _range: TimeRange, anchor_end: Option<i64>) -> bool {
    match (anchor_end, latest_day(buckets)) {
        (Some(end), Some(latest)) => end < latest,
        _ => false,
    }
}

/// The calendar anchor end after ‹: exactly one range duration earlier. Gate the
/// button with [`can_step_back`]; this does not itself clamp. `None` only when
/// there is no data (no end to resolve).
pub fn back_end(buckets: &[DayBucket], range: TimeRange, anchor_end: Option<i64>) -> Option<i64> {
    resolve_end(buckets, anchor_end).map(|end| end - range.days())
}

/// The calendar anchor after ›: `Some(end)` one range duration later, or `None`
/// to re-enter the live/latest view when that step reaches or passes the latest
/// recorded day (so one › from a near-latest window snaps cleanly back to live).
pub fn forward_end(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
) -> Option<i64> {
    let end = resolve_end(buckets, anchor_end)?;
    let next = end + range.days();
    match latest_day(buckets) {
        Some(latest) if next >= latest => None,
        _ => Some(next),
    }
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
            e.dwell_ms += n.dwell_ms;
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
            e.dwell_ms += n.dwell_ms;
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
            dwell_ms: n.dwell_ms,
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

    // Optionally drop isolated (degree-0) nodes once edges are known. Edges only
    // connect surviving nodes, so they're unaffected.
    if filters.hide_isolated {
        let connected: HashSet<&str> = edge_vec
            .iter()
            .flat_map(|e| [e.from.as_str(), e.to.as_str()])
            .collect();
        node_vec.retain(|n| connected.contains(n.key.as_str()));
    }

    GraphProjection {
        nodes: node_vec,
        edges: edge_vec,
    }
}

/// A compact, "nice"-rounded ladder of min-visit thresholds for the filter
/// dropdown, adapted to the data: always starts at `1` ("all sites"), then climbs
/// a 1-2-5 × 10ⁿ ladder up to `max_visits`, capped to a handful of entries so the
/// menu stays scannable. With little browsing you get a couple of options; with a
/// heavy history you get coarser high-end cuts (e.g. ≥100, ≥200).
pub fn visit_thresholds(max_visits: u32) -> Vec<u32> {
    const LADDER: [u32; 16] = [
        2, 5, 10, 20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000,
        200_000,
    ];
    const MAX_OPTS: usize = 6;

    let mut out = vec![1u32];
    for &v in LADDER.iter() {
        if v > max_visits {
            break;
        }
        out.push(v);
    }
    if out.len() > MAX_OPTS {
        // Keep "all" plus the largest (most useful) high-end cuts.
        let tail: Vec<u32> = out.split_off(out.len() - (MAX_OPTS - 1));
        out.truncate(1);
        out.extend(tail);
    }
    out
}

/// Headline activity stats for the selected range, derived from the same bucket
/// history the projection uses (no raw-event read). `new_hosts` counts sites whose
/// *first-ever* appearance (across all history) falls inside the window — "sites
/// you discovered this period". `revisit_rate` is the share of in-window visits
/// that were repeat loads of a host already seen in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeStats {
    pub window_hosts: u32,
    pub new_hosts: u32,
    pub window_visits: u64,
    pub revisit_rate: f32,
}

pub fn range_stats(buckets: &[DayBucket], range: TimeRange, anchor_end: Option<i64>) -> RangeStats {
    let Some(end) = resolve_end(buckets, anchor_end) else {
        return RangeStats::default();
    };
    let cutoff = end - (range.days() - 1);

    // First-seen day per host across *all* history (so "new" means new ever, not
    // just new in this set of buckets).
    let mut first_seen: HashMap<&str, i64> = HashMap::new();
    for b in buckets {
        if let Some(d) = day_number(&b.date) {
            for k in b.nodes.keys() {
                let e = first_seen.entry(k.as_str()).or_insert(d);
                *e = (*e).min(d);
            }
        }
    }

    // Aggregate visits within the window.
    let mut visits_in: HashMap<&str, u64> = HashMap::new();
    for b in buckets {
        let in_win = day_number(&b.date)
            .map(|d| d >= cutoff && d <= end)
            .unwrap_or(true);
        if !in_win {
            continue;
        }
        for (k, n) in &b.nodes {
            *visits_in.entry(k.as_str()).or_insert(0) += n.visits as u64;
        }
    }

    let window_hosts = visits_in.len() as u32;
    let window_visits: u64 = visits_in.values().sum();
    let new_hosts = visits_in
        .keys()
        .filter(|k| first_seen.get(**k).map(|d| *d >= cutoff).unwrap_or(false))
        .count() as u32;
    let revisit_rate = if window_visits > 0 {
        (window_visits - window_hosts as u64) as f32 / window_visits as f32
    } else {
        0.0
    };
    RangeStats {
        window_hosts,
        new_hosts,
        window_visits,
        revisit_rate,
    }
}

/// Per-day visit totals across the selected window, ascending by date — the
/// series behind a trend sparkline. One point per dated bucket that falls in the
/// window (rollup_days holds at most one bucket per UTC day). Session/Day collapse
/// to a single point, so the caller gates the sparkline to Week/Month/Year.
pub fn daily_series(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
) -> Vec<(String, u32)> {
    let Some(end) = resolve_end(buckets, anchor_end) else {
        return Vec::new();
    };
    let cutoff = end - (range.days() - 1);
    let mut pts: Vec<(i64, String, u32)> = buckets
        .iter()
        .filter_map(|b| {
            let d = day_number(&b.date)?;
            if d < cutoff || d > end {
                return None;
            }
            let visits: u32 = b.nodes.values().map(|n| n.visits).sum();
            Some((d, b.date.clone(), visits))
        })
        .collect();
    pts.sort_by_key(|(d, ..)| *d);
    pts.into_iter().map(|(_, date, v)| (date, v)).collect()
}

/// This-window-vs-previous-window comparison (same length, immediately prior), for
/// "↑12% vs last week" deltas. `*_pct` return `None` when the prior window is empty
/// (no baseline to compare against — the caller shows nothing rather than "↑∞%").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PeriodDelta {
    pub visits_now: u64,
    pub visits_prev: u64,
    pub hosts_now: u32,
    pub hosts_prev: u32,
    pub new_hosts_now: u32,
    pub new_hosts_prev: u32,
}

impl PeriodDelta {
    pub fn visits_pct(&self) -> Option<i32> {
        pct_change(self.visits_prev, self.visits_now)
    }
    pub fn hosts_pct(&self) -> Option<i32> {
        pct_change(self.hosts_prev as u64, self.hosts_now as u64)
    }
    pub fn new_hosts_pct(&self) -> Option<i32> {
        pct_change(self.new_hosts_prev as u64, self.new_hosts_now as u64)
    }
    /// True when there's no prior window to compare against (first period of data).
    pub fn no_baseline(&self) -> bool {
        self.visits_prev == 0 && self.hosts_prev == 0
    }
}

fn pct_change(prev: u64, now: u64) -> Option<i32> {
    if prev == 0 {
        return None;
    }
    Some((((now as f64 - prev as f64) / prev as f64) * 100.0).round() as i32)
}

pub fn period_delta(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
) -> Option<PeriodDelta> {
    let end = resolve_end(buckets, anchor_end)?;
    let days = range.days();
    let lo = end - (days - 1);
    let (plo, phi) = (lo - days, lo - 1); // the equal-length window just before

    // First-seen day per host across all history (for "new in window" counts).
    let mut first_seen: HashMap<&str, i64> = HashMap::new();
    for b in buckets {
        if let Some(d) = day_number(&b.date) {
            for k in b.nodes.keys() {
                let e = first_seen.entry(k.as_str()).or_insert(d);
                *e = (*e).min(d);
            }
        }
    }
    // Aggregate one window: distinct hosts, total visits, and hosts first-seen in it.
    let agg = |wlo: i64, whi: i64| -> (u32, u64, u32) {
        let mut hosts: HashSet<&str> = HashSet::new();
        let mut visits = 0u64;
        let mut new_hosts = 0u32;
        for b in buckets {
            let Some(d) = day_number(&b.date) else {
                continue;
            };
            if d < wlo || d > whi {
                continue;
            }
            for (k, n) in &b.nodes {
                if hosts.insert(k.as_str())
                    && first_seen
                        .get(k.as_str())
                        .map(|f| *f >= wlo)
                        .unwrap_or(false)
                {
                    new_hosts += 1;
                }
                visits += n.visits as u64;
            }
        }
        (hosts.len() as u32, visits, new_hosts)
    };
    let (hosts_now, visits_now, new_hosts_now) = agg(lo, end);
    let (hosts_prev, visits_prev, new_hosts_prev) = agg(plo, phi);
    Some(PeriodDelta {
        visits_now,
        visits_prev,
        hosts_now,
        hosts_prev,
        new_hosts_now,
        new_hosts_prev,
    })
}

/// A host whose visits jumped this window versus the equal-length window before
/// it — the sites behind the bare `new_hosts`/discovery scalar, named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surge {
    pub host: String,
    pub now: u32,
    pub prev: u32,
}

impl Surge {
    /// Growth factor (prev floored at 1 so a brand-new host reads as `now×`, not ∞).
    pub fn ratio(&self) -> f32 {
        self.now as f32 / self.prev.max(1) as f32
    }
    pub fn is_new(&self) -> bool {
        self.prev == 0
    }
}

/// Hosts surging this period: visited at least `min_now` times in the current
/// window AND strictly more than in the prior equal-length window, ranked by
/// growth factor. The `min_now` floor is load-bearing — without it a host visited
/// once now and never before (1 vs 0) reads as an infinite surge and drowns the
/// list. Pure, from the same day buckets as `range_stats` (no raw-event read).
pub fn surging_hosts(
    buckets: &[DayBucket],
    range: TimeRange,
    anchor_end: Option<i64>,
    min_now: u32,
    top: usize,
) -> Vec<Surge> {
    let Some(end) = resolve_end(buckets, anchor_end) else {
        return Vec::new();
    };
    let days = range.days();
    let lo = end - (days - 1);
    let (plo, phi) = (lo - days, lo - 1);

    let mut now_v: HashMap<&str, u32> = HashMap::new();
    let mut prev_v: HashMap<&str, u32> = HashMap::new();
    for b in buckets {
        let Some(d) = day_number(&b.date) else {
            continue;
        };
        if d >= lo && d <= end {
            for (k, n) in &b.nodes {
                *now_v.entry(k.as_str()).or_insert(0) += n.visits;
            }
        } else if d >= plo && d <= phi {
            for (k, n) in &b.nodes {
                *prev_v.entry(k.as_str()).or_insert(0) += n.visits;
            }
        }
    }

    let mut out: Vec<Surge> = now_v
        .iter()
        .filter_map(|(&k, &now)| {
            let prev = prev_v.get(k).copied().unwrap_or(0);
            // A surge: enough volume now, and strictly up from before.
            if now < min_now || now <= prev {
                return None;
            }
            Some(Surge {
                host: k.to_string(),
                now,
                prev,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.ratio()
            .partial_cmp(&a.ratio())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.now.cmp(&a.now))
            .then_with(|| a.host.cmp(&b.host))
    });
    out.truncate(top);
    out
}

/// Session record ids in chronological order (oldest → newest) — the order the
/// Session range's ‹/› buttons walk. Ordered by `(start_ts, id)` so ties are
/// stable; this is the newest-first session-picker list reversed. The newest id
/// is the live "Latest" session.
pub fn session_order(sessions: &[SessionRec]) -> Vec<f64> {
    let mut ordered: Vec<&SessionRec> = sessions.iter().collect();
    ordered.sort_by(|a, b| {
        a.start_ts
            .partial_cmp(&b.start_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.id.partial_cmp(&b.id).unwrap_or(std::cmp::Ordering::Equal))
    });
    ordered.into_iter().map(|r| r.id).collect()
}

/// The outcome of a ‹/› step over the Session range's chronological list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SessionStep {
    /// Anchor to this specific session id.
    To(f64),
    /// Re-enter the live "latest session" view (clear the anchor).
    Live,
    /// The step is not possible (button disabled).
    Blocked,
}

/// Index into `order` of the currently displayed session: the anchored id, or the
/// newest (last) when `current` is `None` (the live view). A stale/deleted anchor
/// id resolves to the newest so navigation never dead-ends.
fn session_index(order: &[f64], current: Option<f64>) -> Option<usize> {
    if order.is_empty() {
        return None;
    }
    Some(match current {
        None => order.len() - 1,
        Some(id) => order
            .iter()
            .position(|&x| x == id)
            .unwrap_or(order.len() - 1),
    })
}

/// ‹ over the Session range: the previous (older) session, or `Blocked` at the
/// oldest.
pub fn session_back(order: &[f64], current: Option<f64>) -> SessionStep {
    match session_index(order, current) {
        Some(i) if i > 0 => SessionStep::To(order[i - 1]),
        _ => SessionStep::Blocked,
    }
}

/// › over the Session range: the next (newer) session, `Live` when that newer
/// session is the newest (so › snaps back to the live view), or `Blocked` when
/// already at the newest/live session.
pub fn session_forward(order: &[f64], current: Option<f64>) -> SessionStep {
    if current.is_none() {
        return SessionStep::Blocked; // already live (newest)
    }
    match session_index(order, current) {
        Some(i) => {
            let last = order.len() - 1;
            if i >= last {
                SessionStep::Blocked
            } else if i + 1 == last {
                SessionStep::Live
            } else {
                SessionStep::To(order[i + 1])
            }
        }
        None => SessionStep::Blocked,
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

/// A stable fingerprint of a projection's *layout-relevant shape*: its node set
/// and edge topology, independent of order or per-node visit counts. The same
/// graph shape always hashes the same, so the UI can recognise an idempotent
/// re-projection (e.g. re-picking a range that resolves to the same data) and keep
/// the existing layout instead of re-running the force simulation. Visit counts
/// drive node size/colour, not position, so they're deliberately excluded.
pub fn layout_signature(p: &GraphProjection) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut nodes: Vec<&str> = p.nodes.iter().map(|n| n.key.as_str()).collect();
    nodes.sort_unstable();
    let mut edges: Vec<(&str, &str)> = p
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    edges.sort_unstable();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    nodes.hash(&mut h);
    edges.hash(&mut h);
    h.finish()
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
                ..Default::default()
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
                ..Default::default()
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
    fn hide_isolated_drops_degree_zero_nodes() {
        let mut b = bucket("2021-01-01");
        for h in ["hub.com", "leaf.com", "lonely.com"] {
            b.nodes.insert(
                h.into(),
                NodeStat {
                    visits: 3,
                    ..Default::default()
                },
            );
        }
        b.edges.insert(
            edge_key("hub.com", "leaf.com"),
            EdgeStat {
                weight: 2,
                ..Default::default()
            },
        );
        let filters = Filters {
            hide_isolated: true,
            ..Default::default()
        };
        let g = project(&[b], Granularity::Hostname, &filters);
        let keys: std::collections::HashSet<&str> =
            g.nodes.iter().map(|n| n.key.as_str()).collect();
        assert_eq!(keys, ["hub.com", "leaf.com"].into_iter().collect());
        assert!(!keys.contains("lonely.com"), "degree-0 node dropped");
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
                ..Default::default()
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
                ..Default::default()
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
    fn layout_signature_is_shape_only_and_order_independent() {
        use crate::model::{EdgeAgg, NodeAgg};
        let n = |k: &str, v: u32| NodeAgg {
            key: k.into(),
            visits: v,
            prov: ProvBreakdown::default(),
            ..Default::default()
        };
        let e = |f: &str, t: &str| EdgeAgg {
            from: f.into(),
            to: t.into(),
            weight: 1,
            kinds: KindBreakdown::default(),
        };
        let a = GraphProjection {
            nodes: vec![n("a", 5), n("b", 3), n("c", 1)],
            edges: vec![e("a", "b"), e("b", "c")],
        };
        // Same shape, shuffled order + different visit counts → same signature.
        let b = GraphProjection {
            nodes: vec![n("c", 99), n("a", 1), n("b", 2)],
            edges: vec![e("b", "c"), e("a", "b")],
        };
        assert_eq!(layout_signature(&a), layout_signature(&b));
        // A different edge, or a different node set, must change the signature.
        let diff_edge = GraphProjection {
            nodes: vec![n("a", 5), n("b", 3), n("c", 1)],
            edges: vec![e("a", "b"), e("a", "c")],
        };
        let diff_nodes = GraphProjection {
            nodes: vec![n("a", 5), n("b", 3)],
            edges: vec![e("a", "b")],
        };
        assert_ne!(layout_signature(&a), layout_signature(&diff_edge));
        assert_ne!(layout_signature(&a), layout_signature(&diff_nodes));
    }

    #[test]
    fn visit_thresholds_adapt_to_volume() {
        // Sparse data → just "all", or "all" + a step or two.
        assert_eq!(visit_thresholds(0), vec![1]);
        assert_eq!(visit_thresholds(1), vec![1]);
        assert_eq!(visit_thresholds(3), vec![1, 2]);
        assert_eq!(visit_thresholds(8), vec![1, 2, 5]);
        // A medium history fills the low ladder exactly (no cap yet).
        assert_eq!(visit_thresholds(50), vec![1, 2, 5, 10, 20, 50]);
        // Heavy data is capped to six: "all" + the five largest applicable cuts.
        assert_eq!(visit_thresholds(200), vec![1, 10, 20, 50, 100, 200]);
        let huge = visit_thresholds(1_000_000);
        assert_eq!(huge.len(), 6);
        assert_eq!(huge[0], 1);
        assert!(huge.windows(2).all(|w| w[0] < w[1]), "sorted & unique");
    }

    #[test]
    fn range_stats_counts_new_hosts_and_revisits() {
        let mut older = bucket("2021-01-01");
        older.nodes.insert(
            "old.com".into(),
            NodeStat {
                visits: 1,
                ..Default::default()
            },
        );
        // Window day: old.com (seen before → returning) gets 3 visits, new.com
        // (first ever in-window) gets 1 visit. 4 visits over 2 distinct hosts.
        let mut today = bucket("2021-01-11");
        today.nodes.insert(
            "old.com".into(),
            NodeStat {
                visits: 3,
                ..Default::default()
            },
        );
        today.nodes.insert(
            "new.com".into(),
            NodeStat {
                visits: 1,
                ..Default::default()
            },
        );
        let s = range_stats(&[older, today], TimeRange::Day, None);
        assert_eq!(s.window_hosts, 2);
        assert_eq!(s.window_visits, 4);
        assert_eq!(s.new_hosts, 1, "only new.com is first-seen in-window");
        // 4 visits, 2 distinct → 2 of the loads were revisits → 0.5.
        assert!((s.revisit_rate - 0.5).abs() < 1e-6);
    }

    fn bucket_with(date: &str, host: &str, visits: u32) -> DayBucket {
        let mut b = bucket(date);
        b.nodes.insert(
            host.into(),
            NodeStat {
                visits,
                ..Default::default()
            },
        );
        b
    }

    #[test]
    fn daily_series_is_per_day_visits_ascending_in_window() {
        let bs = vec![
            bucket_with("2021-01-01", "a.com", 9), // before the Week window from 01-11
            bucket_with("2021-01-08", "a.com", 2),
            bucket_with("2021-01-10", "b.com", 5),
            bucket_with("2021-01-11", "a.com", 3), // latest
        ];
        let series = daily_series(&bs, TimeRange::Week, None); // cutoff = 01-05
        assert_eq!(
            series,
            vec![
                ("2021-01-08".to_string(), 2),
                ("2021-01-10".to_string(), 5),
                ("2021-01-11".to_string(), 3),
            ]
        );
        assert!(
            !series.iter().any(|(d, _)| d == "2021-01-01"),
            "pre-window day excluded"
        );
    }

    #[test]
    fn period_delta_compares_window_against_the_prior_window() {
        // Week window from 01-15 = [01-09 .. 01-15]; prior = [01-02 .. 01-08].
        let bs = vec![
            bucket_with("2021-01-03", "old.com", 4), // prior week: 4 visits, host old.com
            bucket_with("2021-01-10", "old.com", 6), // this week: revisit old.com
            bucket_with("2021-01-12", "new.com", 2), // this week: new host
            bucket_with("2021-01-15", "old.com", 2), // latest
        ];
        let d = period_delta(&bs, TimeRange::Week, None).unwrap();
        assert_eq!(d.visits_now, 10); // 6 + 2 + 2
        assert_eq!(d.visits_prev, 4);
        assert_eq!(d.hosts_now, 2); // old.com, new.com
        assert_eq!(d.hosts_prev, 1); // old.com
        assert_eq!(d.new_hosts_now, 1); // new.com first-seen in this window
        assert_eq!(d.visits_pct(), Some(150)); // (10-4)/4 = +150%
        assert!(!d.no_baseline());

        // With no prior data, percentages are None (no "↑∞%").
        let only_now = vec![bucket_with("2021-01-15", "x.com", 3)];
        let d2 = period_delta(&only_now, TimeRange::Week, None).unwrap();
        assert!(d2.no_baseline());
        assert_eq!(d2.visits_pct(), None);
    }

    #[test]
    fn surging_hosts_rank_jumps_and_respect_the_floor() {
        // Week window from 01-15 = [01-09..01-15]; prior = [01-02..01-08].
        let bs = vec![
            bucket_with("2021-01-03", "boom.com", 2),  // prior 2
            bucket_with("2021-01-10", "boom.com", 20), // now 20 → ratio 10
            bucket_with("2021-01-04", "flat.com", 5),  // prior 5
            bucket_with("2021-01-11", "flat.com", 5),  // now 5 == prev → not a surge
            bucket_with("2021-01-12", "fresh.com", 6), // new, now 6 → ratio 6
            bucket_with("2021-01-13", "tiny.com", 1),  // now 1 < floor → excluded
            bucket_with("2021-01-15", "boom.com", 0),  // latest anchor (0 visits ok)
        ];
        let s = surging_hosts(&bs, TimeRange::Week, None, 3, 10);
        let hosts: Vec<&str> = s.iter().map(|x| x.host.as_str()).collect();
        assert_eq!(
            hosts,
            vec!["boom.com", "fresh.com"],
            "ranked by growth, floored"
        );
        assert!(s.iter().find(|x| x.host == "fresh.com").unwrap().is_new());
        assert!(
            !s.iter().any(|x| x.host == "flat.com"),
            "flat is not a surge"
        );
        assert!(
            !s.iter().any(|x| x.host == "tiny.com"),
            "below min_now floor"
        );
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
        let w = select_window(&bs, TimeRange::Week, None);
        let dates: std::collections::HashSet<&str> = w.iter().map(|b| b.date.as_str()).collect();
        assert_eq!(dates, ["2021-01-05", "2021-01-10", "2021-01-11"].into());
        // Day = just the latest day.
        let d = select_window(&bs, TimeRange::Day, None);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].date, "2021-01-11");
        // Year keeps everything here.
        assert_eq!(select_window(&bs, TimeRange::Year, None).len(), 4);
    }

    #[test]
    fn select_window_passes_through_when_no_dates_parse() {
        let bs = vec![bucket("garbage"), bucket("also-bad")];
        assert_eq!(select_window(&bs, TimeRange::Day, None).len(), 2);
    }

    #[test]
    fn ego_returns_focus_plus_neighbors() {
        use crate::model::{EdgeAgg, NodeAgg};
        let n = |k: &str| NodeAgg {
            key: k.into(),
            visits: 1,
            prov: ProvBreakdown::default(),
            ..Default::default()
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
            ..Default::default()
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

    // ── time navigation (§F6) ────────────────────────────────────────────────

    fn dn(date: &str) -> i64 {
        day_number(date).unwrap()
    }

    #[test]
    fn civil_from_days_inverts_day_number() {
        for date in [
            "1970-01-01",
            "2021-01-31",
            "2021-02-01", // month-length boundary (Feb)
            "2020-02-29", // leap day
            "2021-03-01",
            "2024-12-31",
            "2025-01-01", // year boundary
        ] {
            let z = dn(date);
            let (y, m, d) = civil_from_days(z);
            let got = format!("{y:04}-{m:02}-{d:02}");
            assert_eq!(got, date, "round-trip {date}");
        }
        // 1970-01-01 is a Thursday (index 4).
        assert_eq!(weekday(0), 4);
        assert_eq!(WDAYS[weekday(0)], "Thu");
    }

    #[test]
    fn window_day_bounds_span_each_granularity() {
        // One anchor end; each range is `[end-(days-1), end]`.
        let bs = vec![bucket_with("2021-03-05", "a.com", 1)];
        let end = dn("2021-03-05");
        assert_eq!(
            window_day_bounds(&bs, TimeRange::Day, None),
            Some((end, end))
        );
        assert_eq!(
            window_day_bounds(&bs, TimeRange::Week, None),
            Some((end - 6, end))
        );
        // Month is a fixed 30-day trailing window, so its start crosses the
        // short February correctly (Mar 5 − 29 days = Feb 4).
        assert_eq!(
            window_day_bounds(&bs, TimeRange::Month, None),
            Some((dn("2021-02-04"), end))
        );
        assert_eq!(
            window_day_bounds(&bs, TimeRange::Year, None),
            Some((end - 364, end))
        );
        // An explicit anchor overrides the latest bucket.
        let a = dn("2020-06-15");
        assert_eq!(
            window_day_bounds(&bs, TimeRange::Week, Some(a)),
            Some((a - 6, a))
        );
        // No dated buckets, no anchor → no bounds.
        assert_eq!(window_day_bounds(&[], TimeRange::Week, None), None);
    }

    #[test]
    fn window_label_reads_bounds_in_utc() {
        // Day → single end day with weekday. 2021-01-01 was a Friday.
        assert_eq!(
            window_label(TimeRange::Day, dn("2021-01-01"), dn("2021-01-01")),
            "Fri, Jan 1"
        );
        // Week within one month collapses the trailing month name.
        assert_eq!(
            window_label(TimeRange::Week, dn("2021-06-23"), dn("2021-06-29")),
            "Jun 23 – 29"
        );
        // Week crossing a month boundary shows both months.
        assert_eq!(
            window_label(TimeRange::Week, dn("2021-06-30"), dn("2021-07-06")),
            "Jun 30 – Jul 6"
        );
        // A span crossing a calendar year gains the years for disambiguation.
        assert_eq!(
            window_label(TimeRange::Year, dn("2024-07-07"), dn("2025-07-06")),
            "Jul 7, 2024 – Jul 6, 2025"
        );
    }

    #[test]
    fn select_window_anchor_excludes_future_and_earlier_buckets() {
        let bs = vec![
            bucket_with("2021-01-03", "old.com", 1), // before the anchored week
            bucket_with("2021-01-10", "a.com", 1),
            bucket_with("2021-01-12", "b.com", 1),
            bucket_with("2021-01-15", "c.com", 1), // anchor end
            bucket_with("2021-02-01", "future.com", 9), // after the anchor
        ];
        let w = select_window(&bs, TimeRange::Week, Some(dn("2021-01-15")));
        let dates: std::collections::HashSet<&str> = w.iter().map(|b| b.date.as_str()).collect();
        assert_eq!(dates, ["2021-01-10", "2021-01-12", "2021-01-15"].into());
    }

    #[test]
    fn step_back_and_forward_move_one_range_duration() {
        let bs = vec![
            bucket_with("2021-01-01", "e.com", 1), // earliest
            bucket_with("2021-01-31", "l.com", 1), // latest
        ];
        let latest = dn("2021-01-31");
        // ‹ from live steps back exactly one duration.
        assert_eq!(back_end(&bs, TimeRange::Week, None), Some(latest - 7));
        assert_eq!(
            back_end(&bs, TimeRange::Week, Some(latest - 7)),
            Some(latest - 14)
        );
        // › lands one duration later, and snaps to live (None) once it reaches
        // the latest data.
        assert_eq!(
            forward_end(&bs, TimeRange::Week, Some(latest - 14)),
            Some(latest - 7)
        );
        assert_eq!(forward_end(&bs, TimeRange::Week, Some(latest - 7)), None);
        // Overshooting the latest also snaps to live.
        assert_eq!(forward_end(&bs, TimeRange::Week, Some(latest - 3)), None);
    }

    #[test]
    fn can_step_back_clamps_before_earliest_data() {
        let bs = vec![
            bucket_with("2021-01-01", "e.com", 1),
            bucket_with("2021-01-31", "l.com", 1),
        ];
        let earliest = dn("2021-01-01");
        // Day range: steppable back until the window sits on the earliest day.
        assert!(can_step_back(&bs, TimeRange::Day, None));
        assert!(can_step_back(&bs, TimeRange::Day, Some(earliest + 1)));
        assert!(!can_step_back(&bs, TimeRange::Day, Some(earliest)));
        // No data → never steppable.
        assert!(!can_step_back(&[], TimeRange::Day, None));
    }

    #[test]
    fn can_step_forward_only_while_anchored_in_the_past() {
        let bs = vec![
            bucket_with("2021-01-01", "e.com", 1),
            bucket_with("2021-01-31", "l.com", 1),
        ];
        let latest = dn("2021-01-31");
        // Live view: nothing later to move toward.
        assert!(!can_step_forward(&bs, TimeRange::Week, None));
        // Anchored in the past: steppable forward.
        assert!(can_step_forward(&bs, TimeRange::Week, Some(latest - 30)));
        // Anchored at (or past) the latest: not forward-steppable.
        assert!(!can_step_forward(&bs, TimeRange::Week, Some(latest)));
    }

    #[test]
    fn period_delta_and_stats_track_the_anchor() {
        let bs = vec![
            bucket_with("2021-01-03", "old.com", 4), // prior week
            bucket_with("2021-01-10", "old.com", 6), // this (anchored) week
            bucket_with("2021-01-12", "new.com", 2),
            bucket_with("2021-01-15", "old.com", 2), // anchor end
            bucket_with("2021-02-01", "future.com", 99), // latest, but excluded
        ];
        let anchor = Some(dn("2021-01-15"));
        let d = period_delta(&bs, TimeRange::Week, anchor).unwrap();
        assert_eq!(d.visits_now, 10, "6 + 2 + 2 inside the anchored week");
        assert_eq!(d.visits_prev, 4, "prior week is anchor-relative");
        assert_eq!(d.hosts_now, 2);
        assert_eq!(d.new_hosts_now, 1); // new.com first-seen in the window
        assert_eq!(d.visits_pct(), Some(150));

        let s = range_stats(&bs, TimeRange::Week, anchor);
        assert_eq!(s.window_visits, 10);
        assert_eq!(s.window_hosts, 2);
        assert!(
            !bs.is_empty() && s.window_visits != 99,
            "the future bucket is not in the anchored window"
        );

        // Surging is likewise anchor-relative: old.com jumps 4 → 8 in the week.
        let surge = surging_hosts(&bs, TimeRange::Week, anchor, 3, 10);
        assert!(surge
            .iter()
            .any(|x| x.host == "old.com" && x.now == 8 && x.prev == 4));
        assert!(!surge.iter().any(|x| x.host == "future.com"));
    }

    fn sess(id: f64, start_ts: f64) -> SessionRec {
        SessionRec {
            id,
            window_id: 1,
            start_ts,
            end_ts: start_ts + 1000.0,
            start_id: id,
            end_id: id,
            nav_count: 1,
            top_hosts: Vec::new(),
        }
    }

    #[test]
    fn session_stepping_walks_oldest_to_newest_and_snaps_to_live() {
        // Deliberately out of order; ordered by (start_ts, id).
        let sessions = vec![sess(2.0, 200.0), sess(0.0, 0.0), sess(1.0, 100.0)];
        let order = session_order(&sessions);
        assert_eq!(order, vec![0.0, 1.0, 2.0]);

        // From live (newest): ‹ goes to the second-newest, › is blocked.
        assert_eq!(session_back(&order, None), SessionStep::To(1.0));
        assert_eq!(session_forward(&order, None), SessionStep::Blocked);
        // From the middle: ‹ to the oldest, › snaps to Live (newest is next).
        assert_eq!(session_back(&order, Some(1.0)), SessionStep::To(0.0));
        assert_eq!(session_forward(&order, Some(1.0)), SessionStep::Live);
        // At the oldest: ‹ is blocked, › steps to the middle.
        assert_eq!(session_back(&order, Some(0.0)), SessionStep::Blocked);
        assert_eq!(session_forward(&order, Some(0.0)), SessionStep::To(1.0));
        // Empty / single-session stores are fully clamped.
        assert_eq!(session_back(&[], None), SessionStep::Blocked);
        assert_eq!(session_back(&[9.0], None), SessionStep::Blocked);
        assert_eq!(session_forward(&[9.0], None), SessionStep::Blocked);
    }
}
