use super::*;
use crate::{app::Mode, ui::selection_panel::SelectionPanel};
use gpui_kit::{
    AnyWindowHandle, Entity, EntityInputHandler as _, Focusable as _, InputEvent as _, MouseButton,
    VisualTestContext, component::IndexPath, test::TestWindowExt as _,
};

pub(super) fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

pub(super) fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx))
        .unwrap();
    frame(cx, window);
}

pub(super) fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    frame(cx, window);
}

pub(super) fn panel(cx: &TestAppContext, view: &Entity<Viewer>) -> Entity<SelectionPanel> {
    view.read_with(cx, |viewer, _| {
        viewer
            .link_panel
            .as_ref()
            .expect("open panel")
            .panel
            .clone()
    })
}

fn capture_reads(
    cx: &mut TestAppContext,
    view: &Entity<Viewer>,
) -> (Rc<RefCell<Vec<ReadRequest>>>, gpui_kit::Subscription) {
    let reads = Rc::new(RefCell::new(Vec::new()));
    let captured = reads.clone();
    let subscription = cx.update(|cx| {
        cx.subscribe(view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    (reads, subscription)
}

#[gpui_kit::test]
fn selection_panel_opens_only_in_normal_resets_each_time_and_never_auto_executes(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Only](only.md)".into());
    frame(cx, window);
    let (reads, _subscription) = capture_reads(cx, &view);
    for mode_key in ["/", ":"] {
        press(cx, window, mode_key);
        press(cx, window, "f");
        view.read_with(cx, |viewer, cx| {
            assert!(viewer.link_panel.is_none());
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "f");
        });
        press(cx, window, "escape");
    }
    press(cx, window, "f");
    let original = panel(cx, &view);
    original.read_with(cx, |panel, cx| {
        assert_eq!(panel.input.read(cx).value().as_ref(), "");
        assert_eq!(panel.items.len(), 1);
        assert_eq!(
            panel.command.read(cx).selected_index(),
            Some(IndexPath::new(0))
        );
    });
    assert!(reads.borrow().is_empty());
    assert_eq!(
        cx.update(|cx| cx.windows().len()),
        1,
        "overlay must not create an OS window"
    );
    input(cx, window, "only");
    press(cx, window, "escape");
    view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
    press(cx, window, "f");
    assert_ne!(panel(cx, &view), original);
    panel(cx, &view).read_with(cx, |panel, cx| {
        assert!(panel.input.read(cx).value().is_empty());
        assert_eq!(
            panel.command.read(cx).selected_index(),
            Some(IndexPath::new(0))
        );
    });
    assert!(reads.borrow().is_empty());
}

#[gpui_kit::test]
fn selection_panel_empty_enter_is_noop_and_empty_message_is_exact(cx: &mut TestAppContext) {
    for source in ["No links", "[Only](only.md)"] {
        let (window, view) = open(cx, source.into());
        frame(cx, window);
        let (reads, _subscription) = capture_reads(cx, &view);
        press(cx, window, "f");
        if source.starts_with('[') {
            input(cx, window, "zzzzzz");
        }
        panel(cx, &view).read_with(cx, |panel, cx| {
            assert!(panel.items.is_empty());
            assert_eq!(panel.command.read(cx).selected_index(), None);
        });
        // Observe the actual rendered custom empty slot, not just a model flag.
        cx.update_window(window, |_, window, cx| {
            assert_eq!(
                window.find("selection-empty").label(),
                Some("No matching links")
            );
            window.press("enter", cx);
        })
        .unwrap();
        frame(cx, window);
        view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_some()));
        assert!(reads.borrow().is_empty());
    }
}

