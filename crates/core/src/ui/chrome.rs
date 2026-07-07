//! Floating glass control clusters (§7.7): brand + REC, range, view switcher +
//! settings gear, provenance legend, zoom toolbar, host search + filter chips,
//! readout, drill-down focus chip — and `sync_chrome`, the data-driven refresh
//! that updates every control to the current projection/state.

use super::filters::{chip, icon, icon_btn, panel, seg};
use super::onboarding::exit_sample;
use super::settings::refresh_storage_readout;
use super::{
    el, graph_view, on, persist_positions, persist_ui_prefs, plural, recompute_projection,
    rerender, set_text, span, Shared, View,
};
use crate::model::{Granularity, KindBreakdown, ProvBreakdown, Provenance};
use crate::project::TimeRange;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlInputElement, HtmlSelectElement, KeyboardEvent};

// ── drill-down focus chip (top-center, shown only while a node is focused) ─────
pub(super) fn focus_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "focuschip at-fc");
    let _ = p.set_attribute("id", "bg-focuschip");
    let lbl = span(doc, "focus-label", "");
    let _ = lbl.set_attribute("id", "bg-focus-label");
    let x = el(doc, "button");
    let _ = x.set_attribute("type", "button");
    let _ = x.set_attribute("class", "focus-x");
    let _ = x.set_attribute("aria-label", "Clear focus");
    x.set_text_content(Some("✕"));
    {
        let s = shared.clone();
        on(&x, "click", move |_| super::focus_and_animate(&s, None));
    }
    let _ = p.append_child(&lbl);
    let _ = p.append_child(&x);
    p
}

// ── 1. brand + REC (top-left) ────────────────────────────────────────────────
pub(super) fn brand_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "brand at-tl");
    let name = span(doc, "wordmark", "Outdegree");
    let rule = el(doc, "div");
    let _ = rule.set_attribute("class", "vrule");

    // A real <button> (not a clickable <div>) so the privacy control is keyboard-
    // operable and gets a focus ring; its label is updated to match the state.
    let rec = el(doc, "button");
    let _ = rec.set_attribute("type", "button");
    let _ = rec.set_attribute("class", "rec");
    let _ = rec.set_attribute("id", "bg-rec");
    let _ = rec.set_attribute("title", "Toggle recording");
    let _ = rec.set_attribute("aria-label", "Pause recording");
    let dot = el(doc, "span");
    let _ = dot.set_attribute("class", "rec-dot");
    let lbl = span(doc, "rec-label", "REC");
    let _ = lbl.set_attribute("id", "bg-rec-label");
    let _ = rec.append_child(&dot);
    let _ = rec.append_child(&lbl);
    {
        let s = shared.clone();
        on(&rec, "click", move |_| {
            let now = {
                let mut a = s.borrow_mut();
                a.paused = !a.paused;
                a.paused
            };
            crate::bridge::storage_local_set("paused", if now { "true" } else { "false" });
            sync_chrome(&s);
        });
    }

    let _ = p.append_child(&name);
    let _ = p.append_child(&rule);
    let _ = p.append_child(&rec);

    // Sample-data chip (§F4): built hidden inside the brand cluster so it sits
    // right by the REC indicator, and revealed by `sync_chrome` while the demo
    // dataset is loaded. "Exit sample" wipes it and returns to a clean, empty app.
    let demo = el(doc, "div");
    let _ = demo.set_attribute("class", "demo-chip");
    let _ = demo.set_attribute("id", "bg-demochip");
    let demo_rule = el(doc, "div");
    let _ = demo_rule.set_attribute("class", "vrule");
    let demo_label = span(doc, "demo-label", "Sample data");
    let exit = el(doc, "button");
    let _ = exit.set_attribute("type", "button");
    let _ = exit.set_attribute("class", "demo-exit");
    exit.set_text_content(Some("Exit sample"));
    {
        let s = shared.clone();
        on(&exit, "click", move |_| exit_sample(&s));
    }
    let _ = demo.append_child(&demo_rule);
    let _ = demo.append_child(&demo_label);
    let _ = demo.append_child(&exit);
    let _ = p.append_child(&demo);
    p
}

