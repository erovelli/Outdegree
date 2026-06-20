//! Dashboard UI shell (§7.7), built imperatively with web-sys.
//!
//! Deviation from the §2 sketch: the controls are direct-DOM (CSR by
//! construction — the DOM is built at runtime, no server hydration), instead of a
//! reactive framework, to keep the WASM build predictable under the production
//! CSP. The view modules (`app`, `filters`, `graph_view`, `tables`, `sankey`,
//! `session_picker`) mirror the spec's file breakdown.

mod app;
mod filters;
mod graph_view;
mod sankey;
mod session_picker;
mod tables;

use crate::layout::{self, Pos};
use crate::model::{Granularity, GraphProjection};
use crate::project::{self, Filters};
use crate::render::canvas2d::Camera;
use crate::rollup::{DayBucket, SessionRec};
use crate::store::{self, Db};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, EventTarget};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Graph,
    Tables,
    Sankey,
    Raw,
}

/// Whole-dashboard state. `db` is an `Rc<Db>` so async handlers can clone it out
/// of a short borrow and `.await` without holding a `RefCell` borrow.
pub(crate) struct App {
    pub db: Rc<Db>,
    pub buckets: Vec<DayBucket>,
    pub sessions: Vec<SessionRec>,
    pub positions: HashMap<String, (f32, f32)>,
    pub gran: Granularity,
    pub filters: Filters,
    pub view: View,
    pub camera: Camera,
    pub paused: bool,
    pub doc: Document,
    pub root: Element,
    pub proj: GraphProjection,
    pub layout_pos: HashMap<String, Pos>,
    pub hover: Option<String>,
    pub dragging: bool,
    pub did_drag: bool,
    pub last_mouse: (f64, f64),
    pub selected_session: Option<f64>,
    /// Opt-in "in-app navigations" view: fold `events` + `spa` from scratch (§4.2).
    pub spa_mode: bool,
    /// Drill-down focus: when set, the graph shows this node's ego network (§M3).
    pub focus: Option<String>,
}

pub(crate) type Shared = Rc<RefCell<App>>;

/// Entry point invoked by `bridge::mount` after the SW readiness ack.
pub async fn run(root_id: &str) -> Result<(), JsValue> {
    let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let doc = win
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let root = doc
        .get_element_by_id(root_id)
        .ok_or_else(|| JsValue::from_str("missing root element"))?;
    root.set_inner_html("");

    let db = Rc::new(store::open().await?);

    // Incremental fold: read the checkpoint, fold id > watermark, persist.
    let (watermark, mut state) = db.read_cursor().await?;
    let new_events = db.read_events_after(watermark).await?;
    if !new_events.is_empty() {
        let (deltas, sessions) = crate::rollup::fold(&mut state, &new_events);
        db.write_rollups(&deltas).await?;
        db.write_sessions(&sessions).await?;
        db.write_cursor(state.watermark, &state).await?;
    }

    let buckets = db.read_all_rollups().await?;
    let mut sessions = db.read_sessions().await?;
    // provisional open sessions so a just-finished session appears (§4.4, §7.7)
    for os in state.open_sessions.values() {
        sessions.push(os.provisional_record());
    }
    let positions = db.read_positions().await?;
    let paused = crate::bridge::storage_local_get("paused")
        .await
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let app = App {
        db,
        buckets,
        sessions,
        positions,
        gran: Granularity::Hostname,
        filters: Filters::default(),
        view: View::Graph,
        camera: Camera::default(),
        paused,
        doc,
        root,
        proj: GraphProjection::default(),
        layout_pos: HashMap::new(),
        hover: None,
        dragging: false,
        did_drag: false,
        last_mouse: (0.0, 0.0),
        selected_session: None,
        spa_mode: false,
        focus: None,
    };
    let shared: Shared = Rc::new(RefCell::new(app));

    recompute_projection(&shared);
    app::build_shell(&shared)?;
    persist_positions(&shared);
    rerender(&shared)?;
    Ok(())
}

