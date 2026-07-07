//! Table views (§7.7, M2): a range-stats banner, activity summary, frequent
//! journeys (PrefixSpan), top hubs (with dwell), directional launch-pads/
//! destinations, browsing communities (Louvain), where-searches-went, top edges,
//! origination, and a raw event stream (M1).

use super::{body_container, empty_body_html, esc, fmt_dwell, plural, App, Shared};
use crate::export::csv_line;
use crate::graph;
use crate::model::Event;
use crate::project::{self, TimeRange};
use crate::rollup::SessionRec;
use std::collections::HashMap;

/// Per-card row cap so the curated dashboard fits one screen without scrolling.
/// Full data remains available via Export tables (CSV) and the Graph view.
const TOP: usize = 6;

pub(crate) fn render(shared: &Shared) -> Result<(), wasm_bindgen::JsValue> {
    let Some(body) = body_container(shared) else {
        return Ok(());
    };
    let a = shared.borrow();

    if a.proj.nodes.is_empty() {
        body.set_inner_html(&empty_body_html(&a));
        return Ok(());
    }

    let g = graph::build(&a.proj);
    let hubs = graph::hubs(&g, TOP);
    // Anchor-relative (§F6): the KPI banner, sparkline, and surging list all scope
    // to the displayed window (the latest window when `anchor` is `None`).
    let ae = super::anchor_end_day(&a);
    let stats = project::range_stats(&a.buckets, a.time_range, ae);
    let delta = project::period_delta(&a.buckets, a.time_range, ae);
    let series = project::daily_series(&a.buckets, a.time_range, ae);
    let surging = project::surging_hosts(&a.buckets, a.time_range, ae, 3, TOP);
    let next_hops = graph::next_hops(&a.proj, 4, TOP);
    let authorities = graph::pagerank(&a.proj, 0.85, 40);
    let dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.dwell_ms))
        .collect();
    // Foreground dwell + whether the displayed window carries any focus signal
    // (§F7): with signal we show real foreground time; without it, the gap-based
    // estimate prefixed "≈".
    let fg_dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.fg_dwell_ms))
        .collect();
    let has_signal = window_has_focus_signal(&a);

    // Curated single-screen dashboard (no tabs): only the highest-signal widgets,
    // row-capped to fit one screen. The full set of tables stays available via
    // Export tables (CSV) and the Graph view.
    let mut grid = String::new();

    // Activity summary (full width), scoped to the same window as the KPIs.
    let activity = activity_html(&a.sessions, a.time_range, anchor_session_window(&a));
    if !activity.is_empty() {
        grid.push_str(&card("full", &activity));
    }

    // Top hubs.
    {
        let mut s = String::from(
            "<h3>Top hubs (by weighted degree)</h3>\
             <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Degree</th>\
             <th class=\"num\">Time spent</th></tr>",
        );
        for (k, d) in &hubs {
            let est = dwell.get(k.as_str()).copied().unwrap_or(0);
            let fg = fg_dwell.get(k.as_str()).copied().unwrap_or(0);
            s.push_str(&format!(
                "<tr>{}<td class=\"num\">{}</td>{}</tr>",
                host_td(k),
                d,
                time_spent_cell(has_signal, est, fg)
            ));
        }
        s.push_str("</table>");
        grid.push_str(&card("", &s));
    }

    // Authorities (PageRank).
    if !a.proj.edges.is_empty() {
        if let Some((_, top)) = authorities.first().filter(|(_, r)| *r > 0.0) {
            let top = *top;
            let mut s = String::from(
                "<h3>Authorities (PageRank)</h3>\
                 <p class=\"muted tbl-sub\">sites your meaningful paths converge on</p>\
                 <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Score</th></tr>",
            );
            for (k, r) in authorities.iter().take(TOP) {
                let rel = (r / top * 100.0).round() as u32;
                s.push_str(&format!(
                    "<tr>{}<td class=\"num\">{rel}</td></tr>",
                    host_td(k)
                ));
            }
            s.push_str("</table>");
            grid.push_str(&card("", &s));
        }
    }

    // Surging this period.
    if !surging.is_empty() {
        let mut s = String::from(
            "<h3>Surging this period</h3>\
             <p class=\"muted tbl-sub\">sites you're visiting more than before</p>\
             <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Now</th>\
             <th class=\"num\">Before</th><th class=\"num\">×</th></tr>",
        );
        for sg in &surging {
            let mult = if sg.is_new() {
                "new".to_string()
            } else {
                format!("{:.0}×", sg.ratio())
            };
            s.push_str(&format!(
                "<tr>{}<td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                host_td(&sg.host),
                sg.now,
                sg.prev,
                esc(&mult)
            ));
        }
        s.push_str("</table>");
        grid.push_str(&card("", &s));
    }

    // Common journeys (wide; the #bg-journeys div is filled asynchronously).
    grid.push_str(&card(
        "wide",
        "<div id=\"bg-journeys\"><h3>Your common journeys</h3>\
         <p class=\"muted\">Finding the multi-step paths you take most…</p></div>",
    ));

    // Where you usually go next (wide).
    if !next_hops.is_empty() {
        let mut s = String::from(
            "<h3>Where you usually go next</h3>\
             <p class=\"muted tbl-sub\">your most predictable next step from a site</p>\
             <table class=\"tbl\"><tr><th>From</th><th>Usually →</th><th class=\"num\">Share</th></tr>",
        );
        for h in &next_hops {
            let pct = (h.share * 100.0).round() as u32;
            s.push_str(&format!(
                "<tr>{}{}<td class=\"num\">{pct}%</td></tr>",
                host_td(&h.from),
                host_td(&h.to)
            ));
        }
        s.push_str("</table>");
        grid.push_str(&card("wide", &s));
    }

    // Browsing communities (wide).
    let communities_inner = communities_html(&a);
    if !communities_inner.is_empty() {
        grid.push_str(&card("wide", &communities_inner));
    }

    // Assemble: KPI banner at the top of the centered wrapper, then the single
    // curated card grid — one screen, no tabs, no long scroll.
    let mut html = String::from("<div class=\"tbl-wrap dash\">");
    html.push_str(&stats_html(&stats, a.time_range, delta, &series));
    html.push_str("<div class=\"tbl-grid\">");
    html.push_str(&grid);
    html.push_str("</div></div>");

    body.set_inner_html(&html);
    drop(a);

    // The journeys card is always present; mine the paths lazily.
    spawn_journeys(shared);
    Ok(())
}

