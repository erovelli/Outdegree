//! Per-session flow view (§7.7), drawn as a Sankey diagram: hosts become columns
//! (layered by the flow), transitions become ribbons whose width ∝ how often that
//! hop was taken. Per-tab chains are reconstructed from the session's `events`
//! id-range (§4.4) and aggregated by [`crate::flow`].

use super::Shared;
use crate::flow;
use crate::model::Event;
use std::collections::{HashMap, HashSet};
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

        // Reconstruct the session flow across tabs: within-tab Nav sequences are
        // transitions, and a link opened in a new tab bridges the source tab's
        // current host → the new tab's first host (mirrors the derive pass, §7.3).
        let mut current: HashMap<i64, String> = HashMap::new(); // tab → current host
        let mut pending: HashMap<i64, String> = HashMap::new(); // new tab → spawn origin
        let mut seen: HashSet<i64> = HashSet::new();
        let mut transitions: Vec<(String, String)> = Vec::new();
        let mut starts: Vec<String> = Vec::new();
        for ev in &events {
            match ev {
                Event::Link {
                    new_tab_id,
                    source_tab_id,
                    ..
                } => {
                    if let Some(src) = current.get(&(*source_tab_id as i64)) {
                        pending.insert(*new_tab_id as i64, src.clone());
                    }
                }
                Event::Nav { tab_id, to_url, .. } => {
                    let t = *tab_id as i64;
                    let Some(host) = crate::interpret::node_key(to_url, gran) else {
                        continue;
                    };
                    if current.get(&t) == Some(&host) {
                        continue; // collapse consecutive duplicates
                    }
                    let origin = pending.remove(&t).or_else(|| current.get(&t).cloned());
                    let first = seen.insert(t);
                    match origin {
                        Some(o) if o != host => transitions.push((o, host.clone())),
                        None if first => starts.push(host.clone()),
                        _ => {}
                    }
                    current.insert(t, host);
                }
                _ => {}
            }
        }

        let Some(flow_el) = s.borrow().doc.get_element_by_id("bg-flow") else {
            return;
        };
        let fg = flow::build_transitions(&transitions, &starts);
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