#[gpui_kit::test]
fn selection_panel_ranked_id_confirmation_and_both_utf8_highlight_fields(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "[Other](other.md)\n\n[日本語 design](日本語-design.md#start)\n\n[Design overview](overview.md)\n\n[Same document](#start)\n\n[Web](https://example.com)".into());
    frame(cx, window);
    let (reads, _subscription) = capture_reads(cx, &view);
    press(cx, window, "f");
    panel(cx, &view).read_with(cx, |panel, _| {
        assert_eq!(
            panel
                .items
                .iter()
                .map(|item| item.badge)
                .collect::<Vec<_>>(),
            ["Markdown", "Markdown heading", "Markdown", "Heading", "Web"]
        );
    });
    press(cx, window, "down");
    input(cx, window, "日dsg");
    let id = panel(cx, &view).read_with(cx, |panel, cx| {
        assert_eq!(panel.items.len(), 1);
        let item = &panel.items[0];
        assert_eq!(item.primary, "日本語 design");
        assert_eq!(item.id, 1);
        assert_eq!(
            panel.command.read(cx).selected_index(),
            Some(IndexPath::new(0))
        );
        for (text, ranges) in [
            (&item.primary, &item.primary_ranges),
            (&item.secondary, &item.secondary_ranges),
        ] {
            assert_eq!(
                ranges
                    .iter()
                    .map(|range| &text[range.clone()])
                    .collect::<String>(),
                "日dsg"
            );
        }
        item.id
    });
    assert_eq!(id, 1);
    press(cx, window, "enter");
    let request = reads.borrow_mut().pop().expect("confirmed local link read");
    assert_eq!(request.path, PathBuf::from(r"C:\fixtures\日本語-design.md"));
    view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(success(request, "# Start\n\nDestination".into()), cx)
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, _| {
        assert!(
            viewer
                .app
                .document()
                .unwrap()
                .source
                .contains("Destination")
        );
        assert!(viewer.link_panel.is_none());
    });
}

#[gpui_kit::test]
fn selection_panel_navigation_wraps_repaint_preserves_selection_and_focus_is_not_reclaimed(
    cx: &mut TestAppContext,
) {
    let source = (0..40)
        .map(|ix| format!("[Link {ix}](link-{ix}.md)\n\n"))
        .collect();
    let (window, view) = open(cx, source);
    frame(cx, window);
    press(cx, window, "f");
    let selection = panel(cx, &view);
    for (key, row) in [
        ("up", 39),
        ("ctrl-n", 0),
        ("down", 1),
        ("ctrl-p", 0),
        ("ctrl-p", 39),
    ] {
        press(cx, window, key);
        selection.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.command.read(cx).selected_index(),
                Some(IndexPath::new(row))
            )
        });
    }
    let mut visual = VisualTestContext::from_window(window, cx);
    assert!(
        visual.debug_bounds("selection-item-39").is_some(),
        "keyboard navigation reveals offscreen selection"
    );
    cx.update_window(window, |_, window, cx| {
        let input = selection.read(cx).input.clone();
        assert!(input.focus_handle(cx).is_focused(window));
        let other = cx.focus_handle();
        other.focus(window, cx);
        view.update(cx, |_, cx| cx.notify());
        window.render_frame(cx);
        assert!(
            other.is_focused(window),
            "render must not refocus the panel"
        );
        input.focus_handle(cx).focus(window, cx);
    })
    .unwrap();
    frame(cx, window);
    selection.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.command.read(cx).selected_index(),
            Some(IndexPath::new(39))
        )
    });
    // The real virtual list also accepts wheel events without scrolling the body.
    let before = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    let row_before = visual.debug_bounds("selection-item-39").unwrap();
    let list_bounds = cx
        .update_window(window, |_, window, _| window.find("command").bounds())
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "command",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.))),
            cx,
        );
    })
    .unwrap();
    frame(cx, window);
    let row_after = visual.debug_bounds("selection-item-39");
    assert!(
        row_after.is_none_or(|row| row.top() >= list_bounds.bottom()
            && row.origin.y - row_before.origin.y == px(120.)),
        "wheel must move the previous visible item below the viewport; overscan may keep it mounted: before={row_before:?}, after={row_after:?}, list={list_bounds:?}"
    );
    assert_eq!(
        view.read_with(cx, |viewer, cx| viewer.document.distance(cx)),
        before
    );
}

