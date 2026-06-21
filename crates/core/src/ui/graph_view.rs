//! Graph view: canvas setup, draw, and pan/zoom/hover interactions (§7.7, M3).

use super::{body_container, el, on, Shared};
use crate::render::canvas2d;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, MouseEvent, WheelEvent};

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let body = body_container(shared).ok_or_else(|| JsValue::from_str("no body"))?;
    body.set_inner_html("");

    if shared.borrow().proj.nodes.is_empty() {
        body.set_inner_html(
            "<div class=\"bg-empty\">No navigations recorded yet. Browse a bit, then reopen this dashboard.</div>",
        );
        return Ok(());
    }

    let doc = shared.borrow().doc.clone();
    let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;

    let canvas: HtmlCanvasElement = el(&doc, "canvas")
        .dyn_into()
        .map_err(|_| JsValue::from_str("canvas cast"))?;
    let _ = canvas.set_attribute("id", "bg-canvas");

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

fn draw_now(shared: &Shared, canvas: &HtmlCanvasElement) {
    let Some(ctx) = ctx_of(canvas) else { return };
    let a = shared.borrow();
    // Draw in CSS pixels onto the dpr-scaled backing store (crisp on HiDPI).
    let d = dpr();
    let _ = ctx.set_transform(d, 0.0, 0.0, d, 0.0, 0.0);
    let (w, h) = (canvas.width() as f64 / d, canvas.height() as f64 / d);
    canvas2d::draw(
        &ctx,
        w,
        h,
        &a.proj,
        &a.layout_pos,
        &a.camera,
        a.hover.as_deref(),
        a.focus.as_deref(),
    );
}

fn attach_interactions(shared: &Shared, canvas: &HtmlCanvasElement) {
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
            }
        });
    }
    // click-to-drill: a click that wasn't a drag focuses the clicked node's ego
    // network (§M3); clicking empty space clears the focus.
    {
        let s = shared.clone();
        let c = canvas.clone();
        on(canvas.as_ref(), "click", move |ev| {
            let Ok(me) = ev.dyn_into::<MouseEvent>() else {
                return;
            };
            let hit = {
                let a = s.borrow();
                if a.did_drag {
                    return;
                }
                let (mx, my) = (me.offset_x() as f64, me.offset_y() as f64);
                let (w, h) = logical_dims(&c);
                canvas2d::hit_test(mx, my, w, h, &a.proj, &a.layout_pos, &a.camera)
            };
            {
                let mut a = s.borrow_mut();
                // toggle: clicking the focused node again clears focus
                a.focus = match (hit, a.focus.clone()) {
                    (Some(k), Some(f)) if k == f => None,
                    (Some(k), _) => Some(k),
                    (None, _) => None,
                };
            }
            super::recompute_projection(&s);
            let _ = super::rerender(&s);
        });
    }
}
