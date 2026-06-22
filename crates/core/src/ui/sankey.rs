//! Per-session flow view (§7.7), drawn as a Sankey diagram: hosts become columns
//! (layered by the flow), transitions become ribbons whose width ∝ how often that
//! hop was taken. Per-tab chains are reconstructed from the session's `events`
//! id-range (§4.4) and aggregated by [`crate::flow`].

use super::Shared;
use crate::flow;
use wasm_bindgen::JsValue;

/// The two color keys shown on the Sankey page: bar provenance + ribbon edge kind
/// (the floating graph legend is hidden on this view, so the flow is self-keyed).
/// Only the categories actually present in this flow are listed; an empty key
/// block (e.g. a single-host session with no ribbons) is omitted entirely.
fn keys_html(fg: &flow::FlowGraph) -> String {
    use crate::model::{EdgeKind, Provenance as P};
    use std::collections::HashSet;

    let provs: HashSet<P> = fg.nodes.iter().map(|n| n.prov).collect();
    let kinds: HashSet<EdgeKind> = fg.links.iter().map(|l| l.kind).collect();
    let prov_rows = [
        ("Search", "dot-search", P::SearchOrigin),
        ("Link", "dot-link", P::Link),
        ("Typed URL", "dot-typed", P::TypedUrl),
        ("Bookmark", "dot-bookmark", P::Bookmark),
        ("Form", "dot-form", P::Form),
        ("External", "dot-external", P::Start),
        ("Other", "dot-other", P::Other),
    ];
    let kind_rows = [
        ("Link", "dot-edge-link", EdgeKind::Link),
        ("Form", "dot-edge-form", EdgeKind::Form),
        ("Search-link", "dot-edge-search", EdgeKind::SearchLink),
    ];
    let has_prov = prov_rows.iter().any(|(_, _, p)| provs.contains(p));
    let has_kind = kind_rows.iter().any(|(_, _, k)| kinds.contains(k));
    if !has_prov && !has_kind {
        return String::new();
    }

    let mut h = String::from("<div class=\"sankey-keys\">");
    if has_prov {
        h.push_str(
            "<div class=\"sankey-key\">\
             <span class=\"sankey-key-title\">Bars · provenance</span>",
        );
        for (label, dot, p) in prov_rows {
            if !provs.contains(&p) {
                continue;
            }
            h.push_str(&format!(
                "<span class=\"key-item\"><span class=\"dot {dot} {glyph}\"></span>{label}</span>",
                glyph = p.shape().css()
            ));
        }
        h.push_str("</div>");
    }
    if has_kind {
        h.push_str(
            "<div class=\"sankey-key\">\
             <span class=\"sankey-key-title\">Ribbons · link type</span>",
        );
        for (label, dot, k) in kind_rows {
            if !kinds.contains(&k) {
                continue;
            }
            h.push_str(&format!(
                "<span class=\"key-item\"><span class=\"dot {dot}\"></span>{label}</span>"
            ));
        }
        h.push_str("</div>");
    }
    h.push_str("</div>");
    h
}

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let doc = shared.borrow().doc.clone();
    let Some(flow_el) = doc.get_element_by_id("bg-flow") else {
        return Ok(());
    };

    let Some(sid) = shared.borrow().selected_session else {
        if flow_el.get_attribute("data-sig").as_deref() != Some("none") {
            flow_el
                .set_inner_html("<div class=\"bg-empty\">Select a session to see its flow.</div>");
            let _ = flow_el.set_attribute("data-sig", "none");
        }
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
    // Only show the loading placeholder when nothing is rendered yet. On a
    // re-render (e.g. toggling Hostname/Domain) keep the current diagram on screen
    // so there's no flash — the async pass below swaps only if the flow actually
    // changed (guarded by a signature of the rendered markup).
    let has_flow = flow_el
        .get_attribute("data-sig")
        .is_some_and(|v| v != "none");
    if !has_flow {
        flow_el.set_inner_html("<div class=\"bg-empty\">Loading session…</div>");
        let _ = flow_el.remove_attribute("data-sig");
    }

    wasm_bindgen_futures::spawn_local(async move {
        let events = db
            .read_events_id_range(sess.start_id, sess.end_id)
            .await
            .unwrap_or_default();

        // Reconstruct the session flow, honoring how each page was reached:
        // link/form chains; typed/bookmark/search starts a fresh flow (§7.2).
        let fg = flow::from_session_events(&events, gran);

        let Some(flow_el) = s.borrow().doc.get_element_by_id("bg-flow") else {
            return;
        };
        let vw = (flow_el.client_width() as f64 - 8.0).max(640.0);
        let mut html = format!(
            "<h3>{}</h3>\
             <p class=\"muted\">{}. Each column is a site you visited; a thicker ribbon means \
             you took that step more often.</p>",
            super::session_when(sess.start_ts, sess.end_ts),
            super::plural(sess.nav_count as u64, "visit"),
        );
        html.push_str(&keys_html(&fg));
        html.push_str(&flow::render_svg(&fg, vw));

        // Same data → same picture: skip the DOM swap when the markup is identical
        // (e.g. a Hostname/Domain toggle that leaves the flow unchanged), so there
        // is no flash.
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            html.hash(&mut h);
            h.finish()
        }
        .to_string();
        if flow_el.get_attribute("data-sig").as_deref() == Some(sig.as_str()) {
            return;
        }
        flow_el.set_inner_html(&html);
        let _ = flow_el.set_attribute("data-sig", &sig);
    });

    Ok(())
}
