//! Optional public-API link picker and real GPUI key-routing integration tests.
use super::Viewer;
use crate::lua::ui::list::Id;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, EntityInputHandler as _, InputEvent as _,
    TestAppContext, VisualTestContext, point, px, size, test::TestWindowExt as _,
};

pub(in crate::ui) fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| w.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}
pub(in crate::ui) fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, window);
}
pub(in crate::ui) fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, window);
}
pub(in crate::ui) struct Panel {
    pub owner: crate::lua::OwnerId,
    pub modal: bool,
    pub root: std::rc::Rc<crate::lua::ui::tree::Node>,
    pub components: std::collections::BTreeMap<String, crate::lua::ui::Component>,
}
pub(in crate::ui) fn panel(cx: &TestAppContext, view: &Entity<Viewer>) -> Panel {
    view.read_with(cx, |v, _| {
        let context = v.lua.context();
        let ui = context.ui.borrow();
        let w = ui.windows.last().expect("open float");
        Panel {
            owner: w.owner,
            modal: w.modal,
            root: w.root.clone(),
            components: w.components.clone(),
        }
    })
}
pub(in crate::ui) fn is_open(cx: &TestAppContext, view: &Entity<Viewer>) -> bool {
    view.read_with(cx, |v, _| !v.lua.context().ui.borrow().windows.is_empty())
}
pub(in crate::ui) fn query(
    cx: &TestAppContext,
    view: &Entity<Viewer>,
) -> Entity<gpui_kit::component::input::InputState> {
    view.read_with(cx, |v, _| {
        v.floating_ui
            .inputs
            .values()
            .next()
            .expect("query")
            .state
            .clone()
    })
}
pub(in crate::ui) fn replace_query(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<Viewer>,
    text: &str,
) {
    let query = query(cx, view);
    cx.update_window(window, |_, w, cx| {
        query.update(cx, |i, cx| {
            let end = i.value().encode_utf16().count();
            i.replace_text_in_range(Some(0..end), text, w, cx);
        })
    })
    .unwrap();
    frame(cx, window);
}
pub(in crate::ui) fn selected(cx: &TestAppContext, view: &Entity<Viewer>) -> Option<Id> {
    panel(cx, view).components["links-list"]
        .node
        .list
        .as_ref()
        .unwrap()
        .selected
        .clone()
}
pub(in crate::ui) fn list_scroll(cx: &TestAppContext, view: &Entity<Viewer>) -> f32 {
    view.read_with(cx, |v, _| {
        let context = v.lua.context();
        let ui = context.ui.borrow();
        let w = ui.windows.last().unwrap();
        f32::from(
            v.floating_ui.lists[&(w.owner, w.components["links-list"].identity)]
                .scroll
                .offset()
                .y,
        )
    })
}
pub(in crate::ui) fn preview_metrics(
    cx: &TestAppContext,
    view: &Entity<Viewer>,
) -> (f32, f32, usize) {
    view.read_with(cx, |v, cx| {
        let p = v
            .floating_ui
            .sources
            .values()
            .next()
            .expect("source")
            .read(cx);
        (p.scroll, p.viewport, p.top_source())
    })
}
fn event(view: &Entity<Viewer>, cx: &mut TestAppContext, source: &str) {
    view.update(cx, |v, cx| {
        let f = v
            .lua
            .lua()
            .load(format!("return function() {source} end"))
            .eval::<mlua::Function>()
            .unwrap();
        let context = v.lua.context();
        let callback = context
            .register_once(v.lua.lua(), f, crate::lua::CallbackScope::app())
            .unwrap()
            .unwrap();
        context.enqueue(v.lua.lua(), callback, ()).unwrap();
        v.pump_lua(cx);
        cx.notify();
    });
}
fn assert_lua(view: &Entity<Viewer>, cx: &TestAppContext, source: &str) {
    view.read_with(cx, |v, _| v.lua.lua().load(source).exec().unwrap());
}
fn text(node: &crate::lua::ui::tree::Node) -> String {
    let mut result = node.text.clone();
    for child in &node.children {
        result.push_str(&text(child));
    }
    result
}

