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
use crate::model::{Granularity, GraphProjection, Provenance};
use crate::project::{self, Filters, TimeRange};
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
    pub time_range: TimeRange,
    pub view: View,
    pub camera: Camera,
    pub paused: bool,
    /// "Lock" the layout: re-project on filter changes but keep node positions
    /// (no fresh force iterations), so the graph stops re-settling.
    pub locked: bool,
    pub doc: Document,
    pub root: Element,
    pub proj: GraphProjection,
    pub layout_pos: HashMap<String, Pos>,
    /// Exact node positions per graph *shape* (see [`project::layout_signature`]),
    /// so revisiting the same graph this session restores an identical picture
    /// instead of re-running the force layout. New shapes warm-start from
    /// `positions` (cross-session spatial memory); only this session is cached.
    pub layouts: HashMap<u64, HashMap<String, (f32, f32)>>,
    pub hover: Option<String>,
    pub dragging: bool,
    pub did_drag: bool,
    /// The node currently being dragged to reposition it (else canvas pan).
    pub drag_node: Option<String>,
    /// Generation counter for the camera tween; bumped to cancel an in-flight
    /// animation when a new one starts or the user takes manual control.
    pub anim_gen: u64,
    pub last_mouse: (f64, f64),
    pub selected_session: Option<f64>,
    /// Opt-in "in-app navigations" view: fold `events` + `spa` from scratch (§4.2).
    pub spa_mode: bool,
    /// Drill-down focus: when set, the graph shows this node's ego network (§M3).
    pub focus: Option<String>,
    /// Legend highlight: a clicked provenance key keeps only its nodes bright.
    pub legend_filter: Option<Provenance>,
    /// Whether the window-resize listener has been installed (install once).
    pub resize_hooked: bool,
    /// Whether the live-refresh listeners have been installed (install once).
    pub live_hooked: bool,
    /// Guards against overlapping live-refresh folds (rollups merge, so a double
    /// fold of the same events would double-count).
    pub refreshing: bool,
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

    let buckets = {
        let mut b = db.read_all_rollups().await?;
        b.extend(crate::derive::provisional_buckets(&state));
        b
    };
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
        time_range: TimeRange::default(),
        view: View::Graph,
        camera: Camera::default(),
        paused,
        locked: false,
        doc,
        root,
        proj: GraphProjection::default(),
        layout_pos: HashMap::new(),
        layouts: HashMap::new(),
        hover: None,
        dragging: false,
        did_drag: false,
        drag_node: None,
        anim_gen: 0,
        last_mouse: (0.0, 0.0),
        selected_session: None,
        spa_mode: false,
        focus: None,
        legend_filter: None,
        resize_hooked: false,
        live_hooked: false,
        refreshing: false,
    };
    let shared: Shared = Rc::new(RefCell::new(app));

    recompute_projection(&shared);
    app::build_shell(&shared)?;
    persist_positions(&shared);
    rerender(&shared)?;
    install_live_refresh(&shared);
    Ok(())
}

/// Fold any events captured since the last fold and refresh the view, so new
/// browsing shows up without reopening the dashboard. Installed on tab focus /
/// visibility and a gentle visible-only poll (`install_live_refresh`).
///
/// When nothing new arrived it returns without touching the DOM (so it never
/// disrupts the user's current pan/zoom). `refit` re-frames the graph (used when
/// returning to the tab, so new nodes aren't left off-screen); when false the
/// graph soft-refreshes — redrawing at the current camera (used by the poll, so
/// it doesn't yank a view you're actively examining).
pub(crate) fn live_refresh(shared: &Shared, refit: bool) {
    {
        let mut a = shared.borrow_mut();
        if a.refreshing {
            return; // a fold is already in flight; rollups merge, don't double it
        }
        a.refreshing = true;
    }
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let (watermark, mut state) = match db.read_cursor().await {
            Ok(v) => v,
            Err(_) => {
                s.borrow_mut().refreshing = false;
                return;
            }
        };
        let new_events = db.read_events_after(watermark).await.unwrap_or_default();
        if new_events.is_empty() {
            s.borrow_mut().refreshing = false;
            return; // nothing new → leave the current view untouched
        }
        let (deltas, sessions_new) = crate::rollup::fold(&mut state, &new_events);
        let _ = db.write_rollups(&deltas).await;
        let _ = db.write_sessions(&sessions_new).await;
        let _ = db.write_cursor(state.watermark, &state).await;

        let buckets = {
            let mut b = db.read_all_rollups().await.unwrap_or_default();
            b.extend(crate::derive::provisional_buckets(&state));
            b
        };
        let mut sessions = db.read_sessions().await.unwrap_or_default();
        for os in state.open_sessions.values() {
            sessions.push(os.provisional_record());
        }
        let view = {
            let mut a = s.borrow_mut();
            a.buckets = buckets;
            a.sessions = sessions;
            a.view
        };
        recompute_projection(&s);
        // Soft refresh on the graph (preserve the camera) unless `refit` was
        // requested; full re-render for the data views or the empty graph state.
        let has_canvas = s.borrow().doc.get_element_by_id("bg-canvas").is_some();
        if view == View::Graph && has_canvas && !refit {
            app::sync_chrome(&s);
            graph_view::redraw(&s);
        } else {
            let _ = rerender(&s);
        }
        s.borrow_mut().refreshing = false;
    });
}