/// The range-stats summary cards (sites, visits, new sites discovered, revisits),
/// each with a vs-previous-period delta chip, plus a daily-visits sparkline for
/// multi-day ranges.
fn stats_html(
    s: &project::RangeStats,
    range: TimeRange,
    delta: Option<project::PeriodDelta>,
    series: &[(String, u32)],
) -> String {
    let pct = (s.revisit_rate * 100.0).round() as u32;
    // A delta chip is shown only when there's a prior period to compare against.
    let baseline = delta.map(|d| !d.no_baseline()).unwrap_or(false);
    let chip = |p: Option<i32>| -> String {
        if !baseline {
            return String::new();
        }
        match p {
            Some(p) if p > 0 => format!(" <span class=\"stat-delta up\">▲{p}%</span>"),
            Some(p) if p < 0 => format!(" <span class=\"stat-delta down\">▼{}%</span>", -p),
            Some(_) => " <span class=\"stat-delta flat\">±0%</span>".to_string(),
            None => " <span class=\"stat-delta flat\">new</span>".to_string(),
        }
    };
    let card = |value: String, label: &str, p: Option<i32>| -> String {
        format!(
            "<div class=\"stat-card\"><div class=\"stat-value\">{value}{}</div>\
             <div class=\"stat-label\">{label}</div></div>",
            chip(p)
        )
    };
    format!(
        "<h2 class=\"range-title\">{}</h2><div class=\"stat-cards\">{}{}{}{}</div>{}",
        range_label(range),
        card(
            s.window_hosts.to_string(),
            "sites",
            delta.and_then(|d| d.hosts_pct())
        ),
        card(
            s.window_visits.to_string(),
            "visits",
            delta.and_then(|d| d.visits_pct())
        ),
        card(
            s.new_hosts.to_string(),
            "newly discovered",
            delta.and_then(|d| d.new_hosts_pct())
        ),
        card(format!("{pct}%"), "were revisits", None),
        sparkline_html(series),
    )
}

