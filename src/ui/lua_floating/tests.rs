use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, px, test::TestWindowExt as _};

#[gpui_kit::test]
fn revision_r3_pending_input_blocks_cross_window_confirm(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');s=u.state();confirmed=nil
        m.keymap.set('x',function()
            result=u.float.open{width=300,height=200,modal=false,render=function()
                return u.list{key='l',selection=s,items={{id=q:get()=='' and 1 or 2}},
                    render_item=function(item)return u.text{text=tostring(item.id)}end,
                    on_confirm=function(id)confirmed=id end}
            end}
            input=u.float.open{width=300,height=100,modal=false,focus='q',render=function()
                return u.input{key='q',bind=q,keys={['ctrl-k']=function()accepted=result:list('l'):confirm()end}}
            end}
        end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| {
        w.input("a", cx);
        w.press("ctrl-k", cx)
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='a');assert(accepted==false and confirmed==nil,'old cross-window confirmation='..tostring(confirmed))",
    );
    component_event(&view, cx, "assert(result:list('l'):confirm())");
    component_assert(&view, cx, "assert(confirmed==2)");
}

#[gpui_kit::test]
fn revision_r2_on_change_can_restore_previous_bound_value(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');changes=0
        m.keymap.set('x',function()w=u.float.open{width=300,height=100,focus='q',
            render=function()return u.input{key='q',bind=q,on_change=function(text)
                changes=changes+1;q:set('')
            end}end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.input("a", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(q:get()=='' and changes==1)");
    let input = view.read_with(cx, |v, _| {
        v.floating_ui.inputs.values().next().unwrap().state.clone()
    });
    input.read_with(cx, |i, _| {
        assert_eq!(
            i.value().as_ref(),
            "",
            "on_change restored bind but native text did not follow"
        )
    });
}

#[gpui_kit::test]
fn revision_r2_immediate_native_edit_cannot_confirm_old_list(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');s=u.state();confirmed=nil
        m.keymap.set('x',function()w=u.float.open{width=300,height=200,focus='q',
            render=function()return u.column{children={
                u.list{key='l',height=100,selection=s,items={{id=q:get()=='' and 1 or 2}},
                    render_item=function(item)return u.text{text=tostring(item.id)}end,
                    on_confirm=function(id)confirmed=id end},
                u.input{key='q',height=40,bind=q,on_submit=function()w:list('l'):confirm()end}
            }}end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| {
        w.input("a", cx);
        w.press("enter", cx)
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='a');assert(confirmed==nil or confirmed==2,'old confirmation='..tostring(confirmed))",
    );
}

#[gpui_kit::test]
fn revision_r2_pending_edit_blocks_custom_confirm_and_old_row_click(cx: &mut TestAppContext) {
    for action in ["ctrl-k", "click"] {
        let (window, view) = open_fixture(
            cx,
            r#"
            local m=require('mmv');local u=m.ui;q=u.state('');s=u.state()
            m.keymap.set('x',function()w=u.float.open{width=300,height=200,focus='q',render=function()return u.column{children={
                u.list{key='l',height=100,selection=s,items={{id=q:get()=='' and 1 or 2}},render_item=function(item)return u.text{key='row',text=tostring(item.id)}end,on_confirm=function(id)confirmed=id end},
                u.input{key='q',height=40,bind=q,keys={['ctrl-k']=function()accepted=w:list('l'):confirm()end}}
            }}end}end)
        "#,
        );
        open_registered(&view, cx);
        component_frame(cx, window);
        cx.update_window(window, |_, w, cx| {
            w.input("a", cx);
            if action == "click" {
                w.click("lua-component-row", cx)
            } else {
                w.press(action, cx)
            }
        })
        .unwrap();
        component_frame(cx, window);
        component_assert(&view, cx, "assert(q:get()=='a' and confirmed~=1)");
        if action == "ctrl-k" {
            component_assert(&view, cx, "assert(accepted==false)");
        }
        component_event(&view, cx, "assert(w:list('l'):confirm())");
        component_assert(&view, cx, "assert(confirmed==2)");
    }
}

#[gpui_kit::test]
fn revision_r2_ime_cancel_preserves_preexisting_undo_history(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler as _;
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('')
        m.keymap.set('x',function()w=u.float.open{width=300,height=100,focus='q',
            render=function()return u.input{key='q',bind=q}end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.input("abc", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(q:get()=='abc')");
    let input = view.read_with(cx, |v, _| {
        v.floating_ui.inputs.values().next().unwrap().state.clone()
    });
    cx.update_window(window, |_, w, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "日", Some(1..1), w, cx)
        })
    })
    .unwrap();
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.press("escape", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(w:is_open() and q:get()=='abc')");
    cx.update_window(window, |_, w, cx| w.press("ctrl-z", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()~='abc','IME cancel erased undo history')",
    );
    component_event(&view, cx, "q:set('abc')");
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| {
        input.update(cx, |i, cx| {
            i.set_selected_range(1..2, cx);
            i.replace_and_mark_text_in_range(None, "日", Some(1..1), w, cx)
        })
    })
    .unwrap();
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.press("escape", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(w:is_open() and q:get()=='abc')");
    input.read_with(cx, |i, _| {
        assert_eq!(i.value().as_ref(), "abc");
        assert_eq!(i.selected_range(), 1..2);
    });
}

fn component_event(
    view: &gpui_kit::Entity<super::super::Viewer>,
    cx: &mut TestAppContext,
    source: &str,
) {
    view.update(cx, |v, cx| {
        let f = v
            .lua
            .lua()
            .load(format!("return function() {source} end"))
            .eval::<mlua::Function>()
            .unwrap();
        let context = v.lua.context();
        let id = context
            .register_once(v.lua.lua(), f, crate::lua::CallbackScope::app())
            .unwrap()
            .unwrap();
        context.enqueue(v.lua.lua(), id, ()).unwrap();
        v.pump_lua(cx);
        cx.notify();
    });
    cx.run_until_parked();
}
fn component_frame(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| w.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}
fn component_assert(
    view: &gpui_kit::Entity<super::super::Viewer>,
    cx: &TestAppContext,
    source: &str,
) {
    view.read_with(cx, |v, _| {
        v.lua
            .lua()
            .load(source)
            .exec()
            .unwrap_or_else(|e| panic!("{source}: {e}"))
    });
}

#[gpui_kit::test]
fn input_real_editing_orders_callbacks_coalesces_sets_and_keeps_programmatic_sets_silent(
    cx: &mut TestAppContext,
) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');s=u.state();extra=u.state('');log='';changes=0;renders=0
        m.keymap.set('x',function()w=u.float.open{width=400,height=240,focus='q',render=function()
            renders=renders+1;local query=q:get();extra:get()
            local items=query=='' and {{id=1},{id=2}} or {{id=2}}
            return u.column{width='auto',height='auto',children={
                u.list{key='l',height=160,selection=s,items=items,reset_on=query,render_item=function(item,selected)return u.text{text=tostring(item.id)..(selected and '*' or '')}end,on_selection_changed=function(id)assert(s:get()==id);log=log..'S'..id end,on_confirm=function(id)confirmed=id end},
                u.input{key='q',height=40,bind=q,placeholder='query',on_change=function(text)assert(q:get()==text);log=log..'C';changes=changes+1;extra:set('a');extra:set('b')end,on_submit=function()w:list('l'):confirm()end,keys={['ctrl-n']=function()w:list('l'):select_next()end,['ctrl-u']=function()error('handled')end}}
            }}
        end,on_error=function(e)assert(e.key=='q');key_error=true end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    component_assert(&view, cx, "assert(log=='S1');log=''");
    let original = view.read_with(cx, |v, _| {
        v.floating_ui.inputs.values().next().unwrap().state.clone()
    });
    cx.update_window(window, |_, w, cx| {
        use gpui_kit::Focusable as _;
        assert!(original.focus_handle(cx).is_focused(w));
        w.input("a", cx);
    })
    .unwrap();
    component_frame(cx, window);
    original.read_with(cx, |i, _| assert_eq!(i.value().as_ref(), "a"));
    component_assert(
        &view,
        cx,
        "assert(q:get()=='a' and changes==1 and log=='CS2' and extra:get()=='b',q:get()..':'..changes..':'..log..':'..extra:get())",
    );
    component_event(&view, cx, "q:set('program')");
    component_frame(cx, window);
    component_assert(&view, cx, "assert(changes==1)");
    let current = view.read_with(cx, |v, _| {
        v.floating_ui.inputs.values().next().unwrap().state.clone()
    });
    assert_eq!(original, current);
    cx.update_window(window, |_, w, cx| {
        w.press("home", cx);
        w.press("shift-right", cx);
        w.press("backspace", cx)
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='rogram' and changes==2,q:get()..':'..changes)",
    );
    cx.update_window(window, |_, w, cx| w.press("ctrl-z", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(q:get()=='program')");
    cx.update_window(window, |_, w, cx| w.press("ctrl-y", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(q:get()=='rogram')");
    cx.update_window(window, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.press("ctrl-c", cx);
        w.press("ctrl-x", cx);
        w.press("ctrl-v", cx)
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(q:get()=='rogram')");
    cx.update_window(window, |_, w, cx| w.press("ctrl-u", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(key_error and w:is_open())");
    cx.update_window(window, |_, w, cx| w.press("enter", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==2)");
}

#[gpui_kit::test]
fn input_actual_marked_range_survives_rerender_and_ime_precedes_custom_enter_escape(
    cx: &mut TestAppContext,
) {
    use gpui_kit::EntityInputHandler as _;
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');tick=u.state(0);changes=0;submitted=0
        m.keymap.set('x',function()w=u.float.open{width=300,height=100,focus='q',render=function()tick:get();return u.input{key='q',bind=q,on_change=function()changes=changes+1 end,on_submit=function()submitted=submitted+1 end,keys={enter=function()submitted=submitted+10 end,escape=function()w:close()end}}end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    let input = view.read_with(cx, |v, _| {
        v.floating_ui.inputs.values().next().unwrap().state.clone()
    });
    cx.update_window(window, |_, w, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "日本", Some(2..2), w, cx)
        })
    })
    .unwrap();
    component_frame(cx, window);
    component_event(&view, cx, "tick:set(1)");
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='' and changes==0 and submitted==0)",
    );
    cx.update_window(window, |_, w, cx| {
        assert!(input.update(cx, |i, cx| i.marked_text_range(w, cx).is_some()));
        w.press("enter", cx);
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(submitted==0 and w:is_open())");
    cx.update_window(window, |_, w, cx| {
        input.update(cx, |i, cx| i.replace_text_in_range(None, "日本", w, cx))
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='日本' and changes==1 and submitted==0)",
    );
    cx.update_window(window, |_, w, cx| w.press("enter", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(submitted==10)");
    cx.update_window(window, |_, w, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "語", Some(1..1), w, cx)
        })
    })
    .unwrap();
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.press("escape", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(w:is_open() and q:get()=='日本' and changes==1)",
    );
    cx.update_window(window, |_, w, cx| {
        assert!(input.update(cx, |i, cx| i.marked_text_range(w, cx).is_none()));
        w.press("escape", cx);
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(not w:is_open())");
}

#[gpui_kit::test]
fn public_search_list_source_fixture_uses_real_focus_scroll_and_stale_handle_validation(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{Focusable as _, VisualTestContext};
    let text = format!(
        "[Alpha](alpha.md)\n\n{}\n[Beta 日本語](beta.md)\n\n{}",
        "raw 日本語 source\n\n".repeat(90),
        "tail source\n\n".repeat(60)
    );
    let (window, view) = super::super::tests::open(cx, text);
    component_frame(cx, window);
    component_event(
        &view,
        cx,
        r#"
        local m=require('mmv');local u=m.ui;doc=m.document.current();links=doc:links();src=u.state(doc:source());q=u.state('');s=u.state();notified=0
        w=u.float.open{width=580,height=300,focus='q',render=function()
            local query=q:get();local items={};local selected
            for _,item in ipairs(links)do if m.match.fuzzy(query,item.display_name)then table.insert(items,item)end;if item.id==s:get()then selected=item end end
            local right=selected and u.source{key='src',source=src:get(),highlight=selected.source_range,height=220,width=270} or u.text{text='No source'}
            return u.column{children={u.row{children={
                u.list{key='l',width=270,height=220,items=items,selection=s,reset_on=query,render_item=function(item,active)return u.text{text=item.display_name..(active and '*' or '')}end,on_selection_changed=function(id)notified=notified+1;for _,item in ipairs(links)do if item.id==id then w:source('src'):scroll_to(item.source_range,{align='center'})end end end,on_confirm=function(id)confirmed=id end},right}},
                u.input{key='q',height=40,bind=q,on_submit=function()w:list('l'):confirm()end,keys={['ctrl-n']=function()w:list('l'):select_next()end,['ctrl-d']=function()w:source('src'):scroll{pages=.5}end}}}}
        end};handle=w:source('src');old_list=w:list('l')
        assert(not pcall(w.source,w,'l'));assert(not pcall(handle.scroll,handle,{}));assert(not pcall(handle.scroll,handle,{pages=0/0}));assert(not pcall(handle.scroll_to,handle,{start=0,stop=1}));assert(not pcall(handle.scroll_to,handle,links[1].source_range,{align='invalid'}))
        assert(not pcall(u.source,{key='bad',source=doc:source(),wrap=false}));assert(not pcall(u.source,{key='bad',source={}}))
    "#,
    );
    component_frame(cx, window);
    component_assert(&view, cx, "assert(notified==1 and s:get()==links[1].id)");
    cx.update_window(window, |_, w, cx| w.input("Beta", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(s:get()==links[2].id and notified==2)");
    let preview = view.read_with(cx, |v, _| {
        v.floating_ui.sources.values().next().unwrap().clone()
    });
    let centered = preview.read_with(cx, |p, _| p.scroll);
    assert!(centered > 500.);
    cx.update_window(window, |_, w, cx| w.press("ctrl-d", cx))
        .unwrap();
    component_frame(cx, window);
    let manual = preview.read_with(cx, |p, _| p.scroll);
    assert!(manual > centered);
    component_event(&view, cx, "q:set('Beta')");
    component_frame(cx, window);
    assert_eq!(preview.read_with(cx, |p, _| p.scroll), manual);
    component_assert(&view, cx, "assert(notified==2)");
    let body_before = view.read_with(cx, |v, cx| v.document.distance(cx));
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "lua-component-src",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-80.))),
            cx,
        )
    })
    .unwrap();
    component_frame(cx, window);
    view.read_with(cx, |v, cx| assert_eq!(v.document.distance(cx), body_before));
    cx.update_window(window, |_, w, cx| {
        assert!(
            view.read(cx)
                .floating_ui
                .inputs
                .values()
                .next()
                .unwrap()
                .state
                .focus_handle(cx)
                .is_focused(w)
        )
    })
    .unwrap();
    cx.update_window(window, |_, w, cx| {
        w.press("tab", cx);
        w.press("down", cx);
        w.press("ctrl-p", cx);
        w.press("enter", cx);
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==links[2].id and notified==2)");
    cx.update_window(window, |_, w, cx| {
        w.press("tab", cx);
        w.press("end", cx);
    })
    .unwrap();
    component_frame(cx, window);
    assert!(preview.read_with(cx, |p, _| p.scroll) > manual);
    cx.update_window(window, |_, w, cx| w.press("tab", cx))
        .unwrap();
    component_frame(cx, window);
    component_event(&view, cx, "q:set('no matches')");
    component_frame(cx, window);
    assert!(view.read_with(cx, |v, _| v.floating_ui.sources.is_empty()));
    component_assert(
        &view,
        cx,
        "assert(s:get()==nil and not handle:scroll({pages=.5}) and not old_list:confirm())",
    );
    component_event(&view, cx, "q:set('')");
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(not handle:scroll({pages=.5}));assert(w:source('src'):scroll{pages=0});assert(w:source('src'):scroll_to(links[1].source_range))",
    );
    // A real candidate click confirms the displayed ID, not its former row number.
    let mut visual = VisualTestContext::from_window(window, cx);
    let list = visual.debug_bounds("lua-component-l").unwrap();
    visual.simulate_click(
        list.origin + gpui_kit::point(px(10.), px(10.)),
        Default::default(),
    );
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==links[1].id)");
}

#[gpui_kit::test]
fn list_native_navigation_scroll_click_and_recreation_keep_display_identity(
    cx: &mut TestAppContext,
) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;s=u.state();fail=u.state(false);hide=u.state(false);confirmed=0
        m.keymap.set('x',function()w=u.float.open{width=220,height=100,focus='l',render=function()
            if hide:get()then return u.text{text='hidden'}end
            local items={};for id=1,50 do items[id]={id=id}end
            return u.list{key='l',height=100,width=220,selection=s,items=items,render_item=function(item)if fail:get()then error('row failure')end;return u.text{key='r'..item.id,height=32,text=tostring(item.id)}end,on_confirm=function(id)confirmed=confirmed+1;last=id end}
        end,on_error=function()end};h=w:list('l')end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    component_event(&view, cx, "assert(h:select(50))");
    component_frame(cx, window);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let list = visual.debug_bounds("lua-component-l").unwrap();
    let row = visual.debug_bounds("lua-component-r50").unwrap();
    assert!(row.top() >= list.top() && row.bottom() <= list.bottom());
    cx.update_window(window, |_, w, cx| w.press("down", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(s:get()==1)");
    cx.update_window(window, |_, w, cx| {
        w.press("up", cx);
        w.press("enter", cx);
    })
    .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(s:get()==50 and confirmed==1 and last==50)",
    );
    let row = visual.debug_bounds("lua-component-r50").unwrap();
    visual.simulate_click(row.center(), Default::default());
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==2 and last==50)");
    component_event(&view, cx, "fail:set(true)");
    component_frame(cx, window);
    visual.simulate_click(row.center(), Default::default());
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==2 and not h:confirm())");
    component_event(&view, cx, "fail:set(false);hide:set(true)");
    component_frame(cx, window);
    component_event(&view, cx, "hide:set(false)");
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(not h:select('invalid') and not h:confirm());assert(w:list('l'):confirm())",
    );
}

#[gpui_kit::test]
fn input_on_change_sets_and_errors_complete_before_selection_adoption(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local m=require('mmv');local u=m.ui;q=u.state('');s=u.state();log='';confirmed=0
        m.keymap.set('x',function()w=u.float.open{width=300,height=200,focus='q',render=function()
            return u.column{children={u.list{key='l',height=100,selection=s,items={{id=q:get()}},render_item=function()if q:get()=='broken'then error('bad row')end;return u.text{text=q:get()}end,on_selection_changed=function(id)assert(id==q:get());log=log..'S' end,on_confirm=function()confirmed=confirmed+1 end},
                u.input{key='q',height=40,bind=q,on_change=function(text)log=log..'C';if text=='error'then q:set('final');error('on_change failure')end end,on_submit=function()w:list('l'):confirm()end}}}
        end,on_error=function(e)last_error=e;log=log..'E'end}end)
    "#,
    );
    open_registered(&view, cx);
    component_frame(cx, window);
    component_assert(&view, cx, "log=''");
    cx.update_window(window, |_, w, cx| w.input("error", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='final' and s:get()=='final' and last_error.phase=='event' and last_error.key=='q');assert(log:sub(1,1)=='C' and log:find('S'))",
    );
    component_event(&view, cx, "q:set('')");
    component_frame(cx, window);
    cx.update_window(window, |_, w, cx| w.input("broken", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(
        &view,
        cx,
        "assert(q:get()=='broken' and last_error.phase=='render' and last_error.key=='l');assert(not w:list('l'):confirm())",
    );
    cx.update_window(window, |_, w, cx| w.press("enter", cx))
        .unwrap();
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==0)");
    component_event(&view, cx, "q:set('fixed')");
    component_frame(cx, window);
    component_assert(&view, cx, "assert(confirmed==0 and s:get()=='fixed')");
}

#[gpui_kit::test]
fn source_snapshot_change_resets_position_and_rejects_ranges_from_identical_old_bytes(
    cx: &mut TestAppContext,
) {
    let text = format!("[Go](next.md)\n\n{}", "source 日本語 line\n".repeat(120));
    let (window, view) = super::super::tests::open(cx, text.clone());
    component_frame(cx, window);
    component_event(
        &view,
        cx,
        r#"
        local m=require('mmv');local u=m.ui;old=m.document.current();range=old:links()[1].source_range;src=u.state(old:source())
        w=u.float.open{width=300,height=180,focus='src',render=function()return u.source{key='src',width=300,height=180,source=src:get()}end};handle=w:source('src');handle:scroll{pages=2}
    "#,
    );
    component_frame(cx, window);
    let previous = view.read_with(cx, |v, _| {
        v.floating_ui.sources.values().next().unwrap().clone()
    });
    assert!(previous.read_with(cx, |p, _| p.scroll) > 0.);
    view.update(cx, |v, _| {
        v.lua.context().documents.borrow_mut().current =
            Some(crate::lua::document::DocumentHandle::new(
                "C:/fixtures/same.md".into(),
                std::sync::Arc::new(text),
                |_, _| Ok(Vec::new()),
            ));
    });
    component_event(
        &view,
        cx,
        "src:set(require('mmv').document.current():source())",
    );
    component_frame(cx, window);
    let current = view.read_with(cx, |v, _| {
        v.floating_ui.sources.values().next().unwrap().clone()
    });
    assert_ne!(current, previous);
    assert_eq!(current.read_with(cx, |p, _| p.scroll), 0.);
    component_assert(
        &view,
        cx,
        "assert(not pcall(handle.scroll_to,handle,range));assert(handle:scroll{pages=0});assert(not pcall(require('mmv').ui.source,{key='wrong',source=src:get(),highlight=range}))",
    );
    component_event(
        &view,
        cx,
        "w:close();assert(not handle:scroll_to({}));w=nil",
    );
    component_frame(cx, window);
    assert!(view.read_with(cx, |v, _| v.floating_ui.sources.is_empty()));
}

#[gpui_kit::test]
fn review_focus_correction_uses_new_display_order(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui; stage=u.state(0); result=''
        local function b(k,enabled)return u.button{key=k,label=k,enabled=enabled,on_click=function()result=result..k end}end
        require('mmv').keymap.set('x',function()w=u.float.open{width=200,height=200,focus='b',render=function()
            local s=stage:get()
            if s==0 then return u.column{children={b('a'),b('b'),b('c')}} end
            if s==1 then return u.column{children={b('a'),b('b',false),b('new'),b('c')}} end
            if s==2 then return u.column{children={b('a'),b('b',false),b('c')}} end
            if s==3 then return u.column{children={b('a')}} end
            return u.text{text='no targets'}
        end}end)
    "#,
    );
    open_registered(&view, cx);
    let original = view.read_with(cx, |v, _| {
        v.lua.context().ui.borrow().windows[0].components.clone()
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    for (stage, expected) in [(1, "new"), (2, "newc"), (3, "newca"), (4, "newca")] {
        visual.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.lua
                    .lua()
                    .load(format!("stage:set({stage})"))
                    .exec()
                    .unwrap();
                cx.notify();
            })
        });
        if stage == 1 {
            view.read_with(&visual, |v, _| {
                let context = v.lua.context();
                let ui = context.ui.borrow();
                assert_eq!(
                    ui.windows[0].components["a"].identity,
                    original["a"].identity
                );
                assert_eq!(
                    ui.windows[0].components["c"].identity,
                    original["c"].identity
                );
            });
        }
        visual.update(|window, cx| window.press("enter", cx));
        view.read_with(&visual, |v, _| {
            assert_eq!(
                v.lua.lua().globals().get::<String>("result").unwrap(),
                expected
            )
        });
    }
}

fn open_fixture(
    cx: &mut TestAppContext,
    script: &str,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<super::super::Viewer>,
) {
    let (runtime, error) =
        crate::lua::LuaRuntime::initialize("", Some((script, "@floating-ui-fixture.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    let result = cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::input::bind_keys(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                super::super::Viewer::with_lua(
                    Default::default(),
                    std::time::Instant::now(),
                    runtime,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    result
}

fn open_registered(view: &gpui_kit::Entity<super::super::Viewer>, cx: &mut TestAppContext) {
    view.update(cx, |v, cx| {
        assert!(v.lua.enqueue_key("x").unwrap());
        v.pump_lua(cx);
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn lua_float_renders_clicks_keys_and_restores_real_focus(cx: &mut TestAppContext) {
    let (runtime,error)=crate::lua::LuaRuntime::initialize("",Some((r#"
        local u=require('mmv').ui; local n=u.state(0); fail=u.state(false); errors=0
        require('mmv').keymap.set('x',function()
            win=u.float.open{width={ratio=.8},height={ratio=.8},focus='ok',render=function()
                if fail:get() then error('render failure') end
                return u.column{key='root',width={ratio=1},height={ratio=1},padding=8,gap=4,children={u.text{key='label',text='Count '..n:get()},u.button{key='ok',label='Go',width=80,height=30,on_click=function()n:set(n:get()+1); result=n:get()end}}}
            end,on_close=function(e) reason=e.reason end,on_error=function(e) errors=errors+1; assert(e.phase=='render') end}
        end)
    "#,"@gpui-fixture.lua"))).unwrap();
    assert!(error.is_none());
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::input::bind_keys(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                super::super::Viewer::with_lua(
                    Default::default(),
                    std::time::Instant::now(),
                    runtime,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    view.update(cx, |v, cx| {
        v.lua.enqueue_key("x").unwrap();
        v.pump_lua(cx);
        cx.notify();
    });
    cx.run_until_parked();
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let frame = visual.debug_bounds("lua-window-0").unwrap();
    let root = visual.debug_bounds("lua-component-root").unwrap();
    assert!(root.size.width <= frame.size.width);
    let button = visual.debug_bounds("lua-component-ok").unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.update(|window, cx| window.press("enter", cx));
    view.read_with(&visual, |v, _| {
        assert_eq!(v.lua.lua().globals().get::<i64>("result").unwrap(), 2)
    });
    let label = visual.debug_bounds("lua-component-label").unwrap();
    visual.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.lua.lua().load("fail:set(true)").exec().unwrap();
            cx.notify();
        })
    });
    assert_eq!(visual.debug_bounds("lua-component-label").unwrap(), label);
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(errors==1 and win:is_open())")
            .exec()
            .unwrap();
        assert_eq!(
            v.lua.context().ui.borrow().windows[0].root.children[0].text,
            "Count 2"
        );
    });
    visual.update(|window, cx| window.press("escape", cx));
    view.read_with(&visual, |v, _| {
        assert_eq!(
            v.lua.lua().globals().get::<String>("reason").unwrap(),
            "escape"
        )
    });
    visual.update(|window, cx| view.read_with(cx, |v, _| assert!(v.body_focus.is_focused(window))));
}

#[gpui_kit::test]
fn nonmodal_keeps_background_keys_click_focus_and_z_order(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui
        require('mmv').keymap.set('x',function()
            function panel(key,size,focus)return u.float.open{width=size,height=size,modal=false,focus=focus,dismiss={outside_click=false},render=function()return u.button{key=key,label=key,width=50,height=30,on_click=function()clicked=key end}end}end
            low=panel('low',400,nil); high=panel('high',200,nil)
        end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.update(|window, cx| view.read_with(cx, |v, _| assert!(v.body_focus.is_focused(window))));
    visual.update(|window, cx| window.press("/", cx));
    visual.simulate_input("日本語query");
    view.read_with(&visual, |v, cx| {
        assert_eq!(v.app.mode, crate::app::Mode::Search);
        assert_eq!(v.input.state.read(cx).value().as_str(), "日本語query");
    });
    visual.update(|window, cx| window.press("escape", cx));
    let lower = visual.debug_bounds("lua-component-low").unwrap();
    let before = visual.debug_bounds("lua-window-1").unwrap();
    visual.simulate_click(lower.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(
            v.lua.lua().globals().get::<String>("clicked").unwrap(),
            "low"
        )
    });
    assert_eq!(visual.debug_bounds("lua-window-1").unwrap(), before);
    let higher = visual.debug_bounds("lua-component-high").unwrap();
    visual.simulate_click(higher.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(
            v.lua.lua().globals().get::<String>("clicked").unwrap(),
            "high"
        )
    });
    // Programmatic focus has no promotion side effect either.
    visual.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.lua.lua().load("assert(low:focus('low'))").exec().unwrap();
            cx.notify();
        })
    });
    assert_eq!(visual.debug_bounds("lua-window-1").unwrap(), before);
    visual.simulate_click(higher.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(
            v.lua.lua().globals().get::<String>("clicked").unwrap(),
            "high"
        )
    });
    visual.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.lua.lua().load("assert(low:focus('low'))").exec().unwrap();
            cx.notify();
        })
    });
    visual.update(|window, cx| window.press("escape", cx));
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(low:is_open() and not high:is_open())")
            .exec()
            .unwrap()
    });
}

