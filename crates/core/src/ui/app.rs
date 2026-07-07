//! Dashboard shell: the full-bleed canvas plus the nine floating glass control
//! clusters from the design handoff, and their wiring (§7.7).
//!
//! The canvas is the app; every control floats over it as a translucent panel
//! pinned to an edge or corner. Chrome is strictly monochrome — the only color
//! belongs to the data spectrum drawn on the canvas.

use super::chrome::{
    brand_panel, focus_panel, legend_panel, range_panel, readout_panel, search_panel, view_panel,
    zoom_panel,
};
use super::inspector::inspector_panel;
use super::settings::{nudge_panel, settings_popover};
use super::shortcuts::{install_palette_shortcut, install_popover_dismiss};
use super::{el, on, Shared, View};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::Element;

/// Build the body layer (canvas/tables mount point) + all floating chrome.
pub(crate) fn build_shell(shared: &Shared) -> Result<(), JsValue> {
    let (doc, root) = {
        let a = shared.borrow();
        (a.doc.clone(), a.root.clone())
    };
    root.set_inner_html("");

    // The view body: graph canvas (mode-graph) or table content (mode-data).
    let body = el(&doc, "div");
    let _ = body.set_attribute("id", "bg-body");
    let _ = body.set_attribute("class", "bg-body mode-graph");
    let _ = root.append_child(&body);
    // Delegated: clicking a host cell in any table jumps to the Graph view focused
    // on that host. One listener on the persistent #bg-body survives the innerHTML
    // rebuilds the table views do.
    {
        let s = shared.clone();
        on(&body, "click", move |ev| {
            let host = ev
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .and_then(|e| e.closest("[data-host]").ok().flatten())
                .and_then(|h| h.get_attribute("data-host"));
            let Some(host) = host else { return };
            {
                let mut a = s.borrow_mut();
                if !a.proj.nodes.iter().any(|n| n.key == host) {
                    return; // host filtered out of the current projection
                }
                a.view = View::Graph;
            }
            // No canvas is mounted in table mode yet, so focus_and_animate falls
            // back to a full re-render that builds the graph already focused.
            super::focus_and_animate(&s, Some(host));
        });
    }

    let _ = root.append_child(&brand_panel(&doc, shared));
    let _ = root.append_child(&range_panel(&doc, shared));
    let _ = root.append_child(&view_panel(&doc, shared));
    let _ = root.append_child(&legend_panel(&doc, shared));
    let _ = root.append_child(&zoom_panel(&doc, shared));
    let _ = root.append_child(&search_panel(&doc, shared));
    let _ = root.append_child(&readout_panel(&doc));
    let _ = root.append_child(&focus_panel(&doc, shared));
    let _ = root.append_child(&inspector_panel(&doc, shared));
    let _ = root.append_child(&nudge_panel(&doc, shared));
    let _ = root.append_child(&settings_popover(&doc, shared));
    install_palette_shortcut(shared);
    install_popover_dismiss(shared);
    // Hide any favicon <img> that fails to load (§F12) — a capturing page-level
    // listener, since resource error events don't bubble and CSP forbids inline
    // onerror. No-op when site icons are off (no such <img> is ever emitted).
    super::install_favicon_error_fallback(shared);

    Ok(())
}
