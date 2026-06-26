//! Table views (§7.7, M2): a range-stats banner, activity summary, frequent
//! journeys (PrefixSpan), top hubs (with dwell), directional launch-pads/
//! destinations, browsing communities (Louvain), where-searches-went, top edges,
//! origination, and a raw event stream (M1).

use super::{body_container, empty_body_html, esc, fmt_dwell, plural, Shared};
use crate::graph;
use crate::model::Event;
use crate::project::{self, TimeRange};
use crate::rollup::SessionRec;
use std::collections::HashMap;

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
    let hubs = graph::hubs(&g, 20);
    let (pads, dests) = graph::directional_hubs(&g, 12);
    let edges = graph::top_edges(&a.proj, 20);
    let prov = project::origination(&a.buckets);
    let stats = project::range_stats(&a.buckets, a.time_range);
    let delta = project::period_delta(&a.buckets, a.time_range);
    let series = project::daily_series(&a.buckets, a.time_range);
    let search_dest = graph::search_destinations(&a.proj, 12);
    let next_hops = graph::next_hops(&a.proj, 4, 12);
    let authorities = graph::pagerank(&a.proj, 0.85, 40);
    let dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.dwell_ms))
        .collect();

    let mut html = String::new();

    // ── range stats banner ───────────────────────────────────────────────────
    html.push_str(&stats_html(&stats, a.time_range, delta, &series));

    // ── activity (from the sessions overlapping the range) ───────────────────
    html.push_str(&activity_html(&a.sessions, a.time_range));

    // ── frequent journeys (filled asynchronously below) ──────────────────────
    html.push_str(
        "<div id=\"bg-journeys\"><h3>Your common journeys</h3>\
         <p class=\"muted\">Finding the multi-step paths you take most…</p></div>",
    );

    // ── top hubs (with time spent) ───────────────────────────────────────────
    html.push_str(
        "<h3>Top hubs (by weighted degree)</h3>\
         <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Degree</th>\
         <th class=\"num\">Time spent</th></tr>",
    );
    for (k, d) in &hubs {
        let t = dwell.get(k.as_str()).copied().unwrap_or(0);
        let tcell = if t >= 1000 {
            fmt_dwell(t)
        } else {
            "—".to_string()
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            esc(k),
            d,
            esc(&tcell)
        ));
    }
    html.push_str("</table>");

    // ── directional hubs: launch pads vs destinations ────────────────────────
    html.push_str("<div class=\"tbl-pair\">");
    html.push_str(&degree_table(
        "Launch pads",
        "where journeys start (outbound)",
        &pads,
    ));
    html.push_str(&degree_table(
        "Destinations",
        "where journeys end (inbound)",
        &dests,
    ));
    html.push_str("</div>");

    // ── browsing communities (Louvain) ───────────────────────────────────────
    html.push_str(&communities_html(&a));

    // ── where your searches went ─────────────────────────────────────────────
    if !search_dest.is_empty() {
        html.push_str(
            "<h3>Where your searches went</h3>\
             <table class=\"tbl\"><tr><th>Destination</th><th class=\"num\">From searches</th></tr>",
        );
        for (k, c) in &search_dest {
            html.push_str(&format!(
                "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(k),
                c
            ));
        }
        html.push_str("</table>");
    }

    // ── authorities (PageRank) ───────────────────────────────────────────────
    // Only meaningful with edges; scores shown relative to the top (raw PageRank
    // values ~0.003 read poorly beside the integer-degree tables).
    if !a.proj.edges.is_empty() {
        if let Some((_, top)) = authorities.first().filter(|(_, r)| *r > 0.0) {
            let top = *top;
            html.push_str(
                "<h3>Authorities (PageRank)</h3>\
                 <p class=\"muted tbl-sub\">sites your meaningful paths converge on</p>\
                 <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Score</th></tr>",
            );
            for (k, r) in authorities.iter().take(15) {
                let rel = (r / top * 100.0).round() as u32;
                html.push_str(&format!(
                    "<tr><td>{}</td><td class=\"num\">{rel}</td></tr>",
                    esc(k)
                ));
            }
            html.push_str("</table>");
        }
    }

    // ── where you usually go next ────────────────────────────────────────────
    if !next_hops.is_empty() {
        html.push_str(
            "<h3>Where you usually go next</h3>\
             <p class=\"muted tbl-sub\">your most predictable next step from a site</p>\
             <table class=\"tbl\"><tr><th>From</th><th>Usually →</th><th class=\"num\">Share</th></tr>",
        );
        for h in &next_hops {
            let pct = (h.share * 100.0).round() as u32;
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{pct}%</td></tr>",
                esc(&h.from),
                esc(&h.to)
            ));
        }
        html.push_str("</table>");
    }

    // ── top edges ────────────────────────────────────────────────────────────
    html.push_str("<h3>Top edges (by weight)</h3>\
        <table class=\"tbl\"><tr><th>From</th><th>To</th><th class=\"num\">Weight</th><th>Kind</th></tr>");
    for e in &edges {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td>{:?}</td></tr>",
            esc(&e.from),
            esc(&e.to),
            e.weight,
            e.kinds.dominant()
        ));
    }
    html.push_str("</table>");

    // ── origination ──────────────────────────────────────────────────────────
    html.push_str(
        "<h3>Origination (how pages were reached)</h3>\
        <table class=\"tbl\"><tr><th>Provenance</th><th class=\"num\">Count</th></tr>",
    );
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
        html.push_str(&format!(
            "<tr><td>{name}</td><td class=\"num\">{count}</td></tr>"
        ));
    }
    html.push_str("</table>");

    body.set_inner_html(&html);
    drop(a);

    // Mine frequent journeys off the raw events for the in-range sessions.
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

