//! Help overlay (§F10): a terse, two-column keyboard + interaction reference card
//! — a reference, not a tour. Opened by pressing `?` (when not typing) or the
//! settings menu's "Help & shortcuts" item; closed by Esc or the ✕. Same centered
//! glass idiom as the welcome overlay ([`super::onboarding`]) and the confirmation
//! modal, with a Tab focus trap and focus restore to the control that opened it
//! (via [`super::focus_trap`]).

use super::filters::panel;
use super::{el, focus_trap, on, span, Shared};
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement};

/// Left column — direct manipulation of the graph / views (pointer gestures).
const INTERACTIONS: [(&str, &str); 7] = [
    ("Click node", "Inspect / drill into it"),
    ("Drag node", "Rearrange (positions persist)"),
    ("Drag canvas", "Pan the graph"),
    ("Wheel", "Zoom in / out"),
    ("Legend row", "Filter by provenance"),
    ("Table host cell", "Jump to it in the graph"),
    ("Heatmap day", "Scope the session list"),
];

/// Right column — keyboard shortcuts. The `← / →` row documents the canvas-focused
/// precedence (see [`super::graph_view`] / [`super::shortcuts`]): stepping the time
/// window is the default, but the arrows pan while the graph canvas holds focus.
const SHORTCUTS: [(&str, &str); 6] = [
    ("⌘ / Ctrl-K", "Focus the host search"),
    ("Esc", "Close / clear focus"),
    ("+ / −", "Zoom the graph"),
    ("0 or F", "Fit the graph to screen"),
    (
        "← / →",
        "Step the time window (pans when the graph is focused)",
    ),
    ("?", "This help"),
];

/// Open the help overlay if it isn't already up, else close it — the `?` toggle.
pub(super) fn toggle_help(shared: &Shared) {
    let doc = shared.borrow().doc.clone();
    if doc.get_element_by_id("bg-help").is_some() {
        close_help(&doc);
    } else {
        show_help_overlay(shared);
    }
}

/// Remove the help overlay and return focus to the control that opened it.
pub(super) fn close_help(doc: &Document) {
    if let Some(o) = doc.get_element_by_id("bg-help") {
        o.remove();
        focus_trap::restore();
    }
}

/// Build and show the help overlay: a centered glass card with a title + ✕ and two
/// reference columns (Interactions / Shortcuts). Re-openable anytime; a no-op if
/// already open.
pub(super) fn show_help_overlay(shared: &Shared) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if doc.get_element_by_id("bg-help").is_some() {
        return;
    }
    // Remember the invoking control before we move focus into the overlay.
    focus_trap::remember();

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay help-overlay");
    let _ = overlay.set_attribute("id", "bg-help");
    let modal = panel(&doc, "modal help-modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");
    let _ = modal.set_attribute("aria-label", "Help and keyboard shortcuts");

    let head = el(&doc, "div");
    let _ = head.set_attribute("class", "help-head");
    let _ = head.append_child(&span(&doc, "help-title", "Help & shortcuts"));
    let close = el(&doc, "button");
    let _ = close.set_attribute("type", "button");
    let _ = close.set_attribute("class", "help-x");
    let _ = close.set_attribute("aria-label", "Close help");
    close.set_text_content(Some("✕"));
    {
        let s = shared.clone();
        on(&close, "click", move |_| {
            close_help(&s.borrow().doc.clone())
        });
    }
    let _ = head.append_child(&close);
    let _ = modal.append_child(&head);

    let cols = el(&doc, "div");
    let _ = cols.set_attribute("class", "help-cols");
    let _ = cols.append_child(&help_col(&doc, "Interactions", &INTERACTIONS));
    let _ = cols.append_child(&help_col(&doc, "Shortcuts", &SHORTCUTS));
    let _ = modal.append_child(&cols);

    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);

    // Close on a backdrop click (mousedown on the overlay, not the card).
    {
        let doc2 = doc.clone();
        on(overlay.as_ref(), "mousedown", move |ev| {
            let on_backdrop = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .map(|e| e.id() == "bg-help")
                .unwrap_or(false);
            if on_backdrop {
                close_help(&doc2);
            }
        });
    }

    // Trap Tab inside the overlay and land focus on the ✕ so Esc/Tab engage.
    focus_trap::install(&overlay);
    if let Ok(h) = close.dyn_into::<HtmlElement>() {
        let _ = h.focus();
    }
}

/// One reference column: an uppercase heading over `(key, description)` rows.
fn help_col(doc: &Document, heading: &str, rows: &[(&str, &str)]) -> Element {
    let col = el(doc, "div");
    let _ = col.set_attribute("class", "help-col");
    let _ = col.append_child(&span(doc, "help-colhead", heading));
    for (key, desc) in rows {
        let row = el(doc, "div");
        let _ = row.set_attribute("class", "help-row");
        let _ = row.append_child(&span(doc, "help-key", key));
        let _ = row.append_child(&span(doc, "help-desc", desc));
        let _ = col.append_child(&row);
    }
    col
}
