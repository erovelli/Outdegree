//! Dashboard shell: the full-bleed canvas plus the nine floating glass control
//! clusters from the design handoff, and their wiring (§7.7).
//!
//! The canvas is the app; every control floats over it as a translucent panel
//! pinned to an edge or corner. Chrome is strictly monochrome — the only color
//! belongs to the data spectrum drawn on the canvas.

use super::filters::{chip, icon, icon_btn, menu_btn, menu_toggle, panel, seg};
use super::{
    el, graph_view, on, persist_positions, persist_ui_prefs, plural, recompute_projection,
    reload_and_rerender, reload_buckets, rerender, Shared, View,
};
use crate::model::{Granularity, KindBreakdown, ProvBreakdown, Provenance};
use crate::project::TimeRange;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, HtmlSelectElement, KeyboardEvent};

/// Build the body layer (canvas/tables mount point) + all floating chrome.
pub(crate) fn build_shell(shared: &Shared) -> Result<(), JsValue> {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    root.set_inner_html("");

    // The view body: graph canvas (mode-graph) or table content (mode-data).
    let body = el(&doc, "div");
    let _ = body.set_attribute("id", "bg-body");
    let _ = body.set_attribute("class", "bg-body mode-graph");
    let _ = root.append_child(&body);
    // Delegated: clicking a host cell in any table jumps to the Graph view focused
    // on that host. One listener on the persistent #bg-body survives the innerHTML
    // rebuilds the table views do.
    {
        let s = shared.clone();
        on(&body, "click", move |ev| {
            let host = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .and_then(|e| e.closest("[data-host]").ok().flatten())
                .and_then(|h| h.get_attribute("data-host"));
            let Some(host) = host else { return };
            {
                let mut a = s.borrow_mut();
                if !a.proj.nodes.iter().any(|n| n.key == host) {
                    return; // host filtered out of the current projection
                }
                a.view = View::Graph;
            }
            // No canvas is mounted in table mode yet, so focus_and_animate falls
            // back to a full re-render that builds the graph already focused.
            super::focus_and_animate(&s, Some(host));
        });
    }

    let _ = root.append_child(&brand_panel(&doc, shared));
    let _ = root.append_child(&range_panel(&doc, shared));
    let _ = root.append_child(&view_panel(&doc, shared));
    let _ = root.append_child(&legend_panel(&doc, shared));
    let _ = root.append_child(&zoom_panel(&doc, shared));
    let _ = root.append_child(&search_panel(&doc, shared));
    let _ = root.append_child(&readout_panel(&doc));
    let _ = root.append_child(&focus_panel(&doc, shared));
    let _ = root.append_child(&nudge_panel(&doc, shared));
    let _ = root.append_child(&settings_popover(&doc, shared));
    install_palette_shortcut(shared);
    install_popover_dismiss(shared);

    Ok(())
}

// ── drill-down focus chip (top-center, shown only while a node is focused) ─────
fn focus_panel(doc: &Document, shared: &Shared) -> Element {
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

fn span(doc: &Document, class: &str, text: &str) -> Element {
    let s = el(doc, "span");
    let _ = s.set_attribute("class", class);
    s.set_text_content(Some(text));
    s
}

// ── 1. brand + REC (top-left) ────────────────────────────────────────────────
fn brand_panel(doc: &Document, shared: &Shared) -> Element {
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
    p
}

// ── 2. range (top-center) ────────────────────────────────────────────────────
fn range_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "seg-panel at-tc");
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
            s.borrow_mut().time_range = range;
            recompute_projection(&s);
            persist_positions(&s);
            persist_ui_prefs(&s);
            let _ = rerender(&s);
            // The Session range needs buckets scoped to the latest session's
            // events; load them (async) so it isn't just the latest UTC day.
            if range == TimeRange::Session {
                super::refresh_session_buckets(&s);
            }
        });
    }
    let _ = p.append_child(&wrap);
    p
}

