//! WASM ⇄ JS bridge (§7.8). Externs into `globalThis.chromeBridge` plus the
//! `mount` entry point exported to the dashboard page.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = storageLocalGet)]
    fn storage_local_get_js(key: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = storageLocalSet)]
    fn storage_local_set_js(key: &str, val: &str);
    #[wasm_bindgen(js_namespace = chromeBridge, js_name = downloadJson)]
    fn download_json_js(name: &str, json: &str);
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

/// Trigger a local-file download of `json` named `name`. A *download*, never an
/// upload (§7.8 / §12.1).
pub fn download_json(name: &str, json: &str) {
    download_json_js(name, json);
}

/// Dashboard entry point (called from `dashboard.ts` after the SW readiness ack).
#[wasm_bindgen]
pub fn mount(root_id: &str) {
    console_error_panic_hook::set_once();
    let root = root_id.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = crate::ui::run(&root).await {
            web_sys::console::error_1(&format!("Outdegree: {e:?}").into());
        }
    });
}
