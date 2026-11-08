use gpui_kit::{
    AnyWindowHandle, AppContext as _, EntityInputHandler as _, ListOffset, TestAppContext,
    VisualTestContext, px, test::TestWindowExt as _,
};

use crate::app::Mode;

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn footer_rows_and_mode_width_stay_fixed_across_modes_and_long_filenames(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, "本文".into());
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window, cx);
    let status = visual.debug_bounds("status-line").unwrap();
    let command = visual.debug_bounds("command-line").unwrap();
    let label = visual.debug_bounds("mode-label").unwrap();
    let filename = visual.debug_bounds("filename").unwrap();
    assert_eq!(status.size.height, px(21.));
    assert_eq!(command.size.height, px(21.));
    assert_eq!(status.bottom(), command.top());
    assert_eq!(label.size.width, px(super::super::MODE_WIDTH));
    assert_eq!(
        visual.debug_bounds("document-body").unwrap().bottom(),
        status.top()
    );
    assert_eq!(
        view.read_with(cx, |viewer, _| viewer.app.mode.label()),
        "NORMAL"
    );
    for (key, expected) in [("/", "SEARCH"), (":", "COMMAND")] {
        press(cx, window, key);
        assert_eq!(
            view.read_with(cx, |viewer, _| viewer.app.mode.label()),
            expected
        );
        assert_eq!(visual.debug_bounds("status-line").unwrap(), status);
        assert_eq!(visual.debug_bounds("command-line").unwrap(), command);
        assert_eq!(visual.debug_bounds("mode-label").unwrap(), label);
        assert_eq!(visual.debug_bounds("filename").unwrap(), filename);
        press(cx, window, "escape");
        assert_eq!(
            view.read_with(cx, |viewer, _| viewer.app.mode.label()),
            "NORMAL"
        );
    }
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let path = viewer
                .path
                .with_file_name(format!("{}.md", "長いファイル名".repeat(40)));
            let request = viewer.app.open(path);
            viewer.complete(
                crate::app::ReadResult {
                    request,
                    content: Ok("本文".into()),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert_eq!(visual.debug_bounds("status-line").unwrap(), status);
    assert_eq!(visual.debug_bounds("command-line").unwrap(), command);
    assert_eq!(visual.debug_bounds("mode-label").unwrap(), label);
    assert_eq!(visual.debug_bounds("filename").unwrap(), filename);
}

#[gpui_kit::test]
fn cancelling_a_preview_restores_the_confirmed_match_without_scrolling(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, "日本語 日本語\n\n".repeat(60));
    cx.run_until_parked();
    press(cx, window, "/");
    input(cx, window, "日本語");
    press(cx, window, "enter");
    press(cx, window, "n");
    let top = view.read_with(cx, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .logical_scroll_top()
    });
    press(cx, window, "/");
    input(cx, window, "unknown");
    view.read_with(cx, |viewer, cx| {
        assert!(matches!(viewer.app.mode, Mode::Search));
        assert!(viewer.app.matches().is_empty());
        let preview_top = viewer
            .document
            .text
            .read(cx)
            .list_state()
            .logical_scroll_top();
        assert_eq!(preview_top.item_ix, top.item_ix);
        assert_eq!(preview_top.offset_in_item, top.offset_in_item);
    });
    press(cx, window, "escape");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert_eq!(viewer.app.matches().len(), 120);
        assert_eq!(viewer.app.current_match(), Some(1));
    });
    press(cx, window, "/");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert_eq!(viewer.app.matches().len(), 120);
    });
}

#[gpui_kit::test]
fn enter_starts_at_the_displayed_block_and_empty_repeat_wraps(cx: &mut TestAppContext) {
    let source = "target\n\n```mermaid\nflowchart TD\nA[target] --> B\n```\n\n".to_string()
        + &"中間の本文\n\n".repeat(35)
        + "~~target~~\n\n"
        + &"後半の本文\n\n".repeat(50);
    let (window, view) = super::open(cx, source);
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .scroll_to(ListOffset {
                    item_ix: 20,
                    offset_in_item: px(0.),
                });
            cx.notify();
        })
    });
    cx.run_until_parked();
    press(cx, window, "/");
    input(cx, window, "target");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.matches().len(), 2);
        assert_eq!(viewer.app.current_match(), Some(1));
        assert_eq!(viewer.app.diagrams().len(), 1);
    });
    press(cx, window, "shift-g");
    press(cx, window, "/");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.current_match(), Some(0))
    });
}

#[gpui_kit::test]
fn missing_and_unmatched_queries_report_in_the_normal_input_line(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, "本文".into());
    cx.run_until_parked();
    press(cx, window, "/");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert_eq!(viewer.message.as_deref(), Some("直前の検索語がありません"));
    });
    press(cx, window, "/");
    input(cx, window, "存在しない検索語");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert!(viewer.app.matches().is_empty());
        assert_eq!(
            viewer.message.as_deref(),
            Some("一致する文字列がありません")
        );
    });
}