#[gpui_kit::test]
fn selection_panel_normal_keys_and_direct_actions_are_guarded_and_resume_after_cancel(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(
        cx,
        "[Link](target.md)\n\n".to_owned() + &"target body\n\n".repeat(100),
    );
    frame(cx, window);
    let (reads, _subscription) = capture_reads(cx, &view);
    press(cx, window, "f");
    let selection = panel(cx, &view);
    for key in ["j", "k", "n", "f"] {
        press(cx, window, key);
    }
    for key in [
        "f5", "ctrl-o", "ctrl-i", "ctrl-d", "ctrl-u", "ctrl-f", "ctrl-b",
    ] {
        press(cx, window, key);
    }
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(super::super::input::Find), cx);
        window.dispatch_action(Box::new(super::super::input::Reload), cx);
    })
    .unwrap();
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.mode, Mode::Normal);
        assert_eq!(viewer.link_panel.as_ref().unwrap().panel, selection);
        assert_eq!(selection.read(cx).input.read(cx).value().as_ref(), "jknf");
        assert_eq!(viewer.document.distance(cx), 0.);
    });
    assert!(reads.borrow().is_empty());
    press(cx, window, "escape");
    press(cx, window, "j");
    view.read_with(cx, |viewer, cx| assert!(viewer.document.distance(cx) > 0.));
    press(cx, window, "f5");
    assert_eq!(reads.borrow().len(), 1);
}

#[gpui_kit::test]
fn selection_panel_backdrop_consumes_click_over_a_body_link_and_restores_focus(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Open](target.md)".into());
    frame(cx, window);
    let (reads, _subscription) = capture_reads(cx, &view);
    press(cx, window, "f");
    let mut visual = VisualTestContext::from_window(window, cx);
    visual.simulate_click(point(px(20.), px(20.)), Default::default());
    frame(cx, window);
    assert!(
        reads.borrow().is_empty(),
        "backdrop click must not follow underlying link"
    );
    cx.update_window(window, |_, window, cx| {
        assert!(view.read(cx).link_panel.is_none());
        assert!(view.read(cx).body_focus.is_focused(window));
    })
    .unwrap();
    // A separate click after dismissal still uses the existing route.
    visual.simulate_mouse_down(
        point(px(20.), px(20.)),
        MouseButton::Left,
        Default::default(),
    );
    visual.simulate_mouse_up(
        point(px(20.), px(20.)),
        MouseButton::Left,
        Default::default(),
    );
    frame(cx, window);
    assert_eq!(reads.borrow().len(), 1);
}

#[gpui_kit::test]
fn selection_panel_reordering_preserves_id_for_keyboard_and_mouse_confirmation(
    cx: &mut TestAppContext,
) {
    for mouse in [false, true] {
        let (window, view) = open(
            cx,
            "[distant slow gap](slow.md)\n\n[DSG](fast.md)\n\n[Other](design.md)".into(),
        );
        frame(cx, window);
        let (reads, _subscription) = capture_reads(cx, &view);
        press(cx, window, "f");
        input(cx, window, "dsg");
        panel(cx, &view).read_with(cx, |panel, _| {
            assert_eq!(panel.items.len(), 3);
            assert_eq!(
                panel.items[0].id, 1,
                "fuzzy score must reorder, not just filter"
            );
        });
        if mouse {
            let mut visual = VisualTestContext::from_window(window, cx);
            let row = visual.debug_bounds("selection-item-1").unwrap();
            visual.simulate_click(row.center(), Default::default());
            frame(cx, window);
        } else {
            press(cx, window, "enter");
        }
        assert_eq!(reads.borrow().len(), 1);
        assert_eq!(
            reads.borrow()[0].path,
            PathBuf::from(r"C:\fixtures\fast.md")
        );
        view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
    }
}

