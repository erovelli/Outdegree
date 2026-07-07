//! First-run onboarding and the sample dataset (§F4): the welcome overlay, the
//! "Load sample data" / "Exit sample" flow, and the demo-mode bookkeeping.

use super::filters::panel;
use super::modal::confirm_dialog;
use super::settings::hide_nudge;
use super::{el, on, plural, reload_and_rerender, span, Shared};
use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

// ── first-run onboarding + sample dataset (§F4) ───────────────────────────────

/// Decide whether to greet a first-time user: show the welcome overlay only on a
/// truly empty, not-yet-onboarded event log. When the sample dataset is loaded the
/// "Sample data" chip (via `sync_chrome`) is the affordance instead, so the
/// overlay is skipped. Runs once on dashboard load; never blocks interaction.
pub(crate) fn evaluate_first_run(shared: &Shared) {
    if shared.borrow().demo_data {
        return; // demo mode: the chip is shown by sync_chrome; no welcome overlay
    }
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let onboarded = db.read_meta_bool("onboarded").await.unwrap_or(false);
        let events = db.count_events().await.unwrap_or(0);
        if !onboarded && events == 0 {
            show_welcome_overlay(&s);
        }
    });
}

/// One `heading + body` block of the welcome overlay.
fn welcome_section(doc: &Document, heading: &str, body: &str) -> Element {
    let sec = el(doc, "div");
    let _ = sec.set_attribute("class", "welcome-sec");
    let _ = sec.append_child(&span(doc, "welcome-head", heading));
    let _ = sec.append_child(&span(doc, "welcome-body", body));
    sec
}

/// Build and show the first-run welcome overlay (§F4): a centered glass card in
/// the dashboard's monochrome idiom with What / Privacy / Now-what sections, a
/// "Start recording" primary (onboards + dismisses; Esc does the same), and a
/// "Load sample data" secondary. Re-openable from the settings menu's
/// "Show welcome".
pub(super) fn show_welcome_overlay(shared: &Shared) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if let Some(old) = doc.get_element_by_id("bg-welcome") {
        old.remove();
    }

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay welcome-overlay");
    let _ = overlay.set_attribute("id", "bg-welcome");
    let modal = panel(&doc, "modal welcome-modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");
    let _ = modal.set_attribute("aria-label", "Welcome to Outdegree");

    let _ = modal.append_child(&span(&doc, "welcome-brand", "Outdegree"));
    let _ = modal.append_child(&welcome_section(
        &doc,
        "What",
        "Outdegree records which sites you visit and how you move between them — \
         as a graph you can explore.",
    ));
    let _ = modal.append_child(&welcome_section(
        &doc,
        "Privacy",
        "100% local. No network. Never in incognito. Pause, export, or delete \
         everything anytime from the gear menu.",
    ));
    let _ = modal.append_child(&welcome_section(
        &doc,
        "Now what",
        "Browse normally and come back later — or load sample data to explore \
         right away.",
    ));

    let actions = el(&doc, "div");
    let _ = actions.set_attribute("class", "modal-actions welcome-actions");
    let sample = el(&doc, "button");
    let _ = sample.set_attribute("type", "button");
    let _ = sample.set_attribute("class", "modal-btn");
    sample.set_text_content(Some("Load sample data"));
    {
        let s = shared.clone();
        on(&sample, "click", move |_| load_sample_data(&s));
    }
    let start = el(&doc, "button");
    let _ = start.set_attribute("type", "button");
    let _ = start.set_attribute("class", "modal-btn modal-confirm");
    start.set_text_content(Some("Start recording"));
    {
        let s = shared.clone();
        on(&start, "click", move |_| dismiss_welcome_onboarded(&s));
    }
    let _ = actions.append_child(&sample);
    let _ = actions.append_child(&start);
    let _ = modal.append_child(&actions);
    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);
}

/// Remove the welcome overlay without persisting anything (used before loading
/// sample data, which supersedes the overlay).
fn remove_welcome(shared: &Shared) {
    if let Some(o) = shared.borrow().doc.get_element_by_id("bg-welcome") {
        o.remove();
    }
}

