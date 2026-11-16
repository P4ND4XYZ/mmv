use super::*;
use crate::lua::{LuaRuntime, Pump};

fn runtime() -> LuaRuntime {
    LuaRuntime::initialize("", None).unwrap().0
}

#[test]
fn markdown_strict_arguments_and_absent_display() {
    let r = runtime();
    r.lua().load(r#"
        local m=require('mmv').markdown
        assert(m.get_font_size()==nil)
        assert(m.set_font_size(8)==false)
        assert(m.adjust_font_size(-1)==false)
        assert(m.reset_font_size()==false)
        for _, name in ipairs({'set_font_size','adjust_font_size'}) do
            for _, args in ipairs({{}, {'16'}, {false}, {{}}, {math.huge}, {-math.huge}, {0/0}, {16,1}}) do
                assert(not pcall(m[name],table.unpack(args)), name)
            end
            assert(not pcall(m[name],nil))
        end
        for _, n in ipairs({7.999,48.001,-1,0}) do assert(not pcall(m.set_font_size,n)) end
        for _, name in ipairs({'get_font_size','reset_font_size'}) do
            assert(not pcall(m[name],nil))
            assert(not pcall(m[name],1))
        end
    "#).exec().unwrap();
    assert_eq!(r.pump_one(), Pump::Idle);
}

#[test]
fn markdown_requests_are_ordered_viewer_owned_and_getter_is_applied_only() {
    let r = runtime();
    *r.context().markdown.borrow_mut() = Some(16.0);
    r.lua()
        .load(
            r#"
        local m=require('mmv').markdown
        assert(m.set_font_size(48))
        assert(m.adjust_font_size(100))
        assert(m.adjust_font_size(-1.5))
        assert(m.adjust_font_size(-1e300))
        assert(m.adjust_font_size(0))
        assert(m.reset_font_size())
        assert(m.set_font_size(8))
        assert(m.get_font_size()==16)
    "#,
        )
        .exec()
        .unwrap();
    // Replacement affects availability, not the already accepted queue.
    *r.context().markdown.borrow_mut() = None;
    let mut size = 16.0;
    for expected in [48.0, 48.0, 46.5, 8.0, 8.0, 20.5, 8.0] {
        let request = r.take_request::<Request>().unwrap();
        size = request
            .operation
            .downcast::<Request>()
            .unwrap()
            .apply(size, 20.5);
        assert_eq!(size, expected);
    }
    assert_eq!(r.pump_one(), Pump::Idle);
}

#[test]
fn markdown_phase_rules_include_nested_render_item() {
    let (r,error) = LuaRuntime::initialize("",Some((r#"
        local m=require('mmv')
        local p=m.markdown
        for _,name in ipairs({'get_font_size','reset_font_size'}) do assert(not pcall(p[name])) end
        assert(not pcall(p.set_font_size,16))
        assert(not pcall(p.adjust_font_size,1))
        m.keymap.set('x',function()
            local selection=m.ui.state(nil)
            m.ui.float.open{width=100,height=100,render=function()
                assert(p.get_font_size()==nil)
                assert(not pcall(p.set_font_size,16))
                assert(not pcall(p.adjust_font_size,1))
                assert(not pcall(p.reset_font_size))
                return m.ui.list{key='items',items={{id=1}},selection=selection,render_item=function()
                    assert(p.get_font_size()==nil)
                    assert(not pcall(p.set_font_size,16))
                    assert(not pcall(p.adjust_font_size,1))
                    assert(not pcall(p.reset_font_size))
                    return m.ui.text{text='item'}
                end}
            end}
        end)
    "#,"@phase.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    assert!(r.enqueue_key("x").unwrap());
    assert_eq!(r.pump_one(), Pump::Dispatched);
    assert!(r.take_error().is_none());
    assert_eq!(r.context().ui.borrow().windows.len(), 1);
}

#[test]
fn setup_font_size_bounds_floats_and_invalid_values() {
    for n in ["8", "48", "20.5"] {
        let config = crate::config::execute(
            &format!("require('mmv').setup({{view={{font_size={n}}}}})"),
            "test",
        )
        .unwrap();
        assert_eq!(config.font_size, n.parse::<f32>().unwrap());
    }
    for n in [
        "7.99",
        "48.01",
        "'16'",
        "false",
        "{}",
        "math.huge",
        "-math.huge",
        "0/0",
    ] {
        assert!(
            crate::config::execute(
                &format!("require('mmv').setup({{view={{font_size={n}}}}})"),
                "test"
            )
            .is_err(),
            "{n}"
        );
    }
    let (r, error) = LuaRuntime::initialize(
        "",
        Some(("require('mmv').setup({view={font_size=80}})", "test")),
    )
    .unwrap();
    assert!(error.is_some());
    assert_eq!(r.config().font_size, 16.0);
}