#[gpui_kit::test]
fn standard_lua_pending_native_query_cannot_confirm_or_defer_the_old_target(
    cx: &mut TestAppContext,
) {
    let (window, view) = crate::ui::tests::open(cx, "[Alpha](alpha.md)\n\n[Beta](beta.md)".into());
    frame(cx, window);
    let reads = std::rc::Rc::new(std::cell::RefCell::new(
        Vec::<crate::app::ReadRequest>::new(),
    ));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, r: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(r.clone())
        })
    });
    press(cx, window, "f");
    let owner = panel(cx, &view).owner;
    let q = query(cx, &view);
    cx.update_window(window, |_, w, cx| {
        q.update(cx, |i, cx| {
            i.replace_text_in_range(Some(0..0), "beta", w, cx)
        });
        // Dispatch before a frame or the deferred native Change callback.
        w.dispatch_event(
            gpui_kit::KeyDownEvent {
                keystroke: gpui_kit::Keystroke::parse("enter").unwrap(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    frame(cx, window);
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(selected(cx, &view), Some(Id::Integer(1)));
    assert!(
        reads.borrow().is_empty(),
        "pending confirm must not be replayed after synchronization"
    );
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
    assert_eq!(reads.borrow().len(), 1);
    assert_eq!(
        reads.borrow()[0].path,
        std::path::PathBuf::from(r"C:\fixtures\beta.md")
    );
}

#[gpui_kit::test]
fn standard_lua_render_failure_preserves_tree_blocks_old_confirm_and_recovers(
    cx: &mut TestAppContext,
) {
    let (window, view) = crate::ui::tests::open(cx, "[Alpha](alpha.md)\n\n[Beta](beta.md)".into());
    frame(cx, window);
    let reads = std::rc::Rc::new(std::cell::RefCell::new(
        Vec::<crate::app::ReadRequest>::new(),
    ));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, r: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(r.clone())
        })
    });
    press(cx, window, "f");
    let previous = panel(cx, &view);
    // Substitute only the pure matcher failure; the bundled render and host
    // input/list/navigation boundaries remain real.
    event(
        &view,
        cx,
        "local m=require('mmv'); saved_fuzzy=m.match.fuzzy; m.match.fuzzy=function() error('controlled standard render failure') end",
    );
    replace_query(cx, window, &view, "beta");
    assert!(std::rc::Rc::ptr_eq(&panel(cx, &view).root, &previous.root));
    press(cx, window, "enter");
    assert_eq!(panel(cx, &view).owner, previous.owner);
    assert!(reads.borrow().is_empty());
    event(
        &view,
        cx,
        "require('mmv').match.fuzzy=saved_fuzzy; saved_fuzzy=nil",
    );
    replace_query(cx, window, &view, "bet");
    assert_eq!(selected(cx, &view), Some(Id::Integer(1)));
    assert!(
        reads.borrow().is_empty(),
        "recovery must not replay the failed confirm"
    );
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
    assert_eq!(reads.borrow().len(), 1);
    assert_eq!(
        reads.borrow()[0].path,
        std::path::PathBuf::from(r"C:\fixtures\beta.md")
    );
}

