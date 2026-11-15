//! Host key dispatch against real prepared PDFs; inspect painted page geometry,
//! not a second scrolling model or the fork's private ScrollHandle.
use super::pdf::{Files, body_pdf, frame, input, press, request};
use super::*;
use crate::{app::Mode, runtime};
use gpui_kit::{EntityInputHandler as _, VisualTestContext};

#[derive(Clone, Copy, Debug)]
struct PagePaint {
    top: f32,
    width: f32,
    height: f32,
    viewport: f32,
}

fn page(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) -> PagePaint {
    let viewport = VisualTestContext::from_window(window, cx)
        .debug_bounds("document-body")
        .unwrap()
        .size
        .height;
    cx.update_window(window, |_, window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter_map(|quad| {
                let width = quad.bounds.size.width.as_f32() / scale;
                let height = quad.bounds.size.height.as_f32() / scale;
                // The self-authored fixture contains only blank Letter pages.
                (width > 100. && (height / width - 792. / 612.).abs() < 0.01).then_some(PagePaint {
                    top: quad.bounds.origin.y.as_f32() / scale,
                    width,
                    height,
                    viewport: f32::from(viewport),
                })
            })
            .min_by(|a, b| a.top.total_cmp(&b.top))
            .expect("real PDF page border must be painted")
    })
    .unwrap()
}

pub(super) fn page_top(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) -> f32 {
    page(cx, window).top
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.1, "{actual} != {expected}");
}

fn open_pdf(
    cx: &mut TestAppContext,
) -> (Files, gpui_kit::AnyWindowHandle, gpui_kit::Entity<Viewer>) {
    let files = Files::new();
    let path = files.pdf("operations.PDF");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &path, "pdf");
    frame(cx, window);
    view.read_with(cx, |view, _| {
        view.lua
            .lua()
            .load(include_str!("../../../plugins/pdf_zoom.lua"))
            .exec()
            .unwrap();
    });
    (files, window, view)
}

#[gpui_kit::test]
fn dispatched_pdf_scroll_uses_40px_body_viewport_edges_and_single_wheel(cx: &mut TestAppContext) {
    let (_files, window, _view) = open_pdf(cx);
    let start = page(cx, window);
    let mut visual = VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    near(start.viewport, f32::from(body.size.height));
    for (down, up, distance) in [
        ("j", "k", 40.),
        ("ctrl-d", "ctrl-u", start.viewport * 0.5),
        ("ctrl-f", "ctrl-b", start.viewport),
    ] {
        press(cx, window, down);
        near(page(cx, window).top, start.top - distance);
        press(cx, window, up);
        near(page(cx, window).top, start.top);
    }
    press(cx, window, "k");
    near(page(cx, window).top, start.top);
    visual = VisualTestContext::from_window(window, cx);
    visual.simulate_event(gpui_kit::ScrollEvent {
        position: body.center(),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-120.))),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
        source: gpui_kit::ScrollSource::Pointer,
    });
    frame(cx, window);
    near(page(cx, window).top, start.top - 120.);
    press(cx, window, "shift-g");
    let end = page(cx, window);
    near(end.top + end.height, f32::from(body.bottom()) - 16.);
    for key in ["j", "ctrl-d", "ctrl-f", "shift-g"] {
        press(cx, window, key);
        near(page(cx, window).top, end.top);
    }
    press(cx, window, "g");
    press(cx, window, "g");
    near(page(cx, window).top, start.top);
}

