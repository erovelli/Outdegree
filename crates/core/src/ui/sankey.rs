//! Per-session flow view (§7.7). A per-window session interleaves concurrent
//! tabs, so the flow is shown per-tab (the spec's note), reconstructed from the
//! session's `events` id-range (§4.4).

use super::{esc, Shared};
use crate::model::Event;
use std::collections::HashMap;
use wasm_bindgen::JsValue;

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let doc = shared.borrow().doc.clone();
    let Some(flow) = doc.get_element_by_id("bg-flow") else {
        return Ok(());
    };

    let Some(sid) = shared.borrow().selected_session else {
        flow.set_inner_html("<div class=\"bg-empty\">Select a session to see its flow.</div>");
        return Ok(());
    };
    let sess = shared
        .borrow()
        .sessions
        .iter()
        .find(|s| s.id == sid)
        .cloned();
    let Some(sess) = sess else {
        return Ok(());
    };

    let gran = shared.borrow().gran;
    let db = shared.borrow().db.clone();
    let s = shared.clone();
    flow.set_inner_html("<div class=\"bg-empty\">Loading session…</div>");

    wasm_bindgen_futures::spawn_local(async move {
        let events = db
            .read_events_id_range(sess.start_id, sess.end_id)
            .await
            .unwrap_or_default();

        // Per-tab host chains, collapsing consecutive duplicates.
        let mut per_tab: Vec<(i64, Vec<String>)> = Vec::new();
        let mut idx: HashMap<i64, usize> = HashMap::new();
        for ev in &events {
            if let Event::Nav { tab_id, to_url, .. } = ev {
                if let Some(k) = crate::interpret::node_key(to_url, gran) {
                    let t = *tab_id as i64;
                    let i = *idx.entry(t).or_insert_with(|| {
                        per_tab.push((t, Vec::new()));
                        per_tab.len() - 1
                    });
                    if per_tab[i].1.last().map(|x| x != &k).unwrap_or(true) {
                        per_tab[i].1.push(k);
                    }
                }
            }
        }

        let Some(flow) = s.borrow().doc.get_element_by_id("bg-flow") else {
            return;
        };
        let mut html = format!(
            "<h3>Session flow · {} navs · window {}</h3>\
             <p style=\"color:var(--muted)\">Per-tab navigation chains within this session.</p>",
            sess.nav_count, sess.window_id
        );
        for (tab, chain) in &per_tab {
            let path = chain
                .iter()
                .map(|h| esc(h))
                .collect::<Vec<_>>()
                .join(" &rarr; ");
            html.push_str(&format!(
                "<div style=\"margin:8px 0\"><b>Tab {tab}</b>: {path}</div>"
            ));
        }
        if per_tab.is_empty() {
            html.push_str("<div class=\"bg-empty\">No navigations in this session.</div>");
        }
        flow.set_inner_html(&html);
    });

    Ok(())
}
