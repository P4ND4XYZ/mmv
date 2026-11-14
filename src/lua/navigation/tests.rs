use super::*;
use crate::lua::{LuaRuntime, Pump, document::DocumentHandle};
use std::{path::PathBuf, sync::Arc};

#[test]
fn normal_installer_has_strict_phase_fuzzy_notify_and_navigation_contracts() {
    let (runtime, error) = LuaRuntime::initialize(
        "",
        Some((
            r#"
        local m=require('mmv')
        assert(m.match.fuzzy('日本','日本語').ranges[1].stop==6)
        assert(m.match.fuzzy('','x').score==m.match.fuzzy('','日本語').score)
        assert(#m.match.fuzzy('','x').ranges==0 and m.match.fuzzy('z','abc')==nil)
        for _,f in ipairs({
          function() m.match.fuzzy(1,'x') end,
          function() m.match.fuzzy('x',string.char(255)) end,
          function() m.document.current() end,
          function() m.document.on_replaced({},function()end) end,
          function() m.notify('x') end,
          function() m.navigation.open('x') end
        }) do assert(not pcall(f)) end
    "#,
            "@init.lua",
        )),
    )
    .unwrap();
    assert!(error.is_none(), "{error:?}");
    runtime
        .context()
        .replace_document(
            runtime.lua(),
            DocumentHandle::new(
                PathBuf::from(if cfg!(windows) {
                    r"C:\docs\a.md"
                } else {
                    "/docs/a.md"
                }),
                Arc::new("x".into()),
                |path, _| {
                    Ok(crate::core::links::candidates(
                        path,
                        [("x".into(), "#x".into(), Some(0..1))],
                    ))
                },
            ),
        )
        .unwrap();
    runtime.lua().load(r#"
        local m=require('mmv'); target=m.document.current():links()[1].target
        for _,f in ipairs({
          function() m.navigation.open('x') end,
          function() m.navigation.open(target,{other=true}) end,
          function() m.navigation.open(target,{owner={}}) end,
          function() m.navigation.open(target,{on_complete=setmetatable({},{__call=function()end})}) end,
          function() m.notify(1) end,
          function() m.notify('x',{level='debug'}) end,
          function() m.notify('x',{other=true}) end,
          function() m.document.on_replaced({},function()end) end
        }) do assert(not pcall(f)) end
        w=m.ui.float.open{width=100,height=100,render=function()
            assert(m.document.current()~=nil and m.match.fuzzy('x','x')~=nil)
            assert(not pcall(function() m.navigation.open(target) end))
            assert(not pcall(function() m.notify('x') end))
            assert(not pcall(function() m.document.on_replaced({owner=w},function()end) end))
            return m.ui.text{text='x'}
        end}
        w:close()
        assert(select('#',m.navigation.open(target,{owner=w,on_complete=function() error('must not run') end}))==0)
    "#).exec().unwrap();
    assert_eq!(runtime.pump_one(), Pump::HostPending);
    let request = runtime.take_request::<OpenRequest>().unwrap();
    let request = request.operation.downcast::<OpenRequest>().unwrap();
    assert!(request.delivery.handler && request.delivery.callback.is_none() && !request.stale);
}

#[test]
fn results_have_only_the_documented_shape() {
    let lua = Lua::new();
    for (outcome, expected) in [
        (
            Outcome::success(),
            "assert(r.ok and r.code==nil and r.warning==nil)",
        ),
        (
            Outcome::failure("stale_target", "Old target"),
            "assert(not r.ok and r.code=='stale_target' and r.warning==nil)",
        ),
        (
            Outcome::warning("heading_not_found", "No heading"),
            "assert(r.ok and r.code==nil and r.warning.code=='heading_not_found')",
        ),
    ] {
        lua.globals()
            .set("r", outcome.value(&lua).unwrap())
            .unwrap();
        lua.load(expected).exec().unwrap();
    }
}
