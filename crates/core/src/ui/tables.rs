//! Table views (§7.7, M2): hubs, top edges, origination breakdown, and a raw
//! event stream (M1).

use super::{body_container, esc, plural, Shared};
use crate::graph;
use crate::model::Event;
use crate::project;

pub(crate) fn render(shared: &Shared) -> Result<(), wasm_bindgen::JsValue> {
    let Some(body) = body_container(shared) else {
        return Ok(());
    };
    let a = shared.borrow();

    if a.proj.nodes.is_empty() {
        body.set_inner_html(
            "<div class=\"bg-empty\">No data for this range yet. Browse a bit, then reopen this dashboard.</div>",
        );
        return Ok(());
    }

    let g = graph::build(&a.proj);
    let hubs = graph::hubs(&g, 20);
    let edges = graph::top_edges(&a.proj, 20);
    let prov = project::origination(&a.buckets);

    let mut html = String::new();

    html.push_str(
        "<h3>Top hubs (by weighted degree)</h3>\
         <table class=\"tbl\"><tr><th>Host</th><th class=\"num\">Degree</th></tr>",
    );
    for (k, d) in &hubs {
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
            esc(k),
            d
        ));
    }
    html.push_str("</table>");

    html.push_str("<h3>Top edges (by weight)</h3>\
        <table class=\"tbl\"><tr><th>From</th><th>To</th><th class=\"num\">Weight</th><th>Kind</th></tr>");
    for e in &edges {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td>{:?}</td></tr>",
            esc(&e.from),
            esc(&e.to),
            e.weight,
            e.kinds.dominant()
        ));
    }
    html.push_str("</table>");

    html.push_str(
        "<h3>Origination (how pages were reached)</h3>\
        <table class=\"tbl\"><tr><th>Provenance</th><th class=\"num\">Count</th></tr>",
    );
    for (name, count) in [
        ("Link", prov.link),
        ("Form", prov.form),
        ("Typed URL", prov.typed_url),
        ("Search origin", prov.search_origin),
        ("Bookmark", prov.bookmark),
        ("Start", prov.start),
        ("Reload", prov.reload),
        ("Other", prov.other),
    ] {
        html.push_str(&format!(
            "<tr><td>{name}</td><td class=\"num\">{count}</td></tr>"
        ));
    }
    html.push_str("</table>");

    body.set_inner_html(&html);
    Ok(())
}

/// Raw event stream (M1) — an async read since events are not held in memory.
pub(crate) fn render_raw(shared: &Shared) {
    let Some(body) = body_container(shared) else {
        return;
    };
    body.set_inner_html("<div class=\"bg-empty\">Loading events…</div>");
    let db = shared.borrow().db.clone();
    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events = db.read_events_after(0.0).await.unwrap_or_default();
        let Some(body) = body_container(&s) else {
            return;
        };
        let total = events.len();
        // Only claim to truncate when we actually did (total > 1000); otherwise the
        // "showing first 1000" line contradicts a smaller, fully-shown count.
        let heading = if total > 1000 {
            format!("Raw events ({total} total, showing first 1000)")
        } else {
            format!("Raw events ({})", plural(total as u64, "event"))
        };
        let mut html = format!(
            "<h3>{heading}</h3><table><tr><th>id</th><th>kind</th><th>ts</th><th>detail</th></tr>"
        );
        for ev in events.iter().take(1000) {
            let (id, kind, ts, detail) = describe(ev);
            html.push_str(&format!(
                "<tr><td class=\"mono\">{id:.0}</td><td>{kind}</td>\
                 <td class=\"mono\" title=\"{ts:.0}\">{}</td><td>{}</td></tr>",
                esc(&fmt_ts(ts)),
                esc(&detail)
            ));
        }
        html.push_str("</table>");
        body.set_inner_html(&html);
    });
}

/// Format an epoch-millisecond timestamp as an ISO-8601 UTC string — unambiguous
/// and sortable, matching the UTC-day bucketing the rest of the app uses (the raw
/// millisecond value is kept in the cell's `title` for debugging).
fn fmt_ts(ms: f64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms))
        .to_iso_string()
        .into()
}

fn describe(ev: &Event) -> (f64, &'static str, f64, String) {
    match ev {
        Event::Nav {
            id,
            ts,
            tab_id,
            to_url,
            transition_type,
            qualifiers,
            ..
        } => (
            *id,
            "nav",
            *ts,
            format!(
                "tab {} → {} [{}{}]",
                tab_id,
                to_url,
                transition_type,
                if qualifiers.is_empty() {
                    String::new()
                } else {
                    format!(" {}", qualifiers.join(","))
                }
            ),
        ),
        Event::Link {
            id,
            ts,
            new_tab_id,
            source_tab_id,
        } => (
            *id,
            "link",
            *ts,
            format!("tab {source_tab_id} → new tab {new_tab_id}"),
        ),
        Event::Close { id, ts, tab_id } => (*id, "close", *ts, format!("tab {tab_id}")),
        Event::Start { id, ts } => (*id, "start", *ts, "browser start".into()),
    }
}
