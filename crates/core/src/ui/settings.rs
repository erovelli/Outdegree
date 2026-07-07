//! Settings gear popover and data stewardship (§8): exports (JSON/CSV/PNG/SVG),
//! import, forget-domain / delete-range / delete-all / rebuild, the read-only
//! storage readout, and the dismissible backup nudge.

use super::filters::{menu_btn, menu_toggle, panel};
use super::modal::confirm_dialog;
use super::onboarding::show_welcome_overlay;
use super::saved_views::open_saved_views_dialog;
use super::{
    el, on, persist_ui_prefs, plural, reload_and_rerender, reload_buckets, rerender, set_text,
    span, Shared, View,
};
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlInputElement};

// ── 8. settings popover (hidden until the gear is clicked) ────────────────────
pub(super) fn settings_popover(doc: &Document, shared: &Shared) -> Element {
    let pop = panel(doc, "popover at-pop");
    let _ = pop.set_attribute("id", "bg-settings");

    // Reflect the restored in-app-navigations mode (§7.7) so the checkbox matches
    // the graph it produced on first open, rather than a stale default.
    let (spa_row, spa_input) = menu_toggle(doc, "In-app navigations", shared.borrow().spa_mode);
    {
        let s = shared.clone();
        on(&spa_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().spa_mode = c;
            persist_ui_prefs(&s);
            reload_buckets(&s);
        });
    }

    // Opt-in, default off: surface search terms parsed from already-captured
    // result URLs. Reflects the persisted choice and writes it back on change.
    let (search_row, search_input) =
        menu_toggle(doc, "Show search terms", shared.borrow().show_searches);
    {
        let s = shared.clone();
        on(&search_input, "change", move |ev| {
            let c = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.checked())
                .unwrap_or(false);
            s.borrow_mut().show_searches = c;
            crate::bridge::storage_local_set("showSearches", if c { "true" } else { "false" });
            super::reload_searches(&s);
        });
    }

    let raw = menu_btn(doc, "Raw events");
    {
        let s = shared.clone();
        on(&raw, "click", move |_| {
            s.borrow_mut().view = View::Raw;
            close_popover(&s);
            let _ = rerender(&s);
        });
    }

    let saved_views = menu_btn(doc, "Saved views…");
    {
        let s = shared.clone();
        on(&saved_views, "click", move |_| {
            close_popover(&s);
            open_saved_views_dialog(&s);
        });
    }

    let export = menu_btn(doc, "Export JSON");
    {
        let s = shared.clone();
        on(&export, "click", move |_| {
            close_popover(&s);
            run_json_export(&s);
        });
    }

    let export_csv = menu_btn(doc, "Export tables (CSV)");
    {
        let s = shared.clone();
        on(&export_csv, "click", move |_| {
            close_popover(&s);
            let csv = super::tables::tables_csv(&s.borrow());
            crate::bridge::download_text("outdegree-tables.csv", "text/csv", &csv);
        });
    }

    let export_png = menu_btn(doc, "Export graph (PNG)");
    {
        let s = shared.clone();
        on(&export_png, "click", move |_| {
            close_popover(&s);
            super::graph_view::export_png(&s);
        });
    }

    let export_svg = menu_btn(doc, "Export graph (SVG)");
    {
        let s = shared.clone();
        on(&export_svg, "click", move |_| {
            close_popover(&s);
            super::graph_view::export_svg(&s);
        });
    }

    let import = menu_btn(doc, "Import JSON");
    {
        let s = shared.clone();
        on(&import, "click", move |_| {
            close_popover(&s);
            let doc = s.borrow().doc.clone();
            let Ok(el) = doc.create_element("input") else {
                return;
            };
            let _ = el.set_attribute("type", "file");
            let _ = el.set_attribute("accept", "application/json,.json");
            let Ok(inp) = el.dyn_into::<HtmlInputElement>() else {
                return;
            };
            let s2 = s.clone();
            let picker = inp.clone();
            on(&inp, "change", move |_| {
                let Some(file) = picker.files().and_then(|f| f.get(0)) else {
                    return;
                };
                let s3 = s2.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Read the picked file first (harmless — it touches no store), then
                    // confirm before replacing anything, so Cancel truly aborts (§8).
                    let json = match wasm_bindgen_futures::JsFuture::from(file.text()).await {
                        Ok(v) => v.as_string().unwrap_or_default(),
                        Err(e) => return super::log_err(&e),
                    };
                    let db = s3.borrow().db.clone();
                    let events =
                        db.count_events().await.unwrap_or(0) + db.count_spa().await.unwrap_or(0);
                    let msg = format!(
                        "Importing replaces your current data ({}). This can't be undone — \
                         consider Export JSON first.",
                        plural(events as u64, "event"),
                    );
                    let s4 = s3.clone();
                    confirm_dialog(
                        &s3,
                        "Import JSON",
                        &msg,
                        None,
                        "Import",
                        true,
                        None,
                        move |_| {
                            let json = json.clone();
                            let s5 = s4.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                let db = s5.borrow().db.clone();
                                // Replace every store, then re-derive from the imported
                                // events so the view is consistent even if the file only
                                // carried `events`.
                                if let Err(e) = db.import_json(&json).await {
                                    return super::log_err(&e);
                                }
                                if let Err(e) = db.reset_derivation().await {
                                    return super::log_err(&e);
                                }
                                reload_and_rerender(&s5);
                            });
                            true
                        },
                    );
                });
            });
            inp.click();
        });
    }

    let forget = menu_btn(doc, "Forget domain…");
    {
        let s = shared.clone();
        on(&forget, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Forget a domain",
                "Permanently remove every stored record for this host or domain, then rebuild. \
                 This can't be undone.",
                Some("host or domain, e.g. example.com"),
                "Forget",
                true,
                None,
                move |val| {
                    let domain = val.unwrap_or_default().trim().to_string();
                    if domain.is_empty() {
                        set_text(
                            &s2.borrow().doc,
                            "bg-modal-error",
                            "Enter a host or domain.",
                        );
                        return false;
                    }
                    let db = s2.borrow().db.clone();
                    let s3 = s2.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = db.forget_domain(&domain).await {
                            return super::log_err(&e);
                        }
                        reload_and_rerender(&s3);
                    });
                    true
                },
            );
        });
    }

    let delete = menu_btn(doc, "Delete last N days…");
    {
        let s = shared.clone();
        on(&delete, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Delete recent history",
                "Permanently remove all records from the last N days, then rebuild. \
                 This can't be undone.",
                Some("number of days, e.g. 7"),
                "Delete",
                true,
                None,
                move |val| {
                    // Validate: a whole number of days in a sane range — so a stray
                    // "99999" can't silently wipe everything and "-3" can't no-op.
                    match val.unwrap_or_default().trim().parse::<u32>() {
                        Ok(days) if (1..=3650).contains(&days) => {
                            let now = js_sys::Date::now();
                            let from = now - days as f64 * 86_400_000.0;
                            let db = s2.borrow().db.clone();
                            let s3 = s2.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Err(e) = db.delete_range(from, now).await {
                                    return super::log_err(&e);
                                }
                                reload_and_rerender(&s3);
                            });
                            true
                        }
                        _ => {
                            set_text(
                                &s2.borrow().doc,
                                "bg-modal-error",
                                "Enter a whole number of days between 1 and 3650.",
                            );
                            false
                        }
                    }
                },
            );
        });
    }

    // The nuclear option: wipe every IndexedDB store. Gated behind typing DELETE
    // so it can't be triggered by a stray click (§8). Preferences survive.
    let delete_all = menu_btn(doc, "Delete all data…");
    {
        let s = shared.clone();
        on(&delete_all, "click", move |_| {
            close_popover(&s);
            let s2 = s.clone();
            confirm_dialog(
                &s,
                "Delete all data",
                "Permanently erase every stored navigation, rollup, session, and saved \
                 layout. Your preferences (pause, view settings, saved views) are kept. \
                 This can't be undone. Type DELETE to confirm.",
                Some("type DELETE"),
                "Delete everything",
                true,
                Some("DELETE"),
                move |val| {
                    // The typed-phrase gate already guards the button; re-check here so
                    // the action itself is never reachable without the exact word.
                    if val.unwrap_or_default().trim() != "DELETE" {
                        set_text(
                            &s2.borrow().doc,
                            "bg-modal-error",
                            "Type DELETE to confirm.",
                        );
                        return false;
                    }
                    let db = s2.borrow().db.clone();
                    let s3 = s2.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = db.clear_all().await {
                            return super::log_err(&e);
                        }
                        // Everything is gone; re-derive (a no-op over an empty log) and
                        // rerender to land on the empty state that captures anew.
                        reload_and_rerender(&s3);
                    });
                    true
                },
            );
        });
    }

    // Recovery: clear the derived cache + cursor and re-derive everything from the
    // raw event log (fixes a derivation cursor that has drifted past the events).
    let rebuild = menu_btn(doc, "Rebuild from raw events");
    {
        let s = shared.clone();
        on(&rebuild, "click", move |_| {
            close_popover(&s);
            let db = s.borrow().db.clone();
            let s2 = s.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = db.reset_derivation().await {
                    return super::log_err(&e);
                }
                reload_and_rerender(&s2);
            });
        });
    }

    // Re-open the first-run welcome overlay on demand (§F4), so the "what/privacy/
    // load sample data" intro (and the sample-data loader) stays reachable after
    // the first run.
    let welcome = menu_btn(doc, "Show welcome");
    {
        let s = shared.clone();
        on(&welcome, "click", move |_| {
            close_popover(&s);
            show_welcome_overlay(&s);
        });
    }

    // Read-only storage usage readout atop the Data section — event/rollup/session
    // counts plus an approximate byte figure from navigator.storage.estimate()
    // (a local API, CSP-safe). Filled/refreshed each time the popover opens.
    let storage_line = el(doc, "div");
    let _ = storage_line.set_attribute("class", "menu-readout");
    let _ = storage_line.set_attribute("id", "bg-storage-line");
    storage_line.set_text_content(Some("Storage · …"));

    let _ = pop.append_child(&spa_row);
    let _ = pop.append_child(&search_row);
    let sep = el(doc, "div");
    let _ = sep.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep);
    let _ = pop.append_child(&raw);
    let _ = pop.append_child(&saved_views);
    let sep2 = el(doc, "div");
    let _ = sep2.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep2);
    let _ = pop.append_child(&storage_line);
    let _ = pop.append_child(&export);
    let _ = pop.append_child(&export_csv);
    let _ = pop.append_child(&export_png);
    let _ = pop.append_child(&export_svg);
    let _ = pop.append_child(&import);
    let _ = pop.append_child(&forget);
    let _ = pop.append_child(&delete);
    let _ = pop.append_child(&delete_all);
    let _ = pop.append_child(&rebuild);
    let sep3 = el(doc, "div");
    let _ = sep3.set_attribute("class", "menu-sep");
    let _ = pop.append_child(&sep3);
    let _ = pop.append_child(&welcome);
    pop
}

