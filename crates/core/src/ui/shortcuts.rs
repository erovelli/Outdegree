//! Global keyboard + click-away handling: ⌘K/Ctrl-K focuses the host search box,
//! Esc dismisses the modal/welcome/menu and exits drill-down focus, and a
//! mousedown outside the settings popover closes it.

use super::chrome::set_legend;
use super::onboarding::dismiss_welcome_onboarded;
use super::settings::close_popover;
use super::{graph_view, on, Shared};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

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
            // A modal takes priority: dismiss it and stop (don't also clear focus).
            if let Some(m) = s.borrow().doc.get_element_by_id("bg-modal") {
                m.remove();
                return;
            }
            // The welcome overlay dismisses like its "Start recording" button:
            // Esc onboards and closes it (§F4).
            if s.borrow().doc.get_element_by_id("bg-welcome").is_some() {
                dismiss_welcome_onboarded(&s);
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
            }
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