#[gpui_kit::test]
fn modal_shields_lower_windows_allows_above_and_dismissal_consumes_one_click(
    cx: &mut TestAppContext,
) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui; clicks=0; reasons={}
        require('mmv').keymap.set('x',function()
            function panel(key,size,modal)return u.float.open{width=size,height=size,modal=modal,focus=key,dismiss={outside_click=false},render=function()return u.button{key=key,label=key,width=60,height=30,on_click=function()clicks=clicks+1;last=key end}end,on_close=function(e)table.insert(reasons,e.reason)end}end
            low=panel('low',500,false); middle=panel('middle',400,true); modal=panel('modal',300,true); high=panel('high',100,false)
            assert(not low:focus('low'))
        end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let low = visual.debug_bounds("lua-component-low").unwrap();
    visual.simulate_click(low.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(v.lua.lua().globals().get::<i64>("clicks").unwrap(), 0)
    });
    let high = visual.debug_bounds("lua-component-high").unwrap();
    visual.simulate_click(high.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(v.lua.lua().globals().get::<String>("last").unwrap(), "high")
    });
    visual.update(|window, cx| window.press("escape", cx));
    view.read_with(&visual,|v,_|v.lua.lua().load("assert(not high:is_open() and modal:is_open() and low:is_open()); assert(#reasons==1)").exec().unwrap());
    visual.update(|window, cx| window.press("escape", cx));
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(not modal:is_open() and low:is_open()); assert(#reasons==2)")
            .exec()
            .unwrap()
    });
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(middle:is_open() and not low:focus('low'))")
            .exec()
            .unwrap()
    });
    visual.update(|window, cx| window.press("escape", cx));
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(not middle:is_open() and low:is_open() and #reasons==3)")
            .exec()
            .unwrap()
    });
    // New default-dismiss window consumes an outside click, even if a lower
    // button occupies that coordinate. The next separate click may activate it.
    visual.update(|_,cx|view.update(cx,|v,cx| {v.lua.lua().load("dismiss=require('mmv').ui.float.open{width=100,height=100,render=function()return require('mmv').ui.text{text='dismiss'}end}").exec().unwrap();cx.notify();}));
    visual.simulate_click(low.center(), Default::default());
    view.read_with(&visual, |v, _| {
        v.lua
            .lua()
            .load("assert(not dismiss:is_open() and clicks==1)")
            .exec()
            .unwrap()
    });
    visual.simulate_click(low.center(), Default::default());
    view.read_with(&visual, |v, _| {
        assert_eq!(v.lua.lua().globals().get::<i64>("clicks").unwrap(), 2)
    });
}

