//! Table views (§7.7, M2): a range-stats banner, activity summary, frequent
//! journeys (PrefixSpan), top hubs (with dwell), directional launch-pads/
//! destinations, browsing communities (Louvain), where-searches-went, top edges,
//! origination, and a raw event stream (M1).

use super::{body_container, empty_body_html, esc, fmt_dwell, plural, App, Shared, TablesTab};
use crate::export::csv_line;
use crate::graph;
use crate::model::Event;
use crate::project::{self, TimeRange};
use crate::rollup::SessionRec;
use std::collections::HashMap;

/// Per-card row cap so each dashboard tab fits one screen without scrolling. Full
/// data remains available via Export tables (CSV) and the Graph view.
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
    let (pads, dests) = graph::directional_hubs(&g, TOP);
    let edges = graph::top_edges(&a.proj, TOP + 2);
    let prov = project::origination(&a.buckets);
    let stats = project::range_stats(&a.buckets, a.time_range);
    let delta = project::period_delta(&a.buckets, a.time_range);
    let series = project::daily_series(&a.buckets, a.time_range);
    let surging = project::surging_hosts(&a.buckets, a.time_range, 3, TOP);
    let search_dest = graph::search_destinations(&a.proj, TOP);
    let next_hops = graph::next_hops(&a.proj, 4, TOP);
    let authorities = graph::pagerank(&a.proj, 0.85, 40);
    let reciprocal = graph::reciprocal_pairs(&a.proj, TOP);
    let bridges = graph::bridges(&a.proj, TOP);
    let dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.dwell_ms))
        .collect();

    // Each analytic section builds into its category bucket; a category's divider
    // header is emitted only when that bucket has content, so the view groups the
    // depth (Overview / Key sites / Patterns / Communities / Reference) instead of
    // presenting one long flat scroll. The range banner stays the page header.
    let mut overview = String::new();
    let mut key_sites = String::new();
    let mut patterns = String::new();
    let mut reference = String::new();

    // ── Overview: activity + surging ─────────────────────────────────────────
    let activity = activity_html(&a.sessions, a.time_range);
    if !activity.is_empty() {
        overview.push_str(&card("full", &activity));
    }
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
        overview.push_str(&card("", &s));
    }

    // ── Key sites: hubs, launch/destinations, authorities, bridges ───────────
    {
        let mut s = String::from(
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
            s.push_str(&format!(
                "<tr>{}<td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                host_td(k),
                d,
                esc(&tcell)
            ));
        }
        s.push_str("</table>");
        key_sites.push_str(&card("", &s));
    }

    key_sites.push_str(&degree_table(
        "Launch pads",
        "where journeys start (outbound)",
        &pads,
    ));
    key_sites.push_str(&degree_table(
        "Destinations",
        "where journeys end (inbound)",
        &dests,
    ));

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
            key_sites.push_str(&card("", &s));
        }
    }

    if !bridges.is_empty() {
        if let Some((_, top)) = bridges.first() {
            let top = top.max(1.0);
            let mut s = String::from(
                "<h3>Bridge sites</h3>\
                 <p class=\"muted tbl-sub\">gateways between your browsing communities</p>\
                 <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Score</th></tr>",
            );
            for (k, b) in &bridges {
                let rel = (b / top * 100.0).round() as u32;
                s.push_str(&format!(
                    "<tr>{}<td class=\"num\">{rel}</td></tr>",
                    host_td(k)
                ));
            }
            s.push_str("</table>");
            key_sites.push_str(&card("", &s));
        }
    }

    // ── Patterns: journeys, next-hop, back-and-forth, searches ───────────────
    // The journeys block keeps its id so spawn_journeys fills it asynchronously.
    patterns.push_str(&card(
        "wide",
        "<div id=\"bg-journeys\"><h3>Your common journeys</h3>\
         <p class=\"muted\">Finding the multi-step paths you take most…</p></div>",
    ));
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
        patterns.push_str(&card("wide", &s));
    }
    if !reciprocal.is_empty() {
        let mut s = String::from(
            "<h3>Back-and-forth</h3>\
             <p class=\"muted tbl-sub\">sites you bounce between (both directions)</p>\
             <table class=\"tbl\"><tr><th>Pair</th><th class=\"num\">→</th><th class=\"num\">←</th></tr>",
        );
        for r in &reciprocal {
            s.push_str(&format!(
                "<tr><td>{} ⇄ {}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                host_span(&r.a),
                host_span(&r.b),
                r.ab,
                r.ba
            ));
        }
        s.push_str("</table>");
        patterns.push_str(&card("wide", &s));
    }
    if !search_dest.is_empty() {
        let mut s = String::from(
            "<h3>Where your searches went</h3>\
             <table class=\"tbl\"><tr><th>Destination</th><th class=\"num\">From searches</th></tr>",
        );
        for (k, c) in &search_dest {
            s.push_str(&format!(
                "<tr>{}<td class=\"num\">{}</td></tr>",
                host_td(k),
                c
            ));
        }
        s.push_str("</table>");
        patterns.push_str(&card("", &s));
    }
    // Opt-in only (default off): the actual search terms, parsed from already-
    // captured result URLs. Achromatic text — search terms get no data hue.
    if a.show_searches && !a.searches.is_empty() {
        let mut s = String::from(
            "<h3>Top search terms</h3>\
             <p class=\"muted tbl-sub\">parsed from your search-result URLs (opt-in)</p>\
             <table class=\"tbl\"><tr><th>Query</th><th>Engine</th><th class=\"num\">Times</th></tr>",
        );
        for sc in a.searches.iter().take(TOP + 2) {
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(&sc.terms),
                esc(&sc.engine),
                sc.count
            ));
        }
        s.push_str("</table>");
        patterns.push_str(&card("wide", &s));
    }

    // ── Communities (its own group; may be empty) ────────────────────────────
    let communities_inner = communities_html(&a);
    let communities = if communities_inner.is_empty() {
        String::new()
    } else {
        card("wide", &communities_inner)
    };

    // ── Reference: top edges, origination ────────────────────────────────────
    {
        let mut s = String::from("<h3>Top edges (by weight)</h3>\
            <table class=\"tbl\"><tr><th>From</th><th>To</th><th class=\"num\">Weight</th><th>Kind</th></tr>");
        for e in &edges {
            s.push_str(&format!(
                "<tr>{}{}<td class=\"num\">{}</td><td>{:?}</td></tr>",
                host_td(&e.from),
                host_td(&e.to),
                e.weight,
                e.kinds.dominant()
            ));
        }
        s.push_str("</table>");
        reference.push_str(&card("wide", &s));
    }
    {
        let mut s = String::from(
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
            s.push_str(&format!(
                "<tr><td>{name}</td><td class=\"num\">{count}</td></tr>"
            ));
        }
        s.push_str("</table>");
        reference.push_str(&card("", &s));
    }

    // Dashboard assembly: a segmented toggle at the very top swaps to the active
    // category's card grid — each view is one screen instead of one long scroll.
    // The KPI banner is the head of the Overview tab (no separate pinned bar).
    // Network folds the Communities + Reference groups together.
    let active = a.tables_tab;
    let network = format!("{communities}{reference}");
    let body_html = match active {
        TablesTab::Overview => &overview,
        TablesTab::Sites => &key_sites,
        TablesTab::Patterns => &patterns,
        TablesTab::Network => &network,
    };

    let mut html = String::from("<div class=\"tbl-wrap dash\">");
    html.push_str(&subtabs_html(active));
    if active == TablesTab::Overview {
        html.push_str(&stats_html(&stats, a.time_range, delta, &series));
    }
    if body_html.is_empty() {
        html.push_str("<p class=\"muted dash-empty\">Nothing to show in this view for the selected range.</p>");
    } else {
        html.push_str("<div class=\"tbl-grid\">");
        html.push_str(body_html);
        html.push_str("</div>");
    }
    html.push_str("</div>");

    body.set_inner_html(&html);
    drop(a);

    // The journeys card only exists on the Patterns tab; mine them lazily then.
    if active == TablesTab::Patterns {
        spawn_journeys(shared);
    }
    Ok(())
}

