use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use mlua::{Lua, RegistryKey, UserData, UserDataMethods, Value};

use super::{Access, LuaContext};

#[derive(Clone)]
pub(crate) struct StateHandle {
    value: Rc<RefCell<RegistryKey>>,
    binding: Rc<RefCell<Option<Binding>>>,
    context: LuaContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindingType {
    String,
    SelectionId,
}

#[derive(Clone, Copy)]
struct Binding {
    owner: super::OwnerId,
    identity: u64,
    kind: BindingType,
}

impl BindingType {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => matches!(value, Value::String(s) if s.to_str().is_ok()),
            Self::SelectionId => matches!(value, Value::Nil | Value::String(_) | Value::Integer(_)),
        }
    }
}

impl StateHandle {
    pub(crate) fn new(lua: &Lua, context: &LuaContext, value: Value) -> mlua::Result<Self> {
        context.check(Access::InitMutation)?;
        let value = copy(lua, context, value)?;
        Ok(Self {
            value: Rc::new(RefCell::new(lua.create_registry_value(value)?)),
            binding: Default::default(),
            context: context.clone(),
        })
    }

    pub(crate) fn get(&self, lua: &Lua) -> mlua::Result<Value> {
        copy(
            lua,
            &self.context,
            lua.registry_value(&self.value.borrow())?,
        )
    }

    pub(crate) fn set(&self, lua: &Lua, value: Value) -> mlua::Result<()> {
        self.context.check(Access::InitMutation)?;
        if let Some(binding) = self.active_binding()
            && !binding.kind.accepts(&value)
        {
            return Err(mlua::Error::runtime(
                "value does not match state binding type",
            ));
        }
        let value = copy(lua, &self.context, value)?;
        let value = lua.create_registry_value(value)?;
        *self.value.borrow_mut() = value;
        let mut ui = self.context.ui.borrow_mut();
        ui.dirty |= !ui.windows.is_empty();
        for window in &mut ui.windows {
            window.synchronized = false;
        }
        Ok(())
    }

    pub(crate) fn same_state(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.value, &other.value)
    }

    pub(crate) fn replace_display_value(&self, lua: &Lua, value: Value) -> mlua::Result<()> {
        *self.value.borrow_mut() = lua.create_registry_value(copy(lua, &self.context, value)?)?;
        Ok(())
    }

    fn active_binding(&self) -> Option<Binding> {
        self.binding.borrow().filter(|b| {
            self.context.is_open(b.owner)
                && self.context.ui.borrow().windows.iter().any(|w| {
                    w.owner == b.owner && w.components.values().any(|c| c.identity == b.identity)
                })
        })
    }

    /// D calls this only when adopting a validated tree, not during render.
    /// A removed/closed component releases its claim by identity, so reusing a
    /// key never reconnects a stale binding. Validate all claims before adoption.
    pub(crate) fn check_binding(
        &self,
        lua: &Lua,
        owner: super::OwnerId,
        identity: u64,
        kind: BindingType,
    ) -> mlua::Result<()> {
        if self
            .active_binding()
            .is_some_and(|b| b.owner != owner || b.identity != identity)
        {
            return Err(mlua::Error::runtime("state already has a two-way binding"));
        }
        let value: Value = lua.registry_value(&self.value.borrow())?;
        if !kind.accepts(&value) {
            return Err(mlua::Error::runtime(
                "value does not match state binding type",
            ));
        }
        Ok(())
    }

    /// Pure candidate-tree validation for D. All claims belong to the one
    /// replaced window. Commit releases that window's old claims, adopts its
    /// complete tree, then binds these claims without running Lua in between.
    pub(crate) fn check_tree_bindings(
        lua: &Lua,
        owner: super::OwnerId,
        claims: &[(Self, u64, BindingType)],
    ) -> mlua::Result<()> {
        for (index, (state, _, kind)) in claims.iter().enumerate() {
            if claims[..index]
                .iter()
                .any(|(other, _, _)| Rc::ptr_eq(&state.value, &other.value))
            {
                return Err(mlua::Error::runtime("state already has a two-way binding"));
            }
            if state.active_binding().is_some_and(|b| b.owner != owner) {
                return Err(mlua::Error::runtime("state already has a two-way binding"));
            }
            let value: Value = lua.registry_value(&state.value.borrow())?;
            if !kind.accepts(&value) {
                return Err(mlua::Error::runtime(
                    "value does not match state binding type",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn unbind(&self, owner: super::OwnerId, identity: u64) {
        let mut binding = self.binding.borrow_mut();
        if binding.is_some_and(|b| b.owner == owner && b.identity == identity) {
            *binding = None;
        }
    }

    pub(crate) fn bind(
        &self,
        lua: &Lua,
        owner: super::OwnerId,
        identity: u64,
        kind: BindingType,
    ) -> mlua::Result<()> {
        self.check_binding(lua, owner, identity, kind)?;
        *self.binding.borrow_mut() = Some(Binding {
            owner,
            identity,
            kind,
        });
        Ok(())
    }
}

impl UserData for StateHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |lua, this, ()| this.get(lua));
        methods.add_method("set", |lua, this, value: Value| this.set(lua, value));
    }
}

/// Copy boundaries preserve aliasing, but reject cycles (including a shared
/// table that points back to an ancestor). Raw access never invokes Lua code.
pub(crate) fn copy(lua: &Lua, context: &LuaContext, value: Value) -> mlua::Result<Value> {
    fn visit(
        lua: &Lua,
        context: &LuaContext,
        value: Value,
        copies: &mut HashMap<usize, mlua::Table>,
        active: &mut HashSet<usize>,
    ) -> mlua::Result<Value> {
        Ok(match value {
            Value::Nil | Value::Boolean(_) | Value::Integer(_) | Value::String(_) => value,
            Value::Number(n) if n.is_finite() => value,
            Value::UserData(ref data) if context.readonly.contains(data) => value,
            Value::Table(table) => {
                if table.metatable().is_some() {
                    return Err(mlua::Error::runtime(
                        "state tables must not have a metatable",
                    ));
                }
                let id = table.to_pointer() as usize;
                if active.contains(&id) {
                    return Err(mlua::Error::runtime("state tables must not contain cycles"));
                }
                if let Some(table) = copies.get(&id) {
                    return Ok(Value::Table(table.clone()));
                }
                let result = lua.create_table()?;
                copies.insert(id, result.clone());
                active.insert(id);
                for entry in table.pairs::<Value, Value>() {
                    let (key, value) = entry?;
                    match &key {
                        Value::String(_) | Value::Integer(_) | Value::Boolean(_) => (),
                        Value::Number(n) if n.is_finite() => (),
                        _ => return Err(mlua::Error::runtime("invalid state table key")),
                    }
                    result.raw_set(key, visit(lua, context, value, copies, active)?)?;
                }
                active.remove(&id);
                Value::Table(result)
            }
            _ => return Err(mlua::Error::runtime("invalid state value")),
        })
    }
    visit(
        lua,
        context,
        value,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )
}
