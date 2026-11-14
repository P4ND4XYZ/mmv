#[cfg(test)]
mod tests;
pub(crate) mod tree;
pub(crate) mod window;

use super::{Access, CallbackScope, ErrorInfo, LuaContext, OwnerId, Phase, callbacks::CloseReason};
use mlua::{Lua, MultiValue, Table, Value};
use std::{collections::BTreeMap, rc::Rc};
use tree::{Kind, LuaFunction, Node};
use window::WindowData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FocusTarget {
    pub owner: OwnerId,
    pub identity: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct Component {
    pub kind: Kind,
    pub identity: u64,
    pub enabled: bool,
    pub on_click: Option<LuaFunction>,
}

#[derive(Default)]
pub(crate) struct UiState {
    pub dirty: bool,
    pub windows: Vec<WindowData>,
    pub focus: Option<FocusTarget>,
    pub focus_revision: u64,
    pub restoring_focus: bool,
    pub shutting_down: bool,
    next_identity: u64,
}

impl UiState {
    pub(crate) fn set_focus(&mut self, focus: Option<FocusTarget>) {
        self.focus = focus;
        self.focus_revision += 1;
        self.restoring_focus = false;
    }

    pub(crate) fn accessible(&self, owner: OwnerId) -> bool {
        let Some(index) = self.windows.iter().position(|w| w.owner == owner) else {
            return false;
        };
        self.windows
            .iter()
            .rposition(|w| w.modal)
            .is_none_or(|modal| index >= modal)
    }

    pub(crate) fn remove(&mut self, owner: OwnerId) {
        let Some(index) = self.windows.iter().position(|w| w.owner == owner) else {
            return;
        };
        let previous = self.windows.remove(index).previous_focus;
        if self.focus.is_some_and(|focus| focus.owner == owner) {
            let valid =
                previous.filter(|focus| self.valid_focus(*focus) && self.accessible(focus.owner));
            self.set_focus(valid.or_else(|| {
                self.windows
                    .iter()
                    .rev()
                    .find(|w| w.modal)
                    .map(|w| FocusTarget {
                        owner: w.owner,
                        identity: None,
                    })
            }));
            self.restoring_focus = true;
        }
        // A closed intermediate window must not break a later restoration chain.
        for window in &mut self.windows {
            if window
                .previous_focus
                .is_some_and(|focus| focus.owner == owner)
            {
                window.previous_focus = previous;
            }
        }
    }

    pub(crate) fn valid_focus(&self, focus: FocusTarget) -> bool {
        self.windows
            .iter()
            .find(|w| w.owner == focus.owner)
            .is_some_and(|w| {
                focus.identity.is_none_or(|id| {
                    w.components
                        .values()
                        .any(|c| c.identity == id && c.kind == Kind::Button && c.enabled)
                })
            })
    }

    fn components(
        &mut self,
        root: &Node,
        old: &BTreeMap<String, Component>,
    ) -> (BTreeMap<String, Component>, Vec<u64>) {
        fn visit(
            state: &mut UiState,
            node: &Node,
            old: &BTreeMap<String, Component>,
            result: &mut BTreeMap<String, Component>,
            order: &mut Vec<u64>,
        ) {
            if let Some(key) = &node.key {
                let identity = old
                    .get(key)
                    .filter(|c| c.kind == node.kind)
                    .map(|c| c.identity)
                    .unwrap_or_else(|| {
                        state.next_identity += 1;
                        state.next_identity
                    });
                result.insert(
                    key.clone(),
                    Component {
                        kind: node.kind,
                        identity,
                        enabled: node.enabled,
                        on_click: node.on_click.clone(),
                    },
                );
                if node.kind == Kind::Button && node.enabled {
                    order.push(identity);
                }
            }
            for child in &node.children {
                visit(state, child, old, result, order);
            }
        }
        let mut result = BTreeMap::new();
        let mut order = Vec::new();
        visit(self, root, old, &mut result, &mut order);
        (result, order)
    }

    pub(crate) fn adopt(&mut self, owner: OwnerId, root: Rc<Node>) {
        let Some(index) = self.windows.iter().position(|w| w.owner == owner) else {
            return;
        };
        let old = self.windows[index].components.clone();
        let (components, order) = self.components(&root, &old);
        if let Some(focus) = self.focus.filter(|focus| focus.owner == owner)
            && let Some(identity) = focus.identity
            && !order.contains(&identity)
        {
            fn keyed_order(node: &Node, keys: &mut Vec<String>) {
                if let Some(key) = &node.key {
                    keys.push(key.clone());
                }
                for child in &node.children {
                    keyed_order(child, keys);
                }
            }
            let mut new_keys = Vec::new();
            let mut old_keys = Vec::new();
            keyed_order(&root, &mut new_keys);
            keyed_order(&self.windows[index].root, &mut old_keys);
            let focused_key = old
                .iter()
                .find(|(_, c)| c.identity == identity)
                .map(|(key, _)| key);
            // A disabled/type-changed key still anchors its new display slot.
            // After deletion, anchor after the nearest surviving predecessor.
            let start = focused_key
                .and_then(|key| {
                    new_keys
                        .iter()
                        .position(|k| k == key)
                        .map(|i| i + 1)
                        .or_else(|| {
                            old_keys.iter().position(|k| k == key).and_then(|i| {
                                old_keys[..i].iter().rev().find_map(|previous| {
                                    new_keys.iter().position(|k| k == previous).map(|i| i + 1)
                                })
                            })
                        })
                })
                .unwrap_or(0);
            let next = new_keys
                .iter()
                .cycle()
                .skip(start)
                .take(new_keys.len())
                .find_map(|key| {
                    components
                        .get(key)
                        .filter(|c| c.kind == Kind::Button && c.enabled)
                        .map(|c| c.identity)
                });
            self.set_focus(Some(FocusTarget {
                owner,
                identity: next,
            }));
        }
        let window = &mut self.windows[index];
        window.root = root;
        window.components = components;
        window.focus_order = order;
    }
}

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let ui = lua.create_table()?;
    let ctx = context.clone();
    ui.set(
        "state",
        lua.create_function(move |lua, args: MultiValue| {
            if args.len() > 1 {
                return Err(tree::err("state expects at most one argument"));
            }
            super::state::StateHandle::new(lua, &ctx, args.into_iter().next().unwrap_or(Value::Nil))
        })?,
    )?;
    for (name, kind) in [
        ("row", Kind::Row),
        ("column", Kind::Column),
        ("text", Kind::Text),
        ("button", Kind::Button),
    ] {
        let ctx = context.clone();
        ui.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                if args.len() != 1 {
                    return Err(tree::err("component expects one options table"));
                }
                let value = args.into_iter().next().unwrap();
                let key = match &value {
                    Value::Table(t) => match t.raw_get::<Value>("key")? {
                        Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                        _ => None,
                    },
                    _ => None,
                };
                tree::build(lua, kind, value).map_err(|error| {
                    if ctx.phase() == Phase::Render
                        && let Some(key) = key
                    {
                        mlua::Error::external(super::callbacks::RenderFailure { error, key })
                    } else {
                        error
                    }
                })
            })?,
        )?;
    }
    let float = lua.create_table()?;
    let ctx = context.clone();
    float.set(
        "open",
        lua.create_function(move |lua, args: MultiValue| window::open(lua, &ctx, args))?,
    )?;
    ui.set("float", float)?;
    api.set("ui", ui)
}