/// The Tables dashboard's segmented sub-tab bar. The active tab's `active` class
/// is baked into the markup (re-rendered each switch); a delegated `[data-tab]`
/// listener on `#bg-body` handles the clicks.
fn subtabs_html(active: TablesTab) -> String {
    let tab = |val: &str, label: &str, t: TablesTab| {
        let cls = if active == t { "active" } else { "" };
        format!("<button class=\"{cls}\" data-tab=\"{val}\">{label}</button>")
    };
    format!(
        "<div class=\"seg seg-ghost tbl-tabs\">{}{}{}{}</div>",
        tab("overview", "Overview", TablesTab::Overview),
        tab("sites", "Sites", TablesTab::Sites),
        tab("patterns", "Patterns", TablesTab::Patterns),
        tab("network", "Network", TablesTab::Network),
    )
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

/// A category divider between groups of sections in the Tables view.
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

fn range_label(range: TimeRange) -> &'static str {
    match range {
        TimeRange::Session => "This session",
        TimeRange::Day => "Today",
        TimeRange::Week => "This week",
        TimeRange::Month => "This month",
        TimeRange::Year => "This year",
    }
}

/// A small "Host / Weight" table (launch pads / destinations), wrapped as a card.
fn degree_table(title: &str, sub: &str, rows: &[(String, u32)]) -> String {
    let mut h = format!(
        "<h3>{title}</h3><p class=\"muted tbl-sub\">{sub}</p>\
         <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Weight</th></tr>"
    );
    if rows.is_empty() {
        h.push_str("<tr><td class=\"muted\" colspan=\"2\">—</td></tr>");
    }
    for (k, d) in rows {
        h.push_str(&format!(
            "<tr>{}<td class=\"num\">{}</td></tr>",
            host_td(k),
            d
        ));
    }
    h.push_str("</table>");
    card("", &h)
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
    for m in clusters.iter().take(TOP) {
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
    let prov = project::origination(&a.buckets);
    let stats = project::range_stats(&a.buckets, a.time_range);
    let search_dest = graph::search_destinations(&a.proj, 10_000);
    let next_hops = graph::next_hops(&a.proj, 4, 10_000);
    let authorities = graph::pagerank(&a.proj, 0.85, 40);
    let bridges = graph::bridges(&a.proj, 10_000);
    let reciprocal = graph::reciprocal_pairs(&a.proj, 10_000);
    let surging = project::surging_hosts(&a.buckets, a.time_range, 3, 10_000);
    let dwell: HashMap<&str, u64> = a
        .proj
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.dwell_ms))
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

    section!("Top hubs", &["Host", "Degree", "Dwell (ms)"]);
    for (k, d) in &hubs {
        let d = d.to_string();
        let t = dwell.get(k.as_str()).copied().unwrap_or(0).to_string();
        row!(k, &d, &t);
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