/// A small achromatic daily-visits sparkline (SVG polyline). Empty for fewer than
/// two points (Session/Day, or a single-bucket window) — nothing to trend.
fn sparkline_html(series: &[(String, u32)]) -> String {
    if series.len() < 2 {
        return String::new();
    }
    let (w, h, pad) = (260.0_f64, 42.0_f64, 5.0_f64);
    let max = series.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1) as f64;
    let n = series.len();
    let dx = (w - 2.0 * pad) / ((n - 1) as f64);
    let pts: Vec<String> = series
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let x = pad + i as f64 * dx;
            let y = h - pad - (*v as f64 / max) * (h - 2.0 * pad);
            format!("{x:.1},{y:.1}")
        })
        .collect();
    let last = pts.last().cloned().unwrap_or_default();
    let (lx, ly) = last.split_once(',').unwrap_or(("0", "0"));
    format!(
        "<div class=\"sparkline-wrap\"><span class=\"sparkline-cap\">daily visits</span>\
         <svg class=\"sparkline\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">\
         <polyline points=\"{}\" fill=\"none\"/><circle cx=\"{lx}\" cy=\"{ly}\" r=\"2.5\"/></svg></div>",
        pts.join(" ")
    )
}

/// Wrap one analytic section as a grid card. `span` is "" (one column), "wide"
/// (two columns, for tables with long rows like journeys/edges), or "full" (the
/// whole row, for the activity summary).
fn card(span: &str, inner: &str) -> String {
    let cls = if span.is_empty() {
        "tbl-card".to_string()
    } else {
        format!("tbl-card {span}")
    };
    format!("<section class=\"{cls}\">{inner}</section>")
}

/// A host table cell tagged for click-to-focus: clicking it jumps to the Graph
/// view focused on that host (a delegated listener on #bg-body reads `data-host`).
fn host_td(host: &str) -> String {
    let h = esc(host);
    format!("<td class=\"host-cell\" data-host=\"{h}\">{h}</td>")
}

/// Inline host span variant for cells that list several hosts (pairs, community
/// members), so each name is independently clickable.
fn host_span(host: &str) -> String {
    let h = esc(host);
    format!("<span class=\"host-cell\" data-host=\"{h}\">{h}</span>")
}

/// Whether the currently displayed window carries any focus signal (§F7) — i.e.
/// any of its day buckets saw a focus/window-focus event. Mirrors the window the
/// projection uses (the session-scoped buckets for the Session range, else the
/// anchored calendar window), so the dwell column matches what's on screen.
fn window_has_focus_signal(a: &App) -> bool {
    let window = if a.time_range == TimeRange::Session && !a.session_buckets.is_empty() {
        a.session_buckets.clone()
    } else {
        project::select_window(&a.buckets, a.time_range, super::anchor_end_day(a))
    };
    window.iter().any(|b| b.has_focus_signal)
}