#[gpui_kit::test]
fn dispatched_zoom_preserves_center_and_fit_tracks_tree_and_resize(cx: &mut TestAppContext) {
    let (_files, window, view) = open_pdf(cx);
    let pdf = body_pdf(cx, &view);
    let initial_zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    press(cx, window, "ctrl-d");
    press(cx, window, "-");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - initial_zoom / 1.1).abs() < 0.0001);
    let before = page(cx, window);
    let anchor = (before.viewport * 0.5 - before.top) / before.height;
    press(cx, window, "+");
    let zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    assert!((zoom - initial_zoom).abs() < 0.0001);
    let enlarged = page(cx, window);
    assert!(((enlarged.viewport * 0.5 - enlarged.top) / enlarged.height - anchor).abs() < 0.003);
    press(cx, window, "+");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - initial_zoom * 1.1).abs() < 0.0001);
    press(cx, window, "=");
    let fitted = page(cx, window);
    press(cx, window, "=");
    near(page(cx, window).top, fitted.top);
    press(cx, window, "ctrl-shift-e");
    let narrow = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    assert!(narrow < initial_zoom - 0.1);
    press(cx, window, "ctrl-shift-e");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - initial_zoom).abs() < 0.0001);
    press(cx, window, "+");
    let manual = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    press(cx, window, "ctrl-shift-e");
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), manual);
    press(cx, window, "=");
    cx.simulate_window_resize(window, size(px(900.), px(500.)));
    frame(cx, window);
    assert!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) > narrow + 0.1);
    let resized = page(cx, window);
    assert!(resized.width > 500.);
}

#[gpui_kit::test]
fn pdf_plugin_obeys_app_normal_routing_and_protects_command_search_and_composition(
    cx: &mut TestAppContext,
) {
    let (files, window, view) = open_pdf(cx);
    let pdf = body_pdf(cx, &view);
    let zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    press(cx, window, ":");
    for key in ["+", "-", "="] {
        press(cx, window, key);
    }
    view.read_with(cx, |view, cx| {
        assert_eq!(view.app.mode, Mode::Command);
        assert_eq!(view.input.state.read(cx).value().as_ref(), "+-=");
    });
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), zoom);
    press(cx, window, "escape");
    press(cx, window, "ctrl-shift-e");
    press(cx, window, "ctrl-w");
    press(cx, window, "h");
    let tree_zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    // App Normal keymaps have no body-focus restriction: unhandled tree keys
    // reach the app layer, unlike the removed PDF-only built-in actions.
    press(cx, window, "+");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - tree_zoom * 1.1).abs() < 0.0001);
    press(cx, window, "-");
    press(cx, window, "=");
    let tree_zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    for key in ["j", "ctrl-d"] {
        press(cx, window, key);
    }
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), tree_zoom);
    press(cx, window, "ctrl-w");
    press(cx, window, "l");
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.input.state.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "未", None, window, cx);
            });
        });
    })
    .unwrap();
    frame(cx, window);
    for key in ["+", "-", "="] {
        press(cx, window, key);
    }
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), tree_zoom);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.input
                .state
                .update(cx, |input, cx| input.unmark_text(window, cx));
        });
    })
    .unwrap();
    let markdown = files.write("draft.md", b"[PDF](operations.PDF)");
    request(cx, &view, &markdown, "md");
    frame(cx, window);
    press(cx, window, "f");
    for key in ["+", "-", "="] {
        press(cx, window, key);
    }
    assert_eq!(
        super::selection_panel::query(cx, &view)
            .read_with(cx, |input, _| input.value().to_string()),
        "+-="
    );
    press(cx, window, "escape");
    press(cx, window, "/");
    request(cx, &view, &files.0.join("operations.PDF"), "search-pdf");
    frame(cx, window);
    let pdf = body_pdf(cx, &view);
    let zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    input(cx, window, "+-=");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.app.mode, Mode::Search);
        assert_eq!(view.input.state.read(cx).value().as_ref(), "+-=");
    });
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), zoom);
}

#[gpui_kit::test]
fn pdf_zoom_keys_are_unbound_without_plugin_and_user_can_override_or_remove(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let path = files.pdf("unbound.pdf");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &path, "pdf");
    frame(cx, window);
    let pdf = body_pdf(cx, &view);
    let initial = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    for key in ["+", "-", "=", "p"] {
        press(cx, window, key);
    }
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), initial);
    view.read_with(cx, |view, _| {
        view.lua.lua().load(include_str!("../../../plugins/pdf_zoom.lua")).exec().unwrap();
        view.lua.lua().load("local m=require('mmv'); m.keymap.set('+',function() m.pdf.set_zoom(0.75) end); assert(m.keymap.remove('p'))").exec().unwrap();
    });
    press(cx, window, "+");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - 0.75).abs() < 0.0001);
    press(cx, window, "p");
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - 0.75).abs() < 0.0001);
}

