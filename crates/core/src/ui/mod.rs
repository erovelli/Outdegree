//! Dashboard UI shell (§7.7), built imperatively with web-sys.
//!
//! Deviation from the §2 sketch: the controls are direct-DOM (CSR by
//! construction — the DOM is built at runtime, no server hydration), instead of a
//! reactive framework, to keep the WASM build predictable under the production
//! CSP. `app` is the thin composition root; the floating chrome (`chrome`,
//! `settings`, `modal`, `saved_views`, `shortcuts`, `onboarding`) and the view
//! modules (`filters`, `graph_view`, `tables`, `sankey`, `session_picker`) mirror
//! the spec's file breakdown.

mod app;
mod chrome;
mod filters;
mod graph_view;
mod inspector;
mod modal;
mod onboarding;
mod sankey;
mod saved_views;
mod session_picker;
mod settings;
mod shortcuts;
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

/// The END of the displayed time window when the user has stepped back through
/// history (§F6 time navigation). `App.anchor == None` is the live "latest" view
/// — today's behavior, which keeps re-projecting as new events arrive. An anchor
/// freezes the view on a past window. It is deliberately **never persisted** (not
/// part of `uiPrefs` or saved views), so every reload returns to the live view.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Anchor {
    /// A calendar window (Day/Week/Month/Year) ending on this UTC day-number —
    /// the same key `project.rs` buckets days by.
    Day(i64),
    /// A specific session in the sessions store, by its record `id` (Session
    /// range). ‹/› walk the store; the window label is that session's time range.
    Session(f64),
}

impl View {
    /// The persistable projection of this view. `Raw` collapses to `Graph`: the
    /// raw-events view is never persisted, so reopening after leaving it on Raw
    /// lands on the graph (Raw stays reachable only from the settings menu, §7.7).
    fn pref(self) -> crate::ui_prefs::PrefView {
        match self {
            View::Sankey => crate::ui_prefs::PrefView::Sankey,
            View::Tables => crate::ui_prefs::PrefView::Tables,
            View::Graph | View::Raw => crate::ui_prefs::PrefView::Graph,
        }
    }
}

impl From<crate::ui_prefs::PrefView> for View {
    fn from(p: crate::ui_prefs::PrefView) -> Self {
        match p {
            crate::ui_prefs::PrefView::Graph => View::Graph,
            crate::ui_prefs::PrefView::Sankey => View::Sankey,
            crate::ui_prefs::PrefView::Tables => View::Tables,
        }
    }
}

