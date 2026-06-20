//! §11 browser tests (wasm-bindgen-test, headless Chrome): IndexedDB store
//! round-trips and bridge externs. The readiness-ping path needs an extension
//! context (chrome.runtime) and is covered by the manual M0 gate instead.
#![cfg(target_arch = "wasm32")]

use browsing_graph_core::model::ProvBreakdown;
use browsing_graph_core::rollup::{DayBucket, DeriveState, NodeStat, SessionRec};
use browsing_graph_core::store;
use std::collections::HashMap;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Each derived store round-trips through IndexedDB. Uses unique keys/dates so it
/// is robust against any residual data in the shared DB.
#[wasm_bindgen_test]
async fn store_roundtrips() {
    let db = store::open().await.expect("open db");

    // derive cursor (watermark + state)
    let st = DeriveState {
        watermark: 42.0,
        ..Default::default()
    };
    db.write_cursor(42.0, &st).await.expect("write cursor");
    let (wm, _state) = db.read_cursor().await.expect("read cursor");
    assert_eq!(wm, 42.0);

    // layout positions
    let mut pos = HashMap::new();
    pos.insert("x.test".to_string(), (1.0f32, 2.0f32));
    db.write_positions(&pos).await.expect("write positions");
    let got = db.read_positions().await.expect("read positions");
    assert_eq!(got.get("x.test"), Some(&(1.0, 2.0)));

    // rollup delta merged into a uniquely-dated bucket
    let mut b = DayBucket {
        date: "2099-01-01".into(),
        max_id: 7,
        ..Default::default()
    };
    b.nodes.insert(
        "node.test".into(),
        NodeStat {
            visits: 3,
            prov: ProvBreakdown::default(),
        },
    );
    db.write_rollups(&[b]).await.expect("write rollups");
    let all = db.read_all_rollups().await.expect("read rollups");
    let found = all
        .iter()
        .find(|d| d.date == "2099-01-01")
        .expect("bucket present");
    assert_eq!(found.nodes.get("node.test").unwrap().visits, 3);

    // session record
    let rec = SessionRec {
        id: 999_999.0,
        window_id: 1,
        start_ts: 1.0,
        end_ts: 2.0,
        start_id: 1.0,
        end_id: 2.0,
        nav_count: 5,
        top_hosts: vec![("h".into(), 1)],
    };
    db.write_sessions(&[rec]).await.expect("write sessions");
    let sessions = db.read_sessions().await.expect("read sessions");
    assert!(sessions
        .iter()
        .any(|s| s.id == 999_999.0 && s.nav_count == 5));
}

/// Export reflects an imported document (self-contained: import clears first).
#[wasm_bindgen_test]
async fn store_export_import() {
    let db = store::open().await.expect("open db");
    let doc = r#"{"version":1,"events":[{"kind":"start","id":1,"ts":100}],
                 "spa":[],"rollup_days":[],"sessions":[],"meta":[]}"#;
    db.import_json(doc).await.expect("import");
    let n = db.count_events().await.expect("count");
    assert_eq!(n, 1);
    let json = db.export_json().await.expect("export");
    assert!(
        json.contains("\"start\""),
        "export missing imported record: {json}"
    );
}

/// The bridge externs resolve against `globalThis.chromeBridge` and call through.
#[wasm_bindgen_test]
async fn bridge_externs_resolve() {
    js_sys::eval(
        r#"
        globalThis.__calls = [];
        globalThis.chromeBridge = {
            storageLocalGet: (k) => Promise.resolve(k === 'paused' ? 'true' : null),
            storageLocalSet: (k, v) => { globalThis.__calls.push('set:' + k + '=' + v); },
            downloadJson: (name, json) => { globalThis.__calls.push('dl:' + name + ':' + json.length); },
        };
        "#,
    )
    .expect("install mock bridge");

    browsing_graph_core::bridge::storage_local_set("paused", "true");
    browsing_graph_core::bridge::download_json("f.json", "{}");
    let got = browsing_graph_core::bridge::storage_local_get("paused").await;
    assert_eq!(got, Some("true".to_string()));

    let calls = js_sys::Reflect::get(&js_sys::global(), &"__calls".into()).unwrap();
    let joined = js_sys::Array::from(&calls)
        .join(",")
        .as_string()
        .unwrap_or_default();
    assert!(joined.contains("set:paused=true"), "calls: {joined}");
    assert!(joined.contains("dl:f.json:2"), "calls: {joined}");
}

/// The camera fit centers the laid-out nodes in the viewport, so the graph is
/// visible even when sparse/edgeless (regression guard for the blank-graph bug).
#[wasm_bindgen_test]
fn fit_frames_nodes() {
    use browsing_graph_core::layout::Pos;
    use browsing_graph_core::model::{GraphProjection, NodeAgg};
    use browsing_graph_core::render::canvas2d::fit;

    let node = |k: &str| NodeAgg {
        key: k.into(),
        visits: 1,
        prov: ProvBreakdown::default(),
    };
    let proj = GraphProjection {
        nodes: vec![node("a"), node("b")],
        edges: vec![],
    };
    let mut pos = HashMap::new();
    pos.insert("a".to_string(), Pos { x: 200.0, y: 200.0 });
    pos.insert("b".to_string(), Pos { x: 400.0, y: 400.0 });

    let (w, h) = (1000.0, 1000.0);
    let cam = fit(&proj, &pos, w, h);

    // bbox center (300,300) must map to the canvas center (w/2, h/2).
    let cx = 300.0 * cam.scale + cam.x + w / 2.0;
    let cy = 300.0 * cam.scale + cam.y + h / 2.0;
    assert!((cx - w / 2.0).abs() < 1e-6, "x not centered: {cx}");
    assert!((cy - h / 2.0).abs() < 1e-6, "y not centered: {cy}");
    assert!(cam.scale > 0.0 && cam.scale <= 3.0);
}
