//! Saved / named views: snapshot the current range, filters, and granularity
//! under a name, then re-apply or delete them from a small manager dialog.

use super::filters::panel;
use super::{el, on, persist_ui_prefs, reload_buckets, set_text, span, Shared};
use crate::model::Granularity;
use crate::project::TimeRange;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlInputElement};

// ── saved / named views ───────────────────────────────────────────────────────

/// Human label for a time range (matches the Range control's segments).
fn range_label(r: TimeRange) -> &'static str {
    match r {
        TimeRange::Session => "Session",
        TimeRange::Day => "Day",
        TimeRange::Week => "Week",
        TimeRange::Month => "Month",
        TimeRange::Year => "Year",
    }
}

/// One-line summary of a saved view's non-default knobs, for the list row.
fn view_summary(v: &crate::views::SavedView) -> String {
    let mut parts = vec![range_label(v.range).to_string()];
    if v.gran == Granularity::Registrable {
        parts.push("domains".into());
    }
    if v.filters.min_visits > 1 {
        parts.push(format!("≥{} visits", v.filters.min_visits));
    }
    if v.filters.hide_search_hubs {
        parts.push("no search hubs".into());
    }
    if v.filters.hide_isolated {
        parts.push("no isolated".into());
    }
    if v.spa_mode {
        parts.push("in-app navs".into());
    }
    parts.join(" · ")
}

/// Snapshot the dashboard's current view controls into a named `SavedView`.
fn snapshot_view(a: &super::App, name: String) -> crate::views::SavedView {
    crate::views::SavedView {
        name,
        range: a.time_range,
        gran: a.gran,
        filters: a.filters.clone(),
        spa_mode: a.spa_mode,
    }
}

/// Apply a saved view to the live state and rebuild the projection/chrome.
fn apply_saved_view(shared: &Shared, v: &crate::views::SavedView) {
    {
        let mut a = shared.borrow_mut();
        a.time_range = v.range;
        a.gran = v.gran;
        a.filters = v.filters.clone();
        a.spa_mode = v.spa_mode;
    }
    // Applying a saved view updates the live controls, so mirror it into the
    // persisted UI prefs via the normal write-through path (§7.7, no savedViews
    // schema change).
    persist_ui_prefs(shared);
    // reload_buckets rebuilds buckets per spa_mode, recomputes the projection, and
    // rerenders (sync_chrome then reflects the new range/filters in the controls).
    reload_buckets(shared);
    if v.range == TimeRange::Session {
        super::refresh_session_buckets(shared);
    }
}

/// Open the saved-views manager (reads the persisted list async, then builds the
/// modal). Save the current view, apply one, or delete one.
pub(super) fn open_saved_views_dialog(shared: &Shared) {
    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
            .await
            .unwrap_or_default();
        build_saved_views_modal(&s, crate::views::parse(&json));
    });
}