/// Whole-dashboard state. `db` is an `Rc<Db>` so async handlers can clone it out
/// of a short borrow and `.await` without holding a `RefCell` borrow.
pub(crate) struct App {
    pub db: Rc<Db>,
    pub buckets: Vec<DayBucket>,
    /// Buckets scoped to the most recent *session* (folded from that session's
    /// id-range), so the "Session" range is a real session — not just the latest
    /// UTC day, which a day-bucket window can't distinguish from "Day". The
    /// session id they were built for is cached so we only reload when it changes.
    pub session_buckets: Vec<DayBucket>,
    pub session_for: Option<f64>,
    pub sessions: Vec<SessionRec>,
    pub positions: HashMap<String, (f32, f32)>,
    pub gran: Granularity,
    pub filters: Filters,
    pub time_range: TimeRange,
    /// Time-navigation anchor (§F6): `None` is the live "latest" view; `Some`
    /// freezes a past window. Never persisted (see [`Anchor`]).
    pub anchor: Option<Anchor>,
    pub view: View,
    pub camera: Camera,
    pub paused: bool,
    /// "Lock" the layout: re-project on filter changes but keep node positions
    /// (no fresh force iterations), so the graph stops re-settling.
    pub locked: bool,
    pub doc: Document,
    pub root: Element,
    pub proj: GraphProjection,
    /// Louvain community id per node key for the current projection (drives the
    /// layout's cohesion force and the faint background hulls). Recomputed with
    /// the projection; achromatic in the render so it never adds a second hue.
    pub communities: HashMap<String, usize>,
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
    /// Selected day in the Sankey session picker's activity heatmap, as a local
    /// calendar-day key (`year*10000 + month0*100 + date`, see [`local_day_key`]).
    /// The session list is scoped to this day so months of sessions stay
    /// navigable; `None` until the picker defaults it to the latest session's day.
    pub selected_day: Option<i64>,
    /// The participating flow graph currently drawn in the Sankey, cached so a
    /// click can re-render with a focus highlight without re-reading the session's
    /// events. `sankey_focus` is the clicked `(seed_up, seed_down)` (equal for a
    /// node, the edge's endpoints for a ribbon); `None` means nothing is focused.
    pub sankey_flow: Option<crate::flow::FlowGraph>,
    pub sankey_focus: Option<(usize, usize)>,
    pub sankey_vw: f64,
    /// Session-picker filters: a host substring query and whether to hide trivial
    /// single-visit sessions (which are usually just a stray tab open).
    pub session_query: String,
    pub hide_trivial_sessions: bool,
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
    /// Opt-in (default off): parse search terms from already-captured result URLs
    /// and surface them in a Tables section. Off by default because search terms
    /// are sensitive; persisted under the "showSearches" storage key.
    pub show_searches: bool,
    /// Aggregated top search terms, populated only while `show_searches` is on.
    pub searches: Vec<crate::search::SearchCount>,
    /// Whether the loaded data is the onboarding sample dataset (§F4). Drives the
    /// persistent "Sample data" chip and suppresses the backup nudge. Sourced from
    /// the `demoData` meta flag on open; set on "Load sample data", cleared on
    /// "Exit sample" (which wipes `meta` via `clear_all`).
    pub demo_data: bool,
    /// Node inspector (§F8) — the right-docked detail panel, one state with the
    /// drill-down `focus` (open iff `focus` is set on the Graph view). These fields
    /// cache its rendering so `sync_chrome` doesn't rebuild the panel (resetting its
    /// scroll) or re-scan the events store on every redraw.
    ///
    /// Signature of the currently-rendered *static* panel content (host + window +
    /// projection shape + toggles). `None` when the panel is closed; unchanged →
    /// no rebuild.
    pub inspector_sig: Option<String>,
    /// Key of the async "Top pages" scan the cached results below belong to: the
    /// `(node, window, granularity)` identity. **Includes the window** (range +
    /// anchor), so stepping to another window invalidates the cache instead of
    /// showing stale URLs. `None` before the first scan.
    pub inspector_scan_key: Option<String>,
    /// Whether the async scan for `inspector_scan_key` is still in flight.
    pub inspector_scan_pending: bool,
    /// Cached most-visited pages for the current scan key (§F8 item f).
    pub inspector_pages: Vec<crate::inspect::PageVisit>,
    /// Whether the page scan hit its 20k-event cap (drives the "scanned the most
    /// recent N" note).
    pub inspector_pages_capped: bool,
    /// Cached per-host search terms for the current scan key (§F8 item g). Always
    /// collected by the scan (a local re-parse of the same already-read URLs) but
    /// *rendered* only while "Show search terms" is on — so toggling the setting
    /// surfaces or hides them from cache, without a rescan.
    pub inspector_searches: Vec<crate::search::SearchCount>,
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

    // Derived-schema reconcile (§F7): if this install's cached checkpoint/rollups
    // predate the current fold (e.g. an upgrade that added foreground-dwell/focus
    // state), invalidate them so the fold below rebuilds from raw exactly once. A
    // no-op on an already-current install.
    db.reconcile_derived_schema().await?;

    // Incremental fold: read the checkpoint, fold id > watermark, persist.
    let (watermark, mut state) = db.read_cursor().await?;
    let new_events = db.read_events_after(watermark).await?;
    if !new_events.is_empty() {
        let (deltas, sessions) = crate::rollup::fold(&mut state, &new_events);
        db.write_rollups(&deltas).await?;
        db.write_sessions(&sessions).await?;
        db.write_cursor(state.watermark, &state).await?;
    }

