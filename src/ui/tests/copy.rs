use super::*;
use gpui_kit::{ClipboardItem, MouseButton, VisualTestContext, base::TextSelection};

fn right_click(visual: &mut VisualTestContext, position: gpui_kit::Point<gpui_kit::Pixels>) {
    visual.simulate_mouse_down(position, MouseButton::Right, Default::default());
    visual.simulate_mouse_up(position, MouseButton::Right, Default::default());
    visual.run_until_parked();
    visual.update(|window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn context_copy_preserves_selection_and_copies_rendered_text(cx: &mut TestAppContext) {
    let (window, _) = open(cx, "**日本語** copy text".into());
    selection_panel::frame(cx, window);
    let mut visual = VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    let start = body.origin + point(px(17.), px(20.));
    let end = body.origin + point(px(300.), px(20.));
    visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
    visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
    visual.update(|window, cx| {
        window.render_frame(cx);
        assert_eq!(
            TextSelection::selected_text(window, cx).trim(),
            "日本語 copy text"
        );
        cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
        window.press("ctrl-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "日本語 copy text"
        );
        cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
    });
    right_click(&mut visual, start);
    visual.update(|window, cx| {
        assert_eq!(
            TextSelection::selected_text(window, cx).trim(),
            "日本語 copy text"
        );
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "unchanged"
        );
        // The menu must copy its snapshot even if live selection changes.
        TextSelection::clear(window, cx);
    });
    let copy_item = start + point(px(30.), px(17.));
    visual.simulate_mouse_move(copy_item, None, Default::default());
    visual.simulate_click(copy_item, Default::default());
    visual.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "日本語 copy text"
        );
    });
}

#[gpui_kit::test]
fn context_copy_without_selection_does_not_change_clipboard(cx: &mut TestAppContext) {
    let (window, _) = open(cx, "No selection".into());
    selection_panel::frame(cx, window);
    let mut visual = VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    let position = body.origin + point(px(20.), px(20.));
    visual.update(|window, cx| {
        assert!(TextSelection::selected_text(window, cx).is_empty());
        cx.write_to_clipboard(ClipboardItem::new_string("keep clipboard".into()));
    });
    right_click(&mut visual, position);
    let copy_item = position + point(px(30.), px(17.));
    visual.simulate_mouse_move(copy_item, None, Default::default());
    visual.simulate_click(copy_item, Default::default());
    visual.update(|window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "keep clipboard"
        );
        window.press("escape", cx);
    });
}
