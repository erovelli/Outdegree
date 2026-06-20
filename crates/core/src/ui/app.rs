//! Dashboard shell: toolbar, view tabs, and their wiring (§7.7).

use super::filters::{button, checkbox, number, select};
use super::{
    el, on, persist_positions, recompute_projection, reload_and_rerender, rerender, Shared, View,
};
use crate::model::Granularity;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlInputElement, HtmlSelectElement};

/// Build the toolbar + tabs + body container and wire every control.
pub(crate) fn build_shell(shared: &Shared) -> Result<(), JsValue> {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };

    // ── toolbar ────────────────────────────────────────────────────────────────
    let toolbar = el(&doc, "div");
    let _ = toolbar.set_attribute("class", "bg-toolbar");
    let title = el(&doc, "h1");
    title.set_text_content(Some("Browsing Graph"));
    let _ = toolbar.append_child(&title);

    // granularity (decision #9)
    let (gran_wrap, gran_sel) = select(
        &doc,
        "Granularity",
        &[("hostname", "Hostname"), ("registrable", "eTLD+1")],
        "hostname",
    );
    let _ = toolbar.append_child(&gran_wrap);
    {
        let s = shared.clone();
        on(&gran_sel, "change", move |ev| {
            let v = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlSelectElement>().ok())
                .map(|s| s.value())
                .unwrap_or_default();
            s.borrow_mut().gran = if v == "registrable" {
                Granularity::Registrable
            } else {
                Granularity::Hostname
            };
            recompute_projection(&s);
            persist_positions(&s);
            let _ = rerender(&s);
        });
    }

    // min visits filter
    let (mv_wrap, mv_input) = number(&doc, "Min visits", 0);
    let _ = toolbar.append_child(&mv_wrap);
    {
        let s = shared.clone();
        on(&mv_input, "input", move |ev| {
            let v = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value())
                .unwrap_or_default();
            s.borrow_mut().filters.min_visits = v.parse().unwrap_or(0);
            recompute_projection(&s);
            let _ = rerender(&s);
        });
    }

    // hide search hubs
    let (hs_wrap, hs_input) = checkbox(&doc, "Hide search hubs", false);
    let _ = toolbar.append_child(&hs_wrap);
    {
        let s = shared.clone();
        on(&hs_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().filters.hide_search_hubs = c;
            recompute_projection(&s);
            let _ = rerender(&s);
        });
    }

    // spacer
    let spacer = el(&doc, "span");
    let _ = spacer.set_attribute("style", "flex:1");
    let _ = toolbar.append_child(&spacer);

    // pause / resume capture
    let pause_btn = button(&doc, pause_label(shared.borrow().paused));
    let _ = pause_btn.set_attribute("id", "bg-pause");
    let _ = toolbar.append_child(&pause_btn);
    {
        let s = shared.clone();
        on(&pause_btn, "click", move |_| {
            let now = {
                let mut a = s.borrow_mut();
                a.paused = !a.paused;
                a.paused
            };
            crate::bridge::storage_local_set("paused", if now { "true" } else { "false" });
            if let Some(b) = s.borrow().doc.get_element_by_id("bg-pause") {
                b.set_text_content(Some(pause_label(now)));
            }
        });
    }

    // export (local download)
    let export_btn = button(&doc, "Export");
    let _ = toolbar.append_child(&export_btn);
    {
        let s = shared.clone();
        on(&export_btn, "click", move |_| {
            let db = s.borrow().db.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match db.export_json().await {
                    Ok(json) => crate::bridge::download_json("browsing-graph-export.json", &json),
                    Err(e) => super::log_err(&e),
                }
            });
        });
    }

    // forget a domain
    let forget_btn = button(&doc, "Forget domain");
    let _ = toolbar.append_child(&forget_btn);
    {
        let s = shared.clone();
        on(&forget_btn, "click", move |_| {
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

    // delete a recent time range
    let delete_btn = button(&doc, "Delete last N days");
    let _ = toolbar.append_child(&delete_btn);
    {
        let s = shared.clone();
        on(&delete_btn, "click", move |_| {
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

    let _ = root.append_child(&toolbar);

    // ── tabs ─────────────────────────────────────────────────────────────────
    let tabs = el(&doc, "div");
    let _ = tabs.set_attribute("class", "bg-tabs");
    for (id, label, view) in [
        ("bg-tab-graph", "Graph", View::Graph),
        ("bg-tab-tables", "Tables", View::Tables),
        ("bg-tab-sankey", "Sessions", View::Sankey),
        ("bg-tab-raw", "Raw", View::Raw),
    ] {
        let b = button(&doc, label);
        let _ = b.set_attribute("id", id);
        let _ = tabs.append_child(&b);
        let s = shared.clone();
        on(&b, "click", move |_| {
            s.borrow_mut().view = view;
            let _ = rerender(&s);
        });
    }
    let _ = root.append_child(&tabs);

    // ── body container ─────────────────────────────────────────────────────────
    let body = el(&doc, "div");
    let _ = body.set_attribute("class", "bg-body");
    let _ = body.set_attribute("id", "bg-body");
    let _ = root.append_child(&body);

    Ok(())
}

fn pause_label(paused: bool) -> &'static str {
    if paused {
        "Resume capture"
    } else {
        "Pause capture"
    }
}

/// Reflect the active view on the tab buttons.
pub(crate) fn set_active_tab(shared: &Shared) {
    let (doc, view) = {
        let a = shared.borrow();
        (a.doc.clone(), a.view)
    };
    for (id, v) in [
        ("bg-tab-graph", View::Graph),
        ("bg-tab-tables", View::Tables),
        ("bg-tab-sankey", View::Sankey),
        ("bg-tab-raw", View::Raw),
    ] {
        if let Some(b) = doc.get_element_by_id(id) {
            let _ = b.set_attribute("class", if v == view { "active" } else { "" });
        }
    }
}