#[gpui_kit::test]
fn standard_lua_pending_edit_close_releases_host_resources_and_subscriptions_repeatedly(
    cx: &mut TestAppContext,
) {
    let (window, view) = crate::ui::tests::open(cx, "[Alpha](alpha.md)".into());
    frame(cx, window);
    event(
        &view,
        cx,
        r#"
        local u=require('mmv').ui
        weak=setmetatable({}, {__mode='v'}); serial=0
        local original=u.float.open
        u.float.open=function(options)
            local win=original(options)
            serial=serial+1; weak[serial]=win
            return win
        end
    "#,
    );
    for _ in 0..12 {
        press(cx, window, "f");
        assert_eq!(
            query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
            ""
        );
        let q = query(cx, &view);
        cx.update_window(window, |_, w, cx| {
            q.update(cx, |i, cx| {
                i.replace_text_in_range(Some(0..0), "pending", w, cx)
            });
            w.dispatch_event(
                gpui_kit::KeyDownEvent {
                    keystroke: gpui_kit::Keystroke::parse("escape").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        frame(cx, window);
        assert!(!is_open(cx, &view));
        view.read_with(cx, |v, _| {
            assert!(v.floating_ui.inputs.is_empty());
            assert!(v.floating_ui.sources.is_empty());
            assert!(v.floating_ui.lists.is_empty());
            assert!(v.floating_ui.handles.is_empty());
            v.lua.lua().gc_collect().unwrap();
            v.lua.lua().gc_collect().unwrap();
        });
        assert_lua(
            &view,
            cx,
            "assert(next(weak)==nil, 'closed standard window retained')",
        );
    }
    view.update(cx, |v, cx| {
        let crate::app::OperationResult::Pending(read) =
            v.request_document(v.path.with_file_name("after-close.md"), None, cx)
        else {
            panic!("replacement read")
        };
        v.complete_fixture(crate::ui::tests::success(read, "replacement".into()), cx);

    });
    frame(cx, window);
    view.read_with(cx, |v, _| {
        assert!(
            v.message.is_none(),
            "cancelled subscriptions must not notify later adoption"
        );
        assert_eq!(v.app.document().unwrap().markdown_source().unwrap().as_str(), "replacement");

    });
}

#[gpui_kit::test]
fn standard_lua_reproduction_layout_ranking_highlights_and_confirm(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "[Other](other.md)\n\n[日本語 design](日本語-design.md#start)\n\n[Design overview](overview.md)\n\n[Same](#start)\n\n[Web](https://example.com)".into());
    frame(cx, window);
    let reads = std::rc::Rc::new(std::cell::RefCell::new(
        Vec::<crate::app::ReadRequest>::new(),
    ));
    let captured = reads.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&view, move |_, r: &crate::app::ReadRequest, _| {
            captured.borrow_mut().push(r.clone())
        })
    });
    press(cx, window, "f");
    let p = panel(cx, &view);
    assert!(p.modal);
    assert_eq!(selected(cx, &view), Some(Id::Integer(0)));
    assert_eq!(
        p.components["links-list"]
            .node
            .list
            .as_ref()
            .unwrap()
            .ids
            .len(),
        5
    );
    assert!(text(&p.root).contains("Markdown heading"));
    assert!(text(&p.root).contains("Heading"));
    assert!(text(&p.root).contains("Web"));
    assert!(reads.borrow().is_empty());
    let mut visual = VisualTestContext::from_window(window, cx);
    let bounds = visual.debug_bounds("lua-window-0").unwrap();
    assert_eq!(bounds.size, size(px(512.), px(288.)));
    assert_eq!(bounds.origin, point(px(64.), px(36.)));
    let left = visual.debug_bounds("lua-component-links-list").unwrap();
    let right = visual.debug_bounds("lua-component-links-source").unwrap();
    let q = visual.debug_bounds("lua-component-links-query").unwrap();
    assert!(left.size.width > px(0.) && right.size.width > px(0.));
    assert!(q.top() >= left.bottom() - px(1.));
    assert!(q.size.width >= left.size.width + right.size.width - px(1.));
    press(cx, window, "down");
    input(cx, window, "日dsg");
    assert_eq!(selected(cx, &view), Some(Id::Integer(1)));
    let p = panel(cx, &view);
    let list = &p.components["links-list"].node;
    assert_eq!(list.list.as_ref().unwrap().ids, vec![Id::Integer(1)]);
    for node in &list.children[0].children[..2] {
        assert_eq!(
            node.highlights
                .iter()
                .map(|h| &node.text[h.range.clone()])
                .collect::<String>(),
            "日dsg"
        );
    }
    assert!(reads.borrow().is_empty(), "one result never auto-confirms");
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
    let read = reads.borrow_mut().pop().unwrap();
    assert_eq!(
        read.path,
        std::path::PathBuf::from(r"C:\fixtures\日本語-design.md")
    );
    view.update(cx, |v, cx| {
        v.complete_fixture(
            crate::app::ReadResult {

                request: read,
                content: Err("missing".into()),
            },
            cx,
        )
    });
    frame(cx, window);
    assert!(!is_open(cx, &view));
    press(cx, window, "f");
    assert_ne!(panel(cx, &view).owner, p.owner);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        ""
    );
    assert_eq!(selected(cx, &view), Some(Id::Integer(0)));
}

