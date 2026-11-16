//! Resident Lua VM. No mutable runtime borrow may cross a call into Lua.
pub(crate) mod callbacks;
pub(crate) mod document;
pub(crate) mod host;
pub(crate) mod keymap;
pub(crate) mod markdown;
pub(crate) mod matching;
pub(crate) mod navigation;
pub(crate) mod notify;
pub(crate) mod pdf;
pub(crate) mod readonly;
pub(crate) mod state;
pub(crate) mod ui;

#[cfg(test)]
mod tests;

use std::{cell::RefCell, rc::Rc};

use mlua::{FromLuaMulti, Function, IntoLuaMulti, Lua, MultiValue, Table};

use crate::config::Config;
pub(crate) use callbacks::{CallbackScope, ErrorInfo, OwnerId};
pub(crate) use host::{HostOutput, Pump};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Init,
    Event,
    Render,
}

/// API availability, shared by later state/UI/document/navigation adapters.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Access {
    Pure,
    InitOnly,
    Read,
    Mutation,
    InitMutation,
    RenderOnly,
}

#[derive(Clone)]
pub(crate) struct LuaContext {
    shared: Rc<RefCell<callbacks::State>>,
    pub(crate) ui: Rc<RefCell<ui::UiState>>,
    pub(crate) readonly: readonly::ReadonlyTypes,
    pub(crate) documents: Rc<RefCell<document::Documents>>,
    pub(crate) pdf: Rc<RefCell<Option<pdf::Target>>>,
    pub(crate) markdown: Rc<RefCell<Option<f32>>>,
}

pub(crate) struct LuaRuntime {
    lua: Lua,
    context: LuaContext,
}

impl LuaRuntime {
    fn new() -> mlua::Result<Self> {
        Self::new_with(&|_, _, _| Ok(()))
    }

    fn new_with(
        install: &impl Fn(&Lua, &Table, &LuaContext) -> mlua::Result<()>,
    ) -> mlua::Result<Self> {
        let lua = Lua::new();
        let context = LuaContext {
            shared: Rc::new(RefCell::new(callbacks::State::default())),
            ui: Default::default(),
            readonly: Default::default(),
            documents: Default::default(),
            pdf: Default::default(),
            markdown: Default::default(),
        };
        let api = lua.create_table()?;
        crate::config::lua::install(&lua, &api, &context)?;
        keymap::install(&lua, &api, &context)?;
        ui::install(&lua, &api, &context)?;
        document::install(&lua, &api, &context)?;
        matching::install(&lua, &api, &context)?;
        navigation::install(&lua, &api, &context)?;
        notify::install(&lua, &api, &context)?;
        pdf::install(&lua, &api, &context)?;
        markdown::install(&lua, &api, &context)?;
        install(&lua, &api, &context)?;
        lua.register_module("mmv", api)?;
        Ok(Self { lua, context })
    }

    /// An optional caller-supplied prelude runs first. A failed candidate VM is dropped in
    /// full, including globals, registry roots and closures, not just Config.
    /// The successful init VM itself becomes the execution VM.
    pub(crate) fn initialize(
        standard: &str,
        user: Option<(&str, &str)>,
    ) -> mlua::Result<(Self, Option<String>)> {
        Self::initialize_with(standard, user, |_, _, _| Ok(()))
    }

    /// Install later APIs into each isolated VM before caller-supplied init.
    /// The installer only builds bindings; it must not perform host operations.
    pub(crate) fn initialize_with(
        standard: &str,
        user: Option<(&str, &str)>,
        install: impl Fn(&Lua, &Table, &LuaContext) -> mlua::Result<()>,
    ) -> mlua::Result<(Self, Option<String>)> {
        let baseline = Self::new_with(&install)?;
        baseline.run_init(standard, "@mmv/standard.lua")?;
        let Some((source, name)) = user else {
            return Ok((baseline, None));
        };
        let candidate = Self::new_with(&install)?;
        candidate.run_init(standard, "@mmv/standard.lua")?;
        match candidate.run_init(source, name) {
            Ok(()) => Ok((candidate, None)),
            Err(error) => Ok((baseline, Some(error.to_string()))),
        }
    }

    /// Config-only construction for existing Rust callers/tests. Production
    /// installs the VM returned by the configuration loader instead.
    pub(crate) fn from_config(config: Config) -> Self {
        let runtime = Self::new().expect("create Lua runtime");
        let mut state = runtime.context.shared.borrow_mut();
        state.config = config;
        state.initializing = false;
        drop(state);
        runtime
    }

    fn run_init(&self, source: &str, name: &str) -> mlua::Result<()> {
        self.context.shared.borrow_mut().initializing = true;
        let result = self.lua.load(source).set_name(name).exec();
        let mut state = self.context.shared.borrow_mut();
        state.initializing = false;
        if let Some(error) = state.validation_error.take() {
            return Err(mlua::Error::runtime(error));
        }
        result
    }

    pub(crate) fn config(&self) -> Config {
        self.context.shared.borrow().config.clone()
    }

    pub(crate) fn lua(&self) -> &Lua {
        &self.lua
    }

    pub(crate) fn context(&self) -> LuaContext {
        self.context.clone()
    }

    /// Queue an app key callback. Consumed is independent of its result/error.
    /// GPUI routing/IME and mode checks are the input adapter's responsibility.
    pub(crate) fn enqueue_key(&self, key: &str) -> mlua::Result<bool> {
        let key = keymap::normalize(key).map_err(mlua::Error::runtime)?;
        let callback = self.context.shared.borrow().keys.get(&key).copied();
        match callback {
            Some(callback) => self.context.enqueue(&self.lua, callback, ()),
            None => Ok(false),
        }
    }

