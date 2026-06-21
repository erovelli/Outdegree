//! Session picker (§7.7): lists closed + provisional-open sessions; selecting one
//! renders its per-tab flow (§4.4, sankey.rs).

use super::{body_container, el, esc, on, persist_positions, recompute_projection, Shared};
use crate::model::Granularity;
use wasm_bindgen::JsValue;

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

    let mut sessions = shared.borrow().sessions.clone();
    sessions.sort_by(|a, b| {
        b.start_ts
            .partial_cmp(&a.start_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if sessions.is_empty() {
        let empty = el(&doc, "div");
        let _ = empty.set_attribute("class", "bg-empty");
        empty.set_text_content(Some("No sessions yet."));
        let _ = list.append_child(&empty);
    }

    for sess in &sessions {
        let item = el(&doc, "div");
        let _ = item.set_attribute("class", "sp-item");
        let top = sess
            .top_hosts
            .iter()
            .map(|(h, _)| h.clone())
            .collect::<Vec<_>>()
            .join(", ");
        item.set_inner_html(&format!(
            "<b>{} navs</b> · window {}<br><span class=\"muted\">{}</span>",
            sess.nav_count,
            sess.window_id,
            esc(&top)
        ));
        let sid = sess.id;
        let s = shared.clone();
        on(item.as_ref(), "click", move |_| {
            s.borrow_mut().selected_session = Some(sid);
            let _ = super::sankey::render(&s);
        });
        let _ = list.append_child(&item);
    }

    // Right pane: a Hostname/Domain grouping toggle above the flow. Granularity
    // controls how the Sankey buckets hosts (the bottom-left filter panel is
    // hidden on this view), so it gets its own toggle here.
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
                return; // already grouping this way → nothing to regenerate
            }
            s.borrow_mut().gran = gran;
            // Keep the graph view's projection consistent for when it's next shown.
            recompute_projection(&s);
            persist_positions(&s);
            // Reflect the active segment and re-render only the flow — a full
            // rerender would tear down and rebuild the whole picker (session list
            // + toolbar), which flashes. `sankey::render` swaps the diagram only if
            // it actually changed.
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

    super::sankey::render(shared)
}
