//! WASM ⇄ JS bridge (§7.8). Externs into `globalThis.chromeBridge` plus the
//! `mount` entry point exported to the dashboard page.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = storageLocalGet)]
    fn storage_local_get_js(key: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = storageLocalSet)]
    fn storage_local_set_js(key: &str, val: &str);
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = downloadText)]
    fn download_text_js(name: &str, mime: &str, body: &str);
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = downloadDataUrl)]
    fn download_data_url_js(name: &str, data_url: &str);
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = sampleData)]
    fn sample_data_js() -> String;
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = faviconBase)]
    fn favicon_base_js() -> String;
}

/// Read a string value from `chrome.storage.local` (SW-owned).
pub async fn storage_local_get(key: &str) -> Option<String> {
    let p = storage_local_get_js(key);
    let v = wasm_bindgen_futures::JsFuture::from(p).await.ok()?;
    v.as_string()
}

/// Write a string value to `chrome.storage.local`. Pause = set "paused" here.
pub fn storage_local_set(key: &str, val: &str) {
    storage_local_set_js(key, val);
}

/// Trigger a local-file download of `body` (named `name`, MIME `mime`). A
/// *download*, never an upload (§7.8 / §12.1).
pub fn download_text(name: &str, mime: &str, body: &str) {
    download_text_js(name, mime, body);
}

/// Convenience for the JSON export path.
pub fn download_json(name: &str, json: &str) {
    download_text(name, "application/json", json);
}

/// The extension-origin base URL of Chrome's LOCAL favicon service
/// (`chrome-extension://<id>/_favicon/`), or `None` when the running browser
/// doesn't grant the `favicon` permission (§F12). The Firefox overlay strips the
/// permission (the API is Chromium-only) and older Chrome predates it, so the
/// bridge returns `""` there — mapped to `None` so the site-icons feature stays
/// inert (no `_favicon` URLs built, no `<img>` emitted, no canvas icon loads).
///
/// No network: Chrome serves the icon from its on-disk favicon cache, so this is
/// consistent with `connect-src 'none'` and the no-egress guarantee (docs/adr/0006).
pub fn favicon_base() -> Option<String> {
    let base = favicon_base_js();
    if base.is_empty() {
        None
    } else {
        Some(base)
    }
}

/// The committed onboarding sample fixture (`extension/src/sample-data.json`),
/// inlined into the dashboard bundle at build time via a `?raw` import (never a
/// `fetch`, so it works under CSP `connect-src 'none'`). The raw text is handed to
/// [`crate::sample::materialize`] on "Load sample data" (§F4).
pub fn sample_data() -> String {
    sample_data_js()
}

/// Trigger a local-file download from a `data:` URL (named `name`) — used for the
/// PNG graph export, whose bytes come from `canvas.toDataURL`. Still a download,
/// never an upload (§7.8 / §12.1).
pub fn download_data_url(name: &str, data_url: &str) {
    download_data_url_js(name, data_url);
}

/// Dashboard entry point (called from `dashboard.ts` after the SW readiness ack).
#[wasm_bindgen]
pub fn mount(root_id: &str) {
    // `panic = abort` still runs the panic hook before aborting, so a panic in the
    // analysis core surfaces a readable trace in the page console.
    console_error_panic_hook::set_once();
    let root = root_id.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = crate::ui::run(&root).await {
            web_sys::console::error_1(&format!("Outdegree: {e:?}").into());
            // Replace the "Loading…" placeholder with a user-facing failure
            // message (styled via the existing .bg-empty class — the page CSP
            // forbids inline styles) so the dashboard never hangs silently if
            // local storage or WASM init is unavailable.
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&root))
            {
                el.set_inner_html(
                    "<div class=\"bg-empty\">Outdegree couldn't load your data — \
                     your browser's local storage may be unavailable (e.g. a \
                     private window or blocked site data). Reopen this tab to retry.</div>",
                );
            }
        }
    });
}
