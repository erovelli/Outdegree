//! Session picker (§7.7): lists closed + provisional-open sessions; selecting one
//! renders its per-tab flow (§4.4, sankey.rs). A GitHub-style activity heatmap
//! (53 weeks × 7 days) sits atop the list so a day can be picked directly instead
//! of scrolling months of sessions; the list is then scoped to that day. Also
//! supports a host-substring filter, a "hide 1-visit sessions" toggle, relative
//! day labels, and auto-selection of the most recent session so the flow pane is
//! never blank on open.

use super::{
    body_container, day_key_of, el, esc, local_day_key, on, persist_positions, plural,
    recompute_projection, session_day_label, session_when, Shared,
};
use crate::model::Granularity;
use std::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlInputElement};

/// Weeks (columns) in the heatmap; 53 covers a full rolling year with week
/// alignment (52×7 = 364 days plus the partial current/leading weeks), à la GitHub.
const WEEKS: i32 = 53;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
/// Left-margin weekday labels; only alternating rows are labelled (like GitHub) so
/// the tiny cells stay legible. Rows are Sun..Sat (JS `getDay`).
const WDAY_LABELS: [&str; 7] = ["", "Mon", "", "Wed", "", "Fri", ""];

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let Some(body) = body_container(shared) else {
        return Ok(());
    };
    body.set_inner_html("");
    let doc = shared.borrow().doc.clone();

    // Auto-select the most recent session so the flow pane isn't blank on open, and
    // default the heatmap to that session's day so the list opens already scoped.
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
        if a.selected_day.is_none() {
            if let Some(id) = a.selected_session {
                if let Some(sess) = a.sessions.iter().find(|s| s.id == id) {
                    a.selected_day = Some(local_day_key(sess.start_ts));
                }
            }
        }
    }

    // Vertical shell: the activity heatmap spans the top; the two-pane
    // list+flow sits below and takes the remaining height.
    let wrap = el(&doc, "div");
    let _ = wrap.set_attribute("class", "sp-wrap");
    let _ = wrap.append_child(&build_heatmap(shared));

    let container = el(&doc, "div");
    let _ = container.set_attribute("class", "sp-row");

    let list = el(&doc, "div");
    let _ = list.set_attribute("class", "sp-list");
    let heading = el(&doc, "h3");
    heading.set_text_content(Some("Sessions"));
    let _ = list.append_child(&heading);

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

    // Which day the list is scoped to (updated by `fill_items` as the heatmap
    // selection changes), so the list header names the day being browsed.
    let daylbl = el(&doc, "div");
    let _ = daylbl.set_attribute("id", "sp-day-label");
    let _ = daylbl.set_attribute("class", "sp-daylabel");
    let _ = list.append_child(&daylbl);

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
    // Delegated click: clicking a node/ribbon in the flow isolates it.
    {
        let s = shared.clone();
        on(flow.as_ref(), "click", move |ev| {
            super::sankey::on_flow_click(&s, &ev)
        });
    }

    let _ = right.append_child(&bar);
    let _ = right.append_child(&flow);

    let _ = container.append_child(&list);
    let _ = container.append_child(&right);
    let _ = wrap.append_child(&container);
    let _ = body.append_child(&wrap);

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

    let (mut sessions, query, hide_trivial, selected, selected_day) = {
        let a = shared.borrow();
        (
            a.sessions.clone(),
            a.session_query.trim().to_lowercase(),
            a.hide_trivial_sessions,
            a.selected_session,
            a.selected_day,
        )
    };
    sessions.sort_by(|a, b| {
        b.start_ts
            .partial_cmp(&a.start_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let shown: Vec<_> = sessions
        .iter()
        // Scope to the day picked in the heatmap so months of sessions collapse to
        // just the selected day; a `None` day (no sessions yet) shows everything.
        .filter(|s| selected_day.is_none_or(|k| local_day_key(s.start_ts) == k))
        .filter(|s| !(hide_trivial && s.nav_count <= 1))
        .filter(|s| {
            query.is_empty()
                || s.top_hosts
                    .iter()
                    .any(|(h, _)| h.to_lowercase().contains(&query))
        })
        .collect();

    // Header naming the browsed day + its session count (e.g. "Mon, Jul 1 · 3 sessions").
    if let Some(lbl) = doc.get_element_by_id("sp-day-label") {
        let text = match selected_day {
            Some(_) if !shown.is_empty() => format!(
                "{} · {}",
                day_full_label(shown[0].start_ts),
                plural(shown.len() as u64, "session")
            ),
            _ => String::new(),
        };
        lbl.set_text_content(Some(&text));
    }

    if shown.is_empty() {
        let empty = el(&doc, "div");
        let _ = empty.set_attribute("class", "bg-empty");
        empty.set_text_content(Some(if sessions.is_empty() {
            "No sessions yet."
        } else if selected_day.is_some() && query.is_empty() && !hide_trivial {
            "No sessions on this day. Pick another day above."
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

/// Full, unambiguous day label for a timestamp: `"Mon, Jul 1"` (local time). Used
/// in the scoped-list header and the heatmap cell tooltips, where the relative
/// `Today`/weekday label of [`session_day_label`] would be ambiguous a year back.
fn day_full_label(ts: f64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ts));
    format!(
        "{}, {} {}",
        WDAYS[d.get_day() as usize],
        MONTHS[d.get_month() as usize],
        d.get_date()
    )
}

/// Build the GitHub-style activity heatmap: a `WEEKS × 7` grid of the trailing
/// year, each cell an intensity-shaded day. Clicking a day with sessions scopes
/// the list to it. Intensity is the day's visit total binned into quartiles of the
/// busiest day, so the ramp adapts to the user's own volume (§7.7).
fn build_heatmap(shared: &Shared) -> Element {
    let doc = shared.borrow().doc.clone();

    // Per-day (sessions, visits), the current selection, and the busiest day's
    // visit total (the quartile scale) — all read under one short borrow.
    let (counts, selected_day, max_visits) = {
        let a = shared.borrow();
        let mut counts: HashMap<i64, (u32, u32)> = HashMap::new();
        for s in &a.sessions {
            let e = counts.entry(local_day_key(s.start_ts)).or_insert((0, 0));
            e.0 += 1;
            e.1 += s.nav_count;
        }
        let max = counts.values().map(|(_, v)| *v).max().unwrap_or(0).max(1);
        (counts, a.selected_day, max)
    };

    let cal = el(&doc, "div");
    let _ = cal.set_attribute("class", "cal");
    let grid = el(&doc, "div");
    let _ = grid.set_attribute("class", "cal-grid");

    // The grid is one CSS row of month labels, then 7 weekday rows, each row
    // led by a weekday-label cell. Cells are appended row-major to match the
    // grid's default (row) auto-placement over `repeat(WEEKS, …)` columns.
    let now = js_sys::Date::new_0();
    let today_key = day_key_of(&now);
    // Start at the Sunday `WEEKS-1` weeks before this week's Sunday. `Date`'s
    // year/month/day constructor normalizes the out-of-range day arithmetic
    // (and DST) for us — no fragile epoch-offset math.
    let start = js_sys::Date::new_with_year_month_day(
        now.get_full_year(),
        now.get_month() as i32,
        now.get_date() as i32 - now.get_day() as i32 - (WEEKS - 1) * 7,
    );
    let (sy, sm, sd) = (
        start.get_full_year(),
        start.get_month() as i32,
        start.get_date() as i32,
    );
    let cell_date = |offset: i32| js_sys::Date::new_with_year_month_day(sy, sm, sd + offset);

    // Month-label row: a leading corner spacer, then one cell per week carrying
    // the month name only where the month changes (labels overflow to the right).
    let corner = el(&doc, "div");
    let _ = corner.set_attribute("class", "cal-corner");
    let _ = grid.append_child(&corner);
    let mut prev_month = -1i32;
    for c in 0..WEEKS {
        let d = cell_date(c * 7); // Sunday (row 0) of this week
        let cell = el(&doc, "div");
        let _ = cell.set_attribute("class", "cal-month");
        let m = d.get_month() as i32;
        if m != prev_month {
            cell.set_text_content(Some(MONTHS[m as usize]));
            prev_month = m;
        }
        let _ = grid.append_child(&cell);
    }

    for r in 0..7 {
        let lbl = el(&doc, "div");
        let _ = lbl.set_attribute("class", "cal-wday");
        lbl.set_text_content(Some(WDAY_LABELS[r as usize]));
        let _ = grid.append_child(&lbl);

        for c in 0..WEEKS {
            let d = cell_date(c * 7 + r);
            let key = day_key_of(&d);
            let cell = el(&doc, "div");

            if key > today_key {
                // Future days in the current/leading weeks: keep the slot, no dot.
                let _ = cell.set_attribute("class", "cal-cell cal-future");
                let _ = grid.append_child(&cell);
                continue;
            }

            let (sess_n, visits) = counts.get(&key).copied().unwrap_or((0, 0));
            let lvl = level(visits, max_visits);
            let mut cls = format!("cal-cell cal-l{lvl}");
            if sess_n > 0 {
                cls.push_str(" is-hit"); // clickable (has sessions)
            }
            if sess_n > 0 && selected_day == Some(key) {
                cls.push_str(" is-sel");
            }
            let _ = cell.set_attribute("class", &cls);

            let day = day_full_label(d.get_time());
            let tip = if sess_n > 0 {
                format!(
                    "{day} · {} · {}",
                    plural(sess_n as u64, "session"),
                    plural(visits as u64, "visit")
                )
            } else {
                format!("{day} · no sessions")
            };
            let _ = cell.set_attribute("title", &tip);

            if sess_n > 0 {
                let s = shared.clone();
                on(cell.as_ref(), "click", move |_| select_day(&s, key));
            }
            let _ = grid.append_child(&cell);
        }
    }

    let _ = cal.append_child(&grid);
    let _ = cal.append_child(&build_scale(&doc));
    cal
}

/// Quartile bin (0 empty, 1–4) of a day's `visits` against the busiest day.
fn level(visits: u32, max: u32) -> u8 {
    if visits == 0 {
        return 0;
    }
    let f = visits as f64 / max as f64;
    if f > 0.75 {
        4
    } else if f > 0.5 {
        3
    } else if f > 0.25 {
        2
    } else {
        1
    }
}

/// The "Less … More" intensity legend shown under the grid.
fn build_scale(doc: &Document) -> Element {
    let scale = el(doc, "div");
    let _ = scale.set_attribute("class", "cal-scale");
    let less = el(doc, "span");
    less.set_text_content(Some("Less"));
    let _ = scale.append_child(&less);
    for lvl in 0..=4 {
        let sw = el(doc, "div");
        let _ = sw.set_attribute("class", &format!("cal-cell cal-l{lvl}"));
        let _ = scale.append_child(&sw);
    }
    let more = el(doc, "span");
    more.set_text_content(Some("More"));
    let _ = scale.append_child(&more);
    scale
}

/// Scope the session list to `key` (a [`local_day_key`]) and auto-select that
/// day's most recent session, then re-render the picker so the heatmap highlight,
/// list, and flow all follow. Re-rendering (vs. patching cells) keeps this simple
/// and avoids a `NodeList` dependency for the highlight swap.
fn select_day(shared: &Shared, key: i64) {
    {
        let mut a = shared.borrow_mut();
        a.selected_day = Some(key);
        a.selected_session = a
            .sessions
            .iter()
            .filter(|s| local_day_key(s.start_ts) == key)
            .max_by(|x, y| {
                x.start_ts
                    .partial_cmp(&y.start_ts)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.id);
    }
    let _ = render(shared);
}