    // Restore persisted UI preferences (§7.7) before the first projection/render,
    // so the dashboard reopens the way it was left with no flash of default state.
    // A missing key, malformed JSON, or an unknown enum value falls back silently
    // to the defaults (see `ui_prefs::parse`).
    let prefs = crate::bridge::storage_local_get(crate::ui_prefs::STORAGE_KEY)
        .await
        .map(|json| crate::ui_prefs::parse(&json))
        .unwrap_or_default();

    // When the in-app-navigations mode was persisted on, the initial buckets must
    // come from the SPA-aware fold (events + spa from scratch), mirroring
    // `reload_buckets` — otherwise the first render would show the plain rollups.
    let buckets = if prefs.spa_mode {
        let events = db.read_events_after(0.0).await?;
        let spa = db.read_spa().await?;
        let merged = crate::rollup::merge_streams(&events, &spa);
        let mut st = crate::rollup::DeriveState::default();
        let mut b = crate::rollup::fold(&mut st, &merged).0;
        b.extend(crate::derive::provisional_buckets(&st));
        b
    } else {
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
    let show_searches = crate::bridge::storage_local_get("showSearches")
        .await
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    // Whether the onboarding sample dataset is currently loaded (§F4): drives the
    // "Sample data" chip and suppresses the backup nudge.
    let demo_data = db.read_meta_bool("demoData").await.unwrap_or(false);

    let app = App {
        db,
        buckets,
        session_buckets: Vec::new(),
        session_for: None,
        sessions,
        positions,
        gran: prefs.granularity,
        filters: Filters {
            min_visits: prefs.min_visits,
            hide_search_hubs: prefs.hide_search_hubs,
            hide_isolated: prefs.hide_isolated,
            // The legend provenance filter is transient and never persisted.
            provenance_in: None,
        },
        time_range: prefs.time_range,
        // The anchor is transient and never restored from prefs (§F6): the
        // dashboard always reopens on the live "latest" view.
        anchor: None,
        view: prefs.view.into(),
        camera: Camera::default(),
        paused,
        locked: prefs.locked,
        doc,
        root,
        proj: GraphProjection::default(),
        communities: HashMap::new(),
        layout_pos: HashMap::new(),
        layouts: HashMap::new(),
        hover: None,
        dragging: false,
        did_drag: false,
        drag_node: None,
        anim_gen: 0,
        last_mouse: (0.0, 0.0),
        selected_session: None,
        selected_day: None,
        sankey_flow: None,
        sankey_focus: None,
        sankey_vw: 0.0,
        session_query: String::new(),
        hide_trivial_sessions: false,
        // Restore the persisted in-app-navigations mode (§7.7): the SPA-aware
        // buckets were already folded above from `prefs.spa_mode`, so the state
        // flag must agree — otherwise the settings toggle and the next
        // `reload_buckets` would silently revert to the plain rollups.
        spa_mode: prefs.spa_mode,
        focus: None,
        legend_filter: None,
        resize_hooked: false,
        live_hooked: false,
        refreshing: false,
        show_searches,
        searches: Vec::new(),
        demo_data,
        inspector_sig: None,
        inspector_scan_key: None,
        inspector_scan_pending: false,
        inspector_pages: Vec::new(),
        inspector_pages_capped: false,
        inspector_searches: Vec::new(),
    };
    let shared: Shared = Rc::new(RefCell::new(app));

    recompute_projection(&shared);
    app::build_shell(&shared)?;
    persist_positions(&shared);
    rerender(&shared)?;
    install_live_refresh(&shared);
    // Warm the Session-range buckets so switching to "Session" is instant.
    refresh_session_buckets(&shared, false);
    // If the user previously opted in, warm the search-term aggregation too.
    if shared.borrow().show_searches {
        reload_searches(&shared);
    }
    // Decide whether the backup nudge should surface (§8.4): pure decision over
    // the live event count + persisted backup/snooze timestamps. Its event-count
    // gate keeps it off the empty/no-data state.
    settings::evaluate_backup_nudge(&shared);
    // First-run onboarding (§F4): show the welcome overlay on a truly empty,
    // not-yet-onboarded log; otherwise the "Sample data" chip (if demo data is
    // loaded) is already reflected by `sync_chrome`.
    onboarding::evaluate_first_run(&shared);
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
        let (view, anchored) = {
            let mut a = s.borrow_mut();
            a.buckets = buckets;
            a.sessions = sessions;
            (a.view, a.anchor.is_some())
        };
        // Anchored to a historical window (§F6): the new events are folded into the
        // model above (so they're current the moment the user returns to live), but
        // the frozen view is left untouched — no auto re-projection or re-render.
        // The live poll must never yank an anchored view out from under the user.
        if anchored {
            s.borrow_mut().refreshing = false;
            return;
        }
        recompute_projection(&s);
        // Soft refresh on the graph (preserve the camera) unless `refit` was
        // requested; full re-render for the data views or the empty graph state.
        let has_canvas = s.borrow().doc.get_element_by_id("bg-canvas").is_some();
        if view == View::Graph && has_canvas && !refit {
            chrome::sync_chrome(&s);
            graph_view::redraw(&s);
        } else {
            let _ = rerender(&s);
        }
        s.borrow_mut().refreshing = false;
        // If a newer session arrived while viewing the (live) Session range, rescope.
        if s.borrow().time_range == TimeRange::Session {
            refresh_session_buckets(&s, false);
        }
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

/// The calendar day-number the current anchor ends on, for the windowed
/// projection/KPI helpers in [`crate::project`] (§F6). `None` (the live view)
/// resolves to the latest bucket inside those helpers; a session anchor maps to
/// that session's UTC start day, so the calendar-window Tables cards scope to it.
pub(crate) fn anchor_end_day(a: &App) -> Option<i64> {
    match a.anchor {
        Some(Anchor::Day(d)) => Some(d),
        Some(Anchor::Session(id)) => a
            .sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| project::day_of_ms(s.start_ts)),
        None => None,
    }
}

/// The newest session (by `start_ts`) — the live "Latest" session for the
/// Session range, and the default when no anchor is set.
pub(crate) fn latest_session(sessions: &[SessionRec]) -> Option<SessionRec> {
    sessions
        .iter()
        .cloned()
        .fold(None, |acc: Option<SessionRec>, s| match acc {
            Some(b) if b.start_ts >= s.start_ts => Some(b),
            _ => Some(s),
        })
}

/// The session the Session range currently displays: the anchored one, or the
/// newest when live (or when the anchored id has gone away).
pub(crate) fn displayed_session(a: &App) -> Option<SessionRec> {
    match a.anchor {
        Some(Anchor::Session(id)) => a.sessions.iter().find(|s| s.id == id).cloned(),
        _ => None,
    }
    .or_else(|| latest_session(&a.sessions))
}

/// The day buckets the projection currently draws from: the session-scoped buckets
/// on the Session range (when they've loaded), else the anchored calendar window
/// (§F6). One source of truth for the "displayed window", so the projection, the
/// dwell fg/≈ signal, the inspector's share-of-visits, and its sparkline all scope
/// to exactly what the graph shows.
pub(crate) fn displayed_window(a: &App) -> Vec<DayBucket> {
    if a.time_range == TimeRange::Session && !a.session_buckets.is_empty() {
        a.session_buckets.clone()
    } else {
        project::select_window(&a.buckets, a.time_range, anchor_end_day(a))
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
    // The "Session" range uses buckets scoped to the latest session's events; if
    // those aren't loaded yet it falls back to the latest day until they arrive.
    let window = displayed_window(&a);
    let mut proj = project::project(&window, a.gran, &a.filters);
    // Drill-down: reduce to the focused node's full connected component, then the
    // graph view fits it on screen (§M3). The focus key is granularity-specific
    // (a hostname vs an eTLD+1) and also depends on the active range/filters, so a
    // key captured under one projection may not exist under another — toggling
    // hostname↔domain rekeys every node, and narrowing the range/filters can drop
    // the focused node entirely. When the focused key no longer resolves, clear
    // the focus and show the full graph instead of collapsing to an empty
    // component ("No navigations recorded").
    if let Some(focus) = a.focus.clone() {
        if proj.nodes.iter().any(|n| n.key == focus) {
            proj = project::component(&proj, &focus);
        } else {
            a.focus = None;
        }
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

    // Community detection (Louvain) drives both the layout's cohesion force and
    // the faint background hulls. Singletons (the disconnected typed/bookmark
    // nodes) each fall into their own community, so only genuine clusters group.
    let g = crate::graph::build(&proj);
    let comm_by_ix = crate::graph::louvain(&g);
    let mut key_comm: HashMap<String, usize> = HashMap::new();
    for (ix, c) in &comm_by_ix {
        key_comm.insert(g[*ix].key.clone(), *c);
    }
    let communities: Vec<usize> = keys
        .iter()
        .map(|k| key_comm.get(k).copied().unwrap_or(0))
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
    let placed = layout::fruchterman_reingold(&keys, &edges, iters, &seed, &communities);

    let mut layout_pos = HashMap::new();
    let mut snapshot = HashMap::new();
    for (k, p) in keys.iter().zip(placed.iter()) {
        layout_pos.insert(k.clone(), *p);
        a.positions.insert(k.clone(), (p.x, p.y));
        snapshot.insert(k.clone(), (p.x, p.y));
    }
    a.layouts.insert(sig, snapshot);
    a.proj = proj;
    a.communities = key_comm;
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
        chrome::sync_chrome(shared);
        graph_view::animate_to_fit(shared);
    } else {
        let _ = rerender(shared);
    }
}

/// Load buckets scoped to the session the Session range currently displays — the
/// anchored session, or the newest when live (§F6) — by folding that session's
/// id-range of raw events, then recompute + re-render. Lets the "Session" range
/// show a genuine session rather than the latest UTC day. No-op when there are no
/// sessions. When the target session's buckets are already loaded, it only
/// re-projects/re-renders if `force` is set (a ‹/› step or "Latest" jump changed
/// the label/graph); a passive live poll passes `false` to avoid a redundant redraw.
pub(crate) fn refresh_session_buckets(shared: &Shared, force: bool) {
    let (db, target, cached) = {
        let a = shared.borrow();
        let target = displayed_session(&a);
        let cached = matches!((&target, a.session_for), (Some(t), Some(f)) if t.id == f);
        (a.db.clone(), target, cached)
    };
    let Some(target) = target else {
        return; // no sessions: Session range falls back to the latest day
    };
    if cached {
        // Buckets for this session are already loaded; re-project + render only on
        // demand (an anchor/label change), and only while on the Session range.
        if force && shared.borrow().time_range == TimeRange::Session {
            recompute_projection(shared);
            let _ = rerender(shared);
        }
        return;
    }
    let s = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let events = db
            .read_events_id_range(target.start_id, target.end_id)
            .await
            .unwrap_or_default();
        // Fold this session's events from scratch into day buckets, plus the
        // still-open page (provisional) so the current page is included.
        let mut st = crate::rollup::DeriveState::default();
        let mut buckets = crate::rollup::fold(&mut st, &events).0;
        buckets.extend(crate::derive::provisional_buckets(&st));
        {
            let mut a = s.borrow_mut();
            a.session_buckets = buckets;
            a.session_for = Some(target.id);
        }
        if s.borrow().time_range == TimeRange::Session {
            recompute_projection(&s);
            let _ = rerender(&s);
        }
    });
}

