use super::callbacks::{CallbackId, CloseReason};
use super::*;
use mlua::{Value, Variadic};

fn runtime(source: &str) -> LuaRuntime {
    let (runtime, error) = LuaRuntime::initialize("", Some((source, "@test-init.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    runtime
}

fn function(runtime: &LuaRuntime, source: &str) -> Function {
    runtime.lua().load(source).eval().unwrap()
}

fn register(runtime: &LuaRuntime, scope: CallbackScope, source: &str) -> CallbackId {
    runtime
        .context()
        .register(runtime.lua(), function(runtime, source), scope)
        .unwrap()
        .unwrap()
}

fn drain(runtime: &LuaRuntime) -> Vec<ErrorInfo> {
    let mut errors = Vec::new();
    for _ in 0..100 {
        match runtime.pump_one() {
            Pump::Dispatched => (),
            Pump::HostPending => match runtime.take_output().unwrap() {
                HostOutput::Error(error) => errors.push(error),
                HostOutput::Request(_) => panic!("unexpected request"),
            },
            Pump::Idle => return errors,
            Pump::Busy => panic!("unexpected reentry"),
        }
    }
    panic!("pump did not settle");
}

#[test]
fn editor_annotations_are_valid_lua_without_executing_them() {
    Lua::new()
        .load(include_str!("../../plugins/mmv.lua"))
        .into_function()
        .unwrap();
}

#[test]
fn init_closure_and_require_identity_survive_and_keys_can_change_at_runtime() {
    let runtime = runtime(
        r#"
        local mmv = require('mmv')
        assert(mmv.setup({view={max_width=700}}) == true)
        assert(mmv.set_hl('Normal', {}) == true)
        local count = 40
        mmv.keymap.set('x', function()
            count = count + 1
            result = count
            assert(require('mmv') == mmv)
            assert(not pcall(function() mmv.setup({}) end))
            assert(not pcall(function() mmv.set_hl('Normal', {}) end))
            assert(mmv.keymap.set('y', function() result = 99 end))
            assert(mmv.keymap.remove('x'))
        end)
    "#,
    );
    assert_eq!(runtime.config().max_width, 700.0);
    assert!(runtime.enqueue_key("x").unwrap());
    assert!(drain(&runtime).is_empty());
    assert_eq!(runtime.lua().globals().get::<i64>("result").unwrap(), 41);
    assert!(!runtime.enqueue_key("x").unwrap());
    assert!(runtime.enqueue_key("y").unwrap());
    assert!(drain(&runtime).is_empty());
    assert_eq!(runtime.lua().globals().get::<i64>("result").unwrap(), 99);
    assert_eq!(runtime.config().max_width, 700.0);
}

#[test]
fn failed_init_restores_standard_settings_keys_and_vm_without_partial_globals() {
    let standard = "local m=require('mmv'); m.keymap.set('f',function() result='standard' end)";
    for failure in [
        "this is not Lua !",
        "error('unhandled')",
        "pcall(function() m.setup({unknown=true}) end)",
        "pcall(function() m.set_hl('Normal',{fg='broken'}) end)",
        "pcall(function() m.keymap.set('A',function()end) end)",
        "pcall(function() m.keymap.set('z',42) end)",
        "pcall(function() m.keymap.remove('f',{mode='search'}) end)",
    ] {
        let source = format!(
            "local m=require('mmv'); candidate=true; m.setup({{view={{position='left'}}}}); m.keymap.remove('f'); m.keymap.set('x',function()end); {failure}"
        );
        let (runtime, error) =
            LuaRuntime::initialize(standard, Some((&source, "@broken.lua"))).unwrap();
        assert!(error.is_some(), "accepted {failure}");
        assert_eq!(runtime.config(), Config::default());
        assert!(
            runtime
                .lua()
                .globals()
                .get::<Value>("candidate")
                .unwrap()
                .is_nil()
        );
        assert!(!runtime.enqueue_key("x").unwrap());
        assert!(runtime.enqueue_key("f").unwrap());
        assert!(drain(&runtime).is_empty());
        assert_eq!(
            runtime.lua().globals().get::<String>("result").unwrap(),
            "standard"
        );
    }
}

#[test]
fn successful_init_atomically_overrides_and_removes_standard_registrations() {
    let (runtime, error) = LuaRuntime::initialize(
        "local m=require('mmv'); m.keymap.set('f',function() result='standard' end); m.keymap.set('x',function()end)",
        Some(("local m=require('mmv'); m.setup({colorscheme='light'}); m.keymap.set('f',function() result='user' end); assert(m.keymap.remove('x')); assert(not m.keymap.remove('z'))", "@user.lua")),
    ).unwrap();
    assert!(error.is_none());
    assert_eq!(
        runtime.config().colorscheme,
        crate::config::ColorScheme::Light
    );
    assert!(!runtime.enqueue_key("x").unwrap());
    runtime.enqueue_key("f").unwrap();
    drain(&runtime);
    assert_eq!(
        runtime.lua().globals().get::<String>("result").unwrap(),
        "user"
    );
}

#[test]
fn caught_non_registration_errors_are_not_sticky() {
    let runtime = runtime(
        "local m=require('mmv'); pcall(function() error('handled') end); pcall(function() m.not_implemented() end); m.setup({view={max_width=123}})",
    );
    assert_eq!(runtime.config().max_width, 123.0);
    assert!(
        runtime
            .context()
            .api(runtime.lua())
            .unwrap()
            .get::<Value>("not_implemented")
            .unwrap()
            .is_nil()
    );
}

#[test]
fn api_installation_precedes_init_and_discarded_vms_release_all_contexts() {
    for fail in [false, true] {
        let contexts = Rc::new(RefCell::new(Vec::new()));
        let source = format!(
            "local m=require('mmv'); assert(not pcall(m.execution_fixture)); m.keymap.set('x',function()end); {}",
            if fail {
                "pcall(function() m.keymap.set('A',function()end) end)"
            } else {
                ""
            },
        );
        let (runtime, error) = LuaRuntime::initialize_with(
            "",
            Some((&source, "@fixture.lua")),
            |lua, api, context| {
                contexts.borrow_mut().push(Rc::downgrade(&context.shared));
                let context = context.clone();
                api.set(
                    "execution_fixture",
                    lua.create_function(move |_, ()| context.check(Access::Mutation))?,
                )
            },
        )
        .unwrap();
        assert_eq!(error.is_some(), fail);
        assert_eq!(runtime.enqueue_key("x").unwrap(), !fail);
        assert_eq!(contexts.borrow().len(), 2);
        assert_eq!(contexts.borrow()[0].upgrade().is_some(), fail);
        assert_eq!(contexts.borrow()[1].upgrade().is_some(), !fail);
        drop(runtime);
        assert!(
            contexts
                .borrow()
                .iter()
                .all(|context| context.upgrade().is_none())
        );
    }
}

#[test]
fn one_invalid_setup_does_not_partially_apply_even_inside_the_candidate() {
    let runtime = LuaRuntime::new().unwrap();
    let api = runtime.context().api(runtime.lua()).unwrap();
    let setup: Function = api.get("setup").unwrap();
    let options: Table = runtime
        .lua()
        .load("return {view={position='left'},tree={visible='invalid'}}")
        .eval()
        .unwrap();
    assert!(setup.call::<bool>(options).is_err());
    assert_eq!(runtime.config(), Config::default());
    let set_hl: Function = api.get("set_hl").unwrap();
    let attrs: Table = runtime
        .lua()
        .load("return {fg='#123456',bg='invalid'}")
        .eval()
        .unwrap();
    assert!(set_hl.call::<bool>(("Normal", attrs)).is_err());
    assert_eq!(runtime.config(), Config::default());
}

#[test]
fn key_lexing_normalization_and_collisions_follow_the_public_grammar() {
    for (input, expected) in [
        ("SHIFT-CTRL-Enter", "ctrl-shift-enter"),
        ("super-alt-界", "alt-super-界"),
        ("-", "-"),
        ("ctrl--", "ctrl--"),
        ("shift-a", "shift-a"),
        ("F24", "f24"),
        ("Ä", "Ä"),
    ] {
        assert_eq!(keymap::normalize(input).unwrap(), expected);
    }
    for input in [
        "",
        "A",
        "ctrl-A",
        "ctrl-CTRL-a",
        "a b",
        "ctrl",
        "esc",
        "f25",
        "ctrl-",
        " a",
        "\n",
        "control-a",
        "g-g",
    ] {
        assert!(keymap::normalize(input).is_err(), "accepted {input:?}");
    }
    let runtime = runtime("");
    for source in [
        "return {['CTRL-a']=function()end,['ctrl-a']=function()end}",
        "return {['x']=setmetatable({}, {__call=function()end})}",
        "return {function()end}",
    ] {
        let table = runtime.lua().load(source).eval().unwrap();
        assert!(keymap::parse_keys(table).is_err());
    }
    for invalid in [
        "m.keymap.set(42,function()end)",
        "m.keymap.set('x',{})",
        "m.keymap.set('x',function()end,{focus='body'})",
        "m.keymap.set('x',function()end,{mode=1})",
        "m.keymap.set('x',function()end,1)",
        "m.keymap.set(string.char(255),function()end)",
        "m.keymap.remove('x',{},true)",
    ] {
        let (_, error) = LuaRuntime::initialize(
            "",
            Some((
                &format!("local m=require('mmv'); pcall(function() {invalid} end)"),
                "@bad-key.lua",
            )),
        )
        .unwrap();
        assert!(error.is_some(), "accepted {invalid}");
    }
}

#[test]
fn invalid_runtime_key_operations_preserve_the_previous_registration() {
    let runtime = runtime(
        r#"
        local m=require('mmv')
        m.keymap.set('f',function() result='original' end)
        m.keymap.set('x',function()
            assert(not pcall(function() m.keymap.set('f',function() result='invalid' end,{mode='search'}) end))
            assert(not pcall(function() m.keymap.remove('f',{unknown=true}) end))
        end)
    "#,
    );
    runtime.enqueue_key("x").unwrap();
    assert!(drain(&runtime).is_empty());
    assert!(runtime.enqueue_key("f").unwrap());
    assert!(drain(&runtime).is_empty());
    assert_eq!(
        runtime.lua().globals().get::<String>("result").unwrap(),
        "original"
    );
}

#[test]
fn synchronous_construction_finishes_before_deferred_initial_events() {
    let runtime = runtime("trace=''");
    let ctx = runtime.context();
    let owner = ctx.create_owner(runtime.lua(), None, None).unwrap();
    let initial = register(
        &runtime,
        CallbackScope::window(owner, Some("list".into())),
        "return function() trace=trace..'initial;' end",
    );
    let adapter = ctx.clone();
    runtime
        .lua()
        .globals()
        .set(
            "open_fixture",
            runtime
                .lua()
                .create_function(move |lua, render: Function| {
                    let value: i64 = adapter.construct(owner, None, &render, ())?;
                    adapter.enqueue(lua, initial, ())?;
                    assert_eq!(adapter.pump_one(lua), Pump::Busy);
                    Ok(value)
                })
                .unwrap(),
        )
        .unwrap();
    let callback = register(
        &runtime,
        CallbackScope::app(),
        r#"return function()
        trace=trace..'before;'
        local value=open_fixture(function() trace=trace..'render;'; return 7 end)
        assert(value==7)
        trace=trace..'returned;'
    end"#,
    );
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    assert_eq!(runtime.pump_one(), Pump::Dispatched);
    assert_eq!(
        runtime.lua().globals().get::<String>("trace").unwrap(),
        "before;render;returned;"
    );
    assert!(drain(&runtime).is_empty());
    assert_eq!(
        runtime.lua().globals().get::<String>("trace").unwrap(),
        "before;render;returned;initial;"
    );
}

#[test]
fn phase_checks_cover_init_render_nested_items_and_restore_after_errors() {
    let runtime = LuaRuntime::new().unwrap();
    let ctx = runtime.context();
    assert_eq!(ctx.phase(), Phase::Init);
    assert!(ctx.check(Access::InitMutation).is_ok());
    assert!(ctx.check(Access::Mutation).is_err());
    assert!(ctx.check(Access::Read).is_err());
    runtime.run_init("", "@empty.lua").unwrap();
    let owner = ctx.create_owner(runtime.lua(), None, None).unwrap();
    let render_ctx = ctx.clone();
    runtime
        .lua()
        .globals()
        .set(
            "check_render",
            runtime
                .lua()
                .create_function(move |lua, ()| {
                    assert_eq!(render_ctx.phase(), Phase::Render);
                    assert!(render_ctx.check(Access::Read).is_ok());
                    assert!(render_ctx.check(Access::Pure).is_ok());
                    assert!(render_ctx.check(Access::InitOnly).is_err());
                    assert!(render_ctx.check(Access::InitMutation).is_err());
                    assert!(render_ctx.request("must not apply").is_err());
                    assert!(
                        render_ctx
                            .close_owner(owner, CloseReason::Programmatic)
                            .is_err()
                    );
                    let inner: Function = lua
                        .load("return function() error('item failed') end")
                        .eval()?;
                    render_ctx.render_item::<()>("list".into(), &inner, ())
                })
                .unwrap(),
        )
        .unwrap();
    let render = function(&runtime, "return function() check_render() end");
    assert!(ctx.construct::<()>(owner, None, &render, ()).is_err());
    assert_eq!(ctx.phase(), Phase::Event);
    assert!(ctx.is_open(owner));
    assert!(
        drain(&runtime).is_empty(),
        "initial construction errors belong only to caller"
    );
    ctx.discard_owner(owner);
    assert!(!ctx.is_open(owner));
}

#[test]
fn closing_invalidates_pending_callbacks_and_calls_on_close_once_after_cleanup() {
    let runtime = runtime("count=0; closed=0");
    let ctx = runtime.context();
    let on_close = function(
        &runtime,
        "return function(e) closed=closed+1; reason=e.reason end",
    );
    let owner = ctx
        .create_owner(runtime.lua(), Some(on_close), None)
        .unwrap();
    let callback = register(
        &runtime,
        CallbackScope::window(owner, None),
        "return function() count=count+1 end",
    );
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    assert!(ctx.close_owner(owner, CloseReason::Escape).unwrap());
    assert!(!ctx.is_open(owner));
    assert!(!ctx.close_owner(owner, CloseReason::Programmatic).unwrap());
    assert!(!ctx.enqueue(runtime.lua(), callback, ()).unwrap());
    assert!(
        ctx.register(
            runtime.lua(),
            function(&runtime, "return function()end"),
            CallbackScope::window(owner, None)
        )
        .unwrap()
        .is_none()
    );
    assert!(drain(&runtime).is_empty());
    assert_eq!(runtime.lua().globals().get::<i64>("count").unwrap(), 0);
    assert_eq!(runtime.lua().globals().get::<i64>("closed").unwrap(), 1);
    assert_eq!(
        runtime.lua().globals().get::<String>("reason").unwrap(),
        "escape"
    );
}

#[test]
fn running_callback_continues_after_owner_close_and_late_error_goes_to_host() {
    let runtime = runtime("continued=false; handled=0; closed=0");
    let ctx = runtime.context();
    let owner = ctx
        .create_owner(
            runtime.lua(),
            Some(function(&runtime, "return function() closed=closed+1 end")),
            Some(function(
                &runtime,
                "return function() handled=handled+1 end",
            )),
        )
        .unwrap();
    let adapter = ctx.clone();
    runtime
        .lua()
        .globals()
        .set(
            "close_fixture",
            runtime
                .lua()
                .create_function(move |_, ()| adapter.close_owner(owner, CloseReason::Programmatic))
                .unwrap(),
        )
        .unwrap();
    let callback = register(
        &runtime,
        CallbackScope::window(owner, Some("input".into())),
        "return function() close_fixture(); continued=true; error('after close') end",
    );
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    let errors = drain(&runtime);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("after close"));
    assert_eq!(errors[0].key.as_deref(), Some("input"));
    assert!(runtime.lua().globals().get::<bool>("continued").unwrap());
    assert_eq!(runtime.lua().globals().get::<i64>("handled").unwrap(), 0);
    assert_eq!(runtime.lua().globals().get::<i64>("closed").unwrap(), 1);
}

#[test]
fn error_table_has_phase_key_and_traceback_and_caught_errors_are_not_reported() {
    let runtime = runtime("errors={}");
    let ctx = runtime.context();
    let owner = ctx
        .create_owner(
            runtime.lua(),
            None,
            Some(function(
                &runtime,
                "return function(e) errors[#errors+1]=e end",
            )),
        )
        .unwrap();
    let caught = register(
        &runtime,
        CallbackScope::window(owner, None),
        "return function() pcall(function() error('caught') end) end",
    );
    let failure = register(
        &runtime,
        CallbackScope::window(owner, Some("search".into())),
        "return function() error('event failed') end",
    );
    ctx.enqueue(runtime.lua(), caught, ()).unwrap();
    ctx.enqueue(runtime.lua(), failure, ()).unwrap();
    assert!(drain(&runtime).is_empty());
    let item_context = ctx.clone();
    runtime
        .lua()
        .globals()
        .set(
            "render_item_fixture",
            runtime
                .lua()
                .create_function(move |lua, ()| {
                    let item: Function = lua
                        .load("return function() error('render failed') end")
                        .eval()?;
                    item_context.render_item::<()>("list".into(), &item, ())
                })
                .unwrap(),
        )
        .unwrap();
    let render_scope = CallbackScope {
        owner: Some(owner),
        phase: Phase::Render,
        key: None,
    };
    let render = register(
        &runtime,
        render_scope.clone(),
        "return function() render_item_fixture() end",
    );
    let caught_item = register(
        &runtime,
        render_scope,
        "return function() pcall(render_item_fixture); error('unrelated root error') end",
    );
    ctx.enqueue(runtime.lua(), render, ()).unwrap();
    ctx.enqueue(runtime.lua(), caught_item, ()).unwrap();
    assert!(drain(&runtime).is_empty());
    let errors: Table = runtime.lua().globals().get("errors").unwrap();
    assert_eq!(errors.raw_len(), 3);
    let event: Table = errors.get(1).unwrap();
    assert_eq!(event.get::<String>("phase").unwrap(), "event");
    assert_eq!(event.get::<String>("key").unwrap(), "search");
    assert!(
        event
            .get::<String>("message")
            .unwrap()
            .contains("event failed")
    );
    assert!(
        event
            .get::<String>("traceback")
            .unwrap()
            .contains("stack traceback")
    );
    let render: Table = errors.get(2).unwrap();
    assert_eq!(render.get::<String>("phase").unwrap(), "render");
    assert_eq!(render.get::<String>("key").unwrap(), "list");
    let root: Table = errors.get(3).unwrap();
    assert!(root.get::<Value>("key").unwrap().is_nil());
    assert!(
        root.get::<String>("message")
            .unwrap()
            .contains("unrelated root error")
    );
}

#[test]
fn on_error_failure_on_close_failure_and_app_errors_fall_back_without_recursion() {
    let runtime = runtime("handled=0");
    let ctx = runtime.context();
    let owner = ctx
        .create_owner(
            runtime.lua(),
            Some(function(
                &runtime,
                "return function() error('close failed') end",
            )),
            Some(function(
                &runtime,
                "return function() handled=handled+1; error('handler failed') end",
            )),
        )
        .unwrap();
    let failure = register(
        &runtime,
        CallbackScope::window(owner, None),
        "return function() error('original') end",
    );
    ctx.enqueue(runtime.lua(), failure, ()).unwrap();
    let errors = drain(&runtime);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("handler failed"));
    assert_eq!(runtime.lua().globals().get::<i64>("handled").unwrap(), 1);
    let captured_owner = ctx.clone();
    let app = ctx
        .register(
            runtime.lua(),
            runtime
                .lua()
                .create_function(move |_, ()| {
                    assert!(captured_owner.is_open(owner));
                    Err::<(), _>(mlua::Error::runtime("app failed"))
                })
                .unwrap(),
            CallbackScope::app(),
        )
        .unwrap()
        .unwrap();
    ctx.enqueue(runtime.lua(), app, ()).unwrap();
    assert!(drain(&runtime)[0].message.contains("app failed"));
    assert_eq!(
        runtime.lua().globals().get::<i64>("handled").unwrap(),
        1,
        "capturing an owner does not confer window scope"
    );
    ctx.close_owner(owner, CloseReason::Shutdown).unwrap();
    assert!(drain(&runtime)[0].message.contains("close failed"));
}

#[test]
fn owner_closed_before_error_delivery_uses_fallback_and_missing_handler_does_too() {
    let runtime = runtime("handled=0");
    let ctx = runtime.context();
    let owner = ctx
        .create_owner(
            runtime.lua(),
            None,
            Some(function(
                &runtime,
                "return function() handled=handled+1 end",
            )),
        )
        .unwrap();
    ctx.report_error(
        Some(owner),
        ErrorInfo::new(mlua::Error::runtime("pending"), Phase::Render, None),
    );
    ctx.close_owner(owner, CloseReason::OutsideClick).unwrap();
    assert_eq!(drain(&runtime)[0].message, "pending");
    assert_eq!(runtime.lua().globals().get::<i64>("handled").unwrap(), 0);
    let owner = ctx.create_owner(runtime.lua(), None, None).unwrap();
    let failure = register(
        &runtime,
        CallbackScope::window(owner, None),
        "return function() error('no handler') end",
    );
    ctx.enqueue(runtime.lua(), failure, ()).unwrap();
    assert!(drain(&runtime)[0].message.contains("no handler"));
}

#[test]
fn cancellation_and_close_release_callback_roots_without_waiting_for_lua_success() {
    let runtime = runtime("weak=setmetatable({}, {__mode='v'})");
    let ctx = runtime.context();
    let owner = ctx.create_owner(runtime.lua(), None, None).unwrap();
    let callback = register(
        &runtime,
        CallbackScope::window(owner, None),
        "local payload={}; weak[1]=payload; return function() return payload end",
    );
    ctx.enqueue(runtime.lua(), callback, (Value::Nil, 4, Value::Nil))
        .unwrap();
    assert!(ctx.cancel(callback));
    assert!(!ctx.cancel(callback));
    assert!(drain(&runtime).is_empty());
    runtime.lua().gc_collect().unwrap();
    assert!(
        runtime
            .lua()
            .globals()
            .get::<Table>("weak")
            .unwrap()
            .get::<Value>(1)
            .unwrap()
            .is_nil()
    );
    let callback = register(
        &runtime,
        CallbackScope::window(owner, None),
        "local payload={}; weak[2]=payload; return function() return payload end",
    );
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    ctx.close_owner(owner, CloseReason::Programmatic).unwrap();
    drain(&runtime);
    runtime.lua().gc_collect().unwrap();
    assert!(
        runtime
            .lua()
            .globals()
            .get::<Table>("weak")
            .unwrap()
            .get::<Value>(2)
            .unwrap()
            .is_nil()
    );
}

#[test]
fn one_shot_delivery_releases_registration_and_duplicate_events_even_on_error() {
    let runtime = runtime("delivered=0; weak=setmetatable({}, {__mode='v'})");
    let ctx = runtime.context();
    let callback = ctx.register_once(runtime.lua(), function(&runtime,
        "local payload={}; weak[1]=payload; return function() delivered=delivered+1; local retained=payload; error('completion failed') end"
    ), CallbackScope::app()).unwrap().unwrap();
    assert!(ctx.enqueue(runtime.lua(), callback, ()).unwrap());
    assert!(ctx.enqueue(runtime.lua(), callback, ()).unwrap());
    let errors = drain(&runtime);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("completion failed"));
    assert_eq!(runtime.lua().globals().get::<i64>("delivered").unwrap(), 1);
    assert!(!ctx.enqueue(runtime.lua(), callback, ()).unwrap());
    assert!(!ctx.cancel(callback));
    runtime.lua().gc_collect().unwrap();
    assert!(
        runtime
            .lua()
            .globals()
            .get::<Table>("weak")
            .unwrap()
            .get::<Value>(1)
            .unwrap()
            .is_nil()
    );
}

