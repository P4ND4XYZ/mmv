//! List identity and selection; display construction never invokes ordinary events.
use super::{
    Component,
    tree::{self, LuaFunction},
};
use crate::lua::{Access, CallbackScope, LuaContext, OwnerId, state::StateHandle};
use mlua::{Lua, MultiValue, UserData, UserDataMethods, Value};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Id {
    Integer(i64),
    String(Vec<u8>),
}
impl Id {
    pub(crate) fn parse(value: Value) -> mlua::Result<Option<Self>> {
        match value {
            Value::Nil => Ok(None),
            Value::Integer(n) => Ok(Some(Self::Integer(n))),
            Value::String(s) => Ok(Some(Self::String(s.as_bytes().to_vec()))),
            _ => Err(tree::err("selection ID must be nil, string or Lua integer")),
        }
    }
    pub(crate) fn value(id: &Option<Self>, lua: &Lua) -> mlua::Result<Value> {
        Ok(match id {
            None => Value::Nil,
            Some(Self::Integer(n)) => Value::Integer(*n),
            Some(Self::String(s)) => Value::String(lua.create_string(s)?),
        })
    }
}
#[derive(Clone)]
pub(crate) struct List {
    pub state: StateHandle,
    pub ids: Vec<Id>,
    pub selected: Option<Id>,
    pub reset: Value,
    pub wrap: bool,
    pub reveal: std::rc::Rc<std::cell::Cell<bool>>,
    pub notification_revision: std::rc::Rc<std::cell::Cell<u64>>,
    pub notified: std::rc::Rc<std::cell::Cell<bool>>,
    pub changed: Option<LuaFunction>,
    pub confirm: Option<LuaFunction>,
}
impl List {
    pub(crate) fn normalize(&self, previous: Option<&List>, value: Option<Id>) -> Option<Id> {
        if previous.is_some_and(|p| p.reset != self.reset) || previous.is_none() {
            return value
                .filter(|id| previous.is_none() && self.ids.contains(id))
                .or_else(|| self.ids.first().cloned());
        }
        match value {
            Some(id) if !self.ids.contains(&id) => self.ids.first().cloned(),
            other => other,
        }
    }
}
#[derive(Clone)]
pub(crate) struct ListHandle {
    pub owner: OwnerId,
    pub identity: u64,
    pub context: LuaContext,
}
impl ListHandle {
    fn component(&self) -> Option<Component> {
        self.context
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == self.owner)?
            .components
            .values()
            .find(|c| c.identity == self.identity)
            .cloned()
    }
    fn select(&self, lua: &Lua, value: Value) -> mlua::Result<bool> {
        let Some(c) = self.component() else {
            return Ok(false);
        };
        self.context.check(Access::Mutation)?;
        let list = c.node.list.as_ref().unwrap();
        let id = Id::parse(value)?;
        if id.as_ref().is_some_and(|id| !list.ids.contains(id)) {
            return Err(tree::err("unknown list ID"));
        }
        if Id::parse(list.state.get(lua)?)? != id {
            list.state.set(lua, Id::value(&id, lua)?)?;
        }
        list.reveal.set(id.is_some());
        Ok(true)
    }
    pub(crate) fn step(&self, lua: &Lua, forward: bool) -> mlua::Result<bool> {
        let Some(c) = self.component() else {
            return Ok(false);
        };
        self.context.check(Access::Mutation)?;
        let l = c.node.list.as_ref().unwrap();
        if l.ids.is_empty() {
            return Ok(false);
        }
        let current = Id::parse(l.state.get(lua)?)?;
        let index = current
            .as_ref()
            .and_then(|id| l.ids.iter().position(|i| i == id));
        let next = match index {
            None if forward => 0,
            None => l.ids.len() - 1,
            Some(i) if forward && i + 1 < l.ids.len() => i + 1,
            Some(i) if !forward && i > 0 => i - 1,
            Some(_) if l.wrap && forward => 0,
            Some(_) if l.wrap => l.ids.len() - 1,
            Some(i) => i,
        };
        self.select(lua, Id::value(&Some(l.ids[next].clone()), lua)?)
    }
    pub(crate) fn confirm(&self, lua: &Lua) -> mlua::Result<bool> {
        let Some(c) = self.component() else {
            return Ok(false);
        };
        self.context.check(Access::Mutation)?;
        let ui = self.context.ui.borrow();
        let w = ui.windows.iter().find(|w| w.owner == self.owner).unwrap();
        if !w.synchronized || w.native_pending {
            return Ok(false);
        }
        let l = c.node.list.as_ref().unwrap();
        let Some(id) = l.selected.clone() else {
            return Ok(false);
        };
        if Id::parse(l.state.get(lua)?)? != Some(id.clone()) {
            return Ok(false);
        }
        let f = l.confirm.clone();
        drop(ui);
        if let Some(f) = f {
            self.context.queue_component(
                lua,
                self.owner,
                c.node.key.clone(),
                &f,
                Id::value(&Some(id), lua)?,
            )?;
        }
        Ok(true)
    }
    pub(crate) fn click(&self, lua: &Lua, id: Id) -> mlua::Result<bool> {
        let Some(c) = self.component() else {
            return Ok(false);
        };
        {
            let ui = self.context.ui.borrow();
            if !ui
                .windows
                .iter()
                .any(|w| w.owner == self.owner && w.synchronized && !w.native_pending)
            {
                return Ok(false);
            }
        }
        if !c.node.list.as_ref().unwrap().ids.contains(&id) {
            return Ok(false);
        }
        self.select(lua, Id::value(&Some(id), lua)?)?;
        self.context.flush_ui(lua);
        self.confirm(lua)
    }
}
impl UserData for ListHandle {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("select", |lua, this, args: MultiValue| {
            if this.component().is_none() {
                return Ok(false);
            }
            if args.len() != 1 {
                return Err(tree::err("select expects one ID"));
            }
            this.select(lua, args.into_iter().next().unwrap())
        });
        for (name, forward) in [("select_next", true), ("select_previous", false)] {
            m.add_method(name, move |lua, this, args: MultiValue| {
                if this.component().is_none() {
                    return Ok(false);
                }
                if !args.is_empty() {
                    return Err(tree::err("navigation expects no arguments"));
                }
                this.step(lua, forward)
            });
        }
        m.add_method("confirm", |lua, this, args: MultiValue| {
            if this.component().is_none() {
                return Ok(false);
            }
            if !args.is_empty() {
                return Err(tree::err("confirm expects no arguments"));
            }
            this.confirm(lua)
        });
    }
}
impl LuaContext {
    pub(crate) fn queue_component(
        &self,
        lua: &Lua,
        owner: OwnerId,
        key: Option<String>,
        f: &LuaFunction,
        args: impl mlua::IntoLuaMulti,
    ) -> mlua::Result<bool> {
        let Some(id) = self.register_once(
            lua,
            tree::load_function(lua, f)?,
            CallbackScope::window(owner, key),
        )?
        else {
            return Ok(false);
        };
        self.enqueue(lua, id, args)
    }
}