fn range_label(range: TimeRange) -> &'static str {
    match range {
        TimeRange::Session => "This session",
        TimeRange::Day => "Today",
        TimeRange::Week => "This week",
        TimeRange::Month => "This month",
        TimeRange::Year => "This year",
    }
}

/// A small "Host / Degree" table used for launch pads and destinations.
fn degree_table(title: &str, sub: &str, rows: &[(String, u32)]) -> String {
    let mut h = format!(
        "<div class=\"tbl-col\"><h3>{title}</h3><p class=\"muted tbl-sub\">{sub}</p>\
         <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Weight</th></tr>"
    );
    if rows.is_empty() {
        h.push_str("<tr><td class=\"muted\" colspan=\"2\">—</td></tr>");
    }
    for (k, d) in rows {
        h.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
            esc(k),
            d
        ));
    }
    h.push_str("</table></div>");
    h
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
            .map(|s| esc(s))
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

/// Activity summary from the sessions overlapping the selected range: how many,
/// typical length, busiest local weekday + hour, and the longest run. Uses the
/// browser's local `Date` (so weekday/hour match the user's clock).
fn activity_html(sessions: &[SessionRec], range: TimeRange) -> String {
    let in_range = sessions_in_range(sessions, range);
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

/// Sessions whose span overlaps the trailing window of the selected range,
/// measured back from the most recent session (matches the projection's window).
fn sessions_in_range(sessions: &[SessionRec], range: TimeRange) -> Vec<SessionRec> {
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
        (
            a.db.clone(),
            a.gran,
            sessions_in_range(&a.sessions, a.time_range),
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
        // Paths taken at least twice, up to 5 hops. Prefer longer paths at equal
        // support so genuine multi-step journeys rank above their 2-hop prefixes.
        let mut journeys = graph::frequent_sequences(&chains, 2, 5);
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
        let mut html = format!(
            "<h3>{heading}</h3><table><tr><th>id</th><th>kind</th><th>ts</th><th>detail</th></tr>"
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
        Event::Start { id, ts } => (*id, "start", *ts, "browser start".into()),
    }
}