// ── 3. view + settings gear (top-right) ──────────────────────────────────────
fn view_panel(doc: &Document, shared: &Shared) -> Element {
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
fn legend_panel(doc: &Document, shared: &Shared) -> Element {
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
fn set_legend(shared: &Shared) {
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
fn zoom_panel(doc: &Document, shared: &Shared) -> Element {
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
fn search_panel(doc: &Document, shared: &Shared) -> Element {
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
fn readout_panel(doc: &Document) -> Element {
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

// ── 8. settings popover (hidden until the gear is clicked) ────────────────────
fn settings_popover(doc: &Document, shared: &Shared) -> Element {
    let pop = panel(doc, "popover at-pop");
    let _ = pop.set_attribute("id", "bg-settings");

    // Reflect the restored in-app-navigations mode (§7.7) so the checkbox matches
    // the graph it produced on first open, rather than a stale default.
    let (spa_row, spa_input) = menu_toggle(doc, "In-app navigations", shared.borrow().spa_mode);
    {
        let s = shared.clone();
        on(&spa_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().spa_mode = c;
            persist_ui_prefs(&s);
            reload_buckets(&s);
        });
    }

    // Opt-in, default off: surface search terms parsed from already-captured
    // result URLs. Reflects the persisted choice and writes it back on change.
    let (search_row, search_input) =
        menu_toggle(doc, "Show search terms", shared.borrow().show_searches);
    {
        let s = shared.clone();
        on(&search_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().show_searches = c;
            crate::bridge::storage_local_set("showSearches", if c { "true" } else { "false" });
            super::reload_searches(&s);
        });
    }

    let raw = menu_btn(doc, "Raw events");
    {
        let s = shared.clone();
        on(&raw, "click", move |_| {
            s.borrow_mut().view = View::Raw;
            close_popover(&s);
            let _ = rerender(&s);
        });
    }

    let saved_views = menu_btn(doc, "Saved views…");
    {
        let s = shared.clone();
        on(&saved_views, "click", move |_| {
            close_popover(&s);
            open_saved_views_dialog(&s);
        });
    }

    let export = menu_btn(doc, "Export JSON");
    {
        let s = shared.clone();
        on(&export, "click", move |_| {
            close_popover(&s);
            run_json_export(&s);
        });
    }

    let export_csv = menu_btn(doc, "Export tables (CSV)");
    {
        let s = shared.clone();
        on(&export_csv, "click", move |_| {
            close_popover(&s);
            let csv = super::tables::tables_csv(&s.borrow());
            crate::bridge::download_text("outdegree-tables.csv", "text/csv", &csv);
        });
    }

    let export_png = menu_btn(doc, "Export graph (PNG)");
    {
        let s = shared.clone();
        on(&export_png, "click", move |_| {
            close_popover(&s);
            super::graph_view::export_png(&s);
        });
    }

    let export_svg = menu_btn(doc, "Export graph (SVG)");
    {
        let s = shared.clone();
        on(&export_svg, "click", move |_| {
            close_popover(&s);
            super::graph_view::export_svg(&s);
        });
    }

    let import = menu_btn(doc, "Import JSON");
    {
        let s = shared.clone();
        on(&import, "click", move |_| {
            close_popover(&s);
            let doc = s.borrow().doc.clone();
            let Ok(el) = doc.create_element("input") else {
                return;
            };
            let _ = el.set_attribute("type", "file");
            let _ = el.set_attribute("accept", "application/json,.json");
            let Ok(inp) = el.dyn_into::<HtmlInputElement>() else {
                return;
            };
            let s2 = s.clone();
            let picker = inp.clone();
            on(&inp, "change", move |_| {
                let Some(file) = picker.files().and_then(|f| f.get(0)) else {
                    return;
                };
                let s3 = s2.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Read the picked file first (harmless — it touches no store), then
                    // confirm before replacing anything, so Cancel truly aborts (§8).
                    let json = match wasm_bindgen_futures::JsFuture::from(file.text()).await {
                        Ok(v) => v.as_string().unwrap_or_default(),
                        Err(e) => return super::log_err(&e),
                    };
                    let db = s3.borrow().db.clone();
                    let events =
                        db.count_events().await.unwrap_or(0) + db.count_spa().await.unwrap_or(0);
                    let msg = format!(
                        "Importing replaces your current data ({}). This can't be undone — \
                         consider Export JSON first.",
                        plural(events as u64, "event"),
                    );
                    let s4 = s3.clone();
                    confirm_dialog(
                        &s3,
                        "Import JSON",
                        &msg,
                        None,
                        "Import",
                        true,
                        None,
                        move |_| {
                            let json = json.clone();
                            let s5 = s4.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                let db = s5.borrow().db.clone();
                                // Replace every store, then re-derive from the imported
                                // events so the view is consistent even if the file only
                                // carried `events`.
                                if let Err(e) = db.import_json(&json).await {
                                    return super::log_err(&e);
                                }
                                if let Err(e) = db.reset_derivation().await {
                                    return super::log_err(&e);
                                }
                                reload_and_rerender(&s5);
                            });
                            true
                        },
                    );
                });
            });
            inp.click();
        });
    }

    let forget = menu_btn(doc, "Forget domain…");
    {
        let s = shared.clone();
        on(&forget, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Forget a domain",
                "Permanently remove every stored record for this host or domain, then rebuild. \
                 This can't be undone.",
                Some("host or domain, e.g. example.com"),
                "Forget",
                true,
                None,
                move |val| {
                    let domain = val.unwrap_or_default().trim().to_string();
                    if domain.is_empty() {
                        set_text(
                            &s2.borrow().doc,
                            "bg-modal-error",
                            "Enter a host or domain.",
                        );
                        return false;
                    }
                    let db = s2.borrow().db.clone();
                    let s3 = s2.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = db.forget_domain(&domain).await {
                            return super::log_err(&e);
                        }
                        reload_and_rerender(&s3);
                    });
                    true
                },
            );
        });
    }

    let delete = menu_btn(doc, "Delete last N days…");
    {
        let s = shared.clone();
        on(&delete, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Delete recent history",
                "Permanently remove all records from the last N days, then rebuild. \
                 This can't be undone.",
                Some("number of days, e.g. 7"),
                "Delete",
                true,
                None,
                move |val| {
                    // Validate: a whole number of days in a sane range — so a stray
                    // "99999" can't silently wipe everything and "-3" can't no-op.
                    match val.unwrap_or_default().trim().parse::<u32>() {
                        Ok(days) if (1..=3650).contains(&days) => {
                            let now = js_sys::Date::now();
                            let from = now - days as f64 * 86_400_000.0;
                            let db = s2.borrow().db.clone();
                            let s3 = s2.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Err(e) = db.delete_range(from, now).await {
                                    return super::log_err(&e);
                                }
                                reload_and_rerender(&s3);
                            });
                            true
                        }
                        _ => {
                            set_text(
                                &s2.borrow().doc,
                                "bg-modal-error",
                                "Enter a whole number of days between 1 and 3650.",
                            );
                            false
                        }
                    }
                },
            );
        });
    }

    // The nuclear option: wipe every IndexedDB store. Gated behind typing DELETE
    // so it can't be triggered by a stray click (§8). Preferences survive.
    let delete_all = menu_btn(doc, "Delete all data…");
    {
        let s = shared.clone();
        on(&delete_all, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Delete all data",
                "Permanently erase every stored navigation, rollup, session, and saved \
                 layout. Your preferences (pause, view settings, saved views) are kept. \
                 This can't be undone. Type DELETE to confirm.",
                Some("type DELETE"),
                "Delete everything",
                true,
                Some("DELETE"),
                move |val| {
                    // The typed-phrase gate already guards the button; re-check here so
                    // the action itself is never reachable without the exact word.
                    if val.unwrap_or_default().trim() != "DELETE" {
                        set_text(
                            &s2.borrow().doc,
                            "bg-modal-error",
                            "Type DELETE to confirm.",
                        );
                        return false;
                    }
                    let db = s2.borrow().db.clone();
                    let s3 = s2.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = db.clear_all().await {
                            return super::log_err(&e);
                        }
                        // Everything is gone; re-derive (a no-op over an empty log) and
                        // rerender to land on the empty state that captures anew.
                        reload_and_rerender(&s3);
                    });
                    true
                },
            );
        });
    }

    // Recovery: clear the derived cache + cursor and re-derive everything from the
    // raw event log (fixes a derivation cursor that has drifted past the events).
    let rebuild = menu_btn(doc, "Rebuild from raw events");
    {
        let s = shared.clone();
        on(&rebuild, "click", move |_| {
            close_popover(&s);
            let db = s.borrow().db.clone();
            let s2 = s.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = db.reset_derivation().await {
                    return super::log_err(&e);
                }
                reload_and_rerender(&s2);
            });
        });
    }

    // Read-only storage usage readout atop the Data section — event/rollup/session
    // counts plus an approximate byte figure from navigator.storage.estimate()
    // (a local API, CSP-safe). Filled/refreshed each time the popover opens.
    let storage_line = el(doc, "div");
    let _ = storage_line.set_attribute("class", "menu-readout");
    let _ = storage_line.set_attribute("id", "bg-storage-line");
    storage_line.set_text_content(Some("Storage · …"));

    let _ = pop.append_child(&spa_row);
    let _ = pop.append_child(&search_row);
    let sep = el(doc, "div");
    let _ = sep.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep);
    let _ = pop.append_child(&raw);
    let _ = pop.append_child(&saved_views);
    let sep2 = el(doc, "div");
    let _ = sep2.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep2);
    let _ = pop.append_child(&storage_line);
    let _ = pop.append_child(&export);
    let _ = pop.append_child(&export_csv);
    let _ = pop.append_child(&export_png);
    let _ = pop.append_child(&export_svg);
    let _ = pop.append_child(&import);
    let _ = pop.append_child(&forget);
    let _ = pop.append_child(&delete);
    let _ = pop.append_child(&delete_all);
    let _ = pop.append_child(&rebuild);
    pop
}

