//! Per-session flow view (§7.7), drawn as a Sankey diagram: hosts become columns
//! (layered by the flow), transitions become ribbons whose width ∝ how often that
//! hop was taken. Per-tab chains are reconstructed from the session's `events`
//! id-range (§4.4) and aggregated by [`crate::flow`].

use super::Shared;
use crate::flow;
use crate::model::Event;
use std::collections::HashMap;
use wasm_bindgen::JsValue;

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let doc = shared.borrow().doc.clone();
    let Some(flow_el) = doc.get_element_by_id("bg-flow") else {
        return Ok(());
    };

    let Some(sid) = shared.borrow().selected_session else {
        flow_el.set_inner_html("<div class=\"bg-empty\">Select a session to see its flow.</div>");
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
    flow_el.set_inner_html("<div class=\"bg-empty\">Loading session…</div>");

    wasm_bindgen_futures::spawn_local(async move {
        let events = db
            .read_events_id_range(sess.start_id, sess.end_id)
            .await
            .unwrap_or_default();

        // Per-tab host chains, collapsing consecutive duplicates.
        let mut per_tab: Vec<Vec<String>> = Vec::new();
        let mut idx: HashMap<i64, usize> = HashMap::new();
        for ev in &events {
            if let Event::Nav { tab_id, to_url, .. } = ev {
                if let Some(k) = crate::interpret::node_key(to_url, gran) {
                    let t = *tab_id as i64;
                    let i = *idx.entry(t).or_insert_with(|| {
                        per_tab.push(Vec::new());
                        per_tab.len() - 1
                    });
                    if per_tab[i].last().map(|x| x != &k).unwrap_or(true) {
                        per_tab[i].push(k);
                    }
                }
            }
        }

        let Some(flow_el) = s.borrow().doc.get_element_by_id("bg-flow") else {
            return;
        };
        let fg = flow::build(&per_tab);
        let vw = (flow_el.client_width() as f64 - 8.0).max(640.0);
        let mut html = format!(
            "<h3>Session flow · {} navs · window {}</h3>\
             <p class=\"muted\">Hosts are columns; ribbon width ∝ how often that hop was taken.</p>",
            sess.nav_count, sess.window_id
        );
        html.push_str(&flow::render_svg(&fg, vw));
        flow_el.set_inner_html(&html);
    });

    Ok(())
}
