//! Graph view: canvas setup, draw, and pan/zoom/hover interactions (§7.7, M3).

use super::{body_container, el, on, Shared, View};
use crate::render::canvas2d::{self, Camera};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, KeyboardEvent, MouseEvent,
    WheelEvent,
};

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let body = body_container(shared).ok_or_else(|| JsValue::from_str("no body"))?;
    body.set_inner_html("");

    if shared.borrow().proj.nodes.is_empty() {
        body.set_inner_html(&super::empty_body_html(&shared.borrow()));
        return Ok(());
    }

    let doc = shared.borrow().doc.clone();
    let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;

    let canvas: HtmlCanvasElement = el(&doc, "canvas")
        .dyn_into()
        .map_err(|_| JsValue::from_str("canvas cast"))?;
    let _ = canvas.set_attribute("id", "bg-canvas");
    // The canvas can't be read by assistive tech; describe the graph and point at
    // the Tables view, which is the keyboard/screen-reader-navigable equivalent of
    // the same projection (hubs, journeys, communities, edges).
    let _ = canvas.set_attribute("role", "img");
    {
        let a = shared.borrow();
        let label = format!(
            "Browsing graph: {}, {}. Use the Tables view (top-right) for a \
             screen-reader-accessible breakdown of the same data.",
            super::plural(a.proj.nodes.len() as u64, "site"),
            super::plural(a.proj.edges.len() as u64, "link"),
        );
        let _ = canvas.set_attribute("aria-label", &label);
    }
    // Keyboard-focusable (§F10 a11y): Tab reaches the canvas, then arrows pan /
    // +/− zoom / 0 or F fit / Esc clears focus (see `attach_interactions`). This
    // is viewport control only — keyboard node *targeting* is intentionally left
    // as future work; the Tables view (pointed at by the aria-label above) is the
    // screen-reader-navigable equivalent.
    let _ = canvas.set_attribute("tabindex", "0");

    // The canvas *is* the app: full-bleed under the floating chrome. Its
    // position/size live in CSS (`#bg-canvas`) — not an inline style — so it
    // survives the page CSP. The backing store is scaled by devicePixelRatio so
    // text/edges stay crisp on HiDPI displays; drawing happens in CSS pixels (the
    // draw transform is set to `dpr` in `draw_now`).
    let w = win_dim(&win, true).max(320.0);
    let h = win_dim(&win, false).max(320.0);
    let d = dpr();
    canvas.set_width((w * d) as u32);
    canvas.set_height((h * d) as u32);
    let _ = body.append_child(&canvas);

    // Frame the laid-out nodes so the graph is visible even when sparse/edgeless.
    // Manual pan/zoom then adjusts from here until the next render.
    {
        let mut a = shared.borrow_mut();
        let cam = canvas2d::fit(&a.proj, &a.layout_pos, w, h);
        a.camera = cam;
    }

    draw_now(shared, &canvas);
    attach_interactions(shared, &canvas);
    install_resize_hook(shared);
    Ok(())
}

/// Device pixel ratio, clamped so a 4× display can't blow up the backing store.
fn dpr() -> f64 {
    web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0)
        .clamp(1.0, 3.0)
}

/// The canvas size in CSS pixels (the backing store is `dpr×` larger).
fn logical_dims(canvas: &HtmlCanvasElement) -> (f64, f64) {
    let d = dpr();
    (canvas.width() as f64 / d, canvas.height() as f64 / d)
}

/// Inner viewport `width` (or height) in CSS pixels.
fn win_dim(win: &web_sys::Window, width: bool) -> f64 {
    let v = if width {
        win.inner_width()
    } else {
        win.inner_height()
    };
    v.ok().and_then(|j| j.as_f64()).unwrap_or(800.0)
}

/// Install a one-time window-resize listener that re-frames the graph to the new
/// viewport size (only while the Graph tab is active).
fn install_resize_hook(shared: &Shared) {
    if shared.borrow().resize_hooked {
        return;
    }
    shared.borrow_mut().resize_hooked = true;
    let Some(win) = web_sys::window() else { return };
    let s = shared.clone();
    on(win.as_ref(), "resize", move |_| {
        if s.borrow().view == super::View::Graph {
            let _ = render(&s);
        }
    });
}

fn ctx_of(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas.get_context("2d").ok().flatten()?.dyn_into().ok()
}

/// The live graph canvas, if the Graph view is mounted.
fn canvas_el(shared: &Shared) -> Option<HtmlCanvasElement> {
    shared
        .borrow()
        .doc
        .get_element_by_id("bg-canvas")?
        .dyn_into()
        .ok()
}

