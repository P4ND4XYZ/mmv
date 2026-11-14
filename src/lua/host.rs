use std::any::Any;

use super::{Access, ErrorInfo, LuaContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostRequestId(u64);

/// Rust-only, typed adapter payload. Future adapters downcast their own command
/// enums; no Lua Values, GPUI borrows, or callbacks are carried across this seam.
pub(crate) struct HostRequest {
    pub id: HostRequestId,
    pub operation: Box<dyn Any>,
}

pub(crate) enum HostOutput {
    Request(HostRequest),
    Error(ErrorInfo),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pump {
    Busy,
    HostPending,
    Dispatched,
    Idle,
}

impl LuaContext {
    /// Synchronous acceptance only. Host application happens after Lua returns.
    /// Requests survive owner close and subsequent callback failure. Completion
    /// callbacks are separate registered resources, subject to owner lifetime.
    pub(crate) fn request<T: 'static>(&self, operation: T) -> mlua::Result<HostRequestId> {
        self.check(Access::Mutation)?;
        let mut state = self.shared.borrow_mut();
        let id = HostRequestId(state.id());
        state.outputs.push_back(HostOutput::Request(HostRequest {
            id,
            operation: Box::new(operation),
        }));
        Ok(id)
    }

    pub(super) fn pump_one(&self, lua: &mlua::Lua) -> Pump {
        let pending = {
            let mut state = self.shared.borrow_mut();
            if state.initializing || !state.frames.is_empty() {
                return Pump::Busy;
            }
            if !state.outputs.is_empty() {
                return Pump::HostPending;
            }
            state.pending.pop_front()
        };
        let Some(pending) = pending else {
            lua.expire_registry_values();
            return Pump::Idle;
        };
        self.dispatch(lua, pending);
        lua.expire_registry_values();
        Pump::Dispatched
    }

    pub(super) fn take_output(&self) -> Option<HostOutput> {
        let mut state = self.shared.borrow_mut();
        if state.initializing || !state.frames.is_empty() {
            return None;
        }
        state.outputs.pop_front()
    }
}
