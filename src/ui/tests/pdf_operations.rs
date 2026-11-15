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
fn pdf_zoom_keys_do_not_capture_command_search_tree_or_composition(cx: &mut TestAppContext) {
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
    for key in ["+", "-", "=", "j", "ctrl-d"] {
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
    view.read_with(cx, |view, cx| {
        let panel = &view.link_panel.as_ref().unwrap().panel;
        assert_eq!(panel.read(cx).input.read(cx).value().as_ref(), "+-=");
    });
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