pub(super) fn close_popover(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    if let Some(pop) = doc.get_element_by_id("bg-settings") {
        pop.set_class_name("panel popover at-pop");
    }
    if let Some(g) = doc.get_element_by_id("bg-gear") {
        let _ = g.set_attribute("aria-expanded", "false");
    }
}

// ── data stewardship: storage readout · export · backup nudge (§8) ────────────

/// Run the JSON export: download the file, stamp `lastExportTs` into `meta` so the
/// backup nudge resets, and hide the nudge chip. Shared by the settings menu's
/// "Export JSON" and the backup nudge's "Export now" — a *download*, never an
/// upload (§7.8 / §12.1). DO NOT add a network sink here.
fn run_json_export(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match db.export_json().await {
            Ok(json) => {
                crate::bridge::download_json("outdegree-export.json", &json);
                // Stamp the backup so the nudge decision resets (best-effort).
                let _ = db.write_last_export_ts(js_sys::Date::now()).await;
                hide_nudge(&s);
            }
            Err(e) => super::log_err(&e),
        }
    });
}

/// The backup-nudge chip (near the settings gear, top-right): a dismissible glass
/// chip nudging a local backup, with "Export now" / "Snooze" actions (§8.4). Built
/// hidden; [`evaluate_backup_nudge`] reveals it only when the pure decision says so.
pub(super) fn nudge_panel(doc: &Document, shared: &Shared) -> Element {
    let p = panel(doc, "nudge at-nudge");
    let _ = p.set_attribute("id", "bg-nudge");
    let _ = p.set_attribute("role", "status");

    let msg = span(
        doc,
        "nudge-msg",
        "It's been a while since your last backup.",
    );
    let _ = p.append_child(&msg);

    let actions = el(doc, "div");
    let _ = actions.set_attribute("class", "nudge-actions");

    let export_now = el(doc, "button");
    let _ = export_now.set_attribute("type", "button");
    let _ = export_now.set_attribute("class", "nudge-btn nudge-primary");
    export_now.set_text_content(Some("Export now"));
    {
        let s = shared.clone();
        on(&export_now, "click", move |_| run_json_export(&s));
    }

    let snooze = el(doc, "button");
    let _ = snooze.set_attribute("type", "button");
    let _ = snooze.set_attribute("class", "nudge-btn");
    snooze.set_text_content(Some("Snooze"));
    {
        let s = shared.clone();
        on(&snooze, "click", move |_| snooze_nudge(&s));
    }

    let dismiss = el(doc, "button");
    let _ = dismiss.set_attribute("type", "button");
    let _ = dismiss.set_attribute("class", "nudge-x");
    let _ = dismiss.set_attribute("aria-label", "Dismiss");
    dismiss.set_text_content(Some("✕"));
    {
        // A bare dismiss hides it for this session only (no persistence) — a
        // lighter touch than Snooze, which suppresses it for 30 days.
        let s = shared.clone();
        on(&dismiss, "click", move |_| hide_nudge(&s));
    }

    let _ = actions.append_child(&export_now);
    let _ = actions.append_child(&snooze);
    let _ = actions.append_child(&dismiss);
    let _ = p.append_child(&actions);
    p
}

