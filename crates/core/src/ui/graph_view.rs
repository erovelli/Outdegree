//! Graph view: canvas setup, draw, and pan/zoom/hover interactions (§7.7, M3).

use super::{body_container, el, on, Shared};
use crate::render::canvas2d;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, MouseEvent, WheelEvent};

pub(crate) fn render(shared: &Shared) -> Result<(), JsValue> {
    let body = body_container(shared).ok_or_else(|| JsValue::from_str("no body"))?;
    body.set_inner_html("");
    let doc = shared.borrow().doc.clone();

    let canvas: HtmlCanvasElement = el(&doc, "canvas")
        .dyn_into()
        .map_err(|_| JsValue::from_str("canvas cast"))?;
    let w = body.client_width().max(320);
    let h = body.client_height().max(320);
    canvas.set_width(w as u32);
    canvas.set_height(h as u32);
    let _ = canvas.set_attribute("id", "bg-canvas");
    let _ = body.append_child(&canvas);

    if shared.borrow().proj.nodes.is_empty() {
        body.set_inner_html(
            "<div class=\"bg-empty\">No navigations recorded yet. Browse a bit, then reopen this dashboard.</div>",
        );
        return Ok(());
    }

    draw_now(shared, &canvas);
    attach_interactions(shared, &canvas);
    Ok(())
}

fn ctx_of(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas.get_context("2d").ok().flatten()?.dyn_into().ok()
}

fn draw_now(shared: &Shared, canvas: &HtmlCanvasElement) {
    let Some(ctx) = ctx_of(canvas) else { return };
    let a = shared.borrow();
    let w = canvas.width() as f64;
    let h = canvas.height() as f64;
    canvas2d::draw(&ctx, w, h, &a.proj, &a.layout_pos, &a.camera, a.hover.as_deref());
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
    // drag start
    {
        let s = shared.clone();
        on(canvas.as_ref(), "mousedown", move |ev| {
            if let Ok(me) = ev.dyn_into::<MouseEvent>() {
                let mut a = s.borrow_mut();
                a.dragging = true;
                a.last_mouse = (me.offset_x() as f64, me.offset_y() as f64);
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
                if a.dragging {
                    let (lx, ly) = a.last_mouse;
                    a.camera.x += mx - lx;
                    a.camera.y += my - ly;
                    a.last_mouse = (mx, my);
                    redraw = true;
                } else {
                    let w = c.width() as f64;
                    let h = c.height() as f64;
                    let hov =
                        canvas2d::hit_test(mx, my, w, h, &a.proj, &a.layout_pos, &a.camera);
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
    // drag end
    for event in ["mouseup", "mouseleave"] {
        let s = shared.clone();
        on(canvas.as_ref(), event, move |_| {
            s.borrow_mut().dragging = false;
        });
    }
}
