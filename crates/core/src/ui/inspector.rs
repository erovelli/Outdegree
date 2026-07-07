//! Node inspector (§F8): the right-docked "what is my relationship with this site?"
//! detail panel. It is **one state with the drill-down `focus`** — open exactly
//! when a node is focused on the Graph view, closed by the same Esc / ✕ / click-
//! again that clears focus (wired through [`super::focus_and_animate`]).
//!
//! Everything is scoped to the currently displayed window and filters. The static
//! content (marker, stats, first/last seen, sparkline, connections, footer) renders
//! synchronously from the projection + buckets; the "Top pages" / per-host search
//! terms fill asynchronously from a **bounded** events-store scan (never the whole
//! store, never blocking the render loop) and are cached per `(node, window,
//! granularity)` so redraws and node-to-node hops don't rescan.

use super::filters::panel;
use super::{displayed_window, esc, on, plural, App, Shared, View};
use crate::inspect;
use crate::interpret;
use crate::model::{Event, Granularity, NodeAgg, Provenance};
use crate::project::{self, TimeRange};
use crate::search;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element};

/// Events examined by the bounded async "Top pages" scan before it stops — so a
/// large history is never fully read (§F8 item f).
const SCAN_BUDGET: u32 = 20_000;
/// Rows in the top-pages / top-searches lists.
const TOP: usize = 8;
/// Rows in each of the "Came from" / "Went to" connection lists.
const TOP_CONN: usize = 5;

/// Build the inspector panel once (hidden). A single delegated click handler
/// survives the innerHTML rebuilds and routes the three interactive affordances:
/// close (✕), forget-this-site, and re-focus on a connection row (`data-host`).
pub(super) fn inspector_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "inspector at-inspector");
    let _ = p.set_attribute("id", "bg-inspector");
    let _ = p.set_attribute("role", "complementary");
    let _ = p.set_attribute("aria-label", "Site inspector");
    {
        let s = shared.clone();
        on(&p, "click", move |ev| {
            let Some(t) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            // Close: clears focus, which closes the panel (one state).
            if closest_has(&t, ".insp-x") {
                super::focus_and_animate(&s, None);
                return;
            }
            // Forget this site: routes to the existing confirm+delete flow for the
            // currently-focused host; on completion the host is gone, focus clears,
            // and the panel closes.
            if closest_has(&t, ".insp-forget") {
                let host = s.borrow().focus.clone();
                if let Some(host) = host {
                    super::settings::forget_domain_confirm(&s, &host);
                }
                return;
            }
            // A connection row re-focuses the graph + inspector on that node.
            let host = t
                .closest("[data-host]")
                .ok()
                .flatten()
                .and_then(|e| e.get_attribute("data-host"));
            if let Some(host) = host {
                let present = s.borrow().proj.nodes.iter().any(|n| n.key == host);
                if present {
                    super::focus_and_animate(&s, Some(host));
                }
            }
        });
    }
    p
}

fn closest_has(t: &Element, sel: &str) -> bool {
    t.closest(sel).ok().flatten().is_some()
}

/// Refresh the inspector to the current state. Called at the end of `sync_chrome`,
/// so it tracks every projection/window/filter change. Hides the panel unless a
/// node is focused on the Graph view; otherwise rebuilds it only when its content
/// signature changed (so a plain redraw doesn't reset the panel's scroll), and
/// (re)launches the bounded page scan only when its `(node, window, gran)` key
/// changed.
pub(super) fn sync(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    let Some(pel) = doc.get_element_by_id("bg-inspector") else {
        return;
    };

    // Open iff drilled into a node on the graph — inspector + focus are one state.
    let host = {
        let a = shared.borrow();
        match (a.view, a.focus.clone()) {
            (View::Graph, Some(h)) => Some(h),
            _ => None,
        }
    };
    let Some(host) = host else {
        hide(&pel, shared);
        return;
    };

    let plan = {
        let a = shared.borrow();
        build_plan(&a, &host)
    };
    let Some(plan) = plan else {
        hide(&pel, shared); // focused host not in the projection (shouldn't happen)
        return;
    };

    // Static content unchanged → leave the panel (and its scroll) alone; any
    // in-flight scan patches its own region.
    if shared.borrow().inspector_sig.as_deref() == Some(plan.sig.as_str()) {
        return;
    }

    pel.set_inner_html(&plan.html);
    pel.set_class_name("panel inspector at-inspector show");
    shared.borrow_mut().inspector_sig = Some(plan.sig.clone());

    if plan.need_scan {
        {
            let mut a = shared.borrow_mut();
            a.inspector_scan_key = Some(plan.scan_key.clone());
            a.inspector_scan_pending = true;
            a.inspector_pages.clear();
            a.inspector_pages_capped = false;
            a.inspector_searches.clear();
        }
        spawn_scan(shared, host, plan);
    }
}