/// Redraw the canvas at the current camera (no re-fit). Used by toolbar zoom.
pub(crate) fn redraw(shared: &Shared) {
    if let Some(c) = canvas_el(shared) {
        draw_now(shared, &c);
    }
}

/// Multiply the zoom about the canvas center and redraw.
pub(crate) fn zoom(shared: &Shared, factor: f64) {
    {
        let mut a = shared.borrow_mut();
        a.camera.scale = (a.camera.scale * factor).clamp(0.1, 8.0);
    }
    redraw(shared);
}

/// Pan the camera by a screen-space delta and redraw (keyboard arrow panning).
/// Cancels any in-flight tween first, like a manual drag. Instant — no animation
/// — so there is nothing for prefers-reduced-motion to suppress.
fn pan(shared: &Shared, dx: f64, dy: f64) {
    {
        let mut a = shared.borrow_mut();
        a.anim_gen = a.anim_gen.wrapping_add(1); // cancel any camera tween
        a.camera.x += dx;
        a.camera.y += dy;
    }
    redraw(shared);
}

/// Re-frame all nodes to fit the canvas and redraw (the "fit-to-screen" button).
pub(crate) fn fit_view(shared: &Shared) {
    if let Some(c) = canvas_el(shared) {
        let (w, h) = logical_dims(&c);
        let cam = {
            let a = shared.borrow();
            canvas2d::fit(&a.proj, &a.layout_pos, w, h)
        };
        shared.borrow_mut().camera = cam;
        draw_now(shared, &c);
    }
}

/// Animate the camera to frame the current projection, tweening from the current
/// view — the smooth pan/zoom into a selected node's component.
pub(crate) fn animate_to_fit(shared: &Shared) {
    let Some(c) = canvas_el(shared) else { return };
    let (w, h) = logical_dims(&c);
    let target = {
        let a = shared.borrow();
        canvas2d::fit(&a.proj, &a.layout_pos, w, h)
    };
    animate_to(shared, target);
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn cam_close(a: &Camera, b: &Camera) -> bool {
    (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5 && (a.scale - b.scale).abs() < 1e-3
}
fn request_frame(cb: &Closure<dyn FnMut()>) {
    if let Some(win) = web_sys::window() {
        let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
    }
}

/// Self-referencing handle for the rAF loop (so the closure can re-request and
/// drop itself).
type RafHandle = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// Whether the user prefers reduced motion (OS/browser setting). When set, camera
/// transitions jump instead of tweening.
fn reduced_motion() -> bool {
    web_sys::window()
        .and_then(|w| {
            w.match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .map(|m| m.matches())
        .unwrap_or(false)
}

/// Tween the camera to `target` (ease-out, ~380ms), redrawing each frame.
/// Cancellable via `App::anim_gen` (a new tween or a manual pan/zoom supersedes).
fn animate_to(shared: &Shared, target: Camera) {
    // Respect reduced-motion: snap straight to the framed view, no tween.
    if reduced_motion() {
        let mut a = shared.borrow_mut();
        a.anim_gen = a.anim_gen.wrapping_add(1); // cancel any in-flight tween
        a.camera = target;
        drop(a);
        redraw(shared);
        return;
    }
    let start = {
        let mut a = shared.borrow_mut();
        a.anim_gen = a.anim_gen.wrapping_add(1);
        a.camera
    };
    let gen = shared.borrow().anim_gen;
    if cam_close(&start, &target) {
        shared.borrow_mut().camera = target;
        redraw(shared);
        return;
    }

    let dur = 380.0_f64;
    let t0 = js_sys::Date::now();
    let f: RafHandle = Rc::new(RefCell::new(None));
    let g = f.clone();
    let s = shared.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        if s.borrow().anim_gen != gen {
            let _ = f.borrow_mut().take(); // superseded → stop
            return;
        }
        let t = ((js_sys::Date::now() - t0) / dur).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - t).powi(3); // ease-out cubic
        {
            let mut a = s.borrow_mut();
            a.camera = Camera {
                x: lerp(start.x, target.x, e),
                y: lerp(start.y, target.y, e),
                scale: lerp(start.scale, target.scale, e),
            };
        }
        redraw(&s);
        if t < 1.0 {
            if let Some(cb) = f.borrow().as_ref() {
                request_frame(cb);
            }
        } else {
            let _ = f.borrow_mut().take();
        }
    }) as Box<dyn FnMut()>));
    {
        let b = g.borrow();
        if let Some(cb) = b.as_ref() {
            request_frame(cb);
        }
    }
}