#[gpui_kit::test]
fn selection_panel_heading_and_web_confirmation_use_the_existing_resolved_route(
    cx: &mut TestAppContext,
) {
    let source = "[Section](#target)\n\n[Website](https://example.com/docs)\n\n".to_owned()
        + &"Paragraph\n\n".repeat(50)
        + "# Target\n\nTail";
    let (window, view) = open(cx, source);
    frame(cx, window);
    let (reads, _subscription) = capture_reads(cx, &view);
    press(cx, window, "f");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, cx| {
        assert!(viewer.link_panel.is_none());
        assert!(viewer.document.distance(cx) > 0.);
    });
    assert!(
        reads.borrow().is_empty(),
        "same-document heading must not read again"
    );
    press(cx, window, "f");
    input(cx, window, "website");
    press(cx, window, "enter");
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    assert!(reads.borrow().is_empty());
    view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
}

#[gpui_kit::test]
fn selection_panel_ime_enter_escape_and_commit_before_press_enter_event(cx: &mut TestAppContext) {
    for order in ["marked", "commit-before-event", "platform-consumed"] {
        let (window, view) = open(cx, "[日本語](target.md)".into());
        frame(cx, window);
        let (reads, _subscription) = capture_reads(cx, &view);
        press(cx, window, "f");
        let selection = panel(cx, &view);
        let query = selection.read_with(cx, |panel, _| panel.input.clone());
        cx.update_window(window, |_, window, cx| {
            query.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "日本語", Some(3..3), window, cx)
            });
        })
        .unwrap();
        frame(cx, window);
        cx.update_window(window, |_, window, cx| {
            // Character-input preference lets the IME process the actual keydown
            // first. Then commit before the widget's real Enter action, or consume
            // the key completely (no PressEnter until a later independent key).
            if order != "marked" {
                let key = gpui_kit::Keystroke::parse("enter").unwrap();
                window.dispatch_event(
                    gpui_kit::KeyDownEvent {
                        keystroke: key.clone(),
                        is_held: false,
                        prefer_character_input: true,
                    }
                    .to_platform_input(),
                    cx,
                );
                query.update(cx, |input, cx| {
                    input.replace_text_in_range(None, "日本語", window, cx)
                });
                if order == "commit-before-event" {
                    window.dispatch_action(
                        Box::new(gpui_kit::base::input::Enter {
                            secondary: false,
                            shift: false,
                        }),
                        cx,
                    );
                }
                window.dispatch_event(
                    gpui_kit::KeyUpEvent { keystroke: key }.to_platform_input(),
                    cx,
                );
            } else {
                window.press("enter", cx);
            }
        })
        .unwrap();
        frame(cx, window);
        assert!(reads.borrow().is_empty());
        view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_some()));
        if order == "marked" {
            cx.update_window(window, |_, window, cx| {
                query.update(cx, |input, cx| {
                    input.replace_text_in_range(None, "日本語", window, cx)
                })
            })
            .unwrap();
            frame(cx, window);
        }
        press(cx, window, "enter");
        assert_eq!(reads.borrow().len(), 1);
        view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
    }
    let (window, view) = open(cx, "[日本語](target.md)".into());
    frame(cx, window);
    press(cx, window, "f");
    let query = panel(cx, &view).read_with(cx, |panel, _| panel.input.clone());
    cx.update_window(window, |_, window, cx| {
        query.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "日本語", Some(3..3), window, cx)
        })
    })
    .unwrap();
    frame(cx, window);
    press(cx, window, "escape");
    view.read_with(cx, |viewer, _| {
        assert!(
            viewer.link_panel.is_some(),
            "composition Esc must not dismiss"
        )
    });
    cx.update_window(window, |_, window, cx| {
        query.update(cx, |input, cx| {
            assert!(input.marked_text_range(window, cx).is_none())
        })
    })
    .unwrap();
    press(cx, window, "escape");
    view.read_with(cx, |viewer, _| assert!(viewer.link_panel.is_none()));
}