fn show_nudge(shared: &Shared) {
    if let Some(el) = shared.borrow().doc.get_element_by_id("bg-nudge") {
        el.set_class_name("panel nudge at-nudge show");
    }
}

pub(super) fn hide_nudge(shared: &Shared) {
    if let Some(el) = shared.borrow().doc.get_element_by_id("bg-nudge") {
        el.set_class_name("panel nudge at-nudge");
    }
}

/// "Snooze": suppress the backup nudge for 30 days by writing `nudgeSnoozedUntil`
/// to `meta`, then hide the chip (§8.4).
fn snooze_nudge(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let until = js_sys::Date::now() + crate::stewardship::SNOOZE_MS;
        let _ = db.write_nudge_snoozed_until(until).await;
        hide_nudge(&s);
    });
}

/// Evaluate the backup nudge on dashboard load and reveal the chip only when the
/// pure decision ([`crate::stewardship::nudge_state`]) says [`Nudge::Show`] (§8.4):
/// enough real events, no recent backup, and no active snooze. The event-count
/// gate keeps it off the empty/no-data state; it never blocks interaction.
pub(crate) fn evaluate_backup_nudge(shared: &Shared) {
    // Suppress the nudge entirely while the onboarding sample dataset is loaded:
    // those synthetic events aren't the user's data to back up (§F4 owns this
    // suppression, keeping the >5,000-event gate from tripping on the demo).
    if shared.borrow().demo_data {
        hide_nudge(shared);
        return;
    }
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events =
            db.count_events().await.unwrap_or(0) as u64 + db.count_spa().await.unwrap_or(0) as u64;
        let last_export = db.read_last_export_ts().await.unwrap_or(None);
        let snoozed = db.read_nudge_snoozed_until().await.unwrap_or(None);
        let now = js_sys::Date::now();
        if crate::stewardship::nudge_state(events, last_export, snoozed, now)
            == crate::stewardship::Nudge::Show
        {
            show_nudge(&s);
        } else {
            hide_nudge(&s);
        }
    });
}