/// Persist layout positions to the DB (spatial memory across opens, §7.6).
pub(crate) fn persist_positions(shared: &Shared) {
    let db = shared.borrow().db.clone();
    let pos = shared.borrow().positions.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = db.write_positions(&pos).await;
    });
}

/// Write-through the persisted UI preferences (§7.7): snapshot the current view
/// controls into one JSON document under `chrome.storage.local`, so the dashboard
/// reopens the way it was left. Called from every control that changes one of the
/// persisted knobs (view / range / granularity / min-visits / hide-search-hubs /
/// hide-isolated / in-app-navigations / lock). A plain write per change is fine at
/// this (human-paced) frequency; the value is small and the write is fire-and-
/// forget through the JS bridge. Transient state (focus, camera, hover, legend
/// filter, the selected session/day, the session-picker query, the Raw view, and
/// the §F6 time-navigation anchor) is deliberately excluded. The anchor is *not*
/// a field of [`crate::ui_prefs::UiPrefs`], so this write path cannot capture it.
pub(crate) fn persist_ui_prefs(shared: &Shared) {
    let a = shared.borrow();
    let prefs = crate::ui_prefs::UiPrefs {
        view: a.view.pref(),
        time_range: a.time_range,
        granularity: a.gran,
        min_visits: a.filters.min_visits,
        hide_search_hubs: a.filters.hide_search_hubs,
        hide_isolated: a.filters.hide_isolated,
        spa_mode: a.spa_mode,
        locked: a.locked,
    };
    crate::bridge::storage_local_set(
        crate::ui_prefs::STORAGE_KEY,
        &crate::ui_prefs::serialize(&prefs),
    );
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
        {
            let mut a = s.borrow_mut();
            a.buckets = buckets;
            // Switching the SPA/in-app-navigations lens rebuilds the whole bucket
            // set and re-scopes sessions; a stale anchor could point off the new
            // timeline, so return to the live view.
            a.anchor = None;
        }
        recompute_projection(&s);
        let _ = rerender(&s);
    });
}

