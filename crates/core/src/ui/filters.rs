//! Floating "glass" chrome builders (design handoff). Pure element constructors
//! — the wiring lives in `app.rs` so all `Shared` state mutation is in one place.
//!
//! Every control floats over the full-bleed canvas as a translucent panel; the
//! only color in the product is reserved for the data spectrum, so all chrome
//! here is monochrome.

use super::el;
use web_sys::{Document, Element};

/// A floating glass panel pinned by the inline `style` (edge/corner placement).
/// `extra` adds layout modifiers (e.g. `"seg"`, `"toolbar"`).
pub(crate) fn panel(doc: &Document, extra: &str, style: &str) -> Element {
    let p = el(doc, "div");
    let _ = p.set_attribute("class", &format!("panel {extra}"));
    let _ = p.set_attribute("style", style);
    p
}

/// A segmented control. `variant` is `"solid"` (active = filled white pill) or
/// `"ghost"` (active = subtle tint). Returns `(container, [(value, button)])`.
pub(crate) fn seg(
    doc: &Document,
    variant: &str,
    items: &[(&str, &str)],
) -> (Element, Vec<(String, Element)>) {
    let wrap = el(doc, "div");
    let _ = wrap.set_attribute("class", &format!("seg seg-{variant}"));
    let mut out = Vec::new();
    for (value, label) in items {
        let b = el(doc, "button");
        b.set_text_content(Some(label));
        let _ = b.set_attribute("data-seg", value);
        let _ = wrap.append_child(&b);
        out.push((value.to_string(), b));
    }
    (wrap, out)
}

/// A filter chip with an id (so its label can be refreshed) and initial text.
pub(crate) fn chip(doc: &Document, id: &str, label: &str) -> Element {
    let c = el(doc, "button");
    let _ = c.set_attribute("class", "chip");
    let _ = c.set_attribute("id", id);
    c.set_text_content(Some(label));
    c
}

/// A square icon button carrying an inline SVG glyph.
pub(crate) fn icon_btn(doc: &Document, id: &str, title: &str, svg: &str) -> Element {
    let b = el(doc, "button");
    let _ = b.set_attribute("class", "iconbtn");
    let _ = b.set_attribute("id", id);
    let _ = b.set_attribute("title", title);
    b.set_inner_html(svg);
    b
}

/// A left-aligned menu row (button) for the settings popover.
pub(crate) fn menu_btn(doc: &Document, label: &str) -> Element {
    let b = el(doc, "button");
    let _ = b.set_attribute("class", "menu-item");
    b.set_text_content(Some(label));
    b
}

/// A labeled checkbox row for the settings popover. Returns `(row, input)`.
pub(crate) fn menu_toggle(doc: &Document, label: &str, checked: bool) -> (Element, Element) {
    let row = el(doc, "label");
    let _ = row.set_attribute("class", "menu-item menu-toggle");
    let span = el(doc, "span");
    span.set_text_content(Some(label));
    let input = el(doc, "input");
    let _ = input.set_attribute("type", "checkbox");
    if checked {
        let _ = input.set_attribute("checked", "checked");
    }
    let _ = row.append_child(&span);
    let _ = row.append_child(&input);
    (row, input)
}

// ── inline SVG glyphs (1.6px stroke line icons, currentColor) ─────────────────

/// The 2×2-square brand mark (two full white, two at 55%).
pub(crate) const LOGO: &str = concat!(
    "<svg width=\"20\" height=\"20\" viewBox=\"0 0 20 20\" aria-hidden=\"true\">",
    "<rect x=\"0\" y=\"0\" width=\"9\" height=\"9\" rx=\"2\" fill=\"#f4f4f5\"/>",
    "<rect x=\"11\" y=\"0\" width=\"9\" height=\"9\" rx=\"2\" fill=\"#f4f4f5\" opacity=\"0.55\"/>",
    "<rect x=\"0\" y=\"11\" width=\"9\" height=\"9\" rx=\"2\" fill=\"#f4f4f5\" opacity=\"0.55\"/>",
    "<rect x=\"11\" y=\"11\" width=\"9\" height=\"9\" rx=\"2\" fill=\"#f4f4f5\"/>",
    "</svg>",
);

const ICON_OPEN: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" \
     stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\">";

pub(crate) fn icon(name: &str) -> String {
    let body = match name {
        "gear" => "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z\"/>",
        "plus" => "<circle cx=\"11\" cy=\"11\" r=\"7\"/><line x1=\"11\" y1=\"8\" x2=\"11\" y2=\"14\"/><line x1=\"8\" y1=\"11\" x2=\"14\" y2=\"11\"/><line x1=\"21\" y1=\"21\" x2=\"16.65\" y2=\"16.65\"/>",
        "minus" => "<circle cx=\"11\" cy=\"11\" r=\"7\"/><line x1=\"8\" y1=\"11\" x2=\"14\" y2=\"11\"/><line x1=\"21\" y1=\"21\" x2=\"16.65\" y2=\"16.65\"/>",
        "fit" => "<path d=\"M8 3H5a2 2 0 0 0-2 2v3\"/><path d=\"M21 8V5a2 2 0 0 0-2-2h-3\"/><path d=\"M3 16v3a2 2 0 0 0 2 2h3\"/><path d=\"M16 21h3a2 2 0 0 0 2-2v-3\"/>",
        "lock" => "<rect x=\"3\" y=\"11\" width=\"18\" height=\"11\" rx=\"2\" ry=\"2\"/><path d=\"M7 11V7a5 5 0 0 1 10 0v4\"/>",
        "unlock" => "<rect x=\"3\" y=\"11\" width=\"18\" height=\"11\" rx=\"2\" ry=\"2\"/><path d=\"M7 11V7a5 5 0 0 1 9.9-1\"/>",
        "search" => "<circle cx=\"11\" cy=\"11\" r=\"7\"/><line x1=\"21\" y1=\"21\" x2=\"16.65\" y2=\"16.65\"/>",
        _ => "",
    };
    format!("{ICON_OPEN}{body}</svg>")
}
