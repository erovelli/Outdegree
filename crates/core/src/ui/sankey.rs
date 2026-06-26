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

/// The "Direct visits" list: orphan hosts (reached by typing/bookmark/search and
/// linking nowhere) as provenance-glyph chips, so they're acknowledged without
/// cluttering the flow diagram with lone column-0 bars.
fn direct_visits_html(orphans: &[(String, u32)], fg: &flow::FlowGraph) -> String {
    use std::collections::HashMap;
    if orphans.is_empty() {
        return String::new();
    }
    let prov_of: HashMap<&str, crate::model::Provenance> =
        fg.nodes.iter().map(|n| (n.key.as_str(), n.prov)).collect();
    let mut h =
        String::from("<div class=\"direct-visits\"><h4>Direct visits</h4><div class=\"dv-chips\">");
    for (host, count) in orphans {
        let prov = prov_of
            .get(host.as_str())
            .copied()
            .unwrap_or(crate::model::Provenance::Other);
        let dot = match prov {
            crate::model::Provenance::SearchOrigin => "dot-search",
            crate::model::Provenance::Link => "dot-link",
            crate::model::Provenance::TypedUrl => "dot-typed",
            crate::model::Provenance::Bookmark => "dot-bookmark",
            crate::model::Provenance::Form => "dot-form",
            crate::model::Provenance::Start => "dot-external",
            _ => "dot-other",
        };
        let suffix = if *count > 1 {
            format!(" ×{count}")
        } else {
            String::new()
        };
        h.push_str(&format!(
            "<span class=\"dv-chip\"><span class=\"dot {dot} {glyph}\"></span>{}{suffix}</span>",
            super::esc(host),
            glyph = prov.shape().css()
        ));
    }
    h.push_str("</div></div>");
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
        let fg_full = flow::from_session_events(&events, gran);
        // Split the hosts you actually linked between from the direct visits
        // (typed/bookmark/search landings that flow nowhere), so the diagram isn't
        // mostly empty column-0 bars — the direct visits become a side list.
        let (fg, orphans) = flow::split_orphans(&fg_full);

        let Some(flow_el) = s.borrow().doc.get_element_by_id("bg-flow") else {
            return;
        };
        let vw = (flow_el.client_width() as f64 - 8.0).max(560.0);
        let has_flow = !fg.nodes.is_empty();
        let hint = if has_flow {
            " Click a site or ribbon to isolate its flow."
        } else {
            ""
        };
        let mut html = format!(
            "<h3>{}</h3>\
             <p class=\"muted\">{}. Each column is a site you visited; a thicker ribbon means \
             you took that step more often.{hint}</p>",
            super::session_when(sess.start_ts, sess.end_ts),
            super::plural(sess.nav_count as u64, "visit"),
        );
        html.push_str(&keys_html(&fg_full));
        // A focus set by clicking should survive a re-render the user didn't ask
        // for (the 15s live-refresh poll). Keep it only if it still points at the
        // SAME hosts in the freshly-built flow — bounds-check the cached (up, down)
        // indices and verify the node keys (and, for an edge, the link) are
        // unchanged; otherwise the flow reshaped (new events) and we drop it.
        let retained_focus = {
            let a = s.borrow();
            match (a.sankey_focus, a.sankey_flow.as_ref()) {
                (Some((up, down)), Some(old)) => {
                    let n = fg.nodes.len();
                    let key =
                        |g: &flow::FlowGraph, i: usize| g.nodes.get(i).map(|nd| nd.key.clone());
                    let same = up < n
                        && down < n
                        && key(old, up) == key(&fg, up)
                        && key(old, down) == key(&fg, down)
                        && (up == down || fg.links.iter().any(|l| l.from == up && l.to == down));
                    same.then_some((up, down))
                }
                _ => None,
            }
        };
        let focus_set = retained_focus.map(|(up, down)| flow::flow_focus(&fg, up, down));

        // The diagram lives in its own container so a focus click can re-render
        // just the SVG (from the cached flow) without rebuilding the header/keys.
        html.push_str("<div id=\"bg-flow-svg\">");
        if has_flow {
            html.push_str(&flow::render_svg(&fg, vw, focus_set.as_ref()));
        } else {
            html.push_str(
                "<p class=\"muted\">No links were followed — every visit this session was direct \
                 (typed, bookmark, or search).</p>",
            );
        }
        html.push_str("</div>");
        html.push_str(&direct_visits_html(&orphans, &fg_full));

        // Cache the drawn flow so clicks can re-render with a focus highlight, and
        // carry a still-valid focus across a poll-driven re-render.
        {
            let mut a = s.borrow_mut();
            a.sankey_flow = Some(fg.clone());
            a.sankey_focus = retained_focus;
            a.sankey_vw = vw;
        }

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

/// Click handler for the flow pane: clicking a node (or ribbon) isolates the flow
/// threading through it — its ancestors and descendants stay lit, the rest dims.
/// Clicking the same point again, or empty space, clears the focus. Re-renders
/// only the cached SVG (no event re-read).
pub(crate) fn on_flow_click(shared: &Shared, ev: &web_sys::Event) {
    use wasm_bindgen::JsCast;
    // Resolve the clicked element to a node index or a link index, if any.
    let target = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    let seed = target.and_then(|el| {
        if let Ok(Some(node)) = el.closest("[data-node]") {
            node.get_attribute("data-node")
                .and_then(|v| v.parse::<usize>().ok())
                .map(|i| (i, i))
        } else if let Ok(Some(link)) = el.closest("[data-link]") {
            link.get_attribute("data-link")
                .and_then(|v| v.parse::<usize>().ok())
                .and_then(|li| {
                    shared
                        .borrow()
                        .sankey_flow
                        .as_ref()
                        .and_then(|fg| fg.links.get(li).map(|l| (l.from, l.to)))
                })
        } else {
            None
        }
    });

    // Toggle: clicking the focused point again clears; a new point replaces; a
    // click on empty space clears.
    let (focus, fg, vw) = {
        let mut a = shared.borrow_mut();
        let new_focus = match (seed, a.sankey_focus) {
            (Some(s), Some(cur)) if s == cur => None,
            (s, _) => s,
        };
        a.sankey_focus = new_focus;
        (new_focus, a.sankey_flow.clone(), a.sankey_vw)
    };
    let Some(fg) = fg else { return };

    let svg = match focus {
        Some((up, down)) => {
            let f = crate::flow::flow_focus(&fg, up, down);
            crate::flow::render_svg(&fg, vw, Some(&f))
        }
        None => crate::flow::render_svg(&fg, vw, None),
    };
    let doc = shared.borrow().doc.clone();
    if let Some(holder) = doc.get_element_by_id("bg-flow-svg") {
        holder.set_inner_html(&svg);
    }
    // The cached `data-sig` on #bg-flow no longer matches the visible DOM (we just
    // changed it out of band); clear it so the next data render always re-swaps.
    if let Some(flow_el) = doc.get_element_by_id("bg-flow") {
        let _ = flow_el.remove_attribute("data-sig");
    }
}