#[gpui_kit::test]
fn rerender_preserves_real_button_focus_and_disabled_focus_moves_forward(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui; enabled=u.state(true); result=''; renders=0
        require('mmv').keymap.set('x',function()
            w=u.float.open{width=300,height=200,focus='b',render=function()
                renders=renders+1
                local function b(k)return u.button{key=k,label=k,height=30,enabled=k~='b' or enabled:get(),on_click=function() result=result..k end}end
                return u.column{children={b('a'),b('b'),b('c')}}
            end}
        end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.lua.lua().load("enabled:set(true)").exec().unwrap();
            cx.notify();
        })
    });
    visual.update(|window, cx| window.press("space", cx));
    view.read_with(&visual, |v, _| {
        assert_eq!(v.lua.lua().globals().get::<String>("result").unwrap(), "b")
    });
    visual.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.lua.lua().load("enabled:set(false)").exec().unwrap();
            cx.notify();
        })
    });
    visual.update(|window, cx| window.press("enter", cx));
    visual.update(|window, cx| window.press("tab", cx));
    visual.update(|window, cx| window.press("space", cx));
    visual.update(|window, cx| window.press("shift-tab", cx));
    visual.update(|window, cx| window.press("enter", cx));
    view.read_with(&visual, |v, _| {
        assert_eq!(
            v.lua.lua().globals().get::<String>("result").unwrap(),
            "bcac"
        )
    });
}

