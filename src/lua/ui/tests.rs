use crate::lua::{LuaRuntime, Pump};

#[test]
fn review_description_cycles_collect_after_close_and_failed_open() {
    let runtime = runtime("ui=require('mmv').ui; weak=setmetatable({},{__mode='v'})");
    runtime.lua().load(r#"
        do
            local body=ui.button{key='control',label='control',on_click=function()end}
            weak[1]=body
            local w=ui.float.open{width=100,height=100,render=function()return body end}
            assert(w:close())
        end
        for i,mode in ipairs({'close','extra','focus'}) do
            local c={payload={}}
            c.body=ui.column{children={ui.button{key='close',label='Close',on_click=function() assert(c.payload); if c.win then c.win:close() end end}}}
            local base=2+(i-1)*3
            weak[base]=c;weak[base+1]=c.body;weak[base+2]=c.payload
            if mode=='extra' then
                assert(not pcall(ui.float.open,{width=100,height=100,render=function()return c.body,nil end}))
            elseif mode=='focus' then
                assert(not pcall(ui.float.open,{width=100,height=100,focus='missing',render=function()return c.body end}))
            else
                c.win=ui.float.open{width=100,height=100,render=function()return c.body end}
                assert(c.win:close())
            end
        end
    "#).exec().unwrap();
    drain(&runtime);
    runtime.lua().gc_collect().unwrap();
    runtime.lua().gc_collect().unwrap();
    runtime
        .lua()
        .load(
            "for i=1,10 do assert(weak[i]==nil,'retained description/controller/payload '..i) end",
        )
        .exec()
        .unwrap();
}

#[test]
fn review_referenced_description_reuses_callbacks_and_then_collects() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; calls=0; weak=setmetatable({},{__mode='v'})
        do
            local controller={payload={}}
            controller.body=ui.column{children={ui.button{key='run',label='Run',on_click=function() calls=calls+1; assert(controller.payload) end}}}
            cached=controller
        end
        weak[1]=cached.body
    "#,
    );
    for n in 1..=3 {
        runtime.lua().load("w=ui.float.open{width=100,height=100,focus='run',render=function()return cached.body end}").exec().unwrap();
        drain(&runtime);
        runtime.lua().gc_collect().unwrap();
        runtime.lua().gc_collect().unwrap();
        assert!(runtime.context().ui_key(runtime.lua(), "enter").unwrap());
        drain(&runtime);
        assert_eq!(runtime.lua().globals().get::<i64>("calls").unwrap(), n);
        runtime
            .lua()
            .load("assert(w:close());w=nil;assert(cached.body==weak[1])")
            .exec()
            .unwrap();
        drain(&runtime);
    }
    runtime.lua().load("cached=nil").exec().unwrap();
    drain(&runtime);
    runtime.lua().gc_collect().unwrap();
    runtime.lua().gc_collect().unwrap();
    runtime.lua().load("assert(weak[1]==nil)").exec().unwrap();
}