/// The "Time spent" cell for a host (§F7). With a focus signal in the window it
/// shows real foreground time; without one, the gap-based estimate prefixed "≈".
/// Either way a `title` tooltip explains which is shown. Sub-second values read
/// "—" (too little attention to be meaningful).
fn time_spent_cell(has_signal: bool, est_ms: u64, fg_ms: u64) -> String {
    let (ms, prefix, title) = if has_signal {
        (
            fg_ms,
            "",
            "Foreground time — how long this site's tab was the focused, active tab.",
        )
    } else {
        (
            est_ms,
            "≈",
            "Estimated from the gaps between navigations (this window has no focus \
             data), so it can include time a tab sat in the background.",
        )
    };
    let text = if ms >= 1000 {
        format!("{prefix}{}", fmt_dwell(ms))
    } else {
        "—".to_string()
    };
    format!(
        "<td class=\"num\" title=\"{}\">{}</td>",
        esc(title),
        esc(&text)
    )
}

fn range_label(range: TimeRange) -> &'static str {
    match range {
        TimeRange::Session => "This session",
        TimeRange::Day => "Today",
        TimeRange::Week => "This week",
        TimeRange::Month => "This month",
        TimeRange::Year => "This year",
    }
}

/// Louvain communities as a table: each multi-host cluster, its size, and its
/// busiest members. Singletons (the disconnected direct-visit nodes) are omitted.
fn communities_html(a: &super::App) -> String {
    // group node keys by community, carrying visit counts for ranking members
    let visits: HashMap<&str, u32> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.visits))
        .collect();
    let mut groups: HashMap<usize, Vec<&str>> = HashMap::new();
    for n in &a.proj.nodes {
        if let Some(&c) = a.communities.get(&n.key) {
            groups.entry(c).or_default().push(n.key.as_str());
        }
    }
    let mut clusters: Vec<Vec<&str>> = groups.into_values().filter(|m| m.len() >= 2).collect();
    if clusters.is_empty() {
        return String::new();
    }
    // sort members within a cluster by visits desc; clusters by size desc
    for m in &mut clusters {
        m.sort_by(|x, y| visits.get(y).cmp(&visits.get(x)).then_with(|| x.cmp(y)));
    }
    clusters.sort_by(|x, y| y.len().cmp(&x.len()).then_with(|| x[0].cmp(y[0])));

    let mut h = String::from(
        "<h3>Browsing communities</h3>\
         <p class=\"muted tbl-sub\">clusters of sites you move between (Louvain)</p>\
         <table class=\"tbl\"><tr><th>Size</th><th>Top sites</th></tr>",
    );
    for m in clusters.iter().take(10) {
        let top = m
            .iter()
            .take(5)
            .map(|s| host_span(s))
            .collect::<Vec<_>>()
            .join(", ");
        let more = if m.len() > 5 {
            format!(" +{}", m.len() - 5)
        } else {
            String::new()
        };
        h.push_str(&format!(
            "<tr><td class=\"num\">{}</td><td>{top}{more}</td></tr>",
            m.len()
        ));
    }
    h.push_str("</table>");
    h
}

/// The epoch-millisecond `[lo, hi)` window the anchored Tables cards scope their
/// session-derived stats to, matching the day-bucket window the KPIs use (§F6).
/// `None` when live, so those cards keep their trailing-window behavior.
fn anchor_session_window(a: &App) -> Option<(f64, f64)> {
    a.anchor?; // live view (no anchor) → the cards keep their trailing window
    let (start, end) =
        project::window_day_bounds(&a.buckets, a.time_range, super::anchor_end_day(a))?;
    Some((
        start as f64 * 86_400_000.0,
        (end as f64 + 1.0) * 86_400_000.0,
    ))
}

