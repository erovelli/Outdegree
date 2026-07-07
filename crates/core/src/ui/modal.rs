//! The validating confirmation modal (§8): the in-app replacement for
//! `window.prompt`, with input validation and an optional "type DELETE" gate for
//! irreversible actions.

use super::filters::panel;
use super::{el, focus_trap, on, span, Shared};
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, KeyboardEvent};

/// Remove the confirmation modal (if present) and return keyboard focus to the
/// control that opened it. The single close path shared by the Confirm/Cancel
/// buttons, Enter, the backdrop click, and the global Esc handler (§F10).
pub(super) fn close_modal(doc: &Document) {
    if let Some(o) = doc.get_element_by_id("bg-modal") {
        o.remove();
        focus_trap::restore();
    }
}

/// A styled, validating confirmation modal — the in-app replacement for the
/// browser's `window.prompt`, which offered no validation and (for "delete N
/// days") let an unbounded number silently wipe everything. `on_confirm` receives
/// the input value (when there is an input) and returns whether to close: it can
/// reject bad input by writing to `#bg-modal-error` and returning `false`.
///
/// When `require_phrase` is `Some(word)`, the Confirm button starts disabled and
/// only enables once the input's trimmed value matches `word` exactly — the
/// "type DELETE to confirm" gate for irreversible actions (§8). It has no effect
/// without an input (`placeholder = None`).
pub(crate) fn confirm_dialog(
    shared: &Shared,
    title: &str,
    message: &str,
    placeholder: Option<&str>,
    confirm_label: &str,
    danger: bool,
    require_phrase: Option<&str>,
    on_confirm: impl FnMut(Option<String>) -> bool + 'static,
) {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    if let Some(old) = doc.get_element_by_id("bg-modal") {
        old.remove();
    }
    // Remember the invoking control so focus returns to it on close (§F10).
    focus_trap::remember();

    let overlay = el(&doc, "div");
    let _ = overlay.set_attribute("class", "modal-overlay");
    let _ = overlay.set_attribute("id", "bg-modal");
    let modal = panel(&doc, "modal");
    let _ = modal.set_attribute("role", "dialog");
    let _ = modal.set_attribute("aria-modal", "true");

    let _ = modal.append_child(&span(&doc, "modal-title", title));
    let _ = modal.append_child(&span(&doc, "modal-msg", message));

    let input_opt = placeholder.map(|ph| {
        let inp = el(&doc, "input");
        let _ = inp.set_attribute("type", "text");
        let _ = inp.set_attribute("class", "modal-input");
        let _ = inp.set_attribute("id", "bg-modal-input");
        let _ = inp.set_attribute("placeholder", ph);
        let _ = modal.append_child(&inp);
        inp
    });

    let err = span(&doc, "modal-error", "");
    let _ = err.set_attribute("id", "bg-modal-error");
    let _ = modal.append_child(&err);

    let actions = el(&doc, "div");
    let _ = actions.set_attribute("class", "modal-actions");
    let cancel = el(&doc, "button");
    let _ = cancel.set_attribute("type", "button");
    let _ = cancel.set_attribute("class", "modal-btn");
    cancel.set_text_content(Some("Cancel"));
    let confirm = el(&doc, "button");
    let _ = confirm.set_attribute("type", "button");
    let _ = confirm.set_attribute(
        "class",
        if danger {
            "modal-btn modal-confirm danger"
        } else {
            "modal-btn modal-confirm"
        },
    );
    confirm.set_text_content(Some(confirm_label));
    // Gate irreversible actions behind a typed confirmation phrase: the button
    // starts disabled and only the exact word enables it (§8).
    if require_phrase.is_some() {
        let _ = confirm.set_attribute("disabled", "disabled");
    }
    let _ = actions.append_child(&cancel);
    let _ = actions.append_child(&confirm);
    let _ = modal.append_child(&actions);
    let _ = overlay.append_child(&modal);
    let _ = root.append_child(&overlay);

    // Trap Tab inside the dialog, then land focus: the input if there is one, else
    // the first control (Cancel) so Esc/Tab engage and Enter has a target (§F10).
    focus_trap::install(&overlay);
    if let Some(inp) = &input_opt {
        if let Ok(h) = inp.clone().dyn_into::<HtmlElement>() {
            let _ = h.focus();
        }
    } else {
        focus_trap::focus_first(&overlay);
    }

    let cb = std::rc::Rc::new(std::cell::RefCell::new(on_confirm));
    let do_confirm = {
        let cb = cb.clone();
        let inp = input_opt.clone();
        let doc = doc.clone();
        move || {
            let val = inp
                .as_ref()
                .and_then(|i| i.clone().dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value());
            if (cb.borrow_mut())(val) {
                close_modal(&doc);
            }
        }
    };
    {
        let f = do_confirm.clone();
        on(&confirm, "click", move |_| f());
    }
    {
        let doc = doc.clone();
        on(&cancel, "click", move |_| close_modal(&doc));
    }
    {
        // Click the backdrop (not the dialog) to dismiss.
        let doc = doc.clone();
        on(overlay.as_ref(), "mousedown", move |ev| {
            let on_backdrop = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .map(|e| e.id() == "bg-modal")
                .unwrap_or(false);
            if on_backdrop {
                close_modal(&doc);
            }
        });
    }
    if let Some(inp) = &input_opt {
        let confirm = confirm.clone();
        on(inp.as_ref(), "keydown", move |ev| {
            if let Ok(ke) = ev.dyn_into::<KeyboardEvent>() {
                if ke.key() == "Enter" {
                    // A disabled Confirm swallows `.click()`, so Enter can't
                    // bypass the typed-phrase gate below.
                    if let Ok(h) = confirm.clone().dyn_into::<HtmlElement>() {
                        h.click();
                    }
                }
            }
        });
    }
    // Enable Confirm only once the exact confirmation phrase is typed (§8).
    if let (Some(inp), Some(phrase)) = (&input_opt, require_phrase.map(str::to_string)) {
        let confirm = confirm.clone();
        on(inp.as_ref(), "input", move |ev| {
            let matches = ev
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .map(|i| i.value().trim() == phrase)
                .unwrap_or(false);
            if matches {
                let _ = confirm.remove_attribute("disabled");
            } else {
                let _ = confirm.set_attribute("disabled", "disabled");
            }
        });
    }
}