#[test]
fn review_active_owner_keeps_description_callbacks_alive_without_external_controller() {
    let runtime = runtime("ui=require('mmv').ui; weak=setmetatable({},{__mode='v'});calls=0");
    runtime.lua().load(r#"
        do
            local controller={payload={}}
            controller.body=ui.button{key='run',label='Run',on_click=function()assert(controller.payload);calls=calls+1 end}
            weak[1]=controller;weak[2]=controller.body;weak[3]=controller.payload
            live=ui.float.open{width=100,height=100,focus='run',render=function()return controller.body end}
        end
    "#).exec().unwrap();
    drain(&runtime);
    runtime.lua().gc_collect().unwrap();
    runtime.lua().gc_collect().unwrap();
    runtime
        .lua()
        .load("assert(weak[1]~=nil and weak[2]~=nil and weak[3]~=nil)")
        .exec()
        .unwrap();
    assert!(runtime.context().ui_key(runtime.lua(), "enter").unwrap());
    drain(&runtime);
    runtime
        .lua()
        .load("assert(calls==1);assert(live:close());live=nil")
        .exec()
        .unwrap();
    drain(&runtime);
    runtime.lua().gc_collect().unwrap();
    runtime.lua().gc_collect().unwrap();
    runtime
        .lua()
        .load("assert(weak[1]==nil and weak[2]==nil and weak[3]==nil)")
        .exec()
        .unwrap();
}

fn runtime(source: &str) -> LuaRuntime {
    let (runtime, error) = LuaRuntime::initialize("", Some((source, "@ui-fixture.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    runtime
}
fn drain(runtime: &LuaRuntime) {
    for _ in 0..100 {
        match runtime.pump_one() {
            Pump::Idle => return,
            Pump::Dispatched => (),
            Pump::HostPending => panic!("unexpected host output"),
            Pump::Busy => panic!("unexpected reentry"),
        }
    }
    panic!("UI pump did not settle");
}

#[test]
fn state_copies_aliases_and_binary_strings_without_accepting_cycles_or_userdata() {
    let runtime = runtime(
        r#"
        local u=require('mmv').ui
        local child={n=1}; local initial={a=child,b=child,[true]='yes',[2.5]='fraction'}
        local s=u.state(initial); initial.a.n=7
        local a=s:get(); assert(a.a.n==1 and a.a==a.b and a[true]=='yes')
        a.a.n=8; assert(s:get().a.n==1); s:set(a); assert(s:get().b.n==8)
        local raw=string.char(255,0,128); s:set(raw); assert(s:get()==raw)
        local cycle={}; cycle.self=cycle
        for _,bad in ipairs({cycle, setmetatable({},{}), function()end, coroutine.create(function()end), io.stdout, {[{}]=1}, math.huge, 0/0}) do
            assert(not pcall(function() s:set(bad) end)); assert(s:get()==raw)
        end
        assert(u.state():get()==nil)
    "#,
    );
    drain(&runtime);
}

#[test]
fn readonly_registration_uses_concrete_host_types_and_shares_the_handle() {
    #[derive(Clone)]
    struct Immutable;
    impl mlua::UserData for Immutable {}
    struct Other;
    impl mlua::UserData for Other {}
    let (runtime, error) = LuaRuntime::initialize_with("", None, |lua, _, ctx| {
        ctx.readonly.register::<Immutable>();
        lua.globals()
            .set("approved", lua.create_userdata(Immutable)?)?;
        lua.globals().set("other", lua.create_userdata(Other)?)
    })
    .unwrap();
    assert!(error.is_none());
    runtime.lua().load("local u=require('mmv').ui; local s=u.state({x=approved}); assert(rawequal(s:get().x,approved)); assert(not pcall(u.state,other))").exec().unwrap();
}

#[test]
fn open_is_synchronous_strict_and_render_failure_retains_the_last_good_tree() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; state=ui.state(0); renders=0; events={}
        require('mmv').keymap.set('x',function()
            win=ui.float.open{width=300,height={ratio=.5},focus='ok',render=function()
                renders=renders+1
                if state:get()==9 then error('bad render') end
                return ui.button{key='ok',label=tostring(state:get()),on_click=function() state:set(1); state:set(2); error('event failure') end}
            end,on_error=function(e) table.insert(events,e.phase) end}
            assert(win:is_open() and win:focus('ok'))
            assert(renders==1)
        end)
    "#,
    );
    assert!(runtime.enqueue_key("x").unwrap());
    drain(&runtime);
    let context = runtime.context();
    let (owner, id) = {
        let ui = context.ui.borrow();
        (ui.windows[0].owner, ui.windows[0].components["ok"].identity)
    };
    assert!(context.activate_button(runtime.lua(), owner, id).unwrap());
    drain(&runtime);
    runtime
        .lua()
        .load("assert(state:get()==2 and renders==2 and events[1]=='event'); state:set(9)")
        .exec()
        .unwrap();
    drain(&runtime);
    runtime
        .lua()
        .load("assert(events[2]=='render')")
        .exec()
        .unwrap();
    let ui = context.ui.borrow();
    assert_eq!(ui.windows[0].root.text, "2");
    assert_eq!(ui.windows[0].components["ok"].identity, id);
}

#[test]
fn malformed_options_descriptions_and_initial_focus_leave_no_partial_window() {
    let runtime = runtime(
        "ui=require('mmv').ui; assert(not pcall(ui.float.open,{})); assert(ui.text{text='init'})",
    );
    let cases = [
        "{width=1,height=1,render=function()return nil end}",
        "{width=1,height=1,render=function()return ui.text{text='x'},nil end}",
        "{width=1,height=1,render=function()return {ui.text{text='x'}} end}",
        "{width=1,height=1,modal=1,render=function()return ui.text{text='x'} end}",
        "{width=1,height=1,focus='t',render=function()return ui.text{key='t',text='x'} end}",
        "{width=1,height=1,render=function()return ui.row{children={ui.button{key='x',label='a',on_click=function()end},ui.text{key='x',text='b'}}} end}",
        "{width={ratio=0},height=1,render=function()return ui.text{text='x'} end}",
        "{width=1,height=1,unknown=true,render=function()return ui.text{text='x'} end}",
        "{width=1,height=1,keys={['CTRL-x']=function()end,['ctrl-x']=function()end},render=function()return ui.text{text='x'} end}",
    ];
    for options in cases {
        runtime
            .lua()
            .load(format!("assert(not pcall(ui.float.open,{options}))"))
            .exec()
            .unwrap();
        assert!(runtime.context().ui.borrow().windows.is_empty());
    }
    for expression in [
        "ui.row{children={[2]=ui.text{text='x'}}}",
        "ui.text{text=1}",
        "ui.text{text='é',highlights={{range={start=1,stop=2}}}}",
        "ui.text{text='x',wrap=1}",
        "ui.button{key='x',label='x',on_click=setmetatable({},{__call=function()end})}",
        "ui.column{children={},gap=-1}",
        "ui.text{text='x',width='bad'}",
        "ui.text{text=string.char(255)}",
    ] {
        runtime
            .lua()
            .load(format!(
                "assert(not pcall(function() return {expression} end))"
            ))
            .exec()
            .unwrap();
    }
    drain(&runtime);
}

#[test]
fn modal_focus_lifetime_tab_and_close_notifications_follow_owner_rules() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; count=ui.state(0); closed={}
        function button(k,enabled) return ui.button{key=k,label=k,enabled=enabled,on_click=function() count:set(count:get()+1) end} end
        function open(modal,focus) return ui.float.open{width=100,height=100,modal=modal,focus=focus,render=function() return ui.column{children={button('a',count:get()<1),button('b',true),ui.text{key='text',text='x'}}} end,on_close=function(e)table.insert(closed,e.reason)end}end
        require('mmv').keymap.set('x',function() low=open(false,'a'); mid=open(true,nil); top=open(false,'b'); assert(not low:focus('b')); assert(not pcall(function()low:focus('text')end)) end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    drain(&runtime);
    let context = runtime.context();
    let owners = context
        .ui
        .borrow()
        .windows
        .iter()
        .map(|w| w.owner)
        .collect::<Vec<_>>();
    assert!(context.ui_key(runtime.lua(), "enter").unwrap());
    drain(&runtime);
    runtime.lua().load("assert(count:get()==1); assert(top:close()); assert(not top:focus({})); assert(not top:close()); assert(mid:close()); assert(low:focus('b'))").exec().unwrap();
    drain(&runtime);
    assert_eq!(context.ui.borrow().windows.len(), 1);
    assert_eq!(context.ui.borrow().windows[0].owner, owners[0]);
    assert!(context.ui_key(runtime.lua(), "tab").unwrap());
    assert!(context.ui_key(runtime.lua(), "escape").unwrap());
    drain(&runtime);
    runtime
        .lua()
        .load("assert(#closed==3 and closed[1]=='programmatic' and closed[3]=='escape')")
        .exec()
        .unwrap();
    assert!(context.ui.borrow().windows.is_empty());
}

#[test]
fn state_binding_types_are_atomic_unique_and_released_by_component_lifetime() {
    use crate::lua::state::{BindingType, StateHandle};
    let runtime = runtime("ui=require('mmv').ui; s=ui.state('start'); selection=ui.state(nil)");
    runtime.lua().load("w=ui.float.open{width=50,height=50,render=function()return ui.column{children={ui.button{key='a',label='a',on_click=function()end},ui.button{key='b',label='b',on_click=function()end}}}end}").exec().unwrap();
    let context = runtime.context();
    let (owner, a, b) = {
        let ui = context.ui.borrow();
        let w = &ui.windows[0];
        (
            w.owner,
            w.components["a"].identity,
            w.components["b"].identity,
        )
    };
    let s = runtime
        .lua()
        .globals()
        .get::<mlua::AnyUserData>("s")
        .unwrap()
        .borrow::<StateHandle>()
        .unwrap()
        .clone();
    s.bind(runtime.lua(), owner, a, BindingType::String)
        .unwrap();
    assert!(
        s.bind(runtime.lua(), owner, b, BindingType::String)
            .is_err()
    );
    assert!(
        StateHandle::check_tree_bindings(
            runtime.lua(),
            owner,
            &[
                (s.clone(), a, BindingType::String),
                (s.clone(), b, BindingType::String)
            ]
        )
        .is_err()
    );
    StateHandle::check_tree_bindings(runtime.lua(), owner, &[(s.clone(), b, BindingType::String)])
        .unwrap();
    s.unbind(owner, a);
    s.bind(runtime.lua(), owner, b, BindingType::String)
        .unwrap();
    assert!(s.set(runtime.lua(), mlua::Value::Integer(1)).is_err());
    assert_eq!(
        s.get(runtime.lua())
            .unwrap()
            .as_string()
            .unwrap()
            .to_str()
            .unwrap(),
        "start"
    );
    let selection = runtime
        .lua()
        .globals()
        .get::<mlua::AnyUserData>("selection")
        .unwrap()
        .borrow::<StateHandle>()
        .unwrap()
        .clone();
    selection
        .bind(runtime.lua(), owner, b, BindingType::SelectionId)
        .unwrap();
    selection
        .set(runtime.lua(), mlua::Value::Integer(7))
        .unwrap();
    assert!(
        selection
            .set(runtime.lua(), mlua::Value::Number(7.5))
            .is_err()
    );
    runtime
        .lua()
        .load("assert(w:close()); s:set(false); selection:set({})")
        .exec()
        .unwrap();
    drain(&runtime);
}

#[test]
fn rendering_rejects_mutation_and_same_value_sets_coalesce_but_keep_component_identity() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; s=ui.state(0); enabled=ui.state(true); renders=0
        function b(key) return ui.button{key=key,label=key,enabled=key~='b' or enabled:get(),on_click=function()end} end
        require('mmv').keymap.set('x',function()
            w=ui.float.open{width=100,height=100,focus='b',render=function()
                renders=renders+1
                assert(not pcall(function()s:set(3)end))
                assert(not pcall(function()ui.state(0)end))
                assert(not pcall(function()ui.float.open{}end))
                if w then assert(not pcall(function()w:focus('a')end)); assert(not pcall(function()w:close()end)) end
                return ui.column{children={b('a'),b('b'),b('c')}}
            end}
        end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    drain(&runtime);
    let context = runtime.context();
    let (owner, ids) = {
        let ui = context.ui.borrow();
        let w = &ui.windows[0];
        (w.owner, w.focus_order.clone())
    };
    runtime.lua().load("s:set(0); s:set(0)").exec().unwrap();
    drain(&runtime);
    runtime
        .lua()
        .load("assert(renders==2); enabled:set(false)")
        .exec()
        .unwrap();
    drain(&runtime);
    assert_eq!(
        context.ui.borrow().focus,
        Some(super::FocusTarget {
            owner,
            identity: Some(ids[2])
        })
    );
    assert_eq!(
        context.ui.borrow().windows[0].components["b"].identity,
        ids[1]
    );
    assert!(context.ui_key(runtime.lua(), "tab").unwrap());
    assert_eq!(context.ui.borrow().focus.unwrap().identity, Some(ids[0]));
    assert!(context.ui_key(runtime.lua(), "shift-tab").unwrap());
    assert_eq!(context.ui.borrow().focus.unwrap().identity, Some(ids[2]));
}

#[test]
fn type_changes_destroy_identity_and_queued_events_do_not_survive_owner_close() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; phase=ui.state(0); clicked=0; closed=0
        w= nil
        require('mmv').keymap.set('x',function() w=ui.float.open{width=100,height=100,focus='a',render=function()
            if phase:get()==1 then return ui.text{key='a',text='not focusable'} end
            return ui.button{key='a',label='a',on_click=function()clicked=clicked+1 end}
        end,on_close=function(e)closed=closed+1; assert(e.reason=='shutdown'); assert(not w:is_open())end} end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    drain(&runtime);
    let context = runtime.context();
    let owner = context.ui.borrow().windows[0].owner;
    let original = context.ui.borrow().windows[0].components["a"].identity;
    runtime.lua().load("phase:set(1)").exec().unwrap();
    drain(&runtime);
    assert_ne!(
        context.ui.borrow().windows[0].components["a"].identity,
        original
    );
    assert_eq!(context.ui.borrow().focus.unwrap().identity, None);
    runtime.lua().load("phase:set(2)").exec().unwrap();
    drain(&runtime);
    let new = context.ui.borrow().windows[0].components["a"].identity;
    assert_ne!(new, original);
    assert!(
        !context
            .activate_button(runtime.lua(), owner, original)
            .unwrap()
    );
    assert!(context.activate_button(runtime.lua(), owner, new).unwrap());
    context.shutdown_ui().unwrap();
    drain(&runtime);
    runtime
        .lua()
        .load("assert(clicked==0 and closed==1); assert(not pcall(ui.float.open,{}))")
        .exec()
        .unwrap();
}

#[test]
fn construction_failure_is_only_a_caller_error_and_render_validation_reports_its_component() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; bad=ui.state(false); errors={}; closed=0
        require('mmv').keymap.set('x',function()
            assert(not pcall(ui.float.open,{width=20,height=20,render=function()error('initial failure')end,on_error=function()error('must not be called')end,on_close=function()closed=closed+1 end}))
            w=ui.float.open{width=20,height=20,render=function()return ui.text{key='label',text='é',highlights=bad:get() and {{range={start=1,stop=2}}} or {}}end,on_error=function(e)table.insert(errors,e)end}
            assert(not pcall(ui.state,w))
        end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    drain(&runtime);
    assert_eq!(runtime.context().ui.borrow().windows.len(), 1);
    runtime.lua().load("bad:set(true)").exec().unwrap();
    drain(&runtime);
    runtime.lua().load("assert(#errors==1 and errors[1].phase=='render' and errors[1].key=='label' and closed==0)").exec().unwrap();
    let handle = runtime
        .lua()
        .globals()
        .get::<mlua::AnyUserData>("w")
        .unwrap();
    assert_eq!(
        handle
            .borrow::<super::window::WindowHandle>()
            .unwrap()
            .owner(),
        runtime.context().ui.borrow().windows[0].owner
    );
}

#[test]
fn window_custom_keys_override_window_defaults_and_errors_never_unconsume_the_key() {
    let runtime = runtime(
        r#"
        ui=require('mmv').ui; errors={}; n=0
        require('mmv').keymap.set('x',function()w=ui.float.open{width=20,height=20,render=function()return ui.text{text='x'}end,keys={tab=function()n=n+1;error('handled key')end,escape=function()n=n+10 end},on_error=function(e)table.insert(errors,e)end}end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    drain(&runtime);
    let context = runtime.context();
    assert!(context.ui_key(runtime.lua(), "tab").unwrap());
    drain(&runtime);
    assert!(context.ui_key(runtime.lua(), "escape").unwrap());
    drain(&runtime);
    runtime
        .lua()
        .load("assert(n==11 and #errors==1 and errors[1].phase=='event' and w:is_open())")
        .exec()
        .unwrap();
    assert_eq!(context.ui.borrow().focus.unwrap().identity, None);
}