/// Dismiss the welcome overlay as "onboarded": persist `onboarded=true` in `meta`
/// so it doesn't reappear on the next empty-log open, and remove it. Shared by the
/// "Start recording" button and Esc (§F4 step 1).
pub(super) fn dismiss_welcome_onboarded(shared: &Shared) {
    remove_welcome(shared);
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = db.write_meta_bool("onboarded", true).await;
    });
}

/// "Load sample data" (§F4 steps 3–4, 6). Loading replaces every store, so when
/// real data is present it confirms first (mirroring the Import JSON replace
/// gate); on an empty log it loads straight away.
fn load_sample_data(shared: &Shared) {
    remove_welcome(shared);
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events = db.count_events().await.unwrap_or(0) + db.count_spa().await.unwrap_or(0);
        if events > 0 {
            let msg = format!(
                "Loading the sample dataset replaces your current data ({}). This can't be \
                 undone — consider Export JSON first. You can remove the sample and get an \
                 empty dashboard back with “Exit sample”.",
                plural(events as u64, "event"),
            );
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Load sample data",
                &msg,
                None,
                "Load sample",
                true,
                None,
                move |_| {
                    do_load_sample(&s2);
                    true
                },
            );
        } else {
            do_load_sample(&s);
        }
    });
}

/// Materialize the committed fixture and route it through the import path, then
/// mark the demo, force capture paused, and re-render (§F4 steps 3–4).
fn do_load_sample(shared: &Shared) {
    remove_welcome(shared);
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        // The fixture ships schemeless with offset timestamps; materialize shifts
        // them to absolute time (relative to now) and prepends the http(s) scheme —
        // the audit-clean load path (see crate::sample).
        let fixture = crate::bridge::sample_data();
        let doc = match crate::sample::materialize(&fixture, js_sys::Date::now()) {
            Ok(j) => j,
            Err(e) => return super::log_err(&JsValue::from_str(&e)),
        };
        // Replace every store, then re-derive from the imported events.
        if let Err(e) = db.import_json(&doc).await {
            return super::log_err(&e);
        }
        if let Err(e) = db.reset_derivation().await {
            return super::log_err(&e);
        }
        // Stamp the derived-schema version the import wiped with `meta`, so the
        // next open doesn't redo the full rebuild this load already triggered (§F7).
        if let Err(e) = db.reconcile_derived_schema().await {
            return super::log_err(&e);
        }
        // Mark the demo and force the pause flag ON (string "true", flagOn
        // convention) so real navigations don't interleave with the sample.
        let _ = db.write_meta_bool("demoData", true).await;
        crate::bridge::storage_local_set("paused", "true");
        {
            let mut a = s.borrow_mut();
            a.paused = true;
            a.demo_data = true;
        }
        // An already-visible backup nudge would otherwise linger beside the
        // "Sample data" chip; demo mode suppresses it (see evaluate_backup_nudge).
        hide_nudge(&s);
        // Re-fold the imported events and re-render; sync_chrome then shows the
        // PAUSED state and the "Sample data" chip.
        reload_and_rerender(&s);
    });
}

/// "Exit sample" (§F4 step 5): confirm, wipe every store (reusing the clear path,
/// which also clears the `demoData`/`onboarded` meta flags), unpause, and
/// re-surface the welcome overlay on the now-empty dashboard.
pub(super) fn exit_sample(shared: &Shared) {
    let s = shared.clone();
    confirm_dialog(
        shared,
        "Exit sample data",
        "Remove the sample dataset and return to an empty Outdegree, ready to record your \
         own browsing. Your preferences (view settings, saved views) are kept.",
        None,
        "Exit sample",
        true,
        None,
        move |_| {
            let db = s.borrow().db.clone();
            let s2 = s.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // clear_all wipes every store, including the `demoData` and
                // `onboarded` meta flags, so the welcome overlay is eligible again.
                if let Err(e) = db.clear_all().await {
                    return super::log_err(&e);
                }
                crate::bridge::storage_local_set("paused", "false");
                {
                    let mut a = s2.borrow_mut();
                    a.paused = false;
                    a.demo_data = false;
                }
                reload_and_rerender(&s2);
                show_welcome_overlay(&s2);
            });
            true
        },
    );
}