fn hide(pel: &Element, shared: &Shared) {
    pel.set_class_name("panel inspector at-inspector");
    shared.borrow_mut().inspector_sig = None;
}

/// What a `sync` needs to render/scan, computed under one borrow.
struct Plan {
    /// Signature of the static render — everything that affects it. Unchanged → no
    /// rebuild.
    sig: String,
    /// `(node, window, granularity, spa)` identity of the page scan. **Includes the
    /// window** so stepping windows invalidates the cache instead of showing stale
    /// URLs. Deliberately excludes the "Show search terms" toggle: the scan always
    /// collects terms (see [`spawn_scan`]) and only the *rendering* is gated, so
    /// flipping the toggle mid-focus surfaces them from cache without a rescan.
    scan_key: String,
    /// Whether the cached pages belong to a different key (→ (re)scan).
    need_scan: bool,
    html: String,
    scan: ScanSpec,
    gran: Granularity,
}

/// How to read the window's events for the async page scan.
enum ScanSpec {
    /// A session's exact id-range (Session range) — bounded by the session size.
    Session { start_id: f64, end_id: f64 },
    /// A calendar window's `[lo, hi)` epoch-ms bounds (bounded by [`SCAN_BUDGET`]).
    Window { lo: f64, hi: f64 },
    /// No resolvable window (no data) — nothing to scan.
    Empty,
}