fn close_popover(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    if let Some(pop) = doc.get_element_by_id("bg-settings") {
        pop.set_class_name("panel popover at-pop");
    }
    if let Some(g) = doc.get_element_by_id("bg-gear") {
        let _ = g.set_attribute("aria-expanded", "false");
    }
}

// ── data stewardship: storage readout · export · backup nudge (§8) ────────────

/// Run the JSON export: download the file, stamp `lastExportTs` into `meta` so the
/// backup nudge resets, and hide the nudge chip. Shared by the settings menu's
/// "Export JSON" and the backup nudge's "Export now" — a *download*, never an
/// upload (§7.8 / §12.1). DO NOT add a network sink here.
fn run_json_export(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match db.export_json().await {
            Ok(json) => {
                crate::bridge::download_json("outdegree-export.json", &json);
                // Stamp the backup so the nudge decision resets (best-effort).
                let _ = db.write_last_export_ts(js_sys::Date::now()).await;
                hide_nudge(&s);
            }
            Err(e) => super::log_err(&e),
        }
    });
}

/// The backup-nudge chip (near the settings gear, top-right): a dismissible glass
/// chip nudging a local backup, with "Export now" / "Snooze" actions (§8.4). Built
/// hidden; [`evaluate_backup_nudge`] reveals it only when the pure decision says so.
fn nudge_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "nudge at-nudge");
    let _ = p.set_attribute("id", "bg-nudge");
    let _ = p.set_attribute("role", "status");

    let msg = span(
        doc,
        "nudge-msg",
        "It's been a while since your last backup.",
    );
    let _ = p.append_child(&msg);

    let actions = el(doc, "div");
    let _ = actions.set_attribute("class", "nudge-actions");

    let export_now = el(doc, "button");
    let _ = export_now.set_attribute("type", "button");
    let _ = export_now.set_attribute("class", "nudge-btn nudge-primary");
    export_now.set_text_content(Some("Export now"));
    {
        let s = shared.clone();
        on(&export_now, "click", move |_| run_json_export(&s));
    }

    let snooze = el(doc, "button");
    let _ = snooze.set_attribute("type", "button");
    let _ = snooze.set_attribute("class", "nudge-btn");
    snooze.set_text_content(Some("Snooze"));
    {
        let s = shared.clone();
        on(&snooze, "click", move |_| snooze_nudge(&s));
    }

    let dismiss = el(doc, "button");
    let _ = dismiss.set_attribute("type", "button");
    let _ = dismiss.set_attribute("class", "nudge-x");
    let _ = dismiss.set_attribute("aria-label", "Dismiss");
    dismiss.set_text_content(Some("✕"));
    {
        // A bare dismiss hides it for this session only (no persistence) — a
        // lighter touch than Snooze, which suppresses it for 30 days.
        let s = shared.clone();
        on(&dismiss, "click", move |_| hide_nudge(&s));
    }

    let _ = actions.append_child(&export_now);
    let _ = actions.append_child(&snooze);
    let _ = actions.append_child(&dismiss);
    let _ = p.append_child(&actions);
    p
}