    /// Exactly one ordinary callback per step. Drain/apply host outputs before
    /// stepping again; resulting notifications are enqueued, never called inline.
    pub(crate) fn pump_one(&self) -> Pump {
        self.context.flush_ui(&self.lua);
        let result = self.context.pump_one(&self.lua);
        if result == Pump::Dispatched {
            self.context.flush_ui(&self.lua);
        }
        result
    }

    /// Leave unknown typed requests queued for their owning adapter.
    pub(crate) fn take_request<T: 'static>(&self) -> Option<host::HostRequest> {
        let mut state = self.context.shared.borrow_mut();
        if !state.frames.is_empty()
            || !matches!(state.outputs.front(), Some(HostOutput::Request(request)) if request.operation.is::<T>())
        {
            return None;
        }
        match state.outputs.pop_front() {
            Some(HostOutput::Request(request)) => Some(request),
            _ => unreachable!(),
        }
    }

    pub(crate) fn take_output(&self) -> Option<HostOutput> {
        self.context.take_output()
    }

    /// The B-only GUI adapter handles errors but leaves future host requests
    /// intact for the concrete C/E adapter, rather than silently accepting them.
    pub(crate) fn take_error(&self) -> Option<ErrorInfo> {
        let mut state = self.context.shared.borrow_mut();
        if !state.frames.is_empty() || !matches!(state.outputs.front(), Some(HostOutput::Error(_)))
        {
            return None;
        }
        match state.outputs.pop_front() {
            Some(HostOutput::Error(error)) => Some(error),
            _ => unreachable!(),
        }
    }
}

impl LuaContext {
    pub(crate) fn phase(&self) -> Phase {
        let state = self.shared.borrow();
        if state.initializing {
            Phase::Init
        } else {
            state
                .frames
                .last()
                .map_or(Phase::Event, |frame| frame.phase)
        }
    }

    pub(crate) fn check(&self, access: Access) -> mlua::Result<()> {
        let phase = self.phase();
        let allowed = match access {
            Access::Pure => true,
            Access::InitOnly => phase == Phase::Init,
            Access::Read => phase != Phase::Init,
            Access::Mutation => phase == Phase::Event,
            Access::InitMutation => phase != Phase::Render,
            Access::RenderOnly => phase == Phase::Render,
        };
        if allowed {
            Ok(())
        } else {
            Err(mlua::Error::runtime(format!(
                "API is unavailable during {phase:?}"
            )))
        }
    }

    /// Only settings/key registration wrappers call this. Other caught errors
    /// must not poison the initialization transaction.
    pub(crate) fn registration_result<T>(&self, result: mlua::Result<T>) -> mlua::Result<T> {
        if let Err(error) = &result {
            let mut state = self.shared.borrow_mut();
            if state.initializing && state.validation_error.is_none() {
                state.validation_error = Some(error.to_string());
            }
        }
        result
    }

    pub(crate) fn update_config(
        &self,
        args: MultiValue,
        action: fn(&mut Config, MultiValue) -> Result<(), String>,
    ) -> mlua::Result<bool> {
        let result = (|| {
            self.check(Access::InitOnly)?;
            let mut candidate = self.shared.borrow().config.clone();
            action(&mut candidate, args).map_err(mlua::Error::runtime)?;
            self.shared.borrow_mut().config = candidate;
            Ok(true)
        })();
        self.registration_result(result)
    }

    /// Explicit synchronous construction exception. Errors go to the caller
    /// (e.g. float.open), not to on_error; the caller owns provisional cleanup.
    /// Ordinary events queued here remain deferred until the outer Lua returns.
    pub(crate) fn construct<R: FromLuaMulti>(
        &self,
        owner: OwnerId,
        key: Option<String>,
        function: &Function,
        args: impl IntoLuaMulti,
    ) -> mlua::Result<R> {
        self.check(Access::Mutation)?;
        let _frame = self.enter(callbacks::Frame {
            phase: Phase::Render,
            owner: Some(owner),
            key,
            kind: callbacks::Kind::Normal,
        });
        function.call(args)
    }

    /// Nested render_item is part of the same synchronous display construction.
    pub(crate) fn render_item<R: FromLuaMulti>(
        &self,
        key: String,
        function: &Function,
        args: impl IntoLuaMulti,
    ) -> mlua::Result<R> {
        self.check(Access::RenderOnly)?;
        let owner = self
            .shared
            .borrow()
            .frames
            .last()
            .and_then(|frame| frame.owner);
        let _frame = self.enter(callbacks::Frame {
            phase: Phase::Render,
            owner,
            key: Some(key.clone()),
            kind: callbacks::Kind::Normal,
        });
        function
            .call(args)
            .map_err(|error| mlua::Error::external(callbacks::RenderFailure { error, key }))
    }

    /// Add APIs to this VM's require('mmv') table, without publishing stubs for
    /// not-yet-implemented public APIs. Extension closures capture LuaContext,
    /// not LuaRuntime/RefCell<LuaRuntime>, and release borrows before Lua calls.
    pub(crate) fn api(&self, lua: &Lua) -> mlua::Result<Table> {
        lua.globals()
            .get::<Table>("package")?
            .get::<Table>("loaded")?
            .get("mmv")
    }
}