#[test]
fn argument_queue_preserves_nil_positions_and_ignores_callback_returns() {
    let runtime = runtime("");
    let ctx = runtime.context();
    let callback = register(
        &runtime,
        CallbackScope::app(),
        "return function(...) args=table.pack(...); return false,'unused' end",
    );
    ctx.enqueue(
        runtime.lua(),
        callback,
        Variadic::from(vec![Value::Nil, Value::Integer(4), Value::Nil]),
    )
    .unwrap();
    assert!(drain(&runtime).is_empty());
    let args: Table = runtime.lua().globals().get("args").unwrap();
    assert_eq!(args.get::<i64>("n").unwrap(), 3);
    assert_eq!(args.get::<i64>(2).unwrap(), 4);
}

#[test]
fn host_acceptance_is_not_inline_delivery_and_survives_close_and_later_error() {
    #[derive(Debug, PartialEq)]
    enum Operation {
        Adopt,
    }
    let runtime = runtime("continued=false; notified=false");
    let ctx = runtime.context();
    let owner = ctx.create_owner(runtime.lua(), None, None).unwrap();
    let completion = register(
        &runtime,
        CallbackScope::window(owner, None),
        "return function() error('closed completion ran') end",
    );
    let notification = register(
        &runtime,
        CallbackScope::app(),
        "return function() notified=true end",
    );
    let adapter = ctx.clone();
    runtime
        .lua()
        .globals()
        .set(
            "request_fixture",
            runtime
                .lua()
                .create_function(move |lua, ()| {
                    adapter.request(Operation::Adopt)?;
                    assert!(adapter.take_output().is_none());
                    assert_eq!(adapter.pump_one(lua), Pump::Busy);
                    adapter.close_owner(owner, CloseReason::Programmatic)?;
                    Ok(true)
                })
                .unwrap(),
        )
        .unwrap();
    let callback = register(
        &runtime,
        CallbackScope::app(),
        "return function() assert(request_fixture()); continued=true; error('after accepted request') end",
    );
    ctx.enqueue(runtime.lua(), callback, ()).unwrap();
    assert_eq!(runtime.pump_one(), Pump::Dispatched);
    assert!(runtime.lua().globals().get::<bool>("continued").unwrap());
    assert!(!runtime.lua().globals().get::<bool>("notified").unwrap());
    assert_eq!(runtime.pump_one(), Pump::HostPending);
    match runtime.take_output().unwrap() {
        HostOutput::Request(request) => assert_eq!(
            *request.operation.downcast::<Operation>().unwrap(),
            Operation::Adopt
        ),
        _ => panic!("request must precede its later error"),
    }
    assert!(
        !ctx.enqueue(runtime.lua(), completion, ()).unwrap(),
        "owner close cancels callback, not host operation"
    );
    ctx.enqueue(runtime.lua(), notification, ()).unwrap();
    assert_eq!(
        runtime.pump_one(),
        Pump::HostPending,
        "drain remaining output before notifications"
    );
    match runtime.take_output().unwrap() {
        HostOutput::Error(error) => assert!(error.message.contains("after accepted request")),
        _ => panic!("expected error after request"),
    }
    assert!(drain(&runtime).is_empty());
    assert!(runtime.lua().globals().get::<bool>("notified").unwrap());
}