// ── 2. range + time navigation (top-center) ──────────────────────────────────
pub(super) fn range_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "seg-panel at-tc");

    // Row 1: ‹  [ Session | Day | Week | Month | Year ]  › — the range segmented
    // control flanked by the step buttons (§F6).
    let row = el(doc, "div");
    let _ = row.set_attribute("class", "rng-row");
    let prev = step_btn(doc, "rng-prev", "Previous period", "‹");
    {
        let s = shared.clone();
        on(&prev, "click", move |_| step_range(&s, false));
    }
    let (wrap, btns) = seg(
        doc,
        "solid",
        &[
            ("session", "Session"),
            ("day", "Day"),
            ("week", "Week"),
            ("month", "Month"),
            ("year", "Year"),
        ],
    );
    for (val, btn) in &btns {
        let _ = btn.set_attribute("id", &format!("rng-{val}"));
        let range = match val.as_str() {
            "session" => TimeRange::Session,
            "day" => TimeRange::Day,
            "week" => TimeRange::Week,
            "month" => TimeRange::Month,
            _ => TimeRange::Year,
        };
        let s = shared.clone();
        on(btn, "click", move |_| {
            {
                let mut a = s.borrow_mut();
                a.time_range = range;
                // Picking a range restarts navigation at the live "latest" window:
                // the anchor types differ across ranges (a day-number vs a session
                // id), so a clean reset is the least surprising behavior (§F6).
                a.anchor = None;
            }
            recompute_projection(&s);
            persist_positions(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
            // The Session range needs buckets scoped to the latest session's
            // events; load them (async) so it isn't just the latest UTC day.
            if range == TimeRange::Session {
                super::refresh_session_buckets(&s, false);
            }
        });
    }
    let next = step_btn(doc, "rng-next", "Next period", "›");
    {
        let s = shared.clone();
        on(&next, "click", move |_| step_range(&s, true));
    }
    let _ = row.append_child(&prev);
    let _ = row.append_child(&wrap);
    let _ = row.append_child(&next);
    let _ = p.append_child(&row);

    // Row 2: the window-bounds label, plus a "Latest ↩" chip that clears the
    // anchor. `sync_chrome` fills the label and toggles the chip's visibility.
    let meta = el(doc, "div");
    let _ = meta.set_attribute("class", "rng-meta");
    let label = span(doc, "rng-window", "");
    let _ = label.set_attribute("id", "rng-window-label");
    let latest = el(doc, "button");
    let _ = latest.set_attribute("type", "button");
    let _ = latest.set_attribute("class", "rng-latest");
    let _ = latest.set_attribute("id", "rng-latest");
    let _ = latest.set_attribute("aria-label", "Jump to the latest window");
    latest.set_text_content(Some("Latest ↩"));
    {
        let s = shared.clone();
        on(&latest, "click", move |_| go_live(&s));
    }
    let _ = meta.append_child(&label);
    let _ = meta.append_child(&latest);
    let _ = p.append_child(&meta);
    p
}

/// A ‹/› time-navigation step button (a bare glyph button; disabled via the
/// `disabled` attribute by `sync_chrome` at the ends of the timeline).
fn step_btn(doc: &Document, id: &str, aria: &str, glyph: &str) -> Element {
    let b = el(doc, "button");
    let _ = b.set_attribute("type", "button");
    let _ = b.set_attribute("class", "rng-step");
    let _ = b.set_attribute("id", id);
    let _ = b.set_attribute("aria-label", aria);
    b.set_text_content(Some(glyph));
    b
}

/// ‹ / › time-navigation step (§F6): move the displayed window one range duration
/// (calendar ranges) or one session (Session range) back/forward. Gated internally
/// so the keyboard path is as safe as the `disabled`-attribute buttons.
pub(super) fn step_range(shared: &Shared, forward: bool) {
    let range = shared.borrow().time_range;
    if range == TimeRange::Session {
        step_session(shared, forward);
        return;
    }
    let new_anchor = {
        let a = shared.borrow();
        let cur = match a.anchor {
            Some(super::Anchor::Day(d)) => Some(d),
            _ => None,
        };
        if forward {
            if !crate::project::can_step_forward(&a.buckets, range, cur) {
                return;
            }
            crate::project::forward_end(&a.buckets, range, cur)
        } else {
            if !crate::project::can_step_back(&a.buckets, range, cur) {
                return;
            }
            crate::project::back_end(&a.buckets, range, cur)
        }
    };
    shared.borrow_mut().anchor = new_anchor.map(super::Anchor::Day);
    recompute_projection(shared);
    persist_positions(shared);
    let _ = rerender(shared);
}

/// Session-range stepping: walk the sessions store previous/next, snapping back to
/// the live view when › passes the newest session (§F6).
fn step_session(shared: &Shared, forward: bool) {
    let new_anchor = {
        let a = shared.borrow();
        let order = crate::project::session_order(&a.sessions);
        let cur = match a.anchor {
            Some(super::Anchor::Session(id)) => Some(id),
            _ => None,
        };
        let step = if forward {
            crate::project::session_forward(&order, cur)
        } else {
            crate::project::session_back(&order, cur)
        };
        match step {
            crate::project::SessionStep::To(id) => Some(super::Anchor::Session(id)),
            crate::project::SessionStep::Live => None,
            crate::project::SessionStep::Blocked => return,
        }
    };
    shared.borrow_mut().anchor = new_anchor;
    sync_chrome(shared); // instant label / chevron feedback
    super::refresh_session_buckets(shared, true);
}