#[gpui_kit::test]
fn native_fit_page_toggle_keeps_zoom_when_off_and_refits_on_resize_only(cx: &mut TestAppContext) {
    let (_files, window, view) = open_pdf(cx);
    let pdf = body_pdf(cx, &view);
    let width_zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    press(cx, window, "p");
    let fitted = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    assert!(fitted < width_zoom);
    let painted = page(cx, window);
    near(painted.height, painted.viewport - 32.0);
    press(cx, window, "j");
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), fitted);
    press(cx, window, "p"); // off is not a reset
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), fitted);
    cx.simulate_window_resize(window, size(px(700.), px(500.)));
    frame(cx, window);
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), fitted);
    press(cx, window, "p");
    let taller = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    assert!(taller > fitted + 0.05);
    cx.simulate_window_resize(window, size(px(700.), px(400.)));
    frame(cx, window);
    assert!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) < taller - 0.05);
    press(cx, window, "+");
    let manual = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    cx.simulate_window_resize(window, size(px(700.), px(600.)));
    frame(cx, window);
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), manual);
}

#[gpui_kit::test]
fn lua_zoom_reports_applied_value_and_preserves_mode_and_focus(cx: &mut TestAppContext) {
    let (_files, window, view) = open_pdf(cx);
    let pdf = body_pdf(cx, &view);
    press(cx, window, ":");
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let old = pdf.read(cx).zoom(cx);
            view.lua.lua().globals().set("old", old).unwrap();
            view.lua.lua().load("local p=require('mmv').pdf; assert(math.abs(p.get_zoom()-old)<0.0001); assert(p.set_zoom(0.5)); assert(p.zoom_in()); assert(p.zoom_in()); assert(p.zoom_out()); assert(math.abs(p.get_zoom()-old)<0.0001)").exec().unwrap();
            assert_eq!(pdf.read(cx).zoom(cx), old);
            view.pump_lua(cx);
            view.lua.lua().load("assert(math.abs(require('mmv').pdf.get_zoom()-0.55)<0.0001)").exec().unwrap();
            assert_eq!(view.app.mode, Mode::Command);
            assert!(view.input.state.focus_handle(cx).is_focused(window));
        });
    }).unwrap();
    frame(cx, window);
    assert!((pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)) - 0.55).abs() < 0.0001);
}

#[gpui_kit::test]
fn queued_zoom_never_rebinds_to_replacement_pdf_even_at_same_path(cx: &mut TestAppContext) {
    let (_files, window, view) = open_pdf(cx);
    let old = body_pdf(cx, &view);
    view.update(cx, |view, cx| {
        view.lua
            .lua()
            .load("assert(require('mmv').pdf.set_zoom(2.0))")
            .exec()
            .unwrap();
        let prepared = crate::pdf::prepare(
            crate::pdf::tests::fixture("3 0 R", 1, false),
            &Default::default(),
        )
        .unwrap();
        view.document.replace_pdf(prepared, cx);
        view.publish_lua_document(cx); // same app path, different display identity
        view.lua
            .lua()
            .load("assert(require('mmv').pdf.get_zoom()==nil)")
            .exec()
            .unwrap();
    });
    frame(cx, window);
    assert!(old.read_with(cx, |pdf, cx| pdf.is_released(cx)));
    let replacement = body_pdf(cx, &view);
    assert_ne!(replacement, old);
    assert!(replacement.read_with(cx, |pdf, cx| pdf.zoom(cx)) < 1.0);
    view.update(cx, |view, cx| {
        view.lua
            .lua()
            .load("assert(require('mmv').pdf.set_zoom(0.75))")
            .exec()
            .unwrap();
        view.pump_lua(cx);
    });
    assert!((replacement.read_with(cx, |pdf, cx| pdf.zoom(cx)) - 0.75).abs() < 0.0001);
}
