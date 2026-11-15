use super::*;
pub(super) use crate::ui::lua_floating::standard_links::{
    frame, input, is_open, panel, press, query, replace_query, selected,
};
use crate::{app::Mode, lua::ui::list::Id};
use gpui_kit::{
    EntityInputHandler as _, InputEvent as _, VisualTestContext, test::TestWindowExt as _,
};

pub(super) fn capture_reads(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<Viewer>,
) -> (Rc<RefCell<Vec<ReadRequest>>>, gpui_kit::Subscription) {
    let reads = Rc::new(RefCell::new(Vec::new()));
    let captured = reads.clone();
    let subscription = cx.update(|cx| {
        cx.subscribe(view, move |_, r: &ReadRequest, _| {
            captured.borrow_mut().push(r.clone())
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
    let (reads, _sub) = capture_reads(cx, &view);
    for mode in ["/", ":"] {
        press(cx, window, mode);
        press(cx, window, "f");
        assert!(!is_open(cx, &view));
        view.read_with(cx, |v, cx| {
            assert_eq!(v.input.state.read(cx).value().as_ref(), "f")
        });
        press(cx, window, "escape");
    }
    press(cx, window, "f");
    let original = panel(cx, &view).owner;
    assert_eq!(selected(cx, &view), Some(Id::Integer(0)));
    assert!(reads.borrow().is_empty());
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    input(cx, window, "only");
    press(cx, window, "escape");
    press(cx, window, "f");
    assert_ne!(panel(cx, &view).owner, original);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        ""
    );
    assert!(reads.borrow().is_empty());
}

#[gpui_kit::test]
fn selection_panel_navigation_wraps_repaint_and_native_input_keeps_focus(cx: &mut TestAppContext) {
    let source = (0..40)
        .map(|i| format!("[Link {i}](link-{i}.md)\n\n"))
        .collect();
    let (window, view) = open(cx, source);
    frame(cx, window);
    press(cx, window, "f");
    let q = query(cx, &view);
    for (key, row) in [
        ("up", 39),
        ("ctrl-n", 0),
        ("down", 1),
        ("ctrl-p", 0),
        ("ctrl-p", 39),
    ] {
        press(cx, window, key);
        assert_eq!(selected(cx, &view), Some(Id::Integer(row)));
    }
    frame(cx, window);
    assert_eq!(selected(cx, &view), Some(Id::Integer(39)));
    cx.update_window(window, |_, w, cx| assert!(q.focus_handle(cx).is_focused(w)))
        .unwrap();
    let before = view.read_with(cx, |v, cx| v.document.distance(cx));
    let mut visual = VisualTestContext::from_window(window, cx);
    let list_bounds = visual.debug_bounds("lua-component-links-list").unwrap();
    let row = visual.debug_bounds("lua-component-links-item-39").unwrap();
    assert!(
        row.top() >= list_bounds.top() && row.bottom() <= list_bounds.bottom(),
        "navigation reveals selected row"
    );
    let offset = crate::ui::lua_floating::standard_links::list_scroll(cx, &view);
    assert!(offset < 0.);
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "lua-component-links-list",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, window);
    assert_eq!(view.read_with(cx, |v, cx| v.document.distance(cx)), before);
    let after = crate::ui::lua_floating::standard_links::list_scroll(cx, &view);
    assert!(
        (after - offset - 120.).abs() < 1.,
        "list wheel actually scrolls without changing the body"
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
    let (reads, _sub) = capture_reads(cx, &view);
    press(cx, window, "f");
    let owner = panel(cx, &view).owner;
    for key in [
        "j", "k", "n", "f", "f5", "ctrl-o", "ctrl-i", "ctrl-d", "ctrl-u", "ctrl-f", "ctrl-b",
    ] {
        press(cx, window, key);
    }
    cx.update_window(window, |_, w, cx| {
        w.dispatch_action(Box::new(super::super::input::Find), cx);
        w.dispatch_action(Box::new(super::super::input::Reload), cx);
    })
    .unwrap();
    frame(cx, window);
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        "jknf"
    );
    view.read_with(cx, |v, cx| {
        assert_eq!(v.app.mode, Mode::Normal);
        assert_eq!(v.document.distance(cx), 0.);
    });
    assert!(reads.borrow().is_empty());
    press(cx, window, "escape");
    press(cx, window, "j");
    view.read_with(cx, |v, cx| assert!(v.document.distance(cx) > 0.));
    press(cx, window, "f5");
    assert_eq!(reads.borrow().len(), 1);
}

#[gpui_kit::test]
fn selection_panel_backdrop_consumes_click_over_a_body_link_and_restores_focus(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Open](target.md)".into());
    frame(cx, window);
    let (reads, _sub) = capture_reads(cx, &view);
    press(cx, window, "f");
    let mut visual = VisualTestContext::from_window(window, cx);
    visual.simulate_click(point(px(20.), px(20.)), Default::default());
    frame(cx, window);
    assert!(!is_open(cx, &view));
    assert!(reads.borrow().is_empty());
    cx.update_window(window, |_, w, cx| {
        view.read_with(cx, |v, _| assert!(v.body_focus.is_focused(w)))
    })
    .unwrap();
    visual.simulate_click(point(px(20.), px(20.)), Default::default());
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
        let (reads, _sub) = capture_reads(cx, &view);
        press(cx, window, "f");
        input(cx, window, "dsg");
        let p = panel(cx, &view);
        assert_eq!(
            p.components["links-list"]
                .node
                .list
                .as_ref()
                .unwrap()
                .ids
                .len(),
            3
        );
        assert_eq!(selected(cx, &view), Some(Id::Integer(1)));
        if mouse {
            let mut visual = VisualTestContext::from_window(window, cx);
            let list = visual.debug_bounds("lua-component-links-list").unwrap();
            visual.simulate_click(list.origin + point(px(10.), px(10.)), Default::default());
            frame(cx, window);
        } else {
            press(cx, window, "enter");
        }
        assert_eq!(reads.borrow().len(), 1);
        assert_eq!(
            reads.borrow()[0].path,
            PathBuf::from(r"C:\fixtures\fast.md")
        );
        assert!(!is_open(cx, &view));
    }
}