/// Export the graph to a PNG: render the whole projection (fit-to-page, no
/// hover/focus/filter chrome) to an offscreen canvas at 2× for crispness, then
/// hand `toDataURL` bytes to the local download bridge — independent of the live
/// camera's pan/zoom.
pub(crate) fn export_png(shared: &Shared) {
    const PAGE_W: f64 = 1600.0;
    const PAGE_H: f64 = 1000.0;
    const SS: f64 = 2.0; // supersample for crisp text/edges
    let a = shared.borrow();
    let Ok(canvas) = a
        .doc
        .create_element("canvas")
        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().map_err(|_| JsValue::NULL))
    else {
        return;
    };
    canvas.set_width((PAGE_W * SS) as u32);
    canvas.set_height((PAGE_H * SS) as u32);
    let Some(ctx) = ctx_of(&canvas) else { return };
    let _ = ctx.set_transform(SS, 0.0, 0.0, SS, 0.0, 0.0);
    let cam = canvas2d::fit(&a.proj, &a.layout_pos, PAGE_W, PAGE_H);
    // Exports stay in the pure shape/hue language — no favicons (icons are a live,
    // interactive affordance, and an export shouldn't depend on async decode state).
    canvas2d::draw(
        &ctx,
        PAGE_W,
        PAGE_H,
        &a.proj,
        &a.layout_pos,
        &cam,
        None,
        None,
        None,
        &a.communities,
        None,
    );
    match canvas.to_data_url_with_type("image/png") {
        Ok(url) => crate::bridge::download_data_url("outdegree-graph.png", &url),
        Err(e) => super::log_err(&e),
    }
}

/// Export the graph as a standalone, fit-to-page SVG (vector) via the pure
/// `crate::svg` serializer and the local download bridge.
pub(crate) fn export_svg(shared: &Shared) {
    let a = shared.borrow();
    let svg = crate::svg::graph_svg(&a.proj, &a.layout_pos, 1600.0, 1000.0);
    crate::bridge::download_text("outdegree-graph.svg", "image/svg+xml", &svg);
}

fn draw_now(shared: &Shared, canvas: &HtmlCanvasElement) {
    let Some(ctx) = ctx_of(canvas) else { return };
    // Draw under a short borrow, collecting favicon "misses" (§F12); then drop the
    // borrow before kicking off any loads (they take `borrow_mut`).
    let misses = {
        let a = shared.borrow();
        // Draw in CSS pixels onto the dpr-scaled backing store (crisp on HiDPI).
        let d = dpr();
        let _ = ctx.set_transform(d, 0.0, 0.0, d, 0.0, 0.0);
        let (w, h) = (canvas.width() as f64 / d, canvas.height() as f64 / d);
        // Site icons are drawn only when the setting is on AND the browser grants
        // the favicon permission (`site_icon_base` gates both) — otherwise `None`,
        // so `draw` constructs no icon work and reports no misses.
        let icons = super::site_icon_base(&a).is_some().then_some(&a.favicons);
        canvas2d::draw(
            &ctx,
            w,
            h,
            &a.proj,
            &a.layout_pos,
            &a.camera,
            a.hover.as_deref(),
            a.focus.as_deref(),
            a.legend_filter,
            &a.communities,
            icons,
        )
    };
    // Start a load per newly-seen host (load-once, enforced by the cache's `begin`).
    // `misses` is already capped at the cache's remaining capacity (the §F12 churn
    // guard): the cache never evicts, so once it saturates this list is empty on
    // every subsequent frame — no repeat loads, rAF repaints, or image allocations.
    // 32px on HiDPI for crispness, 16px otherwise — the size the spec calls for.
    if !misses.is_empty() {
        let size = if dpr() > 1.0 { 32 } else { 16 };
        for host in misses {
            start_favicon_load(shared, host, size);
        }
    }
}

