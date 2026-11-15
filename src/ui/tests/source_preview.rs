use super::selection_panel::{frame, input, is_open, panel, press, query, replace_query, selected};
use super::*;
use crate::ui::lua_floating::standard_links::preview_metrics;
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
    let owner = panel(cx, &view).owner;
    let mut visual = VisualTestContext::from_window(window, cx);
    let bounds = visual.debug_bounds("lua-window-0").unwrap();
    assert_eq!(bounds.size, size(px(512.), px(288.)));
    assert_eq!(bounds.origin, point(px(64.), px(36.)));
    let left = visual.debug_bounds("lua-component-links-list").unwrap();
    let right = visual.debug_bounds("lua-component-links-source").unwrap();
    let q = visual.debug_bounds("lua-component-links-query").unwrap();
    assert!((f32::from(left.size.width - right.size.width)).abs() <= 1.);
    assert!(q.top() >= left.bottom() - px(1.));
    assert!(q.size.width >= left.size.width + right.size.width - px(1.));
    let before = view.read_with(cx, |v, cx| v.document.distance(cx));
    press(cx, window, "ctrl-d");
    let manual = preview_metrics(cx, &view).0;
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "lua-component-links-source",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, window);
    assert!(preview_metrics(cx, &view).0 > manual);
    assert_eq!(view.read_with(cx, |v, cx| v.document.distance(cx)), before);
    let q = query(cx, &view);
    cx.update_window(window, |_, w, cx| assert!(q.focus_handle(cx).is_focused(w)))
        .unwrap();
    input(cx, window, "zz-no-match");
    press(cx, window, "enter");
    assert!(is_open(cx, &view));
    assert!(!panel(cx, &view).components.contains_key("links-source"));
    cx.simulate_window_resize(window, size(px(320.), px(240.)));
    frame(cx, window);
    let bounds = visual.debug_bounds("lua-window-0").unwrap();
    assert_eq!(bounds.size, size(px(256.), px(192.)));
    assert_eq!(bounds.origin, point(px(32.), px(24.)));
    let left = visual.debug_bounds("lua-component-links-list").unwrap();
    let right = visual
        .debug_bounds("lua-component-links-no-source")
        .unwrap();
    assert!(left.size.width > px(0.) && right.size.width > px(0.));
    assert_eq!(panel(cx, &view).owner, owner);
}

#[gpui_kit::test]
fn source_preview_same_id_external_scroll_reflow_and_revival_are_isolated(cx: &mut TestAppContext) {
    let source = "[First](one.md)\n\n".to_owned()
        + &"body context\n\n".repeat(100)
        + "[Second](two.md)\n\n"
        + &"tail\n\n".repeat(50);
    let (window, view) = open(cx, source);
    frame(cx, window);
    press(cx, window, "f");
    press(cx, window, "ctrl-d");
    let manual = preview_metrics(cx, &view).0;
    for operation in [
        crate::app::ExternalOperation::Bottom,
        crate::app::ExternalOperation::Top,
    ] {
        view.update(cx, |v, cx| {
            v.request_external(
                operation,
                crate::app::ExternalRequestId("preview-isolation".into()),
                cx,
            );
        });
        frame(cx, window);
        assert_eq!(preview_metrics(cx, &view).0, manual);
    }
    replace_query(cx, window, &view, "first");
    assert_eq!(preview_metrics(cx, &view).0, manual);
    let anchor = preview_metrics(cx, &view).2;
    let selection = selected(cx, &view);
    let q = query(cx, &view);
    cx.simulate_window_resize(window, size(px(500.), px(300.)));
    frame(cx, window);
    assert_eq!(preview_metrics(cx, &view).2, anchor);
    assert_eq!(selected(cx, &view), selection);
    assert_eq!(query(cx, &view), q);
    replace_query(cx, window, &view, "missing-query");
    assert!(!panel(cx, &view).components.contains_key("links-source"));
    replace_query(cx, window, &view, "");
    assert_eq!(preview_metrics(cx, &view).0, 0.);
}

#[gpui_kit::test]
fn source_preview_unknown_location_removes_old_component_but_can_confirm(cx: &mut TestAppContext) {
    fn extract(
        path: &std::path::Path,
        source: &str,
    ) -> Result<Vec<crate::core::links::LinkCandidate>, String> {
        let mut links = crate::ui::link_candidates::extract_candidates(path, source)?;
        links[1].source_range = None;
        Ok(links)
    }
    let (window, view) = open(cx, "[First](one.md)\n\n[Second](two.md)".into());
    frame(cx, window);
    view.update(cx, |v, _| {
        let doc = v.app.document().unwrap();
        v.lua.context().documents.borrow_mut().current =
            Some(crate::lua::document::DocumentHandle::new(
                doc.path.clone(),
                doc.source.clone(),
                extract,
            ));
    });
    let (reads, _sub) = super::selection_panel::capture_reads(cx, &view);
    press(cx, window, "f");
    assert!(panel(cx, &view).components.contains_key("links-source"));
    press(cx, window, "down");
    let p = panel(cx, &view);
    assert!(!p.components.contains_key("links-source"));
    assert!(p.components.contains_key("links-unavailable"));
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
    assert_eq!(reads.borrow()[0].path, PathBuf::from(r"C:\fixtures\two.md"));
}