fn build_saved_views_modal(shared: &Shared, views: Vec<crate::views::SavedView>) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if let Some(old) = doc.get_element_by_id("bg-modal") {
        old.remove();
    }

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay");
    let _ = overlay.set_attribute("id", "bg-modal");
    let modal = panel(&doc, "modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");
    let _ = modal.append_child(&span(&doc, "modal-title", "Saved views"));
    let _ = modal.append_child(&span(
        &doc,
        "modal-msg",
        "Save the current range, filters, and granularity under a name, then re-apply it anytime.",
    ));

    // Save-current row: name input + Save button.
    let saverow = el(&doc, "div");
    let _ = saverow.set_attribute("class", "modal-saverow");
    let inp = el(&doc, "input");
    let _ = inp.set_attribute("type", "text");
    let _ = inp.set_attribute("class", "modal-input");
    let _ = inp.set_attribute("id", "bg-view-name");
    let _ = inp.set_attribute("placeholder", "Name this view, e.g. Work");
    let _ = saverow.append_child(&inp);
    let save = el(&doc, "button");
    let _ = save.set_attribute("type", "button");
    let _ = save.set_attribute("class", "modal-btn modal-confirm");
    save.set_text_content(Some("Save current"));
    let _ = saverow.append_child(&save);
    let _ = modal.append_child(&saverow);

    let err = span(&doc, "modal-error", "");
    let _ = err.set_attribute("id", "bg-modal-error");
    let _ = modal.append_child(&err);

    // List of saved views (apply / delete each).
    let list = el(&doc, "div");
    let _ = list.set_attribute("class", "modal-list");
    if views.is_empty() {
        let _ = list.append_child(&span(&doc, "modal-empty", "No saved views yet."));
    } else {
        for v in &views {
            let row = el(&doc, "div");
            let _ = row.set_attribute("class", "modal-list-row");
            let applyb = el(&doc, "button");
            let _ = applyb.set_attribute("type", "button");
            let _ = applyb.set_attribute("class", "modal-list-apply");
            applyb.set_text_content(Some(&format!("{} · {}", v.name, view_summary(v))));
            {
                let s = shared.clone();
                let v = v.clone();
                let doc2 = doc.clone();
                on(&applyb, "click", move |_| {
                    apply_saved_view(&s, &v);
                    if let Some(o) = doc2.get_element_by_id("bg-modal") {
                        o.remove();
                    }
                });
            }
            let _ = row.append_child(&applyb);
            let del = el(&doc, "button");
            let _ = del.set_attribute("type", "button");
            let _ = del.set_attribute("class", "modal-list-del");
            let _ = del.set_attribute("aria-label", &format!("Delete {}", v.name));
            del.set_text_content(Some("✕"));
            {
                let s = shared.clone();
                let name = v.name.clone();
                on(&del, "click", move |_| {
                    let s2 = s.clone();
                    let name = name.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
                            .await
                            .unwrap_or_default();
                        let updated = crate::views::remove(crate::views::parse(&json), &name);
                        crate::bridge::storage_local_set(
                            crate::views::STORAGE_KEY,
                            &crate::views::serialize(&updated),
                        );
                        build_saved_views_modal(&s2, updated);
                    });
                });
            }
            let _ = row.append_child(&del);
            let _ = list.append_child(&row);
        }
    }
    let _ = modal.append_child(&list);

    let actions = el(&doc, "div");
    let _ = actions.set_attribute("class", "modal-actions");
    let close = el(&doc, "button");
    let _ = close.set_attribute("type", "button");
    let _ = close.set_attribute("class", "modal-btn");
    close.set_text_content(Some("Close"));
    let _ = actions.append_child(&close);
    let _ = modal.append_child(&actions);
    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);

    // Save the current view under the typed name.
    {
        let s = shared.clone();
        let inp = inp.clone();
        let doc2 = doc.clone();
        on(&save, "click", move |_| {
            let name = inp
                .clone()
                .dyn_into::<HtmlInputElement>()
                .map(|i| i.value())
                .unwrap_or_default();
            if name.trim().is_empty() {
                set_text(&doc2, "bg-modal-error", "Enter a name for this view.");
                return;
            }
            let view = snapshot_view(&s.borrow(), name);
            let s2 = s.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let json = crate::bridge::storage_local_get(crate::views::STORAGE_KEY)
                    .await
                    .unwrap_or_default();
                let updated = crate::views::upsert(crate::views::parse(&json), view);
                crate::bridge::storage_local_set(
                    crate::views::STORAGE_KEY,
                    &crate::views::serialize(&updated),
                );
                build_saved_views_modal(&s2, updated);
            });
        });
    }
    {
        let doc2 = doc.clone();
        on(&close, "click", move |_| {
            if let Some(o) = doc2.get_element_by_id("bg-modal") {
                o.remove();
            }
        });
    }
    {
        let doc2 = doc.clone();
        on(overlay.as_ref(), "mousedown", move |ev| {
            let on_backdrop = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .map(|e| e.id() == "bg-modal")
                .unwrap_or(false);
            if on_backdrop {
                if let Some(o) = doc2.get_element_by_id("bg-modal") {
                    o.remove();
                }
            }
        });
    }
}
