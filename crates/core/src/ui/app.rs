//! Dashboard shell: the full-bleed canvas plus the nine floating glass control
//! clusters from the design handoff, and their wiring (§7.7).
//!
//! The canvas is the app; every control floats over it as a translucent panel
//! pinned to an edge or corner. Chrome is strictly monochrome — the only color
//! belongs to the data spectrum drawn on the canvas.

use super::filters::{chip, icon, icon_btn, menu_btn, menu_toggle, panel, seg, LOGO};
use super::{
    el, graph_view, on, persist_positions, recompute_projection, reload_and_rerender,
    reload_buckets, rerender, Shared, View,
};
use crate::model::{Granularity, ProvBreakdown};
use crate::project::TimeRange;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, KeyboardEvent};

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
    let _ = root.append_child(&legend_panel(&doc));
    let _ = root.append_child(&zoom_panel(&doc, shared));
    let _ = root.append_child(&search_panel(&doc, shared));
    let _ = root.append_child(&readout_panel(&doc));
    let _ = root.append_child(&settings_popover(&doc, shared));
    install_palette_shortcut(shared);

    Ok(())
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
    let rule = el(doc, "div");
    let _ = rule.set_attribute("class", "vrule");

    let rec = el(doc, "div");
    let _ = rec.set_attribute("class", "rec");
    let _ = rec.set_attribute("id", "bg-rec");
    let _ = rec.set_attribute("title", "Toggle recording");
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
    {
        let s = shared.clone();
        on(&gear, "click", move |_| {
            if let Some(pop) = s.borrow().doc.get_element_by_id("bg-settings") {
                let open = pop.class_name().contains("open");
                pop.set_class_name(if open {
                    "panel popover"
                } else {
                    "panel popover open"
                });
            }
        });
    }
    let _ = p.append_child(&wrap);
    let _ = p.append_child(&gear);
    p
}

