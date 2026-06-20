//! Toolbar control constructors (§7.7). Pure element builders — the wiring lives
//! in `app.rs` so all `Shared` state mutation is in one place.

use super::el;
use web_sys::{Document, Element};

fn labeled(doc: &Document, label: &str, control: &Element) -> Element {
    let wrap = el(doc, "label");
    let span = el(doc, "span");
    span.set_text_content(Some(label));
    let _ = wrap.append_child(&span);
    let _ = wrap.append_child(control);
    wrap
}

/// A `<select>` with `(value, text)` options and an initial selection. Returns
/// `(wrapper, select)`.
pub(crate) fn select(
    doc: &Document,
    label: &str,
    options: &[(&str, &str)],
    current: &str,
) -> (Element, Element) {
    let sel = el(doc, "select");
    for (value, text) in options {
        let opt = el(doc, "option");
        let _ = opt.set_attribute("value", value);
        opt.set_text_content(Some(text));
        if *value == current {
            let _ = opt.set_attribute("selected", "selected");
        }
        let _ = sel.append_child(&opt);
    }
    let wrap = labeled(doc, label, &sel);
    (wrap, sel)
}

/// A numeric `<input>`. Returns `(wrapper, input)`.
pub(crate) fn number(doc: &Document, label: &str, value: u32) -> (Element, Element) {
    let input = el(doc, "input");
    let _ = input.set_attribute("type", "number");
    let _ = input.set_attribute("min", "0");
    let _ = input.set_attribute("value", &value.to_string());
    let _ = input.set_attribute("style", "width:64px");
    let wrap = labeled(doc, label, &input);
    (wrap, input)
}

/// A checkbox `<input>`. Returns `(wrapper, input)`.
pub(crate) fn checkbox(doc: &Document, label: &str, checked: bool) -> (Element, Element) {
    let input = el(doc, "input");
    let _ = input.set_attribute("type", "checkbox");
    if checked {
        let _ = input.set_attribute("checked", "checked");
    }
    let wrap = labeled(doc, label, &input);
    (wrap, input)
}

/// A plain `<button>` with text.
pub(crate) fn button(doc: &Document, text: &str) -> Element {
    let b = el(doc, "button");
    b.set_text_content(Some(text));
    b
}