#[gpui_kit::test]
fn viewer_window_close_closes_float_owners_with_shutdown_once(cx: &mut TestAppContext) {
    let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let captured = observed.clone();
    let (runtime,error)=crate::lua::LuaRuntime::initialize_with("",Some((r#"
        local u=require('mmv').ui
        require('mmv').keymap.set('x',function()w=u.float.open{width=100,height=100,render=function()return u.text{text='shutdown'}end,on_close=function(e)record(e.reason)end}end)
    "#,"@shutdown-fixture.lua")),move |lua,_,_| {
        let captured=captured.clone();
        lua.globals().set("record",lua.create_function(move |_,reason:String|{captured.borrow_mut().push(reason);Ok(())})?)
    }).unwrap();
    assert!(error.is_none());
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                super::super::Viewer::with_lua(
                    Default::default(),
                    std::time::Instant::now(),
                    runtime,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    open_registered(&view, cx);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    assert_eq!(&*observed.borrow(), &["shutdown"]);
}

#[gpui_kit::test]
fn actual_focus_outside_a_closing_nonmodal_is_not_stolen(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui
        require('mmv').keymap.set('x',function()
            function panel(k)return u.float.open{width=100,height=100,modal=false,focus=k,render=function()return u.button{key=k,label=k,on_click=function()end}end}end
            a=panel('a');b=panel('b')
        end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.body_focus.focus(window, cx);
            v.lua.lua().load("assert(b:close())").exec().unwrap();
            cx.notify();
        })
    });
    visual.update(|window, cx| view.read_with(cx, |v, _| assert!(v.body_focus.is_focused(window))));
}

