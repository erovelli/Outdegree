//! Session picker (§7.7): lists closed + provisional-open sessions; selecting one
//! renders its per-tab flow (§4.4, sankey.rs).

use super::{body_container, el, esc, on, Shared};
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

    let flow = el(&doc, "div");
    let _ = flow.set_attribute("id", "bg-flow");
    let _ = flow.set_attribute("class", "sp-flow");

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

    let _ = container.append_child(&list);
    let _ = container.append_child(&flow);
    let _ = body.append_child(&container);

    super::sankey::render(shared)
}