/// Clear the anchor and return to the live "latest" window (the "Latest ↩" chip).
fn go_live(shared: &Shared) {
    if shared.borrow().anchor.is_none() {
        return;
    }
    shared.borrow_mut().anchor = None;
    if shared.borrow().time_range == TimeRange::Session {
        sync_chrome(shared);
        super::refresh_session_buckets(shared, true);
    } else {
        recompute_projection(shared);
        persist_positions(shared);
        let _ = rerender(shared);
    }
}

// ── 3. view + settings gear (top-right) ──────────────────────────────────────
pub(super) fn view_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "viewbar at-tr");
    // The "Sessions" segment opens the session picker; the actual Sankey diagram
    // lives inside it. The internal value stays "sankey" (enum/CSS unchanged) — the
    // user-facing label is what matters.
    let (wrap, btns) = seg(
        doc,
        "ghost",
        &[
            ("graph", "Graph"),
            ("sankey", "Sessions"),
            ("tables", "Tables"),
        ],
    );
    for (val, btn) in &btns {
        let _ = btn.set_attribute("id", &format!("vw-{val}"));
        let view = match val.as_str() {
            "sankey" => View::Sankey,
            "tables" => View::Tables,
            _ => View::Graph,
        };
        let s = shared.clone();
        on(btn, "click", move |_| {
            s.borrow_mut().view = view;
            persist_ui_prefs(&s);
            let _ = rerender(&s);
        });
    }
    let gear = icon_btn(doc, "bg-gear", "Settings", &icon("gear"));
    let _ = gear.set_attribute("aria-haspopup", "menu");
    let _ = gear.set_attribute("aria-expanded", "false");
    {
        let s = shared.clone();
        on(&gear, "click", move |_| {
            let doc = s.borrow().doc.clone();
            if let Some(pop) = doc.get_element_by_id("bg-settings") {
                let now_open = !pop.class_name().contains("open");
                pop.set_class_name(if now_open {
                    "panel popover at-pop open"
                } else {
                    "panel popover at-pop"
                });
                if let Some(g) = doc.get_element_by_id("bg-gear") {
                    let _ =
                        g.set_attribute("aria-expanded", if now_open { "true" } else { "false" });
                }
                // Refresh the storage readout each time the menu opens (§8.1).
                if now_open {
                    refresh_storage_readout(&s);
                }
            }
        });
    }
    let _ = p.append_child(&wrap);
    let _ = p.append_child(&gear);
    p
}

// ── 4. provenance legend (top-left) ──────────────────────────────────────────
pub(super) fn legend_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "legend at-lt");
    let title = span(doc, "legend-title", "PROVENANCE");
    let rows = el(doc, "div");
    let _ = rows.set_attribute("id", "bg-legend-rows");
    let _ = rows.set_attribute("class", "legend-rows");
    // Click a key to highlight only that provenance's nodes (toggle). One
    // delegated listener on the container survives the innerHTML rebuilds.
    {
        let s = shared.clone();
        on(&rows, "click", move |ev| {
            let key = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .and_then(|e| e.closest(".legend-row").ok().flatten())
                .and_then(|row| row.get_attribute("data-prov"));
            let Some(prov) = key.as_deref().and_then(prov_from_key) else {
                return;
            };
            {
                let mut a = s.borrow_mut();
                a.legend_filter = if a.legend_filter == Some(prov) {
                    None
                } else {
                    Some(prov)
                };
            }
            set_legend(&s);
            graph_view::redraw(&s);
        });
    }
    let _ = p.append_child(&title);
    let _ = p.append_child(&rows);

    // Edge-type key (non-interactive): explains the colored solid/dashed lines on
    // the canvas. Edges have no single provenance to filter on, so these rows are a
    // plain key. Populated by `sync_chrome`; the whole section hides when the
    // projection has no edges.
    let edge_sec = el(doc, "div");
    let _ = edge_sec.set_attribute("id", "bg-edge-legend");
    let _ = edge_sec.set_attribute("class", "legend-section is-hidden");
    let edge_title = span(doc, "legend-title legend-title-edge", "EDGE TYPE");
    let edge_rows = el(doc, "div");
    let _ = edge_rows.set_attribute("id", "bg-edge-legend-rows");
    let _ = edge_rows.set_attribute("class", "legend-rows");
    let _ = edge_sec.append_child(&edge_title);
    let _ = edge_sec.append_child(&edge_rows);
    let _ = p.append_child(&edge_sec);
    p
}