#[gpui_kit::test]
fn actual_focus_cannot_remain_behind_a_modal(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui
        require('mmv').keymap.set('x',function()w=u.float.open{width=100,height=100,focus='a',render=function()return u.button{key='a',label='a',on_click=function()end}end}end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.body_focus.focus(window, cx);
            cx.notify();
        })
    });
    visual
        .update(|window, cx| view.read_with(cx, |v, _| assert!(!v.body_focus.is_focused(window))));
}

#[gpui_kit::test]
fn dimensions_use_viewport_and_parent_content_box_and_grow_weights(cx: &mut TestAppContext) {
    let (window, view) = open_fixture(
        cx,
        r#"
        local u=require('mmv').ui
        require('mmv').keymap.set('x',function()
            w=u.float.open{width=100000,height={ratio=.8},render=function()
                return u.column{key='parent',width={ratio=1},height={ratio=1},padding=10,children={
                    u.row{key='row',width={ratio=1},height=100,gap=10,children={u.text{key='one',text='one',grow=1},u.text{key='three',text='three',grow=3}}},
                    u.text{key='half',text='日本語 wrapped text',width={ratio=.5},wrap=true,hl='Link',highlights={{range={start=0,stop=3},bold=true}}}
                }}
            end}
        end)
    "#,
    );
    open_registered(&view, cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let frame = visual.debug_bounds("lua-window-0").unwrap();
    let row = visual.debug_bounds("lua-component-row").unwrap();
    let half = visual.debug_bounds("lua-component-half").unwrap();
    let one = visual.debug_bounds("lua-component-one").unwrap();
    let three = visual.debug_bounds("lua-component-three").unwrap();
    visual.update(|window, _| {
        assert_eq!(frame.size.width, window.viewport_size().width);
        assert_eq!(frame.size.height, window.viewport_size().height * 0.8);
    });
    assert_eq!(row.origin.x, frame.origin.x + px(10.));
    assert!((f32::from(half.size.width) - (f32::from(frame.size.width) - 20.) * 0.5).abs() < 1.);
    assert!((f32::from(three.size.width) - f32::from(one.size.width) * 3.).abs() < 1.);
    // Paint data's composed runs preserve explicit false and later-only attrs.
    visual.update(|_,cx|view.read_with(cx,|v,_| {
        let lua=v.lua.lua(); let u=v.lua.context().api(lua).unwrap().get::<mlua::Table>("ui").unwrap();
        let desc=u.get::<mlua::Function>("text").unwrap().call::<mlua::AnyUserData>(lua.load("return {text='abcd',highlights={{range={start=0,stop=4},bold=true,underline=true,hl='Link'},{range={start=1,stop=3},bold=false},{range={start=2,stop=4},underline=false}}}").eval::<mlua::Table>().unwrap()).unwrap();
        let desc=desc.borrow::<crate::lua::ui::tree::Description>().unwrap();
        let runs=super::text_highlights(&desc.0,&v.palette);
        assert_eq!(runs.len(),4);
        assert_eq!(runs[1].1.font_weight,Some(gpui_kit::FontWeight::NORMAL));
        assert_eq!(runs[1].1.underline.unwrap().thickness,px(1.));
        assert_eq!(runs[2].1.underline.unwrap().thickness,px(0.));
        assert_eq!(runs[2].1.color,Some(v.palette.get(crate::config::Group::Link).fg));
    }));
}