#[gpui_kit::test]
fn standard_lua_reproduction_source_same_id_revival_and_native_ime(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler as _;
    let source = format!(
        "[First](missing.md)\n\n{}[Second](two.md)\n\n{}",
        "raw body\n\n".repeat(100),
        "tail\n\n".repeat(80)
    );
    let (window, view) = crate::ui::tests::open(cx, source);
    frame(cx, window);
    press(cx, window, "f");
    let preview = view.read_with(cx, |v, _| {
        v.floating_ui.sources.values().next().unwrap().clone()
    });
    view.read_with(cx, |v, cx| {
        assert!(std::sync::Arc::ptr_eq(
            &v.app.document().unwrap().markdown_source().unwrap(),

            &preview.read(cx).model.source.text()
        ))
    });
    press(cx, window, "ctrl-d");
    let manual = preview.read_with(cx, |p, _| {
        assert!((p.scroll - p.viewport * 0.5).abs() < 1.);
        p.scroll
    });
    replace_query(cx, window, &view, "first");
    assert_eq!(preview.read_with(cx, |p, _| p.scroll), manual);
    let q = query(cx, &view);
    cx.update_window(window, |_, w, cx| {
        q.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "日", Some(1..1), w, cx)
        })
    })
    .unwrap();
    frame(cx, window);
    press(cx, window, "ctrl-d");
    assert_eq!(preview.read_with(cx, |p, _| p.scroll), manual);
    press(cx, window, "enter");
    assert!(is_open(cx, &view), "IME Enter must not confirm");
    press(cx, window, "escape");
    assert!(is_open(cx, &view), "IME Escape must not close");
    replace_query(cx, window, &view, "zz-no-match");
    assert_eq!(selected(cx, &view), None);
    assert!(!panel(cx, &view).components.contains_key("links-source"));
    assert!(text(&panel(cx, &view).root).contains("No matching links"));
    press(cx, window, "enter");
    assert!(is_open(cx, &view));
    replace_query(cx, window, &view, "");
    assert_eq!(selected(cx, &view), Some(Id::Integer(0)));
    let revived = view.read_with(cx, |v, _| {
        v.floating_ui.sources.values().next().unwrap().clone()
    });
    assert_ne!(revived, preview);
    assert_eq!(revived.read_with(cx, |p, _| p.scroll), 0.);
    press(cx, window, "ctrl-n");
    assert_eq!(selected(cx, &view), Some(Id::Integer(1)));
    assert!(revived.read_with(cx, |p, _| p.scroll) > 500.);
    press(cx, window, "ctrl-p");
    assert_eq!(selected(cx, &view), Some(Id::Integer(0)));
    press(cx, window, "escape");
    assert!(!is_open(cx, &view));
    cx.update_window(window, |_, w, cx| {
        view.read_with(cx, |v, _| assert!(v.body_focus.is_focused(w)))
    })
    .unwrap();
}

#[gpui_kit::test]
fn standard_lua_reproduction_replacement_closes_only_on_adoption(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "[Old](old.md)".into());
    frame(cx, window);
    press(cx, window, "f");
    input(cx, window, "old");
    let owner = panel(cx, &view).owner;
    view.update(cx, |v, cx| {
        let crate::app::OperationResult::Pending(old) =
            v.request_document(v.path.with_file_name("old-request.md"), None, cx)
        else {
            panic!()
        };
        let crate::app::OperationResult::Pending(latest) =
            v.request_document(v.path.with_file_name("missing.md"), None, cx)
        else {
            panic!()
        };
        assert_eq!(v.lua.context().ui.borrow().windows[0].owner, owner);
        v.complete_fixture(
            crate::app::ReadResult {

                request: latest,
                content: Err("missing".into()),
            },
            cx,
        );
        v.complete_fixture(crate::ui::tests::success(old, "stale".into()), cx);

    });
    frame(cx, window);
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        "old"
    );
    view.update(cx, |v, cx| {
        let crate::app::OperationResult::Pending(read) =
            v.request_document(v.path.with_file_name("new.md"), None, cx)
        else {
            panic!()
        };
        v.complete_fixture(crate::ui::tests::success(read, "[New](new.md)".into()), cx);

    });
    frame(cx, window);
    assert!(!is_open(cx, &view));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.message.as_deref(),
            Some("Link selection closed because the document changed")
        )
    });
    press(cx, window, "enter");
    assert!(!is_open(cx, &view));
}