/// Install the live-refresh triggers once: refresh on tab focus / becoming
/// visible, plus a gentle visible-only poll for an unfocused (e.g. second-
/// monitor) dashboard.
fn install_live_refresh(shared: &Shared) {
    if shared.borrow().live_hooked {
        return;
    }
    shared.borrow_mut().live_hooked = true;
    let Some(win) = web_sys::window() else { return };
    let Some(doc) = win.document() else { return };

    // Returning to the tab refits (new nodes shouldn't land off-screen)…
    {
        let s = shared.clone();
        on(win.as_ref(), "focus", move |_| live_refresh(&s, true));
    }
    {
        let s = shared.clone();
        let d = doc.clone();
        on(doc.unchecked_ref(), "visibilitychange", move |_| {
            if !d.hidden() {
                live_refresh(&s, true);
            }
        });
    }
    // …while the gentle poll preserves the view you're actively examining.
    {
        let s = shared.clone();
        let d = doc.clone();
        let cb = Closure::wrap(Box::new(move || {
            if !d.hidden() {
                live_refresh(&s, false);
            }
        }) as Box<dyn FnMut()>);
        let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            15_000,
        );
        cb.forget();
    }
}

/// Re-project the in-memory buckets at the current granularity/filters and warm-
/// start a fresh layout, preserving spatial memory for surviving nodes (§7.6).
pub(crate) fn recompute_projection(shared: &Shared) {
    recompute_projection_inner(shared, true);
}

/// Re-project keeping existing node positions (no fresh force iterations). Used
/// for focus transitions so the camera can animate over a stable layout.
pub(crate) fn recompute_projection_keep(shared: &Shared) {
    recompute_projection_inner(shared, false);
}

fn recompute_projection_inner(shared: &Shared, relayout: bool) {
    let mut a = shared.borrow_mut();
    // Restrict to the selected time window (design "Range" control), then project.
    let window = project::select_window(&a.buckets, a.time_range);
    let mut proj = project::project(&window, a.gran, &a.filters);
    // Drill-down: reduce to the focused node's full connected component, then the
    // graph view fits it on screen (§M3).
    if let Some(focus) = a.focus.clone() {
        proj = project::component(&proj, &focus);
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

    // The same graph shape must produce the same picture, so an idempotent
    // interaction (re-picking a range/granularity, or a window that resolves to
    // the same data) never nudges the layout. If we've laid this shape out this
    // session, restore those exact positions (0 iterations); only a *new* shape
    // runs the force layout, warm-started from cross-session spatial memory.
    let sig = project::layout_signature(&proj);
    let known = a.layouts.contains_key(&sig);
    let iters = if a.locked || known || !relayout {
        0
    } else {
        320
    };
    let seed = if known {
        a.layouts[&sig].clone()
    } else {
        a.positions.clone()
    };
    let placed = layout::fruchterman_reingold(&keys, &edges, iters, &seed);

    let mut layout_pos = HashMap::new();
    let mut snapshot = HashMap::new();
    for (k, p) in keys.iter().zip(placed.iter()) {
        layout_pos.insert(k.clone(), *p);
        a.positions.insert(k.clone(), (p.x, p.y));
        snapshot.insert(k.clone(), (p.x, p.y));
    }
    a.layouts.insert(sig, snapshot);
    a.proj = proj;
    a.layout_pos = layout_pos;
}

/// Re-snapshot the current on-screen layout under its shape signature, so a manual
/// rearrangement (a node drag) becomes the layout this shape restores to — instead
/// of being reverted by the next idempotent re-projection.
pub(crate) fn sync_layout_cache(shared: &Shared) {
    let mut a = shared.borrow_mut();
    let sig = project::layout_signature(&a.proj);
    let snap: HashMap<String, (f32, f32)> = a
        .layout_pos
        .iter()
        .map(|(k, p)| (k.clone(), (p.x, p.y)))
        .collect();
    a.layouts.insert(sig, snap);
}

/// Set the drill-down focus and animate the camera from the current view to frame
/// the focused component (a smooth pan/zoom). Falls back to a full re-render when
/// the graph canvas isn't mounted (e.g. selecting from another tab).
pub(crate) fn focus_and_animate(shared: &Shared, new_focus: Option<String>) {
    shared.borrow_mut().focus = new_focus;
    recompute_projection_keep(shared); // keep positions so only the camera moves
    let ready = {
        let a = shared.borrow();
        a.view == View::Graph && a.doc.get_element_by_id("bg-canvas").is_some()
    };
    if ready {
        app::sync_chrome(shared);
        graph_view::animate_to_fit(shared);
    } else {
        let _ = rerender(shared);
    }
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
            let mut b = crate::rollup::fold(&mut st, &merged).0;
            b.extend(crate::derive::provisional_buckets(&st));
            b
        } else {
            let mut b = db.read_all_rollups().await.unwrap_or_default();
            if let Ok((_, state)) = db.read_cursor().await {
                b.extend(crate::derive::provisional_buckets(&state));
            }
            b
        };
        s.borrow_mut().buckets = buckets;
        recompute_projection(&s);
        let _ = rerender(&s);
    });
}

/// Render the active view into the body container.
pub(crate) fn rerender(shared: &Shared) -> Result<(), JsValue> {
    app::sync_chrome(shared);
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
        let buckets = {
            let mut b = db.read_all_rollups().await.unwrap_or_default();
            b.extend(crate::derive::provisional_buckets(&state));
            b
        };
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

/// Pluralize a regular count noun: `1 nav`, `2 navs`, `0 nodes`. Keeps the
/// readouts and counts grammatical instead of always appending `s`.
pub(crate) fn plural(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
