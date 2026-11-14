use crate::lua::Pump;

use super::Viewer;

impl Viewer {
    pub(super) fn shutdown_lua(&mut self) {
        let _ = self.lua.context().shutdown_ui();
        self.pump_lua();
    }

    /// Called only at an outer host boundary. C/E extend HostOutput handling;
    /// F enqueues keys after IME/mode/layer routing, then requests a host pump.
    /// There is no RefCell<LuaRuntime> borrow across Lua execution.
    pub(super) fn pump_lua(&mut self) {
        loop {
            match self.lua.pump_one() {
                Pump::Dispatched => (),
                Pump::HostPending => {
                    let Some(error) = self.lua.take_error() else {
                        // A later adapter must consume/apply its typed request
                        // before any further ordinary callbacks are dispatched.
                        break;
                    };
                    crate::infra::log_document_failure(
                        std::path::Path::new("[lua]"),
                        &error.traceback,
                    );
                    self.message = Some("Lua callback failed".into());
                    self.message_is_error = true;
                }
                Pump::Busy | Pump::Idle => break,
            }
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};

    #[gpui_kit::test]
    fn viewer_keeps_the_successful_init_vm_and_pumps_its_closures(cx: &mut TestAppContext) {
        let (runtime, error) = crate::lua::LuaRuntime::initialize("", Some((
            "local m=require('mmv'); local n=40; m.keymap.set('x',function() n=n+1; result=n; same=(m==require('mmv')) end)",
            "@fixture.lua",
        ))).unwrap();
        assert!(error.is_none());
        let (_, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    super::Viewer::with_lua(
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
        view.update(cx, |view, _| {
            assert!(view.lua.enqueue_key("x").unwrap());
            view.pump_lua();
            assert_eq!(view.lua.lua().globals().get::<i64>("result").unwrap(), 41);
            assert!(view.lua.lua().globals().get::<bool>("same").unwrap());
        });
    }
}
