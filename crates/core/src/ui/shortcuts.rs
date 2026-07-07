//! Global keyboard + click-away handling: ⌘K/Ctrl-K focuses the host search box,
//! Esc dismisses the modal/welcome/menu and exits drill-down focus, and a
//! mousedown outside the settings popover closes it.

use super::chrome::set_legend;
use super::onboarding::dismiss_welcome_onboarded;
use super::settings::close_popover;
use super::{graph_view, help, modal, on, Shared};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, KeyboardEvent};

/// ⌘K / Ctrl-K focuses the host search box (command-palette affordance); Esc
/// closes the settings menu and exits any drill-down focus.
pub(super) fn install_palette_shortcut(shared: &Shared) {
    let Some(win) = web_sys::window() else { return };
    let s = shared.clone();
    on(win.as_ref(), "keydown", move |ev| {
        let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
            return;
        };
        if ke.key() == "Escape" {
            // Esc peels exactly one layer, in strict priority order (§F10):
            //   modal  >  welcome overlay  >  help overlay  >  settings popover +
            //   drill-down focus / legend filter  >  release graph-canvas focus.
            // Each earlier layer swallows the press so one Esc unwinds one thing.
            let doc = s.borrow().doc.clone();
            if doc.get_element_by_id("bg-modal").is_some() {
                // Close the modal and return focus to its invoking control.
                modal::close_modal(&doc);
                return;
            }
            // The welcome overlay dismisses like its "Start recording" button:
            // Esc onboards and closes it (§F4).
            if doc.get_element_by_id("bg-welcome").is_some() {
                dismiss_welcome_onboarded(&s);
                return;
            }
            // The help overlay closes (restoring focus) before touching the view.
            if doc.get_element_by_id("bg-help").is_some() {
                help::close_help(&doc);
                return;
            }
            close_popover(&s);
            let cleared = {
                let mut a = s.borrow_mut();
                a.legend_filter.take().is_some()
            };
            if s.borrow().focus.is_some() {
                super::focus_and_animate(&s, None); // re-renders: rebuilds legend + canvas
            } else if cleared {
                set_legend(&s);
                graph_view::redraw(&s);
            } else if let Some(active) = doc.active_element() {
                // Nothing left to peel: release keyboard focus from the graph
                // canvas so Esc still "clears" a focused graph with no node
                // selected (matching the canvas's advertised "Esc clear focus").
                if active.id() == "bg-canvas" {
                    if let Ok(h) = active.dyn_into::<HtmlElement>() {
                        let _ = h.blur();
                    }
                }
            }
            return;
        }
        // "?" opens (or re-closes) the help overlay — a terse shortcut reference.
        // Ignored while typing, and while a modal / welcome overlay is up (those
        // own Esc and shouldn't be shadowed by a help card).
        if ke.key() == "?" && !is_text_target(&ke) {
            let doc = s.borrow().doc.clone();
            if doc.get_element_by_id("bg-modal").is_some()
                || doc.get_element_by_id("bg-welcome").is_some()
            {
                return;
            }
            ke.prevent_default();
            help::toggle_help(&s);
            return;
        }
        // ArrowLeft / ArrowRight step time navigation (§F6): back / forward one
        // range duration (or session). Ignored while typing in a field, while a
        // modal or the welcome overlay is up, and on the Sessions (Sankey) view —
        // where the range control is hidden and the picker owns selection.
        if ke.key() == "ArrowLeft" || ke.key() == "ArrowRight" {
            // Yield to the graph canvas when it holds focus: there the arrows pan
            // (see graph_view), so the global handler must not also step the time
            // window — canvas-focused = pan, otherwise = time step (§F10).
            if ke.meta_key()
                || ke.ctrl_key()
                || ke.alt_key()
                || is_text_target(&ke)
                || is_canvas_target(&ke)
            {
                return;
            }
            let (doc, on_sankey) = {
                let a = s.borrow();
                (a.doc.clone(), a.view == super::View::Sankey)
            };
            if on_sankey
                || doc.get_element_by_id("bg-modal").is_some()
                || doc.get_element_by_id("bg-welcome").is_some()
            {
                return;
            }
            ke.prevent_default();
            super::chrome::step_range(&s, ke.key() == "ArrowRight");
            return;
        }
        if (ke.meta_key() || ke.ctrl_key()) && ke.key().eq_ignore_ascii_case("k") {
            ke.prevent_default();
            if let Some(inp) = s.borrow().doc.get_element_by_id("bg-search") {
                if let Ok(h) = inp.dyn_into::<HtmlElement>() {
                    let _ = h.focus();
                }
            }
        }
    });
}

/// Whether a key event targets an editable field (input / textarea / select /
/// content-editable), so global shortcuts don't fire while the user is typing
/// or operating a control that owns its own arrow keys.
fn is_text_target(ke: &KeyboardEvent) -> bool {
    ke.target()
        .and_then(|t| t.dyn_into::<HtmlElement>().ok())
        .map(|el| {
            let tag = el.tag_name().to_ascii_lowercase();
            // <select> included: a focused dropdown cycles options (or opens its
            // popup) with arrows — those must not step the time window instead.
            tag == "input" || tag == "textarea" || tag == "select" || el.is_content_editable()
        })
        .unwrap_or(false)
}

/// Whether a key event targets the graph canvas (which owns its own arrow-key
/// panning while focused), so the global arrow-key time-stepping yields to it.
fn is_canvas_target(ke: &KeyboardEvent) -> bool {
    ke.target()
        .and_then(|t| t.dyn_into::<Element>().ok())
        .map(|e| e.id() == "bg-canvas")
        .unwrap_or(false)
}

/// Dismiss the settings popover on a mousedown outside it (and outside the gear
/// that toggles it) — the expected "click-away closes the menu" behavior.
pub(super) fn install_popover_dismiss(shared: &Shared) {
    let Some(win) = web_sys::window() else { return };
    let Some(doc) = win.document() else { return };
    let s = shared.clone();
    on(doc.unchecked_ref(), "mousedown", move |ev| {
        let doc = s.borrow().doc.clone();
        let Some(pop) = doc.get_element_by_id("bg-settings") else {
            return;
        };
        if !pop.class_name().contains("open") {
            return;
        }
        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
        let in_pop = target
            .as_ref()
            .map(|n| pop.contains(Some(n)))
            .unwrap_or(false);
        let in_gear = doc
            .get_element_by_id("bg-gear")
            .zip(target.as_ref())
            .map(|(g, n)| g.contains(Some(n)))
            .unwrap_or(false);
        if !in_pop && !in_gear {
            close_popover(&s);
        }
    });
}