/// The edge-type key rows (a line swatch — solid, or dashed for search-links —
/// plus label and share). Mirrors what `canvas2d` draws: `Link`/`Form` are solid,
/// `SearchLink` is dashed; color tracks `EdgeKind::color`. Non-interactive, and
/// only kinds actually present in the projection are listed. Pure, like
/// `legend_html`, so `sync_chrome` is the sole renderer.
fn edge_legend_html(k: &KindBreakdown) -> String {
    // (count, label, swatch class). Order mirrors the Sankey ribbon key.
    let rows: Vec<(u32, &str, &str)> = [
        (k.link, "Link", "edge-link"),
        (k.form, "Form", "edge-form"),
        (k.search_link, "Search-link", "edge-search"),
    ]
    .into_iter()
    .filter(|(c, ..)| *c > 0)
    .collect();
    let counts: Vec<u32> = rows.iter().map(|(c, ..)| *c).collect();
    let pcts = percentages(&counts);
    let mut html = String::new();
    for ((_, label, swatch), pct) in rows.iter().zip(pcts) {
        html.push_str(&format!(
            "<div class=\"legend-row is-key\">\
             <span class=\"edge-swatch {swatch}\"></span>\
             <span class=\"legend-label\">{label}</span>\
             <span class=\"legend-pct\">{pct}%</span></div>",
        ));
    }
    html
}

/// Map a legend row's `data-prov` key to its provenance (External = `start_page`).
fn prov_from_key(key: &str) -> Option<Provenance> {
    Some(match key {
        "search" => Provenance::SearchOrigin,
        "link" => Provenance::Link,
        "typed" => Provenance::TypedUrl,
        "bookmark" => Provenance::Bookmark,
        "form" => Provenance::Form,
        "external" => Provenance::Start,
        "other" => Provenance::Other,
        _ => return None,
    })
}

/// The legend rows as HTML buttons (`data-prov` + active/dim state). Pure so both
/// `sync_chrome` and the click handler render an identical list.
fn legend_html(b: &ProvBreakdown, filter: Option<Provenance>) -> String {
    // (count, label, dot color class, provenance, data-prov key). Reload folds
    // into Other; External (start_page) is its own category.
    let rows: Vec<(u32, &str, &str, Provenance, &str)> = [
        (
            b.search_origin,
            "Search",
            "dot-search",
            Provenance::SearchOrigin,
            "search",
        ),
        (b.link, "Link", "dot-link", Provenance::Link, "link"),
        (
            b.typed_url,
            "Typed URL",
            "dot-typed",
            Provenance::TypedUrl,
            "typed",
        ),
        (
            b.bookmark,
            "Bookmark",
            "dot-bookmark",
            Provenance::Bookmark,
            "bookmark",
        ),
        (b.form, "Form", "dot-form", Provenance::Form, "form"),
        (
            b.start,
            "External",
            "dot-external",
            Provenance::Start,
            "external",
        ),
        (
            b.other + b.reload,
            "Other",
            "dot-other",
            Provenance::Other,
            "other",
        ),
    ]
    .into_iter()
    // Omit empty categories: they'd sit at 0% and, when clicked, dim the whole
    // graph to nothing (genuine Other/Reload never produce nodes).
    .filter(|(c, ..)| *c > 0)
    .collect();
    let counts: Vec<u32> = rows.iter().map(|(c, ..)| *c).collect();
    let pcts = percentages(&counts);
    let mut html = String::new();
    for ((_, label, dot, prov, key), pct) in rows.iter().zip(pcts) {
        let state = match filter {
            Some(f) if f == *prov => " is-active",
            Some(_) => " is-dim",
            None => "",
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"legend-row{state}\" data-prov=\"{key}\">\
             <span class=\"dot {dot} {glyph}\"></span>\
             <span class=\"legend-label\">{label}</span>\
             <span class=\"legend-pct\">{pct}%</span></button>",
            glyph = prov.shape().css()
        ));
    }
    html
}

/// Rebuild just the legend rows from current state (used by the click/Esc paths;
/// `sync_chrome` inlines the same call while it already holds the borrow).
pub(super) fn set_legend(shared: &Shared) {
    let a = shared.borrow();
    let mut b = ProvBreakdown::default();
    for n in &a.proj.nodes {
        b.merge(&n.prov);
    }
    let html = legend_html(&b, a.legend_filter);
    if let Some(rows_el) = a.doc.get_element_by_id("bg-legend-rows") {
        rows_el.set_inner_html(&html);
    }
}