fn show_nudge(shared: &Shared) {
    if let Some(el) = shared.borrow().doc.get_element_by_id("bg-nudge") {
        el.set_class_name("panel nudge at-nudge show");
    }
}

fn hide_nudge(shared: &Shared) {
    if let Some(el) = shared.borrow().doc.get_element_by_id("bg-nudge") {
        el.set_class_name("panel nudge at-nudge");
    }
}

/// "Snooze": suppress the backup nudge for 30 days by writing `nudgeSnoozedUntil`
/// to `meta`, then hide the chip (§8.4).
fn snooze_nudge(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let until = js_sys::Date::now() + crate::stewardship::SNOOZE_MS;
        let _ = db.write_nudge_snoozed_until(until).await;
        hide_nudge(&s);
    });
}

/// Evaluate the backup nudge on dashboard load and reveal the chip only when the
/// pure decision ([`crate::stewardship::nudge_state`]) says [`Nudge::Show`] (§8.4):
/// enough real events, no recent backup, and no active snooze. The event-count
/// gate keeps it off the empty/no-data state; it never blocks interaction.
pub(crate) fn evaluate_backup_nudge(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events =
            db.count_events().await.unwrap_or(0) as u64 + db.count_spa().await.unwrap_or(0) as u64;
        let last_export = db.read_last_export_ts().await.unwrap_or(None);
        let snoozed = db.read_nudge_snoozed_until().await.unwrap_or(None);
        let now = js_sys::Date::now();
        if crate::stewardship::nudge_state(events, last_export, snoozed, now)
            == crate::stewardship::Nudge::Show
        {
            show_nudge(&s);
        } else {
            hide_nudge(&s);
        }
    });
}