/// Re-project the in-memory buckets at the current granularity/filters and warm-
/// start a fresh layout, preserving spatial memory for surviving nodes (§7.6).
pub(crate) fn recompute_projection(shared: &Shared) {
    let mut a = shared.borrow_mut();
    let mut proj = project::project(&a.buckets, a.gran, &a.filters);
    // Drill-down: reduce to the focused node's ego network (§M3).
    if let Some(focus) = a.focus.clone() {
        proj = project::ego(&proj, &focus);
    }

    let keys: Vec<String> = proj.nodes.iter().map(|n| n.key.clone()).collect();
    let index: HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    let edges: Vec<(usize, usize)> = proj
        .edges
        .iter()
        .filter_map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?)))
        .collect();

    let seed = a.positions.clone();
    let placed = layout::fruchterman_reingold(&keys, &edges, 200, &seed);

    let mut layout_pos = HashMap::new();
    for (k, p) in keys.iter().zip(placed.iter()) {
        layout_pos.insert(k.clone(), *p);
        a.positions.insert(k.clone(), (p.x, p.y));
    }
    a.proj = proj;
    a.layout_pos = layout_pos;
}

/// Persist layout positions to the DB (spatial memory across opens, §7.6).
pub(crate) fn persist_positions(shared: &Shared) {
    let db = shared.borrow().db.clone();
    let pos = shared.borrow().positions.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = db.write_positions(&pos).await;
    });
}

/// Recompute the in-memory buckets for the current `spa_mode` and re-render
/// (§4.2). When SPA mode is on, fold `events` + `spa` from scratch (on demand);
/// otherwise use the persisted rollup cache.
pub(crate) fn reload_buckets(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    let spa_mode = shared.borrow().spa_mode;
    wasm_bindgen_futures::spawn_local(async move {
        let buckets = if spa_mode {
            let events = db.read_events_after(0.0).await.unwrap_or_default();
            let spa = db.read_spa().await.unwrap_or_default();
            let merged = crate::rollup::merge_streams(&events, &spa);
            let mut st = crate::rollup::DeriveState::default();
            crate::rollup::fold(&mut st, &merged).0
        } else {
            db.read_all_rollups().await.unwrap_or_default()
        };
        s.borrow_mut().buckets = buckets;
        recompute_projection(&s);
        let _ = rerender(&s);
    });
}

/// Render the active view into the body container.
pub(crate) fn rerender(shared: &Shared) -> Result<(), JsValue> {
    app::set_active_tab(shared);
    let view = shared.borrow().view;
    match view {
        View::Graph => graph_view::render(shared)?,
        View::Tables => tables::render(shared)?,
        View::Sankey => session_picker::render(shared)?,
        View::Raw => tables::render_raw(shared),
    }
    Ok(())
}

/// Reload all derived data after a destructive edit, then re-render (§8).
pub(crate) fn reload_and_rerender(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let (watermark, mut state) = match db.read_cursor().await {
            Ok(v) => v,
            Err(e) => return log_err(&e),
        };
        if let Ok(events) = db.read_events_after(watermark).await {
            if !events.is_empty() {
                let (deltas, sessions) = crate::rollup::fold(&mut state, &events);
                let _ = db.write_rollups(&deltas).await;
                let _ = db.write_sessions(&sessions).await;
                let _ = db.write_cursor(state.watermark, &state).await;
            }
        }
        let buckets = db.read_all_rollups().await.unwrap_or_default();
        let mut sessions = db.read_sessions().await.unwrap_or_default();
        for os in state.open_sessions.values() {
            sessions.push(os.provisional_record());
        }
        {
            let mut a = s.borrow_mut();
            a.buckets = buckets;
            a.sessions = sessions;
        }
        recompute_projection(&s);
        let _ = rerender(&s);
    });
}

// ── small DOM helpers (shared by the view modules) ──────────────────────────────

pub(crate) fn el(doc: &Document, tag: &str) -> Element {
    doc.create_element(tag).expect("create_element")
}

pub(crate) fn body_container(shared: &Shared) -> Option<Element> {
    shared.borrow().doc.get_element_by_id("bg-body")
}

/// Attach an event listener, leaking the closure for the page lifetime.
pub(crate) fn on<F>(target: &EventTarget, event: &str, f: F)
where
    F: 'static + FnMut(web_sys::Event),
{
    let cb = Closure::wrap(Box::new(f) as Box<dyn FnMut(web_sys::Event)>);
    target
        .add_event_listener_with_callback(event, cb.as_ref().unchecked_ref())
        .expect("add_event_listener");
    cb.forget();
}

pub(crate) fn log_err(e: &JsValue) {
    web_sys::console::error_1(e);
}

/// Minimal HTML-escape for embedding user URLs/keys in `set_inner_html`.
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
