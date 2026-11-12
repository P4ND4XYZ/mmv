use super::selection_panel::{frame, input, panel, press};
use super::*;
use gpui_kit::VisualTestContext;

#[gpui_kit::test]
fn floating_layout_is_centered_eighty_percent_and_preview_wheel_keeps_input_and_body(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(
        cx,
        "[Link](missing.md)\n\n".to_owned() + &"body\n\n".repeat(150),
    );
    frame(cx, window);
    press(cx, window, "f");
    let mut visual = VisualTestContext::from_window(window, cx);
    let frame_bounds = visual.debug_bounds("selection-frame").unwrap();
    assert!((f32::from(frame_bounds.size.width) - 512.).abs() < 0.1);
    assert!((f32::from(frame_bounds.size.height) - 288.).abs() < 0.1);
    assert_eq!(frame_bounds.origin, point(px(64.), px(36.)));
    let left = visual.debug_bounds("selection-list").unwrap();
    let right = visual.debug_bounds("selection-preview").unwrap();
    let query = visual.debug_bounds("selection-input").unwrap();
    assert!((f32::from(left.size.width - right.size.width)).abs() <= 1.);
    assert!(query.top() >= left.bottom() - px(1.));
    assert!(query.size.width >= left.size.width + right.size.width - px(1.));
    let body_before = view.read_with(cx, |v, cx| v.document.distance(cx));
    press(cx, window, "ctrl-d");
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "source-preview",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            cx,
        );
    })
    .unwrap();
    frame(cx, window);
    assert_eq!(
        view.read_with(cx, |v, cx| v.document.distance(cx)),
        body_before
    );
    let selection = panel(cx, &view);
    cx.update_window(window, |_, window, cx| {
        assert!(selection.read(cx).input.focus_handle(cx).is_focused(window))
    })
    .unwrap();
    input(cx, window, "zzzz-no-match");
    press(cx, window, "enter");
    assert_eq!(panel(cx, &view), selection);
    cx.simulate_window_resize(window, size(px(320.), px(240.)));
    frame(cx, window);
    let frame_bounds = visual.debug_bounds("selection-frame").unwrap();
    assert_eq!(frame_bounds.size, size(px(256.), px(192.)));
    assert_eq!(frame_bounds.origin, point(px(32.), px(24.)));
    let left = visual.debug_bounds("selection-list").unwrap();
    let right = visual.debug_bounds("selection-preview").unwrap();
    assert!(left.size.width > px(0.) && right.size.width > px(0.));
    assert!((f32::from(left.size.width - right.size.width)).abs() <= 1.);
    assert_eq!(panel(cx, &view), selection);
}