/// Refresh the settings "Storage" readout: event/rollup/session record counts plus
/// an approximate byte figure from `navigator.storage.estimate()` (§8.1). When the
/// estimate is unavailable or rejects, the byte suffix is simply omitted.
fn refresh_storage_readout(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events = db.count_events().await.unwrap_or(0) as u64;
        let spa = db.count_spa().await.unwrap_or(0) as u64;
        let rollups = db.count_rollup_days().await.unwrap_or(0) as u64;
        let sessions = db.count_sessions().await.unwrap_or(0) as u64;
        let mut text = format!(
            "Storage · {} · {} · {}",
            plural(events + spa, "event"),
            plural(rollups, "day rollup"),
            plural(sessions, "session"),
        );
        if let Some(bytes) = estimate_usage_bytes().await {
            text.push_str(&format!(" · ~{}", crate::stewardship::human_bytes(bytes)));
        }
        set_text(&s.borrow().doc, "bg-storage-line", &text);
    });
}

/// Approximate bytes used by the origin, via `navigator.storage.estimate()` — a
/// local StorageManager API (no network). `None` if the API is missing or the
/// estimate rejects, so the readout falls back to counts only (§8.1).
async fn estimate_usage_bytes() -> Option<u64> {
    let storage = web_sys::window()?.navigator().storage();
    let promise = storage.estimate().ok()?;
    let estimate = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    let estimate: web_sys::StorageEstimate = estimate.unchecked_into();
    estimate.get_usage().map(|u| u as u64)
}

/// ⌘K / Ctrl-K focuses the host search box (command-palette affordance); Esc
/// closes the settings menu and exits any drill-down focus.
fn install_palette_shortcut(shared: &Shared) {
    let Some(win) = web_sys::window() else { return };
    let s = shared.clone();
    on(win.as_ref(), "keydown", move |ev| {
        let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
            return;
        };
        if ke.key() == "Escape" {
            // A modal takes priority: dismiss it and stop (don't also clear focus).
            if let Some(m) = s.borrow().doc.get_element_by_id("bg-modal") {
                m.remove();
                return;
            }
            close_popover(&s);
            let cleared = {
                let mut a = s.borrow_mut();
                a.legend_filter.take().is_some()
            };
            if s.borrow().focus.is_some() {
                super::focus_and_animate(&s, None); // re-renders: rebuilds legend + canvas
            } else if cleared {
                set_legend(&s);
                graph_view::redraw(&s);
            }
            return;
        }
        if (ke.meta_key() || ke.ctrl_key()) && ke.key().eq_ignore_ascii_case("k") {
            ke.prevent_default();
            if let Some(inp) = s.borrow().doc.get_element_by_id("bg-search") {
                if let Ok(h) = inp.dyn_into::<HtmlElement>() {
                    let _ = h.focus();
                }
            }
        }
    });
}

/// Dismiss the settings popover on a mousedown outside it (and outside the gear
/// that toggles it) — the expected "click-away closes the menu" behavior.
fn install_popover_dismiss(shared: &Shared) {
    let Some(win) = web_sys::window() else { return };
    let Some(doc) = win.document() else { return };
    let s = shared.clone();
    on(doc.unchecked_ref(), "mousedown", move |ev| {
        let doc = s.borrow().doc.clone();
        let Some(pop) = doc.get_element_by_id("bg-settings") else {
            return;
        };
        if !pop.class_name().contains("open") {
            return;
        }
        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
        let in_pop = target
            .as_ref()
            .map(|n| pop.contains(Some(n)))
            .unwrap_or(false);
        let in_gear = doc
            .get_element_by_id("bg-gear")
            .zip(target.as_ref())
            .map(|(g, n)| g.contains(Some(n)))
            .unwrap_or(false);
        if !in_pop && !in_gear {
            close_popover(&s);
        }
    });
}