/// Activity summary from the sessions overlapping the selected window: how many,
/// typical length, busiest local weekday + hour, and the longest run. Uses the
/// browser's local `Date` (so weekday/hour match the user's clock). `win` scopes
/// to an anchored window (§F6); `None` is the live trailing window.
fn activity_html(sessions: &[SessionRec], range: TimeRange, win: Option<(f64, f64)>) -> String {
    let in_range = sessions_in_range(sessions, range, win);
    if in_range.is_empty() {
        return String::new();
    }
    let count = in_range.len();
    let mut navs: Vec<u32> = in_range.iter().map(|s| s.nav_count).collect();
    navs.sort_unstable();
    let median = navs[navs.len() / 2];

    // Local-time histograms over session starts.
    let mut hours = [0u32; 24];
    let mut days = [0u32; 7];
    for s in &in_range {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(s.start_ts));
        hours[(d.get_hours() as usize).min(23)] += 1;
        days[(d.get_day() as usize).min(6)] += 1;
    }
    let busiest_hour = (0..24).max_by_key(|&i| hours[i]).unwrap_or(0);
    let busiest_day = (0..7).max_by_key(|&i| days[i]).unwrap_or(0);

    // Longest run by nav count.
    let longest = in_range.iter().max_by_key(|s| s.nav_count);
    let longest_str = longest
        .map(|s| {
            let host = s.top_hosts.first().map(|(h, _)| h.as_str()).unwrap_or("—");
            format!("{} on {}", plural(s.nav_count as u64, "visit"), esc(host))
        })
        .unwrap_or_else(|| "—".to_string());

    let card = |value: String, label: &str| -> String {
        format!("<div class=\"stat-card\"><div class=\"stat-value\">{value}</div><div class=\"stat-label\">{label}</div></div>")
    };
    format!(
        "<h3>Activity</h3><div class=\"stat-cards\">{}{}{}{}</div>\
         <p class=\"muted\">Longest run: {longest_str}.</p>",
        card(count.to_string(), "sessions"),
        card(median.to_string(), "median visits / session"),
        card(weekday_name(busiest_day).to_string(), "busiest day"),
        card(fmt_hour(busiest_hour), "busiest hour"),
    )
}

/// Sessions overlapping the selected window. When `win` is set (an anchored
/// `[lo, hi)` in epoch ms, §F6) it returns the sessions that overlap it; otherwise
/// it's the live trailing window measured back from the most recent session
/// (matching the projection's window).
fn sessions_in_range(
    sessions: &[SessionRec],
    range: TimeRange,
    win: Option<(f64, f64)>,
) -> Vec<SessionRec> {
    if let Some((lo, hi)) = win {
        return sessions
            .iter()
            .filter(|s| s.end_ts >= lo && s.start_ts < hi)
            .cloned()
            .collect();
    }
    let days = match range {
        TimeRange::Session | TimeRange::Day => 1,
        TimeRange::Week => 7,
        TimeRange::Month => 30,
        TimeRange::Year => 365,
    };
    let Some(latest) = sessions
        .iter()
        .map(|s| s.end_ts)
        .fold(None, |acc: Option<f64>, t| {
            Some(acc.map_or(t, |a| a.max(t)))
        })
    else {
        return Vec::new();
    };
    let cutoff = latest - (days as f64) * 86_400_000.0;
    sessions
        .iter()
        .filter(|s| s.end_ts >= cutoff)
        .cloned()
        .collect()
}

fn weekday_name(d: usize) -> &'static str {
    [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ]
    .get(d)
    .copied()
    .unwrap_or("—")
}

fn fmt_hour(h: usize) -> String {
    let (h12, ap) = match h {
        0 => (12, "AM"),
        12 => (12, "PM"),
        1..=11 => (h, "AM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12} {ap}")
}

/// The most-recent in-range sessions to scan for journey mining, as id-ranges.
/// Capped so a year of history doesn't read every event; the cap is surfaced.
const JOURNEY_SESSION_CAP: usize = 150;

fn spawn_journeys(shared: &Shared) {
    let (db, gran, mut sess) = {
        let a = shared.borrow();
        let win = anchor_session_window(&a);
        (
            a.db.clone(),
            a.gran,
            sessions_in_range(&a.sessions, a.time_range, win),
        )
    };
    // Most recent first, then cap.
    sess.sort_by(|a, b| {
        b.start_ts
            .partial_cmp(&a.start_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = sess.len();
    let capped = total > JOURNEY_SESSION_CAP;
    sess.truncate(JOURNEY_SESSION_CAP);

    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut chains: Vec<Vec<String>> = Vec::new();
        for rec in &sess {
            if let Ok(events) = db.read_events_id_range(rec.start_id, rec.end_id).await {
                chains.extend(crate::flow::session_chains(&events, gran));
            }
        }
        // Paths taken at least twice, up to 5 hops, reduced to closed patterns so a
        // single repeated trail doesn't fill the list with its own prefixes
        // (a→b, a→b→c, a→b→c→d). Then prefer longer paths at equal support.
        let mut journeys = graph::closed_sequences(graph::frequent_sequences(&chains, 2, 5));
        journeys.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.len().cmp(&a.0.len())));

        let Some(el) = s.borrow().doc.get_element_by_id("bg-journeys") else {
            return;
        };
        el.set_inner_html(&journeys_html(&journeys, capped, total));
    });
}