#[gpui_kit::test]
fn selection_panel_heading_and_web_confirmation_use_the_existing_resolved_route(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(
        cx,
        "[Section](#target)\n\n[Website](https://example.com/docs)\n\n".to_owned()
            + &"Paragraph\n\n".repeat(50)
            + "# Target\n\nTail",
    );
    frame(cx, window);
    let (reads, _sub) = capture_reads(cx, &view);
    let urls = Rc::new(RefCell::new(Vec::<String>::new()));
    let captured = urls.clone();
    view.update(cx, |v, _| {
        v.browser_launcher = Box::new(move |url| {
            captured.borrow_mut().push(url.into());
            Ok(())
        })
    });
    press(cx, window, "f");
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
    view.read_with(cx, |v, cx| assert!(v.document.distance(cx) > 0.));
    assert!(reads.borrow().is_empty());
    press(cx, window, "f");
    input(cx, window, "website");
    press(cx, window, "enter");
    assert_eq!(&*urls.borrow(), &["https://example.com/docs"]);
    assert!(reads.borrow().is_empty());
    assert!(!is_open(cx, &view));
}

#[gpui_kit::test]
fn selection_panel_ime_enter_escape_and_commit_before_press_enter_event(cx: &mut TestAppContext) {
    for order in ["marked", "commit-before-event", "platform-consumed"] {
        let (window, view) = open(cx, "[日本語](target.md)".into());
        frame(cx, window);
        let (reads, _sub) = capture_reads(cx, &view);
        press(cx, window, "f");
        let q = query(cx, &view);
        cx.update_window(window, |_, w, cx| {
            q.update(cx, |i, cx| {
                i.replace_and_mark_text_in_range(None, "日本語", Some(3..3), w, cx)
            })
        })
        .unwrap();
        frame(cx, window);
        cx.update_window(window, |_, w, cx| {
            if order == "marked" {
                w.press("enter", cx);
            } else {
                let key = gpui_kit::Keystroke::parse("enter").unwrap();
                w.dispatch_event(
                    gpui_kit::KeyDownEvent {
                        keystroke: key.clone(),
                        is_held: false,
                        prefer_character_input: true,
                    }
                    .to_platform_input(),
                    cx,
                );
                q.update(cx, |i, cx| i.replace_text_in_range(None, "日本語", w, cx));
                if order == "commit-before-event" {
                    w.dispatch_action(
                        Box::new(gpui_kit::base::input::Enter {
                            secondary: false,
                            shift: false,
                        }),
                        cx,
                    );
                }
                w.dispatch_event(
                    gpui_kit::KeyUpEvent { keystroke: key }.to_platform_input(),
                    cx,
                );
            }
        })
        .unwrap();
        frame(cx, window);
        assert!(reads.borrow().is_empty());
        assert!(is_open(cx, &view));
        if order == "marked" {
            replace_query(cx, window, &view, "日本語");
        }
        press(cx, window, "enter");
        assert_eq!(reads.borrow().len(), 1);
        assert!(!is_open(cx, &view));
    }
}