/// A styled, validating confirmation modal — the in-app replacement for the
/// browser's `window.prompt`, which offered no validation and (for "delete N
/// days") let an unbounded number silently wipe everything. `on_confirm` receives
/// the input value (when there is an input) and returns whether to close: it can
/// reject bad input by writing to `#bg-modal-error` and returning `false`.
///
/// When `require_phrase` is `Some(word)`, the Confirm button starts disabled and
/// only enables once the input's trimmed value matches `word` exactly — the
/// "type DELETE to confirm" gate for irreversible actions (§8). It has no effect
/// without an input (`placeholder = None`).
pub(crate) fn confirm_dialog(
    shared: &Shared,
    title: &str,
    message: &str,
    placeholder: Option<&str>,
    confirm_label: &str,
    danger: bool,
    require_phrase: Option<&str>,
    on_confirm: impl FnMut(Option<String>) -> bool + 'static,
) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if let Some(old) = doc.get_element_by_id("bg-modal") {
        old.remove();
    }

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay");
    let _ = overlay.set_attribute("id", "bg-modal");
    let modal = panel(&doc, "modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");

    let _ = modal.append_child(&span(&doc, "modal-title", title));
    let _ = modal.append_child(&span(&doc, "modal-msg", message));

    let input_opt = placeholder.map(|ph| {
        let inp = el(&doc, "input");
        let _ = inp.set_attribute("type", "text");
        let _ = inp.set_attribute("class", "modal-input");
        let _ = inp.set_attribute("id", "bg-modal-input");
        let _ = inp.set_attribute("placeholder", ph);
        let _ = modal.append_child(&inp);
        inp
    });

    let err = span(&doc, "modal-error", "");
    let _ = err.set_attribute("id", "bg-modal-error");
    let _ = modal.append_child(&err);

    let actions = el(&doc, "div");
    let _ = actions.set_attribute("class", "modal-actions");
    let cancel = el(&doc, "button");
    let _ = cancel.set_attribute("type", "button");
    let _ = cancel.set_attribute("class", "modal-btn");
    cancel.set_text_content(Some("Cancel"));
    let confirm = el(&doc, "button");
    let _ = confirm.set_attribute("type", "button");
    let _ = confirm.set_attribute(
        "class",
        if danger {
            "modal-btn modal-confirm danger"
        } else {
            "modal-btn modal-confirm"
        },
    );
    confirm.set_text_content(Some(confirm_label));
    // Gate irreversible actions behind a typed confirmation phrase: the button
    // starts disabled and only the exact word enables it (§8).
    if require_phrase.is_some() {
        let _ = confirm.set_attribute("disabled", "disabled");
    }
    let _ = actions.append_child(&cancel);
    let _ = actions.append_child(&confirm);
    let _ = modal.append_child(&actions);
    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);

    if let Some(inp) = &input_opt {
        if let Ok(h) = inp.clone().dyn_into::<HtmlElement>() {
            let _ = h.focus();
        }
    }

    let cb = std::rc::Rc::new(std::cell::RefCell::new(on_confirm));
    let do_confirm = {
        let cb = cb.clone();
        let inp = input_opt.clone();
        let doc = doc.clone();
        move || {
            let val = inp
                .as_ref()
                .and_then(|i| i.clone().dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value());
            if (cb.borrow_mut())(val) {
                if let Some(o) = doc.get_element_by_id("bg-modal") {
                    o.remove();
                }
            }
        }
    };
    {
        let f = do_confirm.clone();
        on(&confirm, "click", move |_| f());
    }
    {
        let doc = doc.clone();
        on(&cancel, "click", move |_| {
            if let Some(o) = doc.get_element_by_id("bg-modal") {
                o.remove();
            }
        });
    }
    {
        // Click the backdrop (not the dialog) to dismiss.
        let doc = doc.clone();
        on(overlay.as_ref(), "mousedown", move |ev| {
            let on_backdrop = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .map(|e| e.id() == "bg-modal")
                .unwrap_or(false);
            if on_backdrop {
                if let Some(o) = doc.get_element_by_id("bg-modal") {
                    o.remove();
                }
            }
        });
    }
    if let Some(inp) = &input_opt {
        let confirm = confirm.clone();
        on(inp.as_ref(), "keydown", move |ev| {
            if let Ok(ke) = ev.dyn_into::<KeyboardEvent>() {
                if ke.key() == "Enter" {
                    // A disabled Confirm swallows `.click()`, so Enter can't
                    // bypass the typed-phrase gate below.
                    if let Ok(h) = confirm.clone().dyn_into::<HtmlElement>() {
                        h.click();
                    }
                }
            }
        });
    }
    // Enable Confirm only once the exact confirmation phrase is typed (§8).
    if let (Some(inp), Some(phrase)) = (&input_opt, require_phrase.map(str::to_string)) {
        let confirm = confirm.clone();
        on(inp.as_ref(), "input", move |ev| {
            let matches = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value().trim() == phrase)
                .unwrap_or(false);
            if matches {
                let _ = confirm.remove_attribute("disabled");
            } else {
                let _ = confirm.set_attribute("disabled", "disabled");
            }
        });
    }
}

