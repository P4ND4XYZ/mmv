use super::*;
use crate::lua::{LuaRuntime, Pump};

fn runtime() -> LuaRuntime {
    LuaRuntime::initialize("", None).unwrap().0
}

#[test]
fn strict_pdf_arguments_and_no_pdf_booleans_do_not_register_requests() {
    let runtime = runtime();
    runtime.lua().load(r#"
        local p=require('mmv').pdf
        assert(p.get_zoom()==nil)
        assert(p.set_zoom(1)==false)
        for _,name in ipairs({'zoom_in','zoom_out','fit_width','fit_page'}) do
            assert(p[name]()==false)
            for _,value in ipairs({false,{},1,'x'}) do
                assert(not pcall(p[name],value),name)
            end
            assert(not pcall(p[name],nil),name)
        end
        for _,args in ipairs({{}, {0}, {-1}, {'1'}, {false}, {{}}, {1,{}}, {math.huge}, {-math.huge}, {0/0}}) do
            assert(not pcall(p.set_zoom,table.unpack(args)))
        end
        assert(not pcall(p.set_zoom,nil))
        assert(not pcall(p.get_zoom,1))
        assert(not pcall(p.get_zoom,nil))
        assert(not pcall(function() p:set_zoom(1) end))
    "#).exec().unwrap();
    assert_eq!(runtime.pump_one(), Pump::Idle);
}

#[test]
fn accepted_requests_bind_display_and_clamp_before_float_conversion_without_future_reads() {
    let runtime = runtime();
    let target = Target::default();
    *runtime.context().pdf.borrow_mut() = Some(target.clone());
    runtime
        .lua()
        .load(
            r#"
        local p=require('mmv').pdf
        assert(p.get_zoom()==nil)
        assert(p.set_zoom(1e300))
        assert(p.zoom_in())
        assert(p.zoom_out())
        assert(p.fit_width())
        assert(p.fit_page())
        assert(p.set_zoom(1e-300))
        assert(p.get_zoom()==nil)
    "#,
        )
        .exec()
        .unwrap();
    target.publish(0.5);
    runtime
        .lua()
        .load("assert(require('mmv').pdf.get_zoom()==0.5)")
        .exec()
        .unwrap();
    // A replacement must not rebind requests already accepted by the old display.
    *runtime.context().pdf.borrow_mut() = Some(Target::default());
    for expected in [
        Operation::SetZoom(3.0),
        Operation::ZoomIn,
        Operation::ZoomOut,
        Operation::FitWidth,
        Operation::FitPage,
        Operation::SetZoom(0.01),
    ] {
        let request = runtime.take_request::<Request>().unwrap();
        let request = request.operation.downcast::<Request>().unwrap();
        assert!(request.target.same_display(&target));
        assert_eq!(request.operation, expected);
    }
    assert_eq!(runtime.pump_one(), Pump::Idle);
    target.release();
    *runtime.context().pdf.borrow_mut() = Some(target);
    runtime
        .lua()
        .load("local p=require('mmv').pdf; assert(p.get_zoom()==nil and p.zoom_in()==false)")
        .exec()
        .unwrap();
}

#[test]
fn pdf_phase_rules_cover_init_render_and_render_item() {
    let source = r#"
        local m=require('mmv')
        assert(not pcall(m.pdf.set_zoom,1))
        for _,name in ipairs({'zoom_in','zoom_out','fit_width','fit_page','get_zoom'}) do
            assert(not pcall(m.pdf[name]))
        end
        m.keymap.set('x',function()
            local selection=m.ui.state(nil)
            m.ui.float.open{width=100,height=100,render=function()
                assert(m.pdf.get_zoom()==nil)
                for _,name in ipairs({'zoom_in','zoom_out','fit_width','fit_page'}) do
                    assert(not pcall(m.pdf[name]))
                end
                assert(not pcall(m.pdf.set_zoom,1))
                return m.ui.list{key='items',items={{id=1}},selection=selection,render_item=function()
                    assert(m.pdf.get_zoom()==nil)
                    assert(not pcall(m.pdf.zoom_in))
                    return m.ui.text{text='item'}
                end}
            end}
        end)
    "#;
    let (runtime, error) = LuaRuntime::initialize("", Some((source, "@phase.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    assert!(runtime.enqueue_key("x").unwrap());
    assert_eq!(runtime.pump_one(), Pump::Dispatched);
    assert!(runtime.take_error().is_none());
    assert_eq!(runtime.context().ui.borrow().windows.len(), 1);
}

#[test]
fn pdf_plugin_is_opt_in_and_keys_can_be_overridden_or_removed() {
    let runtime = runtime();
    for key in ["+", "-", "=", "p"] {
        assert!(!runtime.enqueue_key(key).unwrap());
    }
    let plugin = include_str!("../../../plugins/pdf_zoom.lua");
    let (runtime, error) = LuaRuntime::initialize("", Some((plugin, "@pdf_zoom.lua"))).unwrap();
    assert!(error.is_none(), "{error:?}");
    for key in ["+", "-", "=", "p"] {
        assert!(runtime.enqueue_key(key).unwrap());
        assert_eq!(runtime.pump_one(), Pump::Dispatched);
        assert_eq!(runtime.pump_one(), Pump::Idle); // no PDF is an ordinary no-op
    }
    runtime
        .lua()
        .load(
            r#"
        local m=require('mmv')
        m.keymap.set('+',function() overridden=true end)
        assert(m.keymap.remove('p'))
    "#,
        )
        .exec()
        .unwrap();
    assert!(!runtime.enqueue_key("p").unwrap());
    assert!(runtime.enqueue_key("+").unwrap());
    assert_eq!(runtime.pump_one(), Pump::Dispatched);
    assert!(runtime.lua().globals().get::<bool>("overridden").unwrap());
}