fn build_plan(a: &App, host: &str) -> Option<Plan> {
    let node = a.proj.nodes.iter().find(|n| n.key == *host)?;
    let gran = a.gran;
    let window = displayed_window(a);

    let (scan, win_id) = scan_spec(a);
    let scan_key = format!("{host}\u{1}{win_id}\u{1}{gran:?}\u{1}{}", a.spa_mode);
    let need_scan = a.inspector_scan_key.as_deref() != Some(scan_key.as_str());

    // Provenance marker — colour + shape match the graph node.
    let prov = node.prov.dominant().display();

    // Domain-mode note: how many hostnames fold into this registrable domain here.
    let domain_note = (gran == Granularity::Registrable)
        .then(|| {
            let mut hosts = std::collections::HashSet::new();
            for b in &window {
                for k in b.nodes.keys() {
                    if interpret::registrable(k) == *host {
                        hosts.insert(k.as_str());
                    }
                }
            }
            hosts.len()
        })
        .filter(|n| *n > 1);

    // Stat row: visits, dwell (fg/≈ convention, shared with Tables), share of window.
    let total_visits: u64 = window
        .iter()
        .flat_map(|b| b.nodes.values())
        .map(|n| n.visits as u64)
        .sum();
    let has_signal = super::tables::window_has_focus_signal(a);
    let (dwell_text, dwell_title) =
        super::tables::dwell_display(has_signal, node.dwell_ms, node.fg_dwell_ms);
    let share = if total_visits > 0 {
        format!(
            "{}%",
            (node.visits as f64 / total_visits as f64 * 100.0).round() as u64
        )
    } else {
        "—".to_string()
    };

    // First/last seen + per-day series over the window (key-aware for the gran).
    let matches_host = |k: &str| match gran {
        Granularity::Hostname => k == host,
        Granularity::Registrable => interpret::registrable(k) == *host,
    };
    let mut dates: Vec<&str> = window
        .iter()
        .filter(|b| b.nodes.keys().any(|k| matches_host(k)))
        .map(|b| b.date.as_str())
        .collect();
    dates.sort_unstable(); // ISO `YYYY-MM-DD` sorts chronologically
    let seen = match (dates.first(), dates.last()) {
        (Some(f), Some(l)) => Some((project::date_label(f), project::date_label(l))),
        _ => None,
    };
    let mut series: Vec<(String, u32)> = window
        .iter()
        .map(|b| {
            let v = b
                .nodes
                .iter()
                .filter(|(k, _)| matches_host(k))
                .map(|(_, n)| n.visits)
                .sum();
            (b.date.clone(), v)
        })
        .collect();
    series.sort_by(|x, y| x.0.cmp(&y.0));

    let conns = inspect::node_connections(&a.proj, host, TOP_CONN);

    let async_inner = if !need_scan && !a.inspector_scan_pending {
        async_region_html(
            &a.inspector_pages,
            a.inspector_pages_capped,
            &a.inspector_searches,
            a.show_searches,
        )
    } else {
        loading_region_html()
    };

    let html = static_html(
        host,
        prov,
        domain_note,
        node,
        &dwell_text,
        dwell_title,
        &share,
        &seen,
        &series,
        &conns,
        &super::favicon_img(a, host),
        &async_inner,
    );

    // Rebuild the static panel whenever any of these change (but not on a mere
    // page-scan completion, which patches its own region — so the panel's scroll
    // survives): the scan key (host/window/gran/spa), the dwell signal, the search
    // toggle, the site-icons toggle (§F12), and the projection shape (drives the
    // connections list).
    let sig = format!(
        "{scan_key}\u{1}{has_signal}\u{1}{}\u{1}{}\u{1}{}",
        a.show_searches,
        a.site_icons,
        project::layout_signature(&a.proj),
    );

    Some(Plan {
        sig,
        scan_key,
        need_scan,
        html,
        scan,
        gran,
    })
}

/// Resolve the events window for the async scan plus a compact identity string for
/// the cache key. Session uses the session's exact id-range; the calendar ranges
/// use the anchored day-window's epoch-ms bounds (§F6).
fn scan_spec(a: &App) -> (ScanSpec, String) {
    if a.time_range == TimeRange::Session {
        return match super::displayed_session(a) {
            Some(s) => (
                ScanSpec::Session {
                    start_id: s.start_id,
                    end_id: s.end_id,
                },
                format!("S{}-{}", s.start_id, s.end_id),
            ),
            None => (ScanSpec::Empty, "S-none".to_string()),
        };
    }
    match project::window_day_bounds(&a.buckets, a.time_range, super::anchor_end_day(a)) {
        Some((start, end)) => (
            ScanSpec::Window {
                lo: start as f64 * 86_400_000.0,
                hi: (end as f64 + 1.0) * 86_400_000.0,
            },
            format!("W{start}-{end}"),
        ),
        None => (ScanSpec::Empty, "W-none".to_string()),
    }
}