// ── saved / named views ───────────────────────────────────────────────────────

/// Human label for a time range (matches the Range control's segments).
fn range_label(r: TimeRange) -> &'static str {
    match r {
        TimeRange::Session => "Session",
        TimeRange::Day => "Day",
        TimeRange::Week => "Week",
        TimeRange::Month => "Month",
        TimeRange::Year => "Year",
    }
}

/// One-line summary of a saved view's non-default knobs, for the list row.
fn view_summary(v: &crate::views::SavedView) -> String {
    let mut parts = vec![range_label(v.range).to_string()];
    if v.gran == Granularity::Registrable {
        parts.push("domains".into());
    }
    if v.filters.min_visits > 1 {
        parts.push(format!("≥{} visits", v.filters.min_visits));
    }
    if v.filters.hide_search_hubs {
        parts.push("no search hubs".into());
    }
    if v.filters.hide_isolated {
        parts.push("no isolated".into());
    }
    if v.spa_mode {
        parts.push("in-app navs".into());
    }
    parts.join(" · ")
}

/// Snapshot the dashboard's current view controls into a named `SavedView`.
fn snapshot_view(a: &super::App, name: String) -> crate::views::SavedView {
    crate::views::SavedView {
        name,
        range: a.time_range,
        gran: a.gran,
        filters: a.filters.clone(),
        spa_mode: a.spa_mode,
    }
}

/// Apply a saved view to the live state and rebuild the projection/chrome.
fn apply_saved_view(shared: &Shared, v: &crate::views::SavedView) {
    {
        let mut a = shared.borrow_mut();
        a.time_range = v.range;
        a.gran = v.gran;
        a.filters = v.filters.clone();
        a.spa_mode = v.spa_mode;
    }
    // Applying a saved view updates the live controls, so mirror it into the
    // persisted UI prefs via the normal write-through path (§7.7, no savedViews
    // schema change).
    persist_ui_prefs(shared);
    // reload_buckets rebuilds buckets per spa_mode, recomputes the projection, and
    // rerenders (sync_chrome then reflects the new range/filters in the controls).
    reload_buckets(shared);
    if v.range == TimeRange::Session {
        super::refresh_session_buckets(shared);
    }
}

/// Open the saved-views manager (reads the persisted list async, then builds the
/// modal). Save the current view, apply one, or delete one.
fn open_saved_views_dialog(shared: &Shared) {
    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
            .await
            .unwrap_or_default();
        build_saved_views_modal(&s, crate::views::parse(&json));
    });
}