/// Recompute the opt-in search-term aggregation from already-captured event URLs
/// and rerender. Clears the list (and rerenders) when the toggle is off. No new
/// capture, no network — it only reads URLs already in the event store.
pub(crate) fn reload_searches(shared: &Shared) {
    let s = shared.clone();
    let db = shared.borrow().db.clone();
    let on = shared.borrow().show_searches;
    wasm_bindgen_futures::spawn_local(async move {
        let searches = if on {
            let events = db.read_events_after(0.0).await.unwrap_or_default();
            let urls: Vec<String> = events
                .into_iter()
                .filter_map(|e| match e {
                    crate::model::Event::Nav { to_url, .. } => Some(to_url),
                    _ => None,
                })
                .collect();
            crate::search::top_searches(&urls, 20)
        } else {
            Vec::new()
        };
        s.borrow_mut().searches = searches;
        let _ = rerender(&s);
    });
}

/// Render the active view into the body container.
pub(crate) fn rerender(shared: &Shared) -> Result<(), JsValue> {
    chrome::sync_chrome(shared);
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
            // A destructive edit re-derives everything; invalidate the cached
            // session scope so it reloads against the new id-ranges.
            a.session_for = None;
            a.session_buckets.clear();
            // A wiped/reimported timeline invalidates any historical anchor (its
            // day-number or session id may no longer exist), so drop back to live.
            a.anchor = None;
        }
        recompute_projection(&s);
        let _ = rerender(&s);
        if s.borrow().time_range == TimeRange::Session {
            refresh_session_buckets(&s, false);
        }
    });
}