fn journeys_html(journeys: &[(Vec<String>, u32)], capped: bool, total: usize) -> String {
    let mut h = String::from("<h3>Your common journeys</h3>");
    if journeys.is_empty() {
        h.push_str(
            "<p class=\"muted\">No repeated multi-step paths yet — they appear once you \
             retrace the same link trail more than once.</p>",
        );
        return h;
    }
    if capped {
        h.push_str(&format!(
            "<p class=\"muted tbl-sub\">from your {JOURNEY_SESSION_CAP} most recent sessions \
             (of {total} in range)</p>"
        ));
    } else {
        h.push_str("<p class=\"muted tbl-sub\">paths you retrace, by how often</p>");
    }
    h.push_str("<table class=\"tbl\"><tr><th>Journey</th><th class=\"num\">Times</th></tr>");
    for (path, sup) in journeys.iter().take(15) {
        let trail = path.iter().map(|s| esc(s)).collect::<Vec<_>>().join(" → ");
        h.push_str(&format!(
            "<tr><td>{trail}</td><td class=\"num\">{sup}×</td></tr>"
        ));
    }
    h.push_str("</table>");
    h
}

/// Assemble the current projection's synchronous tables into one multi-section
/// CSV document (the async journeys/activity panes are omitted). Each section is a
/// title line, a header row, then data rows — practical for spreadsheets. Pure
/// string building over data already in memory; downloaded via the Blob sink.
pub(crate) fn tables_csv(a: &App) -> String {
    let g = graph::build(&a.proj);
    let hubs = graph::hubs(&g, 10_000);
    let (pads, dests) = graph::directional_hubs(&g, 10_000);
    let edges = graph::top_edges(&a.proj, 10_000);
    // `origination` stays all-history in the CSV (it was never range-windowed);
    // the window-scoped stats below track the §F6 anchor via `ae`.
    let prov = project::origination(&a.buckets);
    let ae = super::anchor_end_day(a);
    let stats = project::range_stats(&a.buckets, a.time_range, ae);
    let search_dest = graph::search_destinations(&a.proj, 10_000);
    let next_hops = graph::next_hops(&a.proj, 4, 10_000);
    let authorities = graph::pagerank(&a.proj, 0.85, 40);
    let bridges = graph::bridges(&a.proj, 10_000);
    let reciprocal = graph::reciprocal_pairs(&a.proj, 10_000);
    let surging = project::surging_hosts(&a.buckets, a.time_range, ae, 3, 10_000);
    let dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.dwell_ms))
        .collect();
    let fg_dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.fg_dwell_ms))
        .collect();

    let mut out = String::new();
    // Both helpers append to `out`; as macros (not closures) they avoid holding a
    // persistent mutable borrow that would conflict between section and row writes.
    macro_rules! section {
        ($title:expr, $header:expr $(,)?) => {{
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str($title);
            out.push('\n');
            out.push_str(&csv_line($header));
            out.push('\n');
        }};
    }
    macro_rules! row {
        ($($f:expr),+ $(,)?) => {{
            out.push_str(&csv_line(&[$($f),+]));
            out.push('\n');
        }};
    }

    section!("Range summary", &["Metric", "Value"]);
    let (wh, wv, nh) = (
        stats.window_hosts.to_string(),
        stats.window_visits.to_string(),
        stats.new_hosts.to_string(),
    );
    let rr = format!("{}%", (stats.revisit_rate * 100.0).round() as u32);
    row!("Range", range_label(a.time_range));
    row!("Sites", &wh);
    row!("Visits", &wv);
    row!("Newly discovered", &nh);
    row!("Revisit rate", &rr);

    section!(
        "Top hubs",
        &["Host", "Degree", "Dwell est (ms)", "Foreground (ms)"],
    );
    for (k, d) in &hubs {
        let d = d.to_string();
        let t = dwell.get(k.as_str()).copied().unwrap_or(0).to_string();
        let f = fg_dwell.get(k.as_str()).copied().unwrap_or(0).to_string();
        row!(k, &d, &t, &f);
    }

    section!("Launch pads", &["Host", "Outbound"]);
    for (k, d) in &pads {
        let d = d.to_string();
        row!(k, &d);
    }
    section!("Destinations", &["Host", "Inbound"]);
    for (k, d) in &dests {
        let d = d.to_string();
        row!(k, &d);
    }

    if let Some((_, top)) = authorities.first().filter(|(_, r)| *r > 0.0) {
        let top = *top;
        section!("Authorities (PageRank)", &["Host", "Score"]);
        for (k, r) in authorities.iter().take(50) {
            let s = ((r / top) * 100.0).round().to_string();
            row!(k, &s);
        }
    }
    if let Some((_, top)) = bridges.first() {
        let top = top.max(1.0);
        section!("Bridge sites", &["Host", "Score"]);
        for (k, b) in &bridges {
            let s = ((b / top) * 100.0).round().to_string();
            row!(k, &s);
        }
    }

    if !surging.is_empty() {
        section!("Surging this period", &["Host", "Now", "Before"]);
        for sg in &surging {
            let (n, p) = (sg.now.to_string(), sg.prev.to_string());
            row!(&sg.host, &n, &p);
        }
    }
    if !next_hops.is_empty() {
        section!("Where you usually go next", &["From", "To", "Share %"]);
        for h in &next_hops {
            let pct = ((h.share) * 100.0).round().to_string();
            row!(&h.from, &h.to, &pct);
        }
    }
    if !reciprocal.is_empty() {
        section!("Back-and-forth", &["A", "B", "A→B", "B→A"]);
        for r in &reciprocal {
            let (ab, ba) = (r.ab.to_string(), r.ba.to_string());
            row!(&r.a, &r.b, &ab, &ba);
        }
    }
    if !search_dest.is_empty() {
        section!(
            "Where your searches went",
            &["Destination", "From searches"],
        );
        for (k, c) in &search_dest {
            let c = c.to_string();
            row!(k, &c);
        }
    }

    section!("Top edges", &["From", "To", "Weight", "Kind"]);
    for e in &edges {
        let (w, kind) = (e.weight.to_string(), format!("{:?}", e.kinds.dominant()));
        row!(&e.from, &e.to, &w, &kind);
    }

    section!("Origination", &["Provenance", "Count"]);
    for (name, count) in [
        ("Link", prov.link),
        ("Form", prov.form),
        ("Typed URL", prov.typed_url),
        ("Search origin", prov.search_origin),
        ("Bookmark", prov.bookmark),
        ("External", prov.start),
        ("Reload", prov.reload),
        ("Other", prov.other),
    ] {
        let c = count.to_string();
        row!(name, &c);
    }

    out
}