/// Refresh the settings "Storage" readout: event/rollup/session record counts plus
/// an approximate byte figure from `navigator.storage.estimate()` (§8.1). When the
/// estimate is unavailable or rejects, the byte suffix is simply omitted.
pub(super) fn refresh_storage_readout(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events = db.count_events().await.unwrap_or(0) as u64;
        let spa = db.count_spa().await.unwrap_or(0) as u64;
        let rollups = db.count_rollup_days().await.unwrap_or(0) as u64;
        let sessions = db.count_sessions().await.unwrap_or(0) as u64;
        let mut text = format!(
            "Storage · {} · {} · {}",
            plural(events + spa, "event"),
            plural(rollups, "day rollup"),
            plural(sessions, "session"),
        );
        if let Some(bytes) = estimate_usage_bytes().await {
            text.push_str(&format!(" · ~{}", crate::stewardship::human_bytes(bytes)));
        }
        set_text(&s.borrow().doc, "bg-storage-line", &text);
    });
}

/// Approximate bytes used by the origin, via `navigator.storage.estimate()` — a
/// local StorageManager API (no network). `None` if the API is missing or the
/// estimate rejects, so the readout falls back to counts only (§8.1).
async fn estimate_usage_bytes() -> Option<u64> {
    let storage = web_sys::window()?.navigator().storage();
    let promise = storage.estimate().ok()?;
    let estimate = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    let estimate: web_sys::StorageEstimate = estimate.unchecked_into();
    estimate.get_usage().map(|u| u as u64)
}
