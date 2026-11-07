use std::{path::PathBuf, time::Instant};

use gpui_kit::{
    AppContext as _, Bounds, TestAppContext, WindowBounds, WindowOptions, point, px, size,
    test::TestWindowExt as _,
};

use super::Viewer;
use crate::app::ReadResult;

fn open(
    cx: &mut TestAppContext,
    source: String,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<Viewer>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::bind_keys(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(640.), px(360.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    Viewer::new(
                        PathBuf::from(r"C:\fixtures\sample.md"),
                        Instant::now(),
                        window,
                        cx,
                    )
                });
                view.update(cx, |view, cx| {
                    let request = view.app.open(view.path.clone());
                    view.complete(
                        ReadResult {
                            request,
                            content: Ok(source),
                        },
                        cx,
                    );
                });
                view
            },
        )
        .unwrap()
    })
}

#[gpui_kit::test]
fn search_input_and_navigation_use_rendered_japanese_text(cx: &mut TestAppContext) {
    let (window, view) = open(
        cx,
        "# 見出し\n\n**日本語** と [日本語](https://example.com)".into(),
    );
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("/", cx);
        window.input("日本語", cx);
        assert_eq!(view.read(cx).find.read(cx).value().as_ref(), "日本語");
    })
    .unwrap();
    // InputEvent notifications are delivered when the app update finishes.
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        let viewer = view.read(cx);
        let rendered = viewer.text.read(cx).rendered_text();
        assert!(!rendered.as_str().contains("**"));
        assert!(!rendered.as_str().contains("https://"));
        assert_eq!(viewer.app.matches.len(), 2);
        for range in &viewer.app.matches {
            assert_eq!(&rendered.as_str()[range.clone()], "日本語");
        }
        assert!(viewer.display_error.is_none());
        // 'n' in the input must be text, not the body's NextMatch action.
        window.press("n", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(view.read(cx).find.read(cx).value().as_ref(), "日本語n");
        assert!(view.read(cx).app.matches.is_empty());
        window.press("backspace", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("n", cx);
        assert_eq!(view.read(cx).app.current_match, Some(1));
        window.press("shift-n", cx);
        assert_eq!(view.read(cx).app.current_match, Some(0));
        assert!(view.read(cx).display_error.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn revealing_an_offscreen_match_scrolls_the_real_text_view(cx: &mut TestAppContext) {
    let source = (0..80)
        .map(|ix| format!("段落 {ix} の本文です。\n\n"))
        .collect::<String>()
        + "末尾の一致";
    let (window, view) = open(cx, source);
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("/", cx);
        window.input("末尾の一致", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let viewer = view.read(cx);
        assert_eq!(viewer.app.matches.len(), 1);
        assert!(viewer.display_error.is_none());
        assert!(
            viewer
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top()
                .item_ix
                > 0
        );
    })
    .unwrap();
}
