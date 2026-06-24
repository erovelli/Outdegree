//! Session picker (§7.7): lists closed + provisional-open sessions; selecting one
//! renders its per-tab flow (§4.4, sankey.rs). Supports a host-substring filter,
//! a "hide 1-visit sessions" toggle, relative day labels, and auto-selection of
//! the most recent session so the flow pane is never blank on open.

use super::{
    body_container, el, esc, on, persist_positions, plural, recompute_projection,
    session_day_label, session_when, Shared,
};
use crate::model::Granularity;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlInputElement};

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let Some(body) = body_container(shared) else {
        return Ok(());
    };
    body.set_inner_html("");
    let doc = shared.borrow().doc.clone();

    let container = el(&doc, "div");
    let _ = container.set_attribute("class", "sp-row");

    let list = el(&doc, "div");
    let _ = list.set_attribute("class", "sp-list");
    let heading = el(&doc, "h3");
    heading.set_text_content(Some("Sessions"));
    let _ = list.append_child(&heading);

    // Auto-select the most recent session so the flow pane isn't blank on open.
    {
        let mut a = shared.borrow_mut();
        if a.selected_session.is_none() {
            a.selected_session = a
                .sessions
                .iter()
                .max_by(|x, y| {
                    x.start_ts
                        .partial_cmp(&y.start_ts)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|s| s.id);
        }
    }

    // Filter controls (host search + hide-1-visit toggle).
    let query = shared.borrow().session_query.clone();
    let hide_trivial = shared.borrow().hide_trivial_sessions;
    let controls = el(&doc, "div");
    let _ = controls.set_attribute("class", "sp-filter");

    let qbox = el(&doc, "input");
    let _ = qbox.set_attribute("type", "text");
    let _ = qbox.set_attribute("id", "sp-search");
    let _ = qbox.set_attribute("class", "sp-search");
    let _ = qbox.set_attribute("placeholder", "Filter by site…");
    let _ = qbox.set_attribute("value", &query);
    {
        let s = shared.clone();
        on(qbox.as_ref(), "input", move |ev| {
            let v = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value())
                .unwrap_or_default();
            s.borrow_mut().session_query = v;
            // Rebuild only the item list so the input keeps focus + caret.
            fill_items(&s);
        });
    }

    let (trow, tinput) = super::filters::menu_toggle(&doc, "Hide 1-visit", hide_trivial);
    let _ = trow.set_attribute("class", "sp-toggle");
    {
        let s = shared.clone();
        on(tinput.as_ref(), "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().hide_trivial_sessions = c;
            fill_items(&s);
        });
    }
    let _ = controls.append_child(&qbox);
    let _ = controls.append_child(&trow);
    let _ = list.append_child(&controls);

    // The session items live in their own container so the filter can refill just
    // this part without tearing down the search box (which would drop focus).
    let items = el(&doc, "div");
    let _ = items.set_attribute("id", "sp-items");
    let _ = items.set_attribute("class", "sp-items");
    let _ = list.append_child(&items);

    // Right pane: a Hostname/Domain grouping toggle above the flow.
    let right = el(&doc, "div");
    let _ = right.set_attribute("class", "sp-right");

    let bar = el(&doc, "div");
    let _ = bar.set_attribute("class", "sp-toolbar");
    let lbl = el(&doc, "span");
    let _ = lbl.set_attribute("class", "muted");
    lbl.set_text_content(Some("Group by"));

    let (seg_wrap, btns) = super::filters::seg(
        &doc,
        "ghost",
        &[("hostname", "Hostname"), ("registrable", "Domain")],
    );
    let cur = if shared.borrow().gran == Granularity::Registrable {
        "registrable"
    } else {
        "hostname"
    };
    for (val, btn) in &btns {
        if val.as_str() == cur {
            let _ = btn.set_attribute("class", "active");
        }
        let gran = if val.as_str() == "registrable" {
            Granularity::Registrable
        } else {
            Granularity::Hostname
        };
        let s = shared.clone();
        let sw = seg_wrap.clone();
        on(btn, "click", move |_| {
            if s.borrow().gran == gran {
                return;
            }
            s.borrow_mut().gran = gran;
            recompute_projection(&s);
            persist_positions(&s);
            for (v, active) in [
                ("hostname", gran == Granularity::Hostname),
                ("registrable", gran == Granularity::Registrable),
            ] {
                if let Ok(Some(b)) = sw.query_selector(&format!("[data-seg=\"{v}\"]")) {
                    let _ = b.set_attribute("class", if active { "active" } else { "" });
                }
            }
            let _ = super::sankey::render(&s);
        });
    }
    let _ = bar.append_child(&lbl);
    let _ = bar.append_child(&seg_wrap);

    let flow = el(&doc, "div");
    let _ = flow.set_attribute("id", "bg-flow");
    let _ = flow.set_attribute("class", "sp-flow");

    let _ = right.append_child(&bar);
    let _ = right.append_child(&flow);

    let _ = container.append_child(&list);
    let _ = container.append_child(&right);
    let _ = body.append_child(&container);

    fill_items(shared);
    super::sankey::render(shared)
}

/// (Re)build just the `#sp-items` list from the current sessions + filter state.
fn fill_items(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    let Some(items) = doc.get_element_by_id("sp-items") else {
        return;
    };
    items.set_inner_html("");

    let (mut sessions, query, hide_trivial, selected) = {
        let a = shared.borrow();
        (
            a.sessions.clone(),
            a.session_query.trim().to_lowercase(),
            a.hide_trivial_sessions,
            a.selected_session,
        )
    };
    sessions.sort_by(|a, b| {
        b.start_ts
            .partial_cmp(&a.start_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let shown: Vec<_> = sessions
        .iter()
        .filter(|s| !(hide_trivial && s.nav_count <= 1))
        .filter(|s| {
            query.is_empty()
                || s.top_hosts
                    .iter()
                    .any(|(h, _)| h.to_lowercase().contains(&query))
        })
        .collect();

    if shown.is_empty() {
        let empty = el(&doc, "div");
        let _ = empty.set_attribute("class", "bg-empty");
        empty.set_text_content(Some(if sessions.is_empty() {
            "No sessions yet."
        } else {
            "No sessions match the filter."
        }));
        let _ = items.append_child(&empty);
        return;
    }

    for sess in &shown {
        let item = build_item(&doc, sess, selected);
        let sid = sess.id;
        let s = shared.clone();
        on(item.as_ref(), "click", move |_| {
            s.borrow_mut().selected_session = Some(sid);
            // Refill the items so the selection highlight moves; the search box
            // lives outside #sp-items, so its focus/value is untouched.
            fill_items(&s);
            let _ = super::sankey::render(&s);
        });
        let _ = items.append_child(&item);
    }
}

fn build_item(doc: &Document, sess: &crate::rollup::SessionRec, selected: Option<f64>) -> Element {
    let item = el(doc, "button");
    let _ = item.set_attribute("type", "button");
    let cls = if selected == Some(sess.id) {
        "sp-item is-selected"
    } else {
        "sp-item"
    };
    let _ = item.set_attribute("class", cls);
    let top = sess
        .top_hosts
        .iter()
        .take(3)
        .map(|(h, _)| h.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let visits = plural(sess.nav_count as u64, "visit");
    let meta = if top.is_empty() {
        visits
    } else {
        format!("{visits} · {}", esc(&top))
    };
    item.set_inner_html(&format!(
        "<b>{} · {}</b><br><span class=\"muted\">{}</span>",
        esc(&session_day_label(sess.start_ts)),
        session_when(sess.start_ts, sess.end_ts),
        meta
    ));
    item
}