/// Cause-specific empty-state copy, so a blank view explains *why* it's blank and
/// what to do — instead of the one-size "browse a bit" message that misleads when
/// recording is paused or a filter is simply too tight.
pub(crate) fn empty_body_html(a: &App) -> String {
    let any_data = a.buckets.iter().any(|b| !b.nodes.is_empty());
    let msg = if !any_data {
        if a.paused {
            "Recording is paused. Press REC (top-left) to resume, then browse a little and reopen."
        } else {
            "No navigations recorded yet. Browse a few sites, then reopen this dashboard."
        }
    } else {
        "No sites match the current range and filters. Widen the range, or clear the min-visits / \
         hide-singletons / hide-search-hubs filters."
    };
    format!("<div class=\"bg-empty\">{msg}</div>")
}

// ── small DOM helpers (shared by the view modules) ──────────────────────────────

pub(crate) fn el(doc: &Document, tag: &str) -> Element {
    doc.create_element(tag).expect("create_element")
}

pub(crate) fn span(doc: &Document, class: &str, text: &str) -> Element {
    let s = el(doc, "span");
    let _ = s.set_attribute("class", class);
    s.set_text_content(Some(text));
    s
}

pub(crate) fn set_text(doc: &Document, id: &str, text: &str) {
    if let Some(e) = doc.get_element_by_id(id) {
        e.set_text_content(Some(text));
    }
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

/// Compact human duration (`45s`, `12m`, `1h 20m`, `2d 3h`) for dwell columns.
pub(crate) fn fmt_dwell(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86_400, (s % 86_400) / 3600)
    }
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