impl LuaContext {
    /// An outer boundary only: all sets in one Lua invocation coalesce, including
    /// sets completed before an unhandled event error. Failed trees are not adopted.
    pub(crate) fn flush_ui(&self, lua: &Lua) {
        if self.phase() != Phase::Event || !self.shared.borrow().frames.is_empty() {
            return;
        }
        // Preserve B's outer-boundary ordering: apply host outputs before
        // constructing the next display or dispatching ordinary callbacks.
        if !self.shared.borrow().outputs.is_empty() {
            return;
        }
        if !std::mem::take(&mut self.ui.borrow_mut().dirty) {
            return;
        }
        let windows = self
            .ui
            .borrow()
            .windows
            .iter()
            .map(|w| (w.owner, w.render.clone()))
            .collect::<Vec<_>>();
        for (owner, render) in windows {
            if !self.is_open(owner) {
                continue;
            }
            let result = tree::load_function(lua, &render)
                .and_then(|f| self.construct::<MultiValue>(owner, None, &f, ()))
                .and_then(|values| window::render_result(lua, values));
            match result {
                Ok(root) => self.ui.borrow_mut().adopt(owner, root),
                Err(error) => {
                    self.report_error(Some(owner), ErrorInfo::new(error, Phase::Render, None))
                }
            }
        }
    }

    pub(crate) fn shutdown_ui(&self) -> mlua::Result<()> {
        self.ui.borrow_mut().shutting_down = true;
        let owners = self
            .ui
            .borrow()
            .windows
            .iter()
            .map(|w| w.owner)
            .collect::<Vec<_>>();
        for owner in owners {
            self.close_owner(owner, CloseReason::Shutdown)?;
        }
        Ok(())
    }

