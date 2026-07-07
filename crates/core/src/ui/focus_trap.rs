//! Shared focus management for modal overlays (§F10 accessibility): a Tab /
//! Shift-Tab focus trap that keeps keyboard focus inside an open overlay, plus a
//! small save/restore stack so closing an overlay returns focus to the control
//! that opened it. Used by the confirmation modal ([`super::modal`]) and the help
//! overlay ([`super::help`]).
//!
//! The restore stack is a plain thread-local (the dashboard is single-threaded
//! wasm): [`remember`] pushes the currently-focused element before an overlay
//! opens, and [`restore`] pops + refocuses it when the overlay closes — nesting
//! (a confirm modal over the welcome overlay) unwinds in the right order.

use super::on;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, KeyboardEvent};

thread_local! {
    /// LIFO stack of the controls that opened the currently-stacked overlays.
    /// `Some`/`None` entries stay balanced with [`remember`]/[`restore`] so a
    /// missing active element never desynchronizes the stack.
    static RESTORE: RefCell<Vec<Option<HtmlElement>>> = const { RefCell::new(Vec::new()) };
}

/// Record the currently-focused element as the control to restore focus to when
/// the overlay about to open is closed. Call once, *before* moving focus into the
/// overlay.
pub(super) fn remember() {
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|e| e.dyn_into::<HtmlElement>().ok());
    RESTORE.with(|r| r.borrow_mut().push(active));
}

/// Pop the last [`remember`]ed control and return focus to it (if it is still in
/// the document). Call once when the overlay closes.
pub(super) fn restore() {
    let el = RESTORE.with(|r| r.borrow_mut().pop()).flatten();
    if let Some(el) = el {
        if el.unchecked_ref::<web_sys::Node>().is_connected() {
            let _ = el.focus();
        }
    }
}

/// The tabbable elements inside `container`, in DOM order (skipping disabled
/// controls and explicit `tabindex="-1"`).
fn focusables(container: &Element) -> Vec<HtmlElement> {
    let sel = "a[href], button:not([disabled]), input:not([disabled]), \
               select:not([disabled]), textarea:not([disabled]), \
               [tabindex]:not([tabindex=\"-1\"])";
    let mut out = Vec::new();
    if let Ok(list) = container.query_selector_all(sel) {
        for i in 0..list.length() {
            if let Some(el) = list.item(i).and_then(|n| n.dyn_into::<HtmlElement>().ok()) {
                out.push(el);
            }
        }
    }
    out
}

/// Move focus to the first tabbable element inside `container` (used when an
/// overlay with no text input opens, so the trap engages and Esc/Tab work).
pub(super) fn focus_first(container: &Element) {
    if let Some(first) = focusables(container).into_iter().next() {
        let _ = first.focus();
    }
}

/// Install a Tab / Shift-Tab focus trap on an overlay `container`: Tab past the
/// last control wraps to the first, Shift-Tab before the first wraps to the last,
/// so keyboard focus can never leave the open overlay. Only intercepts Tab; every
/// other key (Enter, Esc, typing) is left alone.
pub(super) fn install(container: &Element) {
    let ov = container.clone();
    on(container.as_ref(), "keydown", move |ev| {
        let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
            return;
        };
        if ke.key() != "Tab" {
            return;
        }
        let items = focusables(&ov);
        let (Some(first), Some(last)) = (items.first(), items.last()) else {
            return;
        };
        let active = ov.owner_document().and_then(|d| d.active_element());
        let active_node = active.as_ref().map(|e| e.unchecked_ref::<web_sys::Node>());
        let idx = items.iter().position(|it| {
            it.unchecked_ref::<web_sys::Node>()
                .is_same_node(active_node)
        });
        if ke.shift_key() {
            // Wrap backwards from the first element (or from anywhere outside).
            if idx.is_none_or(|i| i == 0) {
                ke.prevent_default();
                let _ = last.focus();
            }
        } else if idx.is_none_or(|i| i == items.len() - 1) {
            // Wrap forwards from the last element (or from anywhere outside).
            ke.prevent_default();
            let _ = first.focus();
        }
    });
}