// ── 5. zoom toolbar (bottom-right) ───────────────────────────────────────────
pub(super) fn zoom_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "toolbar at-br");
    let zin = icon_btn(doc, "bg-zoom-in", "Zoom in", &icon("plus"));
    let zout = icon_btn(doc, "bg-zoom-out", "Zoom out", &icon("minus"));
    let fit = icon_btn(doc, "bg-fit", "Fit to screen", &icon("fit"));
    let lock = icon_btn(doc, "bg-lock", "Lock layout", &icon("unlock"));
    {
        let s = shared.clone();
        on(&zin, "click", move |_| graph_view::zoom(&s, 1.2));
    }
    {
        let s = shared.clone();
        on(&zout, "click", move |_| graph_view::zoom(&s, 1.0 / 1.2));
    }
    {
        let s = shared.clone();
        on(&fit, "click", move |_| graph_view::fit_view(&s));
    }
    {
        let s = shared.clone();
        on(&lock, "click", move |_| {
            s.borrow_mut().locked ^= true;
            persist_ui_prefs(&s);
            sync_chrome(&s);
        });
    }
    for b in [&zin, &zout, &fit, &lock] {
        let _ = p.append_child(b);
    }
    p
}

// ── 6. search + filter chips (bottom-left) ───────────────────────────────────
pub(super) fn search_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "search at-bl");

    let sbox = el(doc, "div");
    let _ = sbox.set_attribute("class", "searchbox");
    let mag = el(doc, "span");
    let _ = mag.set_attribute("class", "search-ico");
    mag.set_inner_html(&icon("search"));
    let input = el(doc, "input");
    let _ = input.set_attribute("type", "text");
    let _ = input.set_attribute("id", "bg-search");
    let _ = input.set_attribute("class", "search-input");
    let _ = input.set_attribute("placeholder", "Search hosts…");
    let kbd = span(doc, "kbd", "⌘K");
    let _ = sbox.append_child(&mag);
    let _ = sbox.append_child(&input);
    let _ = sbox.append_child(&kbd);
    {
        let s = shared.clone();
        on(&input, "keydown", move |ev| {
            let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
                return;
            };
            let doc = s.borrow().doc.clone();
            if ke.key() != "Enter" {
                set_text(&doc, "bg-search-hint", ""); // typing clears the hint
                return;
            }
            let q = ke
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value().trim().to_lowercase())
                .unwrap_or_default();
            if q.is_empty() {
                set_text(&doc, "bg-search-hint", "");
                return;
            }
            let hit = s
                .borrow()
                .proj
                .nodes
                .iter()
                .find(|n| n.key.to_lowercase().contains(&q))
                .map(|n| n.key.clone());
            match hit {
                Some(k) => {
                    set_text(&doc, "bg-search-hint", "");
                    s.borrow_mut().view = View::Graph;
                    super::focus_and_animate(&s, Some(k));
                }
                None => set_text(&doc, "bg-search-hint", &format!("No host matches “{q}”")),
            }
        });
    }

    let chips = el(doc, "div");
    let _ = chips.set_attribute("class", "chips");
    // Hostname / Domain grouping — a segmented slide-select matching the Sankey's.
    let (gran_seg, gran_btns) = seg(
        doc,
        "ghost",
        &[("hostname", "Hostname"), ("registrable", "Domain")],
    );
    let _ = gran_seg.set_attribute("id", "seg-gran");
    let _ = gran_seg.set_attribute("title", "Toggle hostname / registrable-domain grouping");
    let cur_gran = if shared.borrow().gran == Granularity::Registrable {
        "registrable"
    } else {
        "hostname"
    };
    for (val, btn) in &gran_btns {
        if val.as_str() == cur_gran {
            let _ = btn.set_attribute("class", "active");
        }
        let gran = if val.as_str() == "registrable" {
            Granularity::Registrable
        } else {
            Granularity::Hostname
        };
        let s = shared.clone();
        on(btn, "click", move |_| {
            s.borrow_mut().gran = gran;
            recompute_projection(&s);
            persist_positions(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
        });
    }
    let mv = el(doc, "select");
    let _ = mv.set_attribute("class", "chip chip-select");
    let _ = mv.set_attribute("id", "chip-minvisits");
    let _ = mv.set_attribute("title", "Only show sites with at least this many visits");
    // Options are filled in by `sync_chrome::populate_min_visits`, which adapts the
    // thresholds to the current visit volume; this placeholder avoids an empty
    // control before the first render.
    {
        let opt = el(doc, "option");
        let _ = opt.set_attribute("value", "1");
        opt.set_text_content(Some("All sites"));
        let _ = mv.append_child(&opt);
    }
    let hubs = chip(doc, "chip-hubs", "Hide search hubs");
    let _ = hubs.set_attribute("title", "Collapse search-engine origin nodes");
    let iso = chip(doc, "chip-isolated", "Hide singletons");
    let _ = iso.set_attribute(
        "title",
        "Hide sites that link to nothing (typed/bookmark/search singletons)",
    );
    let _ = chips.append_child(&gran_seg);
    let _ = chips.append_child(&mv);
    let _ = chips.append_child(&hubs);
    let _ = chips.append_child(&iso);
    {
        let s = shared.clone();
        on(&mv, "change", move |ev| {
            let v = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlSelectElement>().ok())
                .map(|sel| sel.value())
                .unwrap_or_default();
            s.borrow_mut().filters.min_visits = v.parse().unwrap_or(0);
            recompute_projection(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
        });
    }
    {
        let s = shared.clone();
        on(&hubs, "click", move |_| {
            {
                let mut a = s.borrow_mut();
                a.filters.hide_search_hubs ^= true;
            }
            recompute_projection(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
        });
    }
    {
        let s = shared.clone();
        on(&iso, "click", move |_| {
            {
                let mut a = s.borrow_mut();
                a.filters.hide_isolated ^= true;
            }
            recompute_projection(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
        });
    }

    let _ = p.append_child(&sbox);
    let hint = span(doc, "search-hint", "");
    let _ = hint.set_attribute("id", "bg-search-hint");
    let _ = p.append_child(&hint);
    let _ = p.append_child(&chips);
    p
}

// ── 7. readout + spectrum (bottom-center) ────────────────────────────────────
pub(super) fn readout_panel(doc: &Document) -> Element {
    let p = panel(doc, "readout at-bc");
    let nodes = span(doc, "metric", "0 nodes");
    let _ = nodes.set_attribute("id", "bg-count-nodes");
    let edges = span(doc, "metric", "0 edges");
    let _ = edges.set_attribute("id", "bg-count-edges");
    let rule1 = el(doc, "div");
    let _ = rule1.set_attribute("class", "vrule");
    let _ = p.append_child(&nodes);
    let _ = p.append_child(&rule1);
    let _ = p.append_child(&edges);
    p
}

// ── dynamic chrome refresh ────────────────────────────────────────────────────

/// Update every data-driven control to the current state: active segments, node/
/// edge counts, the provenance legend, chip labels, and the REC / lock glyphs.
/// Called by `rerender` after the projection is recomputed.
pub(crate) fn sync_chrome(shared: &Shared) {
    let a = shared.borrow();
    let doc = a.doc.clone();

    if let Some(body) = doc.get_element_by_id("bg-body") {
        body.set_class_name(if a.view == View::Graph {
            "bg-body mode-graph"
        } else {
            "bg-body mode-data"
        });
    }

    // Per-view class on the root drives which supporting panels are shown (some
    // chrome only makes sense for the graph canvas — see dashboard.css).
    if let Some(app) = doc.get_element_by_id("app") {
        app.set_class_name(match a.view {
            View::Graph => "view-graph",
            View::Sankey => "view-sankey",
            View::Tables => "view-tables",
            View::Raw => "view-raw",
        });
    }

    // active segments
    let vv = match a.view {
        View::Graph => "graph",
        View::Sankey => "sankey",
        View::Tables => "tables",
        View::Raw => "",
    };
    for v in ["graph", "sankey", "tables"] {
        toggle_active(&doc, &format!("vw-{v}"), v == vv);
    }
    let rv = match a.time_range {
        TimeRange::Session => "session",
        TimeRange::Day => "day",
        TimeRange::Week => "week",
        TimeRange::Month => "month",
        TimeRange::Year => "year",
    };
    for v in ["session", "day", "week", "month", "year"] {
        toggle_active(&doc, &format!("rng-{v}"), v == rv);
    }

    // time navigation: window label, ‹/› enabled state, "Latest ↩" chip (§F6)
    sync_time_nav(&doc, &a);

    // counts
    set_text(
        &doc,
        "bg-count-nodes",
        &plural(a.proj.nodes.len() as u64, "node"),
    );
    set_text(
        &doc,
        "bg-count-edges",
        &plural(a.proj.edges.len() as u64, "edge"),
    );

    // provenance legend (percentages over the visible projection)
    let mut b = ProvBreakdown::default();
    for n in &a.proj.nodes {
        b.merge(&n.prov);
    }
    if let Some(rows_el) = doc.get_element_by_id("bg-legend-rows") {
        rows_el.set_inner_html(&legend_html(&b, a.legend_filter));
    }

    // edge-type key (line swatches; only kinds present in the projection). Hide the
    // whole section when there are no edges so it never shows an empty title.
    let mut k = KindBreakdown::default();
    for e in &a.proj.edges {
        k.merge(&e.kinds);
    }
    let edge_html = edge_legend_html(&k);
    if let Some(rows_el) = doc.get_element_by_id("bg-edge-legend-rows") {
        rows_el.set_inner_html(&edge_html);
    }
    if let Some(sec) = doc.get_element_by_id("bg-edge-legend") {
        sec.set_class_name(if edge_html.is_empty() {
            "legend-section is-hidden"
        } else {
            "legend-section"
        });
    }

    // chips: reflect the active granularity on the segmented slide-select
    if let Some(seg_el) = doc.get_element_by_id("seg-gran") {
        let registrable = a.gran == Granularity::Registrable;
        for (val, on) in [("hostname", !registrable), ("registrable", registrable)] {
            if let Ok(Some(btn)) = seg_el.query_selector(&format!("[data-seg=\"{val}\"]")) {
                let _ = btn.set_attribute("class", if on { "active" } else { "" });
            }
        }
    }
    toggle_active(&doc, "chip-hubs", a.filters.hide_search_hubs);
    toggle_active(&doc, "chip-isolated", a.filters.hide_isolated);
    populate_min_visits(&doc, &a);

    // REC indicator
    if let Some(rec) = doc.get_element_by_id("bg-rec") {
        rec.set_class_name(if a.paused { "rec paused" } else { "rec" });
        let _ = rec.set_attribute(
            "aria-label",
            if a.paused {
                "Resume recording"
            } else {
                "Pause recording"
            },
        );
    }
    set_text(
        &doc,
        "bg-rec-label",
        if a.paused { "PAUSED" } else { "REC" },
    );

    // Sample-data chip: shown only while the onboarding demo dataset is loaded (§F4).
    if let Some(chip) = doc.get_element_by_id("bg-demochip") {
        chip.set_class_name(if a.demo_data {
            "demo-chip show"
        } else {
            "demo-chip"
        });
    }

    // lock toggle
    if let Some(lock) = doc.get_element_by_id("bg-lock") {
        lock.set_class_name(if a.locked {
            "iconbtn active"
        } else {
            "iconbtn"
        });
        lock.set_inner_html(&icon(if a.locked { "lock" } else { "unlock" }));
    }

    // drill-down focus chip — only meaningful on the graph, and only while focused
    if let Some(chip) = doc.get_element_by_id("bg-focuschip") {
        match (a.view, a.focus.as_deref()) {
            (View::Graph, Some(host)) => {
                chip.set_class_name("panel focuschip at-fc show");
                set_text(&doc, "bg-focus-label", &format!("Focused: {host}"));
            }
            _ => chip.set_class_name("panel focuschip at-fc"),
        }
    }
}

/// §F6 time navigation: fill the window-bounds label, toggle the ‹/› enabled
/// states at the ends of the timeline, and show the "Latest ↩" chip only while an
/// anchor is set. Data-driven from the current range + anchor, so it stays in
/// step with every re-render.
fn sync_time_nav(doc: &Document, a: &super::App) {
    // Window-bounds label. Session shows the displayed session's time range; the
    // calendar ranges show their UTC day-window bounds.
    let label = match a.time_range {
        TimeRange::Session => super::displayed_session(a)
            .map(|s| super::session_when(s.start_ts, s.end_ts))
            .unwrap_or_default(),
        range => crate::project::window_day_bounds(&a.buckets, range, super::anchor_end_day(a))
            .map(|(start, end)| crate::project::window_label(range, start, end))
            .unwrap_or_default(),
    };
    set_text(doc, "rng-window-label", &label);

    // "Latest ↩" chip: visible only when anchored to a past window.
    if let Some(chip) = doc.get_element_by_id("rng-latest") {
        chip.set_class_name(if a.anchor.is_some() {
            "rng-latest show"
        } else {
            "rng-latest"
        });
    }

    // ‹ / › enabled states.
    let (back, fwd) = match a.time_range {
        TimeRange::Session => {
            let order = crate::project::session_order(&a.sessions);
            let cur = match a.anchor {
                Some(super::Anchor::Session(id)) => Some(id),
                _ => None,
            };
            (
                matches!(
                    crate::project::session_back(&order, cur),
                    crate::project::SessionStep::To(_)
                ),
                !matches!(
                    crate::project::session_forward(&order, cur),
                    crate::project::SessionStep::Blocked
                ),
            )
        }
        range => {
            let cur = match a.anchor {
                Some(super::Anchor::Day(d)) => Some(d),
                _ => None,
            };
            (
                crate::project::can_step_back(&a.buckets, range, cur),
                crate::project::can_step_forward(&a.buckets, range, cur),
            )
        }
    };
    set_disabled(doc, "rng-prev", !back);
    set_disabled(doc, "rng-next", !fwd);
}

/// Toggle the `disabled` attribute on a control by id.
fn set_disabled(doc: &Document, id: &str, disabled: bool) {
    if let Some(e) = doc.get_element_by_id(id) {
        if disabled {
            let _ = e.set_attribute("disabled", "disabled");
        } else {
            let _ = e.remove_attribute("disabled");
        }
    }
}

/// Rebuild the min-visits dropdown so its thresholds track the current visit
/// volume — a "nice" 1-2-5 ladder up to the busiest site (see
/// [`crate::project::visit_thresholds`]). Only rewrites the options when the
/// ladder actually changes (guarded by a `data-ths` signature), and always
/// reflects the active selection. The distribution is taken over the currently
/// *visible* sites (every filter but the min-visits cut), so it adapts to the
/// granularity and hub toggles.
fn populate_min_visits(doc: &Document, a: &super::App) {
    let Some(sel) = doc.get_element_by_id("chip-minvisits") else {
        return;
    };
    let Ok(sel) = sel.dyn_into::<HtmlSelectElement>() else {
        return;
    };

    let window = crate::project::select_window(&a.buckets, a.time_range, super::anchor_end_day(a));
    let probe = crate::project::Filters {
        min_visits: 0,
        hide_search_hubs: a.filters.hide_search_hubs,
        hide_isolated: a.filters.hide_isolated,
        provenance_in: a.filters.provenance_in.clone(),
    };
    let max = crate::project::project(&window, a.gran, &probe)
        .nodes
        .iter()
        .map(|n| n.visits)
        .max()
        .unwrap_or(0);

    // `min_visits` 0 and 1 are equivalent (every site has ≥1 visit); show "all".
    let cur = a.filters.min_visits.max(1);
    let mut ths = crate::project::visit_thresholds(max);
    if !ths.contains(&cur) {
        ths.push(cur); // keep the user's explicit choice selectable
        ths.sort_unstable();
    }

    let sig = ths
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");
    if sel.get_attribute("data-ths").as_deref() != Some(sig.as_str()) {
        sel.set_inner_html("");
        for t in &ths {
            let opt = el(doc, "option");
            let _ = opt.set_attribute("value", &t.to_string());
            opt.set_text_content(Some(&if *t <= 1 {
                "All sites".to_string()
            } else {
                format!("Min {t} visits")
            }));
            let _ = sel.append_child(&opt);
        }
        let _ = sel.set_attribute("data-ths", &sig);
    }
    sel.set_value(&cur.to_string());
}

/// Integer percentages that sum to exactly 100 (largest-remainder apportionment),
/// so the provenance legend never reads as 99% or 101% from independent rounding.
fn percentages(counts: &[u32]) -> Vec<u32> {
    let total: u64 = counts.iter().map(|&c| c as u64).sum();
    if total == 0 {
        return vec![0; counts.len()];
    }
    let mut pct: Vec<u32> = counts
        .iter()
        .map(|&c| (c as u64 * 100 / total) as u32)
        .collect();
    let assigned: u32 = pct.iter().sum();
    let mut remaining = 100u32.saturating_sub(assigned);
    // Hand the leftover units to the largest fractional remainders (ties by index).
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|&a, &b| {
        let ra = (counts[a] as u64 * 100) % total;
        let rb = (counts[b] as u64 * 100) % total;
        rb.cmp(&ra).then(a.cmp(&b))
    });
    for &i in &order {
        if remaining == 0 {
            break;
        }
        pct[i] += 1;
        remaining -= 1;
    }
    pct
}

/// Add/remove the `active` class token while preserving any base classes, and
/// mirror the state into `aria-pressed` so assistive tech announces which range /
/// view / filter is currently selected.
fn toggle_active(doc: &Document, id: &str, on: bool) {
    if let Some(e) = doc.get_element_by_id(id) {
        let mut classes: Vec<String> = e
            .class_name()
            .split_whitespace()
            .filter(|c| *c != "active")
            .map(|s| s.to_string())
            .collect();
        if on {
            classes.push("active".into());
        }
        e.set_class_name(&classes.join(" "));
        let _ = e.set_attribute("aria-pressed", if on { "true" } else { "false" });
    }
}
