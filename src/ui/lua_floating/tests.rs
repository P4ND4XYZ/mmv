use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, px, test::TestWindowExt as _};

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
