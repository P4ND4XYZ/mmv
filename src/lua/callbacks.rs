use std::collections::{BTreeMap, VecDeque};

use mlua::{Function, IntoLuaMulti, Lua, RegistryKey, Value};

use super::{Access, LuaContext, Phase, host::HostOutput};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallbackId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct OwnerId(u64);

#[derive(Clone, Debug)]
pub(crate) struct CallbackScope {
    pub owner: Option<OwnerId>,
    pub phase: Phase,
    pub key: Option<String>,
}

impl CallbackScope {
    pub(crate) fn app() -> Self {
        Self {
            owner: None,
            phase: Phase::Event,
            key: None,
        }
    }

    pub(crate) fn window(owner: OwnerId, key: Option<String>) -> Self {
        Self {
            owner: Some(owner),
            phase: Phase::Event,
            key,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CloseReason {
    Programmatic,
    Escape,
    OutsideClick,
    Shutdown,
}

impl CloseReason {
    fn name(self) -> &'static str {
        match self {
            Self::Programmatic => "programmatic",
            Self::Escape => "escape",
            Self::OutsideClick => "outside_click",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Error metadata travels with an unhandled render_item error, not in a sticky
/// side channel: a caught error cannot contaminate a later unrelated failure.
#[derive(Debug)]
pub(super) struct RenderFailure {
    pub error: mlua::Error,
    pub key: String,
}

impl std::fmt::Display for RenderFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for RenderFailure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErrorInfo {
    pub message: String,
    pub traceback: String,
    pub phase: Phase,
    pub key: Option<String>,
}

impl ErrorInfo {
    pub(crate) fn new(error: mlua::Error, phase: Phase, key: Option<String>) -> Self {
        let traceback = error.to_string();
        fn render_key(error: &mlua::Error) -> Option<&str> {
            match error {
                mlua::Error::CallbackError { cause, .. }
                | mlua::Error::BadArgument { cause, .. }
                | mlua::Error::WithContext { cause, .. } => render_key(cause),
                mlua::Error::ExternalError(error) => error
                    .downcast_ref::<RenderFailure>()
                    .map(|error| render_key(&error.error).unwrap_or(error.key.as_str())),
                _ => None,
            }
        }
        let (phase, key) = match render_key(&error) {
            Some(key) => (Phase::Render, Some(key.to_owned())),
            None => (phase, key),
        };
        fn message(error: &mlua::Error) -> String {
            match error {
                mlua::Error::RuntimeError(message) => message
                    .split("\nstack traceback:")
                    .next()
                    .unwrap_or(message)
                    .to_owned(),
                mlua::Error::CallbackError { cause, .. } => message(cause),
                mlua::Error::ExternalError(error) => match error.downcast_ref::<RenderFailure>() {
                    Some(error) => message(&error.error),
                    None => error.to_string(),
                },
                _ => error.to_string(),
            }
        }
        let message = message(&error);
        Self {
            message,
            traceback,
            phase,
            key,
        }
    }

    fn value(&self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        table.set("message", self.message.clone())?;
        table.set("traceback", self.traceback.clone())?;
        table.set(
            "phase",
            if self.phase == Phase::Render {
                "render"
            } else {
                "event"
            },
        )?;
        table.set("key", self.key.clone())?;
        Ok(Value::Table(table))
    }
}

pub(super) struct Callback {
    function: RegistryKey,
    scope: CallbackScope,
    once: bool,
}

pub(super) struct Owner {
    on_close: Option<RegistryKey>,
    on_error: Option<RegistryKey>,
}

pub(super) enum Pending {
    Normal {
        callback: CallbackId,
        args: Vec<RegistryKey>,
    },
    Close {
        function: RegistryKey,
        reason: CloseReason,
    },
    Error {
        owner: OwnerId,
        error: ErrorInfo,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Normal,
    Close,
    Error,
}

pub(super) struct Frame {
    pub phase: Phase,
    pub owner: Option<OwnerId>,
    pub key: Option<String>,
    pub kind: Kind,
}

pub(super) struct State {
    pub config: crate::config::Config,
    pub initializing: bool,
    pub validation_error: Option<String>,
    pub keys: BTreeMap<String, CallbackId>,
    pub callbacks: BTreeMap<CallbackId, Callback>,
    pub owners: BTreeMap<OwnerId, Owner>,
    pub pending: VecDeque<Pending>,
    pub outputs: VecDeque<HostOutput>,
    pub frames: Vec<Frame>,
    pub next_id: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            config: Default::default(),
            initializing: true,
            validation_error: None,
            keys: Default::default(),
            callbacks: Default::default(),
            owners: Default::default(),
            pending: Default::default(),
            outputs: Default::default(),
            frames: Vec::new(),
            next_id: 0,
        }
    }
}

impl State {
    pub(super) fn id(&mut self) -> u64 {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("Lua resource ID exhausted");
        self.next_id
    }
}

pub(super) struct FrameGuard(LuaContext);
impl Drop for FrameGuard {
    fn drop(&mut self) {
        self.0.shared.borrow_mut().frames.pop();
    }
}

impl LuaContext {
    pub(super) fn enter(&self, frame: Frame) -> FrameGuard {
        self.shared.borrow_mut().frames.push(frame);
        FrameGuard(self.clone())
    }

    pub(crate) fn create_owner(
        &self,
        lua: &Lua,
        on_close: Option<Function>,
        on_error: Option<Function>,
    ) -> mlua::Result<OwnerId> {
        self.check(Access::Mutation)?;
        let on_close = on_close.map(|f| lua.create_registry_value(f)).transpose()?;
        let on_error = on_error.map(|f| lua.create_registry_value(f)).transpose()?;
        let mut state = self.shared.borrow_mut();
        let id = OwnerId(state.id());
        state.owners.insert(id, Owner { on_close, on_error });
        Ok(id)
    }

    pub(crate) fn is_open(&self, owner: OwnerId) -> bool {
        self.shared.borrow().owners.contains_key(&owner)
    }

    /// Use for failed provisional construction. No on_close/on_error is sent.
    pub(crate) fn discard_owner(&self, owner: OwnerId) {
        let mut state = self.shared.borrow_mut();
        state.owners.remove(&owner);
        Self::release_callbacks(&mut state, owner);
    }

    /// Cleanup is complete before on_close is enqueued. Running Lua is never
    /// interrupted; its held Function survives removal of its registry root.
    pub(crate) fn close_owner(&self, owner: OwnerId, reason: CloseReason) -> mlua::Result<bool> {
        // Expired handle lifetime takes precedence over phase/argument checks.
        if !self.is_open(owner) {
            return Ok(false);
        }
        self.check(Access::Mutation)?;
        let mut state = self.shared.borrow_mut();
        let Some(owner_state) = state.owners.remove(&owner) else {
            return Ok(false);
        };
        Self::release_callbacks(&mut state, owner);
        if let Some(function) = owner_state.on_close {
            state.pending.push_back(Pending::Close { function, reason });
        }
        Ok(true)
    }

    fn release_callbacks(state: &mut State, owner: OwnerId) {
        state
            .callbacks
            .retain(|_, callback| callback.scope.owner != Some(owner));
        state.pending.retain(|pending| match pending {
            Pending::Normal { callback, .. } => state.callbacks.contains_key(callback),
            // Keep error reports: after close they must go to host fallback.
            _ => true,
        });
    }

    /// Registration source determines scope; closure captures never do.
    /// Closed owners create no callback/root (including navigation completion).
    pub(crate) fn register(
        &self,
        lua: &Lua,
        function: Function,
        scope: CallbackScope,
    ) -> mlua::Result<Option<CallbackId>> {
        self.register_callback(lua, function, scope, false)
    }

    /// One-shot results release their registration before Lua runs, even if
    /// delivery errors. Duplicate queued deliveries are invalidated together.
    pub(crate) fn register_once(
        &self,
        lua: &Lua,
        function: Function,
        scope: CallbackScope,
    ) -> mlua::Result<Option<CallbackId>> {
        self.register_callback(lua, function, scope, true)
    }

    fn register_callback(
        &self,
        lua: &Lua,
        function: Function,
        scope: CallbackScope,
        once: bool,
    ) -> mlua::Result<Option<CallbackId>> {
        if scope.owner.is_some_and(|owner| !self.is_open(owner)) {
            return Ok(None);
        }
        let function = lua.create_registry_value(function)?;
        let mut state = self.shared.borrow_mut();
        let id = CallbackId(state.id());
        state.callbacks.insert(
            id,
            Callback {
                function,
                scope,
                once,
            },
        );
        Ok(Some(id))
    }

    pub(crate) fn cancel(&self, callback: CallbackId) -> bool {
        let mut state = self.shared.borrow_mut();
        let removed = state.callbacks.remove(&callback).is_some();
        state.keys.retain(|_, registered| *registered != callback);
        state.pending.retain(
            |pending| !matches!(pending, Pending::Normal { callback: id, .. } if *id == callback),
        );
        removed
    }

    /// Arguments are registry-rooted independently of the producer's stack.
    /// Producers construct fresh event/result tables at their public boundary.
    pub(crate) fn enqueue(
        &self,
        lua: &Lua,
        callback: CallbackId,
        args: impl IntoLuaMulti,
    ) -> mlua::Result<bool> {
        if !self.shared.borrow().callbacks.contains_key(&callback) {
            return Ok(false);
        }
        let args = args
            .into_lua_multi(lua)?
            .into_iter()
            .map(|value| lua.create_registry_value(value))
            .collect::<mlua::Result<Vec<_>>>()?;
        self.shared
            .borrow_mut()
            .pending
            .push_back(Pending::Normal { callback, args });
        Ok(true)
    }

    pub(crate) fn report_error(&self, owner: Option<OwnerId>, error: ErrorInfo) {
        let mut state = self.shared.borrow_mut();
        if let Some(owner) = owner {
            state.pending.push_front(Pending::Error { owner, error });
        } else {
            state.outputs.push_back(HostOutput::Error(error));
        }
    }

    pub(super) fn dispatch(&self, lua: &Lua, pending: Pending) {
        // Materialize a Function/arguments, then release State before calling
        // Lua. RegistryKey, not Function/Value, is stored in captured State to
        // avoid an owning Lua -> closure -> State -> Lua cycle.
        let prepared = (|| -> mlua::Result<Option<(Function, mlua::MultiValue, Frame)>> {
            match pending {
                Pending::Normal { callback: id, args } => {
                    let state = self.shared.borrow();
                    let Some(callback) = state.callbacks.get(&id) else {
                        return Ok(None);
                    };
                    let function = lua.registry_value(&callback.function)?;
                    let args = args
                        .iter()
                        .map(|arg| lua.registry_value(arg))
                        .collect::<mlua::Result<_>>()?;
                    let frame = Frame {
                        phase: callback.scope.phase,
                        owner: callback.scope.owner,
                        key: callback.scope.key.clone(),
                        kind: Kind::Normal,
                    };
                    let once = callback.once;
                    drop(state);
                    if once {
                        self.cancel(id);
                    }
                    Ok(Some((function, args, frame)))
                }
                Pending::Close { function, reason } => {
                    let event = lua.create_table()?;
                    event.set("reason", reason.name())?;
                    Ok(Some((
                        lua.registry_value(&function)?,
                        (event,).into_lua_multi(lua)?,
                        Frame {
                            phase: Phase::Event,
                            owner: None,
                            key: None,
                            kind: Kind::Close,
                        },
                    )))
                }
                Pending::Error { owner, error } => {
                    let function = {
                        let state = self.shared.borrow();
                        state
                            .owners
                            .get(&owner)
                            .and_then(|owner| owner.on_error.as_ref())
                            .map(|key| lua.registry_value::<Function>(key))
                            .transpose()?
                    };
                    match function {
                        Some(function) => Ok(Some((
                            function,
                            (error.value(lua)?,).into_lua_multi(lua)?,
                            Frame {
                                phase: Phase::Event,
                                owner: Some(owner),
                                key: None,
                                kind: Kind::Error,
                            },
                        ))),
                        None => {
                            self.shared
                                .borrow_mut()
                                .outputs
                                .push_back(HostOutput::Error(error));
                            Ok(None)
                        }
                    }
                }
            }
        })();
        match prepared {
            Ok(Some((function, args, frame))) => {
                let phase = frame.phase;
                let owner = frame.owner;
                let key = frame.key.clone();
                let kind = frame.kind;
                let guard = self.enter(frame);
                let result = function.call::<()>(args);
                drop(guard);
                if let Err(error) = result {
                    let error = ErrorInfo::new(error, phase, key);
                    if kind == Kind::Normal && owner.is_some_and(|owner| self.is_open(owner)) {
                        self.report_error(owner, error);
                    } else {
                        self.shared
                            .borrow_mut()
                            .outputs
                            .push_back(HostOutput::Error(error));
                    }
                }
            }
            Ok(None) => (),
            Err(error) => self
                .shared
                .borrow_mut()
                .outputs
                .push_back(HostOutput::Error(ErrorInfo::new(error, Phase::Event, None))),
        }
    }
}