#[gpui_kit::test]
fn app_lua_routing_overrides_defaults_tree_and_runtime_removal(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "body\n\n".repeat(120));
    frame(cx, window);
    event(
        &view,
        cx,
        "local m=require('mmv'); hits=0; m.keymap.set('j',function() hits=hits+1 end); m.keymap.set('ALT-CTRL-x',function() hits=hits+10 end); m.keymap.set('ctrl--',function() hits=hits+100 end)",
    );
    let before = view.read_with(cx, |v, cx| v.document.distance(cx));
    press(cx, window, "j");
    assert_lua(&view, cx, "assert(hits==1)");
    assert_eq!(view.read_with(cx, |v, cx| v.document.distance(cx)), before);
    press(cx, window, "ctrl-alt-x");
    press(cx, window, "ctrl--");
    assert_lua(&view, cx, "assert(hits==111)");
    cx.update_window(window, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.tree.visible = true;
            v.tree.focus.focus(w, cx);
        })
    })
    .unwrap();
    frame(cx, window);
    press(cx, window, "j");
    assert_lua(&view, cx, "assert(hits==112)");
    event(
        &view,
        cx,
        "assert(require('mmv').keymap.remove('j')); assert(not require('mmv').keymap.remove('j'))",
    );
    press(cx, window, "j");
    assert_lua(&view, cx, "assert(hits==112)");
    event(
        &view,
        cx,
        "require('mmv').keymap.set('f',function() hits=hits+1; error('key failure') end)",
    );
    press(cx, window, "f");
    assert_lua(&view, cx, "assert(hits==113)");
    assert!(!is_open(cx, &view));
}

#[gpui_kit::test]
fn app_lua_routing_nonmodal_native_input_modal_and_search_command(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "[Link](link.md)".into());
    frame(cx, window);
    event(
        &view,
        cx,
        "local m=require('mmv'); hits=0; m.keymap.set('j',function()hits=hits+1 end); m.keymap.set('ctrl-k',function()hits=hits+10 end); q=m.ui.state(''); w=m.ui.float.open{width=200,height=120,modal=false,focus='q',render=function()return m.ui.input{key='q',bind=q}end}",
    );
    frame(cx, window);
    press(cx, window, "j");
    press(cx, window, "ctrl-k");
    assert_lua(&view, cx, "assert(q:get()=='j' and hits==10)");
    event(
        &view,
        cx,
        "w:close(); local u=require('mmv').ui; w=u.float.open{width=100,height=100,modal=false,focus='b',render=function()return u.button{key='b',label='button',on_click=function()end}end}",
    );
    frame(cx, window);
    press(cx, window, "j");
    assert_lua(&view, cx, "assert(hits==11)");
    event(
        &view,
        cx,
        "w:close(); local u=require('mmv').ui; w=u.float.open{width=100,height=100,render=function()return u.text{text='modal'}end}",
    );
    frame(cx, window);
    press(cx, window, "j");
    press(cx, window, "ctrl-k");
    assert_lua(&view, cx, "assert(hits==11)");
    press(cx, window, "escape");
    for mode in ["/", ":"] {
        press(cx, window, mode);
        press(cx, window, "j");
        press(cx, window, "f");
        assert!(!is_open(cx, &view));
        view.read_with(cx, |v, cx| {
            assert_eq!(v.input.state.read(cx).value().as_ref(), "jf")
        });
        assert_lua(&view, cx, "assert(hits==11)");
        press(cx, window, "escape");
    }
}