/// Raw event stream (M1) — a bounded read (first 1000) plus a total count, so the
/// whole store is never deserialized just to show a page.
pub(crate) fn render_raw(shared: &Shared) {
    let Some(body) = body_container(shared) else {
        return;
    };
    body.set_inner_html("<div class=\"bg-empty\">Loading events…</div>");
    let db = shared.borrow().db.clone();
    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        const CAP: u32 = 1000;
        let total = db.count_events().await.unwrap_or(0);
        let events = db.read_events_head(CAP).await.unwrap_or_default();
        let Some(body) = body_container(&s) else {
            return;
        };
        let heading = if total > CAP {
            format!("Raw events ({total} total, showing first {CAP})")
        } else {
            format!("Raw events ({})", plural(total as u64, "event"))
        };
        // Per-kind counts of the shown page (§F7 volume note): focus/wfocus multiply
        // event volume, so the header surfaces how the stream splits across kinds.
        let mut html = format!(
            "<h3>{heading}</h3><p class=\"muted tbl-sub\">{}</p>\
             <table><tr><th>id</th><th>kind</th><th>ts</th><th>detail</th></tr>",
            esc(&kind_counts_line(&events))
        );
        for ev in events.iter() {
            let (id, kind, ts, detail) = describe(ev);
            html.push_str(&format!(
                "<tr><td class=\"mono\">{id:.0}</td><td>{kind}</td>\
                 <td class=\"mono\" title=\"{ts:.0}\">{}</td><td>{}</td></tr>",
                esc(&fmt_ts(ts)),
                esc(&detail)
            ));
        }
        html.push_str("</table>");
        body.set_inner_html(&html);
    });
}