// ── 4. provenance legend (right) ─────────────────────────────────────────────
fn legend_panel(doc: &Document) -> Element {
    let p = panel(doc, "legend at-rt");
    let title = span(doc, "legend-title", "PROVENANCE");
    let rows = el(doc, "div");
    let _ = rows.set_attribute("id", "bg-legend-rows");
    let _ = rows.set_attribute("class", "legend-rows");
    let _ = p.append_child(&title);
    let _ = p.append_child(&rows);
    p
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
            if ke.key() != "Enter" {
                return;
            }
            let q = ke
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value().trim().to_lowercase())
                .unwrap_or_default();
            if q.is_empty() {
                return;
            }
            let hit = s
                .borrow()
                .proj
                .nodes
                .iter()
                .find(|n| n.key.to_lowercase().contains(&q))
                .map(|n| n.key.clone());
            if let Some(k) = hit {
                {
                    let mut a = s.borrow_mut();
                    a.focus = Some(k);
                    a.view = View::Graph;
                }
                recompute_projection(&s);
                let _ = rerender(&s);
            }
        });
    }

    let chips = el(doc, "div");
    let _ = chips.set_attribute("class", "chips");
    let gran = chip(doc, "chip-gran", "Hostname ⌄");
    let mv = chip(doc, "chip-minvisits", "min visits ≥ 0");
    let hubs = chip(doc, "chip-hubs", "hide search hubs");
    let _ = hubs.set_attribute("title", "Collapse search-engine origin nodes");
    let _ = mv.set_attribute("title", "Click to cycle the minimum visit threshold");
    let _ = gran.set_attribute("title", "Toggle hostname / registrable-domain grouping");
    let _ = chips.append_child(&gran);
    let _ = chips.append_child(&mv);
    let _ = chips.append_child(&hubs);
    {
        let s = shared.clone();
        on(&gran, "click", move |_| {
            {
                let mut a = s.borrow_mut();
                a.gran = if a.gran == Granularity::Hostname {
                    Granularity::Registrable
                } else {
                    Granularity::Hostname
                };
            }
            recompute_projection(&s);
            persist_positions(&s);
            let _ = rerender(&s);
        });
    }
    {
        let s = shared.clone();
        on(&mv, "click", move |_| {
            {
                let mut a = s.borrow_mut();
                a.filters.min_visits = match a.filters.min_visits {
                    0 => 5,
                    5 => 10,
                    10 => 25,
                    _ => 0,
                };
            }
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
    let rule2 = el(doc, "div");
    let _ = rule2.set_attribute("class", "vrule");
    let spectrum = el(doc, "div");
    let _ = spectrum.set_attribute("class", "spectrum");
    let slabel = span(doc, "spectrum-label", "visits");
    let _ = p.append_child(&nodes);
    let _ = p.append_child(&rule1);
    let _ = p.append_child(&edges);
    let _ = p.append_child(&rule2);
    let _ = p.append_child(&spectrum);
    let _ = p.append_child(&slabel);
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

    let _ = pop.append_child(&spa_row);
    let sep = el(doc, "div");
    let _ = sep.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep);
    let _ = pop.append_child(&raw);
    let _ = pop.append_child(&export);
    let _ = pop.append_child(&forget);
    let _ = pop.append_child(&delete);
    pop
}

fn close_popover(shared: &Shared) {
    if let Some(pop) = shared.borrow().doc.get_element_by_id("bg-settings") {
        pop.set_class_name("panel popover");
    }
}

/// ⌘K / Ctrl-K focuses the host search box (command-palette affordance).
fn install_palette_shortcut(shared: &Shared) {
    let Some(win) = web_sys::window() else { return };
    let s = shared.clone();
    on(win.as_ref(), "keydown", move |ev| {
        let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
            return;
        };
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
        &format!("{} nodes", a.proj.nodes.len()),
    );
    set_text(
        &doc,
        "bg-count-edges",
        &format!("{} edges", a.proj.edges.len()),
    );

    // provenance legend (percentages over the visible projection)
    let mut b = ProvBreakdown::default();
    for n in &a.proj.nodes {
        b.merge(&n.prov);
    }
    // (count, label, dot color class) — colors live in CSS so no inline style
    // is needed (the page CSP blocks inline styles).
    let rows = [
        (b.search_origin, "Search", "dot-search"),
        (b.link, "Link", "dot-link"),
        (b.typed_url, "Typed URL", "dot-typed"),
        (b.bookmark, "Bookmark", "dot-bookmark"),
        (b.form, "Form", "dot-form"),
        (b.other + b.start + b.reload, "Other", "dot-other"),
    ];
    let total: u32 = rows.iter().map(|(c, _, _)| *c).sum();
    let mut html = String::new();
    for (count, label, dot) in rows {
        let pct = if total > 0 {
            (count as f64 * 100.0 / total as f64).round() as u32
        } else {
            0
        };
        html.push_str(&format!(
            "<div class=\"legend-row\"><span class=\"dot {dot}\"></span>\
             <span class=\"legend-label\">{label}</span>\
             <span class=\"legend-pct\">{pct}%</span></div>"
        ));
    }
    if let Some(rows_el) = doc.get_element_by_id("bg-legend-rows") {
        rows_el.set_inner_html(&html);
    }

    // chips
    set_text(
        &doc,
        "chip-gran",
        if a.gran == Granularity::Registrable {
            "Domain ⌄"
        } else {
            "Hostname ⌄"
        },
    );
    set_text(
        &doc,
        "chip-minvisits",
        &format!("min visits ≥ {}", a.filters.min_visits),
    );
    toggle_active(&doc, "chip-hubs", a.filters.hide_search_hubs);

    // REC indicator
    if let Some(rec) = doc.get_element_by_id("bg-rec") {
        rec.set_class_name(if a.paused { "rec paused" } else { "rec" });
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
