//! Dashboard shell: the full-bleed canvas plus the nine floating glass control
//! clusters from the design handoff, and their wiring (§7.7).
//!
//! The canvas is the app; every control floats over it as a translucent panel
//! pinned to an edge or corner. Chrome is strictly monochrome — the only color
//! belongs to the data spectrum drawn on the canvas.

use super::filters::{chip, icon, icon_btn, menu_btn, menu_toggle, panel, seg, LOGO};
use super::{
    el, graph_view, on, persist_positions, plural, recompute_projection, reload_and_rerender,
    reload_buckets, rerender, Shared, View,
};
use crate::model::{Granularity, ProvBreakdown, Provenance};
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

    let _ = root.append_child(&brand_panel(&doc, shared));
    let _ = root.append_child(&range_panel(&doc, shared));
    let _ = root.append_child(&view_panel(&doc, shared));
    let _ = root.append_child(&legend_panel(&doc, shared));
    let _ = root.append_child(&zoom_panel(&doc, shared));
    let _ = root.append_child(&search_panel(&doc, shared));
    let _ = root.append_child(&readout_panel(&doc));
    let _ = root.append_child(&focus_panel(&doc, shared));
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
    let logo = el(doc, "div");
    let _ = logo.set_attribute("class", "logo");
    logo.set_inner_html(LOGO);
    let name = span(doc, "wordmark", "Browsing Graph");
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

    let _ = p.append_child(&logo);
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
            let _ = rerender(&s);
        });
    }
    let _ = p.append_child(&wrap);
    p
}

// ── 3. view + settings gear (top-right) ──────────────────────────────────────
fn view_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "viewbar at-tr");
    let (wrap, btns) = seg(
        doc,
        "ghost",
        &[
            ("graph", "Graph"),
            ("sankey", "Sankey"),
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
                    "panel popover open"
                } else {
                    "panel popover"
                });
                if let Some(g) = doc.get_element_by_id("bg-gear") {
                    let _ =
                        g.set_attribute("aria-expanded", if now_open { "true" } else { "false" });
                }
            }
        });
    }
    let _ = p.append_child(&wrap);
    let _ = p.append_child(&gear);
    p
}

// ── 4. provenance legend (right) ─────────────────────────────────────────────
fn legend_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "legend at-rt");
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
    p
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
    let _ = chips.append_child(&gran_seg);
    let _ = chips.append_child(&mv);
    let _ = chips.append_child(&hubs);
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

    let (spa_row, spa_input) = menu_toggle(doc, "In-app navigations", false);
    {
        let s = shared.clone();
        on(&spa_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().spa_mode = c;
            reload_buckets(&s);
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

    let export = menu_btn(doc, "Export JSON");
    {
        let s = shared.clone();
        on(&export, "click", move |_| {
            close_popover(&s);
            let db = s.borrow().db.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match db.export_json().await {
                    Ok(json) => crate::bridge::download_json("browsing-graph-export.json", &json),
                    Err(e) => super::log_err(&e),
                }
            });
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
                    let json = match wasm_bindgen_futures::JsFuture::from(file.text()).await {
                        Ok(v) => v.as_string().unwrap_or_default(),
                        Err(e) => return super::log_err(&e),
                    };
                    let db = s3.borrow().db.clone();
                    // Replace every store, then re-derive from the imported events so
                    // the view is consistent even if the file only carried `events`.
                    if let Err(e) = db.import_json(&json).await {
                        return super::log_err(&e);
                    }
                    if let Err(e) = db.reset_derivation().await {
                        return super::log_err(&e);
                    }
                    reload_and_rerender(&s3);
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
            let win = web_sys::window().expect("window");
            if let Ok(Some(domain)) =
                win.prompt_with_message("Forget which domain? (host or eTLD+1)")
            {
                let domain = domain.trim().to_string();
                if domain.is_empty() {
                    return;
                }
                let db = s.borrow().db.clone();
                let s2 = s.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) = db.forget_domain(&domain).await {
                        return super::log_err(&e);
                    }
                    reload_and_rerender(&s2);
                });
            }
        });
    }

    let delete = menu_btn(doc, "Delete last N days…");
    {
        let s = shared.clone();
        on(&delete, "click", move |_| {
            close_popover(&s);
            let win = web_sys::window().expect("window");
            if let Ok(Some(days)) = win.prompt_with_message("Delete the last how many days?") {
                if let Ok(n) = days.trim().parse::<f64>() {
                    let now = js_sys::Date::now();
                    let from = now - n * 86_400_000.0;
                    let db = s.borrow().db.clone();
                    let s2 = s.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = db.delete_range(from, now).await {
                            return super::log_err(&e);
                        }
                        reload_and_rerender(&s2);
                    });
                }
            }
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

    let _ = pop.append_child(&spa_row);
    let sep = el(doc, "div");
    let _ = sep.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep);
    let _ = pop.append_child(&raw);
    let _ = pop.append_child(&export);
    let _ = pop.append_child(&import);
    let _ = pop.append_child(&forget);
    let _ = pop.append_child(&delete);
    let _ = pop.append_child(&rebuild);
    pop
}

fn close_popover(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    if let Some(pop) = doc.get_element_by_id("bg-settings") {
        pop.set_class_name("panel popover");
    }
    if let Some(g) = doc.get_element_by_id("bg-gear") {
        let _ = g.set_attribute("aria-expanded", "false");
    }
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

/// Add/remove the `active` class token while preserving any base classes.
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
    }
}