    pub(crate) fn focus_window(&self, owner: OwnerId, key: Value) -> mlua::Result<bool> {
        if !self.is_open(owner) {
            return Ok(false);
        }
        self.check(Access::Mutation)?;
        let key = tree::string(key, "focus key")?;
        let mut ui = self.ui.borrow_mut();
        let window = ui
            .windows
            .iter()
            .find(|w| w.owner == owner)
            .ok_or_else(|| tree::err("window not found"))?;
        let component = window
            .components
            .get(&key)
            .filter(|c| c.kind == Kind::Button && c.enabled)
            .ok_or_else(|| tree::err("invalid focus target"))?;
        let identity = component.identity;
        if !ui.accessible(owner) {
            return Ok(false);
        }
        ui.set_focus(Some(FocusTarget {
            owner,
            identity: Some(identity),
        }));
        Ok(true)
    }

    pub(crate) fn activate_button(
        &self,
        lua: &Lua,
        owner: OwnerId,
        identity: u64,
    ) -> mlua::Result<bool> {
        if !self.is_open(owner) {
            return Ok(false);
        }
        let action = {
            let ui = self.ui.borrow();
            if !ui.accessible(owner) {
                return Ok(false);
            }
            ui.windows
                .iter()
                .find(|w| w.owner == owner)
                .and_then(|w| {
                    w.components.iter().find(|(_, c)| {
                        c.identity == identity && c.kind == Kind::Button && c.enabled
                    })
                })
                .map(|(key, c)| (key.clone(), c.on_click.clone()))
        };
        let Some((key, Some(function))) = action else {
            return Ok(false);
        };
        self.queue_ui_function(lua, owner, Some(key), &function)
    }

    fn queue_ui_function(
        &self,
        lua: &Lua,
        owner: OwnerId,
        key: Option<String>,
        function: &LuaFunction,
    ) -> mlua::Result<bool> {
        let function = tree::load_function(lua, function)?;
        let Some(id) = self.register_once(lua, function, CallbackScope::window(owner, key))? else {
            return Ok(false);
        };
        self.enqueue(lua, id, ())
    }

    /// Called after IME routing by the GPUI adapter. A handled key is consumed
    /// before its callback runs, independently of callback results or failures.
    pub(crate) fn ui_key(&self, lua: &Lua, key: &str) -> mlua::Result<bool> {
        let key = super::keymap::normalize(key).map_err(tree::err)?;
        let (focus, custom, escape_target, modal) = {
            let ui = self.ui.borrow();
            let modal = ui.windows.iter().any(|w| w.modal);
            let Some(focus) = ui.focus.filter(|f| ui.accessible(f.owner)) else {
                return Ok(modal);
            };
            let Some(window) = ui.windows.iter().find(|w| w.owner == focus.owner) else {
                return Ok(modal);
            };
            let escape_target = ui.windows.last().filter(|w| w.escape).map(|w| w.owner);
            (focus, window.keys.get(&key).cloned(), escape_target, modal)
        };
        // Component defaults precede the window's custom/default layer.
        if matches!(key.as_str(), "enter" | "space")
            && let Some(id) = focus.identity
            && self.activate_button(lua, focus.owner, id)?
        {
            return Ok(true);
        }
        if let Some(function) = custom {
            self.queue_ui_function(lua, focus.owner, None, &function)?;
            return Ok(true);
        }
        if matches!(key.as_str(), "tab" | "shift-tab") {
            let mut ui = self.ui.borrow_mut();
            let order = &ui
                .windows
                .iter()
                .find(|w| w.owner == focus.owner)
                .unwrap()
                .focus_order;
            let identity = if order.is_empty() {
                None
            } else {
                let index = focus
                    .identity
                    .and_then(|id| order.iter().position(|v| *v == id));
                Some(
                    order[match index {
                        Some(i) if key == "shift-tab" => (i + order.len() - 1) % order.len(),
                        Some(i) => (i + 1) % order.len(),
                        None if key == "shift-tab" => order.len() - 1,
                        None => 0,
                    }],
                )
            };
            ui.set_focus(Some(FocusTarget {
                owner: focus.owner,
                identity,
            }));
            return Ok(true);
        }
        if key == "escape"
            && let Some(owner) = escape_target
        {
            self.close_owner(owner, CloseReason::Escape)?;
            return Ok(true);
        }
        Ok(modal)
    }
}
