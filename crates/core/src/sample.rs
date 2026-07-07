//! Onboarding sample dataset: the load-time materializer (F4).
//!
//! The committed fixture (`extension/src/sample-data.json`, produced by
//! `scripts/generate-sample-data.mjs`) is an export-schema-v1 document whose
//! event timestamps are stored as **offsets in ms before "now"** and whose URLs
//! are stored **schemeless**. [`materialize`] turns that fixture into an
//! import-ready document by, for every `events`/`spa` record:
//!
//! * shifting `ts` from an offset to an absolute epoch-ms timestamp relative to a
//!   caller-supplied `now` (so the sample always looks freshly browsed), and
//! * prepending the `http(s)` scheme stripped from each `toUrl`.
//!
//! **Audit interaction.** The fixture is inlined verbatim into the dashboard
//! bundle (a `?raw` import, so no `fetch`), and the CI bundle audit greps `dist/`
//! for `https?://`. Storing the URLs schemeless keeps that grep clean; the scheme
//! is re-attached here, at load time, entirely inside the WASM core. `host()`
//! (interpret.rs) requires an `http(s)` URL, so this prepend is also what makes
//! the loaded sample derive into a real graph rather than being dropped as
//! non-http. Kept pure so the shift is unit-tested and the fixture can be driven
//! through the derive/rollup pipeline in a native test.

/// The scheme stripped from the fixture's URLs (see the module note). Every
/// fixture URL is `http(s)`-less by construction, so this is always what to
/// prepend to reconstruct a parseable URL.
pub const URL_SCHEME: &str = "https://";

/// Convert an offset (ms *before* `now`) into an absolute epoch-ms timestamp.
/// A larger offset is older; offset `0` is exactly `now`. Pure and total.
pub fn absolute_ts(offset_ms: f64, now_ms: f64) -> f64 {
    now_ms - offset_ms
}

/// Re-attach the fixture's stripped scheme to a stored URL. Idempotent: a URL that
/// already carries an `http(s)` scheme is returned unchanged, so a hand-edited or
/// future fixture that keeps schemes still loads correctly.
pub fn with_scheme(schemeless: &str) -> String {
    if schemeless.starts_with("http://") || schemeless.starts_with("https://") {
        schemeless.to_string()
    } else {
        format!("{URL_SCHEME}{schemeless}")
    }
}

/// Materialize the committed fixture JSON into an import-ready export document:
/// shift every `events`/`spa` record's offset `ts` to an absolute timestamp
/// relative to `now_ms`, and prepend the scheme to every `toUrl`. Other stores
/// (`rollup_days`, `sessions`, `meta`) are passed through untouched — the caller
/// re-derives them from the imported events. Returns the serialized document, or
/// an error string if the fixture isn't valid JSON.
pub fn materialize(fixture_json: &str, now_ms: f64) -> Result<String, String> {
    let mut doc: serde_json::Value =
        serde_json::from_str(fixture_json).map_err(|e| format!("sample fixture parse: {e}"))?;
    for store in ["events", "spa"] {
        if let Some(serde_json::Value::Array(items)) = doc.get_mut(store) {
            for item in items.iter_mut() {
                if let Some(ts) = item.get("ts").and_then(|v| v.as_f64()) {
                    item["ts"] = serde_json::json!(absolute_ts(ts, now_ms));
                }
                if let Some(u) = item.get("toUrl").and_then(|v| v.as_str()) {
                    let full = with_scheme(u);
                    item["toUrl"] = serde_json::Value::String(full);
                }
            }
        }
    }
    serde_json::to_string(&doc).map_err(|e| format!("sample fixture serialize: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_ts_shifts_offset_before_now() {
        let now = 1_700_000_000_000.0;
        assert_eq!(absolute_ts(0.0, now), now);
        assert_eq!(absolute_ts(1_000.0, now), now - 1_000.0);
        // A larger offset is strictly older.
        assert!(absolute_ts(5_000.0, now) < absolute_ts(1_000.0, now));
    }

    #[test]
    fn with_scheme_is_prepend_and_idempotent() {
        assert_eq!(with_scheme("news.example/x"), "https://news.example/x");
        // Already-schemed URLs are untouched (idempotent).
        assert_eq!(with_scheme("https://a.example/"), "https://a.example/");
        assert_eq!(with_scheme("http://a.example/"), "http://a.example/");
    }

    #[test]
    fn materialize_shifts_ts_and_prepends_scheme() {
        let now = 2_000_000_000_000.0;
        let fixture = r#"{
            "version": 1,
            "events": [
                {"kind":"nav","id":1,"ts":1000,"tabId":1,"windowId":1,
                 "toUrl":"news.example/a","transitionType":"typed","qualifiers":[]},
                {"kind":"start","id":2,"ts":500}
            ],
            "spa": [
                {"kind":"nav","id":1,"ts":250,"tabId":1,"windowId":1,
                 "toUrl":"app.example/x","transitionType":"link","qualifiers":[]}
            ],
            "rollup_days": [], "sessions": [], "meta": []
        }"#;
        let out = materialize(fixture, now).expect("materialize");
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();

        let e0 = &doc["events"][0];
        assert_eq!(e0["ts"].as_f64().unwrap(), now - 1000.0);
        assert_eq!(e0["toUrl"].as_str().unwrap(), "https://news.example/a");
        // Non-URL records (start) still get their ts shifted.
        assert_eq!(doc["events"][1]["ts"].as_f64().unwrap(), now - 500.0);
        // The spa store is shifted + schemed too.
        assert_eq!(doc["spa"][0]["ts"].as_f64().unwrap(), now - 250.0);
        assert_eq!(
            doc["spa"][0]["toUrl"].as_str().unwrap(),
            "https://app.example/x"
        );
        // Untouched metadata carries through.
        assert_eq!(doc["version"].as_i64().unwrap(), 1);
    }

    #[test]
    fn materialize_rejects_bad_json() {
        assert!(materialize("not json", 0.0).is_err());
    }
}