#[gpui_kit::test]
fn command_escape_and_unknown_command_do_not_quit_but_q_enter_closes_the_window(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, "本文".into());
    cx.run_until_parked();
    press(cx, window, ":");
    input(cx, window, "q");
    press(cx, window, "escape");
    view.read_with(cx, |viewer, _| assert_eq!(viewer.app.mode, Mode::Normal));
    assert!(cx.update(|cx| !cx.windows().is_empty()));
    press(cx, window, ":");
    input(cx, window, "e");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert_eq!(viewer.message.as_deref(), Some("未対応のコマンド: :e"));
    });
    press(cx, window, ":");
    input(cx, window, "q");
    press(cx, window, "enter");
    assert!(cx.update(|cx| cx.windows().is_empty()));
}

#[gpui_kit::test]
fn empty_command_backspace_returns_to_normal_and_restores_body_keys(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, "本文".into());
    cx.run_until_parked();
    press(cx, window, ":");
    press(cx, window, "backspace");
    view.read_with(cx, |viewer, _| assert_eq!(viewer.app.mode, Mode::Normal));
    press(cx, window, "/");
    view.read_with(cx, |viewer, _| {
        assert!(matches!(viewer.app.mode, Mode::Search))
    });
}

#[gpui_kit::test]
fn backspace_keeps_editing_when_deleting_text_and_leaves_empty_search_unchanged(
    cx: &mut TestAppContext,
) {
    for (key, label) in [(":", "COMMAND"), ("/", "SEARCH")] {
        let (window, view) = super::open(cx, "本文".into());
        cx.run_until_parked();
        press(cx, window, key);
        input(cx, window, "q");
        press(cx, window, "backspace");
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.app.mode.label(), label);
            assert!(viewer.input.state.read(cx).value().is_empty());
        });
        press(cx, window, "backspace");
        let expected = if key == ":" { "NORMAL" } else { "SEARCH" };
        view.read_with(cx, |viewer, _| {
            assert_eq!(viewer.app.mode.label(), expected)
        });
    }
}

#[gpui_kit::test]
fn ime_marked_text_does_not_submit_search_or_command(cx: &mut TestAppContext) {
    for (key, text) in [("/", "日本語"), (":", "q")] {
        let (window, view) = super::open(cx, "日本語の本文".into());
        cx.run_until_parked();
        press(cx, window, key);
        cx.update_window(window, |_, window, cx| {
            view.read(cx).input.state.clone().update(cx, |input, cx| {
                let end = text.encode_utf16().count();
                input.replace_and_mark_text_in_range(None, text, Some(end..end), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        press(cx, window, "enter");
        view.read_with(cx, |viewer, _| assert_ne!(viewer.app.mode, Mode::Normal));
        cx.update_window(window, |_, window, cx| {
            view.read(cx).input.state.clone().update(cx, |input, cx| {
                input.replace_text_in_range(None, text, window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        press(cx, window, "enter");
        if key == "/" {
            view.read_with(cx, |viewer, _| {
                assert_eq!(viewer.app.mode, Mode::Normal);
                assert_eq!(viewer.app.matches().len(), 1);
            });
        } else {
            assert!(cx.update(|cx| !cx.windows().contains(&window)));
        }
    }
}

#[gpui_kit::test]
fn normal_scroll_keys_do_not_run_while_editing_a_search_or_command(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, "本文です。\n\n".repeat(120));
    cx.run_until_parked();
    press(cx, window, "j");
    let down = view.read_with(cx, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .logical_scroll_top()
    });
    assert!(down.item_ix > 0 || down.offset_in_item > px(0.));
    press(cx, window, "k");
    for (down, up) in [("ctrl-d", "ctrl-u"), ("ctrl-f", "ctrl-b")] {
        press(cx, window, down);
        let top = view.read_with(cx, |viewer, cx| {
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top()
        });
        assert!(top.item_ix > 0 || top.offset_in_item > px(0.));
        press(cx, window, up);
    }
    press(cx, window, "shift-g");
    assert!(
        view.read_with(cx, |viewer, cx| viewer
            .document
            .text
            .read(cx)
            .list_state()
            .logical_scroll_top()
            .item_ix)
            > 0
    );
    press(cx, window, "g");
    press(cx, window, "g");
    for key in ["/", ":"] {
        press(cx, window, key);
        input(cx, window, "jnN");
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "jnN");
            let top = viewer
                .document
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top();
            assert_eq!(top.item_ix, 0);
            assert_eq!(top.offset_in_item, px(0.));
        });
        press(cx, window, "escape");
    }
}