#[gpui_kit::test]
fn link_picker_user_override_remove_and_atomic_failure_leave_f_unbound(cx: &mut TestAppContext) {
    for (user, fails) in [
        ("require('mmv').keymap.remove('f')", false),
        (
            "require('mmv').keymap.set('f',function()overridden=true end)",
            false,
        ),
        (
            "local m=require('mmv');m.keymap.remove('f');m.setup{view={position='left'}};error('broken')",
            true,
        ),
        (
            "local m=require('mmv');m.keymap.remove('f');pcall(m.keymap.set,'F',function()end)",
            true,
        ),
    ] {
        let (window, view) = crate::ui::tests::open(cx, "[Only](only.md)".into());
        let source = format!("{}\n{user}", include_str!("../../plugins/link_picker.lua"));
        let (runtime, error) =
            crate::lua::LuaRuntime::initialize("", Some((&source, "@test-user"))).unwrap();
        assert_eq!(error.is_some(), fails);
        if fails {
            assert_eq!(runtime.config(), crate::config::Config::default());
        }
        view.update(cx, |v, cx| {
            v.release_lua();
            v.lua = runtime;
            v.publish_lua_document(cx);
        });
        frame(cx, window);
        press(cx, window, "f");
        assert!(!is_open(cx, &view));
        view.read_with(cx, |v, _| {
            if fails {
                assert!(!v.lua.enqueue_key("f").unwrap());
            }
        });
    }
}

#[gpui_kit::test]
fn standard_lua_no_document_and_empty_document_do_not_execute(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "No links".into());
    frame(cx, window);
    view.update(cx, |v, _| {
        v.lua.context().documents.borrow_mut().current = None
    });
    press(cx, window, "f");
    assert!(!is_open(cx, &view));
    view.update(cx, |v, cx| v.publish_lua_document(cx));
    press(cx, window, "f");
    assert_eq!(selected(cx, &view), None);
    press(cx, window, "enter");
    assert!(is_open(cx, &view));
}

#[gpui_kit::test]
fn standard_lua_ties_empty_order_and_palette_keep_selected_match_colors(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(
        cx,
        "[Same](one.md)\n\n[Same](two.md)\n\n[Last](last.md)".into(),
    );
    frame(cx, window);
    press(cx, window, "f");
    assert_eq!(
        panel(cx, &view).components["links-list"]
            .node
            .list
            .as_ref()
            .unwrap()
            .ids,
        vec![Id::Integer(0), Id::Integer(1), Id::Integer(2)]
    );
    input(cx, window, "same");
    let p = panel(cx, &view);
    let list = &p.components["links-list"].node;
    assert_eq!(
        list.list.as_ref().unwrap().ids,
        vec![Id::Integer(0), Id::Integer(1)]
    );
    assert_eq!(
        list.children[0].hl,
        Some(crate::config::Group::PanelSelected)
    );
    assert_eq!(list.children[1].hl, Some(crate::config::Group::Panel));
    let name = &list.children[0].children[0];
    assert!(
        name.highlights
            .iter()
            .all(|h| h.bold == Some(true) && h.hl.is_none())
    );
    view.read_with(cx, |v, _| {
        let runs = super::text_highlights(name, &v.palette);
        assert!(
            runs.iter()
                .all(|(_, style)| style.color.is_none() && style.background_color.is_none())
        );
    });
}

#[gpui_kit::test]
fn app_lua_window_precedes_app_and_errors_consume_default_keys(cx: &mut TestAppContext) {
    let (window, view) = crate::ui::tests::open(cx, "body\n\n".repeat(100));
    frame(cx, window);
    event(
        &view,
        cx,
        "local m=require('mmv');local u=m.ui; hits=0;m.keymap.set('ctrl-k',function()hits=hits+100 end);w=u.float.open{width=100,height=100,modal=false,focus='b',keys={['ctrl-k']=function()hits=hits+1;error('window-key')end},render=function()return u.button{key='b',label='b',on_click=function()end}end}",
    );
    frame(cx, window);
    press(cx, window, "ctrl-k");
    assert_lua(&view, cx, "assert(hits==1)");
    event(
        &view,
        cx,
        "w:close(); require('mmv').keymap.set('j',function()error('app-key')end)",
    );
    frame(cx, window);
    let before = view.read_with(cx, |v, cx| v.document.distance(cx));
    press(cx, window, "j");
    assert_eq!(view.read_with(cx, |v, cx| v.document.distance(cx)), before);
    event(&view, cx, "require('mmv').keymap.remove('j')");
    press(cx, window, "j");
    assert!(view.read_with(cx, |v, cx| v.document.distance(cx)) > before);
}