fn build_saved_views_modal(shared: &Shared, views: Vec<crate::views::SavedView>) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if let Some(old) = doc.get_element_by_id("bg-modal") {
        old.remove();
    }

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay");
    let _ = overlay.set_attribute("id", "bg-modal");
    let modal = panel(&doc, "modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");
    let _ = modal.append_child(&span(&doc, "modal-title", "Saved views"));
    let _ = modal.append_child(&span(
        &doc,
        "modal-msg",
        "Save the current range, filters, and granularity under a name, then re-apply it anytime.",
    ));

    // Save-current row: name input + Save button.
    let saverow = el(&doc, "div");
    let _ = saverow.set_attribute("class", "modal-saverow");
    let inp = el(&doc, "input");
    let _ = inp.set_attribute("type", "text");
    let _ = inp.set_attribute("class", "modal-input");
    let _ = inp.set_attribute("id", "bg-view-name");
    let _ = inp.set_attribute("placeholder", "Name this view, e.g. Work");
    let _ = saverow.append_child(&inp);
    let save = el(&doc, "button");
    let _ = save.set_attribute("type", "button");
    let _ = save.set_attribute("class", "modal-btn modal-confirm");
    save.set_text_content(Some("Save current"));
    let _ = saverow.append_child(&save);
    let _ = modal.append_child(&saverow);

    let err = span(&doc, "modal-error", "");
    let _ = err.set_attribute("id", "bg-modal-error");
    let _ = modal.append_child(&err);

    // List of saved views (apply / delete each).
    let list = el(&doc, "div");
    let _ = list.set_attribute("class", "modal-list");
    if views.is_empty() {
        let _ = list.append_child(&span(&doc, "modal-empty", "No saved views yet."));
    } else {
        for v in &views {
            let row = el(&doc, "div");
            let _ = row.set_attribute("class", "modal-list-row");
            let applyb = el(&doc, "button");
            let _ = applyb.set_attribute("type", "button");
            let _ = applyb.set_attribute("class", "modal-list-apply");
            applyb.set_text_content(Some(&format!("{} · {}", v.name, view_summary(v))));
            {
                let s = shared.clone();
                let v = v.clone();
                let doc2 = doc.clone();
                on(&applyb, "click", move |_| {
                    apply_saved_view(&s, &v);
                    if let Some(o) = doc2.get_element_by_id("bg-modal") {
                        o.remove();
                    }
                });
            }
            let _ = row.append_child(&applyb);
            let del = el(&doc, "button");
            let _ = del.set_attribute("type", "button");
            let _ = del.set_attribute("class", "modal-list-del");
            let _ = del.set_attribute("aria-label", &format!("Delete {}", v.name));
            del.set_text_content(Some("✕"));
            {
                let s = shared.clone();
                let name = v.name.clone();
                on(&del, "click", move |_| {
                    let s2 = s.clone();
                    let name = name.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
                            .await
                            .unwrap_or_default();
                        let updated = crate::views::remove(crate::views::parse(&json), &name);
                        crate::bridge::storage_local_set(
                            crate::views::STORAGE_KEY,
                            &crate::views::serialize(&updated),
                        );
                        build_saved_views_modal(&s2, updated);
                    });
                });
            }
            let _ = row.append_child(&del);
            let _ = list.append_child(&row);
        }
    }
    let _ = modal.append_child(&list);

    let actions = el(&doc, "div");
    let _ = actions.set_attribute("class", "modal-actions");
    let close = el(&doc, "button");
    let _ = close.set_attribute("type", "button");
    let _ = close.set_attribute("class", "modal-btn");
    close.set_text_content(Some("Close"));
    let _ = actions.append_child(&close);
    let _ = modal.append_child(&actions);
    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);

    // Save the current view under the typed name.
    {
        let s = shared.clone();
        let inp = inp.clone();
        let doc2 = doc.clone();
        on(&save, "click", move |_| {
            let name = inp
                .clone()
                .dyn_into::<HtmlInputElement>()
                .map(|i| i.value())
                .unwrap_or_default();
            if name.trim().is_empty() {
                set_text(&doc2, "bg-modal-error", "Enter a name for this view.");
                return;
            }
            let view = snapshot_view(&s.borrow(), name);
            let s2 = s.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
                    .await
                    .unwrap_or_default();
                let updated = crate::views::upsert(crate::views::parse(&json), view);
                crate::bridge::storage_local_set(
                    crate::views::STORAGE_KEY,
                    &crate::views::serialize(&updated),
                );
                build_saved_views_modal(&s2, updated);
            });
        });
    }
    {
        let doc2 = doc.clone();
        on(&close, "click", move |_| {
            if let Some(o) = doc2.get_element_by_id("bg-modal") {
                o.remove();
            }
        });
    }
    {
        let doc2 = doc.clone();
        on(overlay.as_ref(), "mousedown", move |ev| {
            let on_backdrop = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .map(|e| e.id() == "bg-modal")
                .unwrap_or(false);
            if on_backdrop {
                if let Some(o) = doc2.get_element_by_id("bg-modal") {
                    o.remove();
                }
            }
        });
    }
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

    let window = crate::project::select_window(&a.buckets, a.time_range);
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

fn set_text(doc: &Document, id: &str, text: &str) {
    if let Some(e) = doc.get_element_by_id(id) {
        e.set_text_content(Some(text));
    }
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