/// Read the window's events (bounded), aggregate the host's top pages + (opt-in)
/// search terms, cache them, and patch the async region — but only if the panel is
/// still on the same scan key (the user may have moved on while we read). Never
/// blocks the render loop.
fn spawn_scan(shared: &Shared, host: String, plan: Plan) {
    let db = shared.borrow().db.clone();
    let s = shared.clone();
    let Plan {
        scan,
        scan_key,
        gran,
        ..
    } = plan;
    wasm_bindgen_futures::spawn_local(async move {
        let (navs, capped) = match scan {
            ScanSpec::Session { start_id, end_id } => (
                db.read_events_id_range(start_id, end_id)
                    .await
                    .unwrap_or_default(),
                false,
            ),
            ScanSpec::Window { lo, hi } => db
                .read_recent_navs_in_window(lo, hi, SCAN_BUDGET)
                .await
                .unwrap_or_default(),
            ScanSpec::Empty => (Vec::new(), false),
        };
        let pages = inspect::top_pages(&navs, &host, gran, TOP);
        // Search terms are collected regardless of the "Show search terms" toggle —
        // a purely local re-parse of URLs already read by this same scan — and gated
        // at *render* time only ([`async_region_html`]). This keeps the scan result
        // independent of the toggle, so enabling it mid-focus surfaces the cached
        // terms on the rebuild instead of showing a stale empty list (and no rescan
        // is needed). The opt-in gates what's shown, never what leaves the machine
        // (nothing ever does).
        let urls: Vec<String> = navs
            .iter()
            .filter_map(|e| match e {
                Event::Nav { to_url, .. }
                    if interpret::node_key(to_url, gran).as_deref() == Some(host.as_str()) =>
                {
                    Some(to_url.clone())
                }
                _ => None,
            })
            .collect();
        let searches = search::top_searches(&urls, TOP);

        let doc = {
            let mut a = s.borrow_mut();
            if a.inspector_scan_key.as_deref() != Some(scan_key.as_str()) {
                return; // superseded by a newer node/window — discard
            }
            a.inspector_pages = pages;
            a.inspector_pages_capped = capped;
            a.inspector_searches = searches;
            a.inspector_scan_pending = false;
            a.doc.clone()
        };
        if let Some(region) = doc.get_element_by_id("bg-insp-async") {
            let a = s.borrow();
            region.set_inner_html(&async_region_html(
                &a.inspector_pages,
                a.inspector_pages_capped,
                &a.inspector_searches,
                a.show_searches,
            ));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn static_html(
    host: &str,
    prov: Provenance,
    domain_note: Option<usize>,
    node: &NodeAgg,
    dwell_text: &str,
    dwell_title: &str,
    share: &str,
    seen: &Option<(String, String)>,
    series: &[(String, u32)],
    conns: &inspect::NodeConnections,
    icon_html: &str,
    async_inner: &str,
) -> String {
    let host_e = esc(host);
    // The provenance dot keeps the data-color/shape channel; the favicon (§F12,
    // `""` when off) sits between it and the host name as an identity mark.
    let mut h = format!(
        "<div class=\"insp-head\"><span class=\"dot {dot} {glyph}\"></span>{icon_html}\
         <span class=\"insp-host\" title=\"{host_e}\">{host_e}</span>\
         <button type=\"button\" class=\"insp-x\" aria-label=\"Close inspector\">✕</button></div>",
        dot = prov_dot(prov),
        glyph = prov.shape().css(),
    );
    if let Some(n) = domain_note {
        h.push_str(&format!(
            "<div class=\"insp-note\">aggregating {}</div>",
            plural(n as u64, "hostname")
        ));
    }
    h.push_str(&format!(
        "<div class=\"insp-stats\">\
         <div class=\"insp-stat\"><div class=\"insp-stat-v\">{visits}</div>\
         <div class=\"insp-stat-l\">visits</div></div>\
         <div class=\"insp-stat\"><div class=\"insp-stat-v\" title=\"{dtitle}\">{dtext}</div>\
         <div class=\"insp-stat-l\">time spent</div></div>\
         <div class=\"insp-stat\"><div class=\"insp-stat-v\">{share}</div>\
         <div class=\"insp-stat-l\">of visits</div></div></div>",
        visits = node.visits,
        dtitle = esc(dwell_title),
        dtext = esc(dwell_text),
        share = esc(share),
    ));
    if let Some((first, last)) = seen {
        h.push_str(&format!(
            "<div class=\"insp-seen\"><span class=\"insp-muted\">First seen</span>\
             <span>{}</span></div>\
             <div class=\"insp-seen\"><span class=\"insp-muted\">Last seen</span>\
             <span>{}</span></div>",
            esc(first),
            esc(last),
        ));
    }
    // Per-host daily sparkline (reuses the Tables sparkline construction; empty for
    // < 2 points).
    h.push_str(&super::tables::sparkline_html(series));

    if conns.came_from.is_empty() && conns.went_to.is_empty() {
        h.push_str("<p class=\"insp-muted\">No links to or from this site in view.</p>");
    } else {
        if !conns.came_from.is_empty() {
            h.push_str("<h4 class=\"insp-h\">Came from</h4>");
            h.push_str(&conn_rows(&conns.came_from));
        }
        if !conns.went_to.is_empty() {
            h.push_str("<h4 class=\"insp-h\">Went to</h4>");
            h.push_str(&conn_rows(&conns.went_to));
        }
    }

    h.push_str(&format!(
        "<div class=\"insp-async\" id=\"bg-insp-async\">{async_inner}</div>"
    ));
    h.push_str(
        "<div class=\"insp-foot\">\
         <button type=\"button\" class=\"insp-forget\">Forget this site…</button></div>",
    );
    h
}

fn conn_rows(rows: &[(String, u32)]) -> String {
    let mut h = String::new();
    for (host, w) in rows {
        let host_e = esc(host);
        h.push_str(&format!(
            "<button type=\"button\" class=\"insp-conn\" data-host=\"{host_e}\">\
             <span class=\"insp-conn-host\">{host_e}</span>\
             <span class=\"insp-conn-n\">{w}</span></button>"
        ));
    }
    h
}

/// The async region's inner HTML: top pages (always) + per-host search terms (when
/// opted in and the host is a recognized engine → non-empty).
fn async_region_html(
    pages: &[inspect::PageVisit],
    capped: bool,
    searches: &[search::SearchCount],
    show_searches: bool,
) -> String {
    let mut h = String::from("<h4 class=\"insp-h\">Top pages</h4>");
    if pages.is_empty() {
        // Distinguish "no data" from "scan limit reached": a capped scan that found
        // nothing means the window lies beyond the newest-events budget, not that
        // the site has no pages there.
        if capped {
            h.push_str(&format!(
                "<p class=\"insp-muted\">None found — this window is beyond the {} \
                 most recent events the scan covers.</p>",
                fmt_k(SCAN_BUDGET)
            ));
        } else {
            h.push_str(
                "<p class=\"insp-muted\">No page-level detail for this site in the window.</p>",
            );
        }
    } else {
        for p in pages {
            let path = esc(&p.path_query);
            h.push_str(&format!(
                "<div class=\"insp-page\"><span class=\"insp-page-path\" title=\"{path}\">{path}</span>\
                 <span class=\"insp-page-n\">{}</span></div>",
                p.visits
            ));
        }
        if capped {
            h.push_str(&format!(
                "<p class=\"insp-cap\">from the {} most recent events in this window</p>",
                fmt_k(SCAN_BUDGET)
            ));
        }
    }
    if show_searches && !searches.is_empty() {
        h.push_str("<h4 class=\"insp-h\">Top searches</h4>");
        for sc in searches {
            let terms = esc(&sc.terms);
            h.push_str(&format!(
                "<div class=\"insp-page\"><span class=\"insp-page-path\" title=\"{terms}\">{terms}</span>\
                 <span class=\"insp-page-n\">{}</span></div>",
                sc.count
            ));
        }
    }
    h
}

fn loading_region_html() -> String {
    "<h4 class=\"insp-h\">Top pages</h4>\
     <p class=\"insp-muted\">Scanning recent events…</p>"
        .to_string()
}

/// Compact count for the cap note (`20000` → `"20k"`).
fn fmt_k(n: u32) -> String {
    if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// The legend/marker dot CSS class for a provenance (matches the graph + legend).
fn prov_dot(p: Provenance) -> &'static str {
    match p.display() {
        Provenance::SearchOrigin => "dot-search",
        Provenance::Link => "dot-link",
        Provenance::TypedUrl => "dot-typed",
        Provenance::Bookmark => "dot-bookmark",
        Provenance::Form => "dot-form",
        Provenance::Start => "dot-external",
        _ => "dot-other",
    }
}