/// Kick off an async favicon decode for `host` at `size` (§F12). Reserves a
/// load-once cache slot (so repeat frames never restart it), then loads an
/// `HtmlImageElement` off the extension's own `_favicon/` origin — Chrome serves it
/// from its LOCAL cache, no network. On decode it stores the image and schedules a
/// single coalesced redraw; on error (or an empty raster) it marks the slot
/// `Failed`, so the node keeps its provenance shape and is never retried. Never
/// blocks the frame that requested it.
fn start_favicon_load(shared: &Shared, host: String, size: u16) {
    let base = match &shared.borrow().favicon_base {
        Some(b) => b.clone(),
        None => return, // permission not granted → feature inert
    };
    // Reserve the slot; `begin` returns false if a load is already in flight / done.
    if !shared.borrow_mut().favicons.begin(&host) {
        return;
    }
    let url = crate::favicon::favicon_url(&base, &host, size);
    let Ok(img) = HtmlImageElement::new() else {
        shared.borrow_mut().favicons.set_failed(&host);
        return;
    };
    img.set_decoding("async");
    {
        let s = shared.clone();
        let h = host.clone();
        let im = img.clone();
        let onload = Closure::once_into_js(move || {
            // A decoded, non-empty raster is a real icon; a 0×0 result is treated as
            // a failure so we fall back to the shape rather than draw nothing.
            if im.natural_width() > 0 {
                s.borrow_mut().favicons.set_ready(&h, im.clone());
            } else {
                s.borrow_mut().favicons.set_failed(&h);
            }
            schedule_icon_redraw(&s);
        });
        img.set_onload(Some(onload.unchecked_ref()));
    }
    {
        let s = shared.clone();
        let h = host.clone();
        let onerror = Closure::once_into_js(move || {
            // Missing cached icon / blocked load: keep the shape, never retry (§F12).
            s.borrow_mut().favicons.set_failed(&h);
        });
        img.set_onerror(Some(onerror.unchecked_ref()));
    }
    img.set_src(&url);
}

/// Coalesce the per-image `onload` redraws into one animation frame, so a burst of
/// favicon loads repaints the canvas once per frame instead of once per icon.
fn schedule_icon_redraw(shared: &Shared) {
    if shared.borrow().favicon_redraw_pending {
        return;
    }
    shared.borrow_mut().favicon_redraw_pending = true;
    let s = shared.clone();
    let cb = Closure::once_into_js(move || {
        s.borrow_mut().favicon_redraw_pending = false;
        // Only repaint if the graph canvas is still mounted (the user may have
        // switched views while icons were decoding).
        if s.borrow().view == View::Graph {
            redraw(&s);
        }
    });
    if let Some(win) = web_sys::window() {
        let _ = win.request_animation_frame(cb.unchecked_ref());
    }
}