/// Local calendar-day key for a `Date`: `year*10000 + month0*100 + date`. This is
/// monotonic (ordering matches the calendar), unique per local day, and — unlike a
/// UTC-epoch day index — free of timezone/DST reversal ambiguity, so it's the key
/// the Sankey activity heatmap buckets sessions into and highlights on.
pub(crate) fn day_key_of(d: &js_sys::Date) -> i64 {
    d.get_full_year() as i64 * 10_000 + d.get_month() as i64 * 100 + d.get_date() as i64
}

/// [`day_key_of`] for an epoch-ms timestamp, read in the browser's local timezone.
pub(crate) fn local_day_key(ts: f64) -> i64 {
    day_key_of(&js_sys::Date::new(&JsValue::from_f64(ts)))
}

/// 12-hour local clock (`11:23 AM`) for session time labels.
fn clock(d: &js_sys::Date) -> String {
    let h = d.get_hours(); // u32, 0–23, local
    let m = d.get_minutes();
    let (h12, ap) = match h {
        0 => (12, "AM"),
        12 => (12, "PM"),
        1..=11 => (h, "AM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {ap}")
}

/// Short local `M/D` date.
fn md(d: &js_sys::Date) -> String {
    format!("{}/{}", d.get_month() + 1, d.get_date())
}

/// Human, local-time label for a session: `"6/21 · 11:23 AM – 4:22 PM"` (and
/// `"6/21 11:50 PM – 6/22 12:30 AM"` when it crosses midnight). Replaces the
/// opaque Chrome window id in the Sankey header. Reads in the user's own
/// timezone via the browser's local `Date`.
pub(crate) fn session_when(start_ts: f64, end_ts: f64) -> String {
    let a = js_sys::Date::new(&JsValue::from_f64(start_ts));
    let b = js_sys::Date::new(&JsValue::from_f64(end_ts));
    let same_day = a.get_full_year() == b.get_full_year()
        && a.get_month() == b.get_month()
        && a.get_date() == b.get_date();
    if same_day {
        format!("{} · {} – {}", md(&a), clock(&a), clock(&b))
    } else {
        format!("{} {} – {} {}", md(&a), clock(&a), md(&b), clock(&b))
    }
}

/// Time-of-day range without the date (`"11:23 AM – 4:22 PM"`), for session list
/// items already scoped under a day header — repeating the date on every item is
/// noise. Falls back to the dated form when the session crosses midnight.
pub(crate) fn session_clock_range(start_ts: f64, end_ts: f64) -> String {
    let a = js_sys::Date::new(&JsValue::from_f64(start_ts));
    let b = js_sys::Date::new(&JsValue::from_f64(end_ts));
    let same_day = a.get_full_year() == b.get_full_year()
        && a.get_month() == b.get_month()
        && a.get_date() == b.get_date();
    if same_day {
        format!("{} – {}", clock(&a), clock(&b))
    } else {
        session_when(start_ts, end_ts)
    }
}