/// One-line per-kind breakdown of the shown events (§F7 volume note), in a fixed
/// kind order; kinds with zero occurrences are omitted. E.g.
/// `"nav 412 · link 8 · close 51 · focus 203 · wfocus 96 · start 2"`.
fn kind_counts_line(events: &[Event]) -> String {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for ev in events {
        *counts.entry(describe(ev).1).or_insert(0) += 1;
    }
    ["nav", "link", "close", "focus", "wfocus", "start"]
        .iter()
        .filter_map(|k| counts.get(k).map(|c| format!("{k} {c}")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Format an epoch-millisecond timestamp as an ISO-8601 UTC string — unambiguous
/// and sortable, matching the UTC-day bucketing the rest of the app uses (the raw
/// millisecond value is kept in the cell's `title` for debugging).
fn fmt_ts(ms: f64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms))
        .to_iso_string()
        .into()
}

fn describe(ev: &Event) -> (f64, &'static str, f64, String) {
    match ev {
        Event::Nav {
            id,
            ts,
            tab_id,
            to_url,
            transition_type,
            qualifiers,
            ..
        } => (
            *id,
            "nav",
            *ts,
            format!(
                "tab {} → {} [{}{}]",
                tab_id,
                to_url,
                transition_type,
                if qualifiers.is_empty() {
                    String::new()
                } else {
                    format!(" {}", qualifiers.join(","))
                }
            ),
        ),
        Event::Link {
            id,
            ts,
            new_tab_id,
            source_tab_id,
        } => (
            *id,
            "link",
            *ts,
            format!("tab {source_tab_id} → new tab {new_tab_id}"),
        ),
        Event::Close { id, ts, tab_id } => (*id, "close", *ts, format!("tab {tab_id}")),
        Event::Focus {
            id,
            ts,
            tab_id,
            window_id,
        } => (
            *id,
            "focus",
            *ts,
            format!(
                "tab {} active in window {}",
                *tab_id as i64, *window_id as i64
            ),
        ),
        Event::Wfocus { id, ts, window_id } => (
            *id,
            "wfocus",
            *ts,
            if *window_id < 0.0 {
                "browser blurred".to_string()
            } else {
                format!("window {} focused", *window_id as i64)
            },
        ),
        Event::Start { id, ts } => (*id, "start", *ts, "browser start".into()),
        // Unrecognized kinds are dropped before a read reaches here (§F7); this arm
        // keeps the match exhaustive.
        Event::Unknown => (f64::NAN, "unknown", f64::NAN, String::new()),
    }
}