fn attach_interactions(shared: &Shared, canvas: &HtmlCanvasElement) {
    // Keyboard control while the canvas holds focus (§F10 a11y): arrows pan, +/−
    // zoom, 0 or F fit. Esc is left to the global handler (which clears any
    // drill-down focus and then blurs the canvas). All responses are instant (no
    // camera tween), matching the mouse zoom/fit controls, so prefers-reduced-
    // motion has nothing to suppress here.
    //
    // FUTURE WORK: keyboard node *targeting* (Tab/arrow to select a node, Enter to
    // drill in) is deliberately out of scope for this pass — the Tables view stays
    // the accessible, screen-reader-navigable equivalent (see the canvas
    // aria-label). This handler is viewport navigation only.
    {
        let s = shared.clone();
        on(canvas.as_ref(), "keydown", move |ev| {
            let Ok(ke) = ev.dyn_into::<KeyboardEvent>() else {
                return;
            };
            if ke.meta_key() || ke.ctrl_key() || ke.alt_key() {
                return; // don't shadow browser/OS chords
            }
            const PAN: f64 = 48.0;
            let handled = match ke.key().as_str() {
                "ArrowLeft" => {
                    pan(&s, PAN, 0.0);
                    true
                }
                "ArrowRight" => {
                    pan(&s, -PAN, 0.0);
                    true
                }
                "ArrowUp" => {
                    pan(&s, 0.0, PAN);
                    true
                }
                "ArrowDown" => {
                    pan(&s, 0.0, -PAN);
                    true
                }
                "+" | "=" => {
                    zoom(&s, 1.2);
                    true
                }
                "-" | "_" => {
                    zoom(&s, 1.0 / 1.2);
                    true
                }
                "0" | "f" | "F" => {
                    fit_view(&s);
                    true
                }
                _ => false,
            };
            if handled {
                // Prevent page scroll / browser zoom for keys we consume. The global
                // arrow-key handler already yields when the canvas is the target, so
                // there is no double-fire with time-window stepping.
                ke.prevent_default();
            }
        });
    }
    // zoom
    {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), "wheel", move |ev| {
            if let Ok(we) = ev.dyn_into::<WheelEvent>() {
                we.prevent_default();
                let factor = if we.delta_y() < 0.0 { 1.1 } else { 1.0 / 1.1 };
                {
                    let mut a = s.borrow_mut();
                    a.anim_gen = a.anim_gen.wrapping_add(1); // cancel any camera tween
                    a.camera.scale = (a.camera.scale * factor).clamp(0.1, 8.0);
                }
                draw_now(&s, &c);
            }
        });
    }
    // drag start: grabbing a node repositions it; empty space pans the canvas.
    {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), "mousedown", move |ev| {
            if let Ok(me) = ev.dyn_into::<MouseEvent>() {
                let (mx, my) = (me.offset_x() as f64, me.offset_y() as f64);
                let mut a = s.borrow_mut();
                a.anim_gen = a.anim_gen.wrapping_add(1); // cancel any camera tween
                let (w, h) = logical_dims(&c);
                let hit = canvas2d::hit_test(mx, my, w, h, &a.proj, &a.layout_pos, &a.camera);
                a.did_drag = false;
                a.last_mouse = (mx, my);
                match hit {
                    Some(key) => {
                        a.drag_node = Some(key);
                        c.set_class_name("grabbing");
                    }
                    None => a.dragging = true,
                }
            }
        });
    }
    // drag / hover
    {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), "mousemove", move |ev| {
            let Ok(me) = ev.dyn_into::<MouseEvent>() else {
                return;
            };
            let (mx, my) = (me.offset_x() as f64, me.offset_y() as f64);
            let mut redraw = false;
            {
                let mut a = s.borrow_mut();
                let (lx, ly) = a.last_mouse;
                if let Some(key) = a.drag_node.clone() {
                    // Move the node: screen delta → world delta (÷ zoom). Update
                    // both the live layout and the persisted seed so it sticks.
                    if (mx - lx).abs() + (my - ly).abs() > 2.0 {
                        a.did_drag = true;
                    }
                    let (dx, dy) = (
                        ((mx - lx) / a.camera.scale) as f32,
                        ((my - ly) / a.camera.scale) as f32,
                    );
                    if let Some(p) = a.layout_pos.get_mut(&key) {
                        p.x += dx;
                        p.y += dy;
                    }
                    a.positions.entry(key).and_modify(|p| {
                        p.0 += dx;
                        p.1 += dy;
                    });
                    a.last_mouse = (mx, my);
                    redraw = true;
                } else if a.dragging {
                    if (mx - lx).abs() + (my - ly).abs() > 2.0 {
                        a.did_drag = true;
                    }
                    a.camera.x += mx - lx;
                    a.camera.y += my - ly;
                    a.last_mouse = (mx, my);
                    redraw = true;
                } else {
                    let (w, h) = logical_dims(&c);
                    let hov = canvas2d::hit_test(mx, my, w, h, &a.proj, &a.layout_pos, &a.camera);
                    c.set_class_name(if hov.is_some() { "grabbable" } else { "" });
                    if hov != a.hover {
                        a.hover = hov;
                        redraw = true;
                    }
                }
            }
            if redraw {
                draw_now(&s, &c);
            }
        });
    }
    // drag end: persist positions if a node was actually moved.
    for event in ["mouseup", "mouseleave"] {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), event, move |_| {
            let moved = {
                let mut a = s.borrow_mut();
                let moved = a.drag_node.is_some() && a.did_drag;
                a.drag_node = None;
                a.dragging = false;
                moved
            };
            c.set_class_name("");
            if moved {
                super::persist_positions(&s);
                super::sync_layout_cache(&s); // keep the drag from being reverted
            }
        });
    }
    // click-to-drill: a click that wasn't a drag focuses the clicked node's
    // connected component and pans/zooms to it (§M3); empty space clears focus.
    {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), "click", move |ev| {
            let Ok(me) = ev.dyn_into::<MouseEvent>() else {
                return;
            };
            let new_focus = {
                let a = s.borrow();
                if a.did_drag {
                    return;
                }
                let (mx, my) = (me.offset_x() as f64, me.offset_y() as f64);
                let (w, h) = logical_dims(&c);
                let hit = canvas2d::hit_test(mx, my, w, h, &a.proj, &a.layout_pos, &a.camera);
                // toggle: clicking the focused node again clears focus
                match (hit, a.focus.clone()) {
                    (Some(k), Some(f)) if k == f => None,
                    (Some(k), _) => Some(k),
                    (None, _) => None,
                }
            };
            super::focus_and_animate(&s, new_focus);
        });
    }
}
