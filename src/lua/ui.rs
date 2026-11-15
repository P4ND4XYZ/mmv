pub(crate) mod input;
pub(crate) mod list;
pub(crate) mod source;
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
    pub node: Rc<Node>,
}

#[derive(Default)]
pub(crate) struct UiState {
    pub dirty: bool,
    pub windows: Vec<WindowData>,
    pub focus: Option<FocusTarget>,
    pub focus_revision: u64,
    pub restoring_focus: bool,
    pub shutting_down: bool,
    pub defer_changes: BTreeMap<OwnerId, usize>,
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
        self.defer_changes.remove(&owner);
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
                        .any(|c| c.identity == id && c.kind.focusable(c.enabled))
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
                        node: Rc::new(node.clone()),
                    },
                );
                if node.kind.focusable(node.enabled) {
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
                        .filter(|c| c.kind.focusable(c.enabled))
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
        ("list", Kind::List),
        ("input", Kind::Input),
        ("source", Kind::Source),
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
                tree::build(lua, &ctx, kind, value).map_err(|error| {
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
        if self
            .ui
            .borrow()
            .defer_changes
            .values()
            .any(|count| *count > 0)
        {
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
            let result = self
                .build_ui(lua, owner, &render, None)
                .and_then(|root| self.adopt_ui(lua, owner, root));
            match result {
                Ok(()) => (),
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
            .filter(|c| c.kind.focusable(c.enabled))
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
        let component = self
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == focus.owner)
            .and_then(|w| {
                w.components
                    .values()
                    .find(|c| Some(c.identity) == focus.identity)
            })
            .cloned();
        if let Some(c) = component {
            if let Some(input) = &c.node.input {
                if let Some(f) = input.keys.get(&key) {
                    self.queue_ui_function(lua, focus.owner, c.node.key.clone(), f)?;
                    return Ok(true);
                }
                if key == "enter" {
                    if let Some(f) = &input.submit {
                        self.queue_ui_function(lua, focus.owner, c.node.key.clone(), f)?;
                    }
                    return Ok(true);
                }
                if input::native_key(&key) {
                    return Ok(false);
                }
            }
            if c.kind == Kind::List {
                let h = list::ListHandle {
                    owner: focus.owner,
                    identity: c.identity,
                    context: self.clone(),
                };
                match key.as_str() {
                    "down" | "ctrl-n" => {
                        h.step(lua, true)?;
                        return Ok(true);
                    }
                    "up" | "ctrl-p" => {
                        h.step(lua, false)?;
                        return Ok(true);
                    }
                    "enter" => {
                        h.confirm(lua)?;
                        return Ok(true);
                    }
                    _ => (),
                }
            }
            if c.kind == Kind::Source {
                let motion = match key.as_str() {
                    "up" => Some(source::Motion::Lines(-1.)),
                    "down" => Some(source::Motion::Lines(1.)),
                    "pageup" => Some(source::Motion::Pages(-1.)),
                    "pagedown" => Some(source::Motion::Pages(1.)),
                    "home" => Some(source::Motion::Edge(false)),
                    "end" => Some(source::Motion::Edge(true)),
                    _ => None,
                };
                if let Some(m) = motion {
                    source::SourceControl {
                        owner: focus.owner,
                        identity: c.identity,
                        context: self.clone(),
                    }
                    .motion(m)?;
                    return Ok(true);
                }
            }
        }
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

    pub(crate) fn render_component(&self, key: &str) -> Option<Component> {
        let owner = self.shared.borrow().frames.last().and_then(|f| f.owner)?;
        self.ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == owner)?
            .components
            .get(key)
            .cloned()
    }

    pub(crate) fn build_ui(
        &self,
        lua: &Lua,
        owner: OwnerId,
        render: &LuaFunction,
        initial_focus: Option<&str>,
    ) -> mlua::Result<Rc<Node>> {
        let build = || {
            self.check(Access::Mutation)?;
            let _frame = self.enter(super::callbacks::Frame {
                phase: Phase::Render,
                owner: Some(owner),
                key: None,
                kind: super::callbacks::Kind::Normal,
            });
            let values = tree::load_function(lua, render)?.call::<MultiValue>(())?;
            window::render_result(lua, self, values)
        };
        fn collect(
            node: &Node,
            claims: &mut Vec<(super::state::StateHandle, u64, super::state::BindingType)>,
        ) {
            if let Some(l) = &node.list {
                claims.push((l.state.clone(), 0, super::state::BindingType::SelectionId));
            }
            if let Some(i) = &node.input {
                claims.push((i.state.clone(), 0, super::state::BindingType::String));
            }
            for child in &node.children {
                collect(child, claims)
            }
        }
        fn corrections(
            lua: &Lua,
            node: &Node,
            result: &mut Vec<(super::state::StateHandle, Value, Value)>,
        ) -> mlua::Result<()> {
            if let Some(l) = &node.list {
                let old = l.state.get(lua)?;
                if list::Id::parse(old.clone())? != l.selected {
                    result.push((l.state.clone(), old, list::Id::value(&l.selected, lua)?));
                }
            }
            for c in &node.children {
                corrections(lua, c, result)?;
            }
            Ok(())
        }
        // Normalize dependent lists transactionally before publishing any tree
        // or notification. Nested composition can require several passes.
        let mut originals: Vec<(super::state::StateHandle, Value)> = Vec::new();
        type Configuration = (
            Vec<(super::state::StateHandle, Value)>,
            Vec<(Kind, Option<String>, Vec<list::Id>)>,
        );
        fn shape(node: &Node, result: &mut Vec<(Kind, Option<String>, Vec<list::Id>)>) {
            result.push((
                node.kind,
                node.key.clone(),
                node.list
                    .as_ref()
                    .map(|l| l.ids.clone())
                    .unwrap_or_default(),
            ));
            for child in &node.children {
                shape(child, result)
            }
        }
        fn focusable(node: &Node, key: &str) -> bool {
            (node.key.as_deref() == Some(key) && node.kind.focusable(node.enabled))
                || node.children.iter().any(|c| focusable(c, key))
        }
        let mut seen: Vec<Configuration> = Vec::new();
        let result = (|| loop {
            let root = build()?;
            let mut claims = Vec::new();
            collect(&root, &mut claims);
            super::state::StateHandle::check_tree_bindings(lua, owner, &claims)?;
            let mut changes = Vec::new();
            corrections(lua, &root, &mut changes)?;
            if changes.is_empty() {
                if initial_focus.is_some_and(|key| !focusable(&root, key)) {
                    return Err(tree::err("invalid initial focus target"));
                }
                return Ok(root);
            }
            let mut values = claims
                .iter()
                .map(|(s, _, _)| Ok((s.clone(), s.get(lua)?)))
                .collect::<mlua::Result<Vec<_>>>()?;
            for (state, _) in &originals {
                if !values.iter().any(|(s, _)| s.same_state(state)) {
                    values.push((state.clone(), state.get(lua)?));
                }
            }
            let mut structure = Vec::new();
            shape(&root, &mut structure);
            if seen.iter().any(|(previous, previous_shape)| {
                *previous_shape == structure
                    && previous.len() == values.len()
                    && previous
                        .iter()
                        .zip(&values)
                        .all(|((a, av), (b, bv))| a.same_state(b) && av == bv)
            }) {
                return Err(tree::err("selection-dependent render did not stabilize"));
            }
            seen.push((values, structure));
            for (state, old, value) in changes {
                if !originals.iter().any(|(s, _)| s.same_state(&state)) {
                    originals.push((state.clone(), old));
                }
                state.replace_display_value(lua, value)?;
            }
        })();
        if result.is_err() {
            for (state, old) in originals {
                state.replace_display_value(lua, old)?;
            }
        } else if !originals.is_empty() {
            let mut ui = self.ui.borrow_mut();
            ui.dirty |= !ui.windows.is_empty();
            for window in &mut ui.windows {
                window.synchronized = false;
            }
        }
        result
    }

    pub(crate) fn adopt_ui(&self, lua: &Lua, owner: OwnerId, root: Rc<Node>) -> mlua::Result<()> {
        let old = self
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == owner)
            .map(|w| w.components.clone())
            .unwrap_or_default();
        for c in old.values() {
            if let Some(l) = &c.node.list {
                l.state.unbind(owner, c.identity)
            }
            if let Some(i) = &c.node.input {
                i.state.unbind(owner, c.identity)
            }
        }
        fn carry(node: &Node, old: &BTreeMap<String, Component>) -> Rc<Node> {
            let mut n = node.clone();
            if let Some(list) = &mut n.list
                && let Some(previous) = n
                    .key
                    .as_ref()
                    .and_then(|k| old.get(k))
                    .and_then(|c| c.node.list.as_ref())
            {
                list.reveal = previous.reveal.clone();
                list.notification_revision = previous.notification_revision.clone();
                list.notified = previous.notified.clone();
            }
            if let Some(s) = &mut n.source
                && let Some(previous) = n
                    .key
                    .as_ref()
                    .and_then(|k| old.get(k))
                    .and_then(|c| c.node.source.as_ref())
                    .filter(|p| p.source.same_source(&s.source))
            {
                s.motions = previous.motions.clone();
            }
            n.children = n.children.iter().map(|c| carry(c, old)).collect();
            Rc::new(n)
        }
        self.ui.borrow_mut().adopt(owner, carry(&root, &old));
        let components = self
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == owner)
            .unwrap()
            .components
            .clone();
        fn visit(
            context: &LuaContext,
            lua: &Lua,
            owner: OwnerId,
            node: &Node,
            components: &BTreeMap<String, Component>,
            old: &BTreeMap<String, Component>,
        ) -> mlua::Result<()> {
            if let Some(key) = &node.key {
                let c = &components[key];
                if let Some(l) = &c.node.list {
                    l.state.bind(
                        lua,
                        owner,
                        c.identity,
                        crate::lua::state::BindingType::SelectionId,
                    )?;
                    let previous = old
                        .get(key)
                        .filter(|p| p.identity == c.identity)
                        .and_then(|p| p.node.list.as_ref());
                    if (previous.is_none_or(|p| p.selected != l.selected) || !l.notified.get())
                        && let Some(f) = &l.changed
                    {
                        let revision = l.notification_revision.get().wrapping_add(1);
                        l.notification_revision.set(revision);
                        l.notified.set(false);
                        let expected = l.selected.clone();
                        let identity = c.identity;
                        let ctx = context.clone();
                        let argument = list::Id::value(&expected, lua)?;
                        let valid = lua.create_function(move |lua, (): ()| {
                            let current = ctx
                                .ui
                                .borrow()
                                .windows
                                .iter()
                                .find(|w| w.owner == owner && w.synchronized)
                                .and_then(|w| {
                                    w.components.values().find(|c| c.identity == identity)
                                })
                                .and_then(|c| c.node.list.clone());
                            let Some(current) = current else {
                                return Ok(false);
                            };
                            if current.notification_revision.get() != revision
                                || current.selected != expected
                                || list::Id::parse(current.state.get(lua)?)? != expected
                            {
                                return Ok(false);
                            }
                            current.notified.set(true);
                            Ok(true)
                        })?;
                        // Callback edges must remain visible to Lua GC, including
                        // cancellation before this one-shot event is dispatched.
                        let factory=lua.load("return function(valid,callback,id) return function() if valid() then callback(id) end end end").eval::<mlua::Function>()?;
                        let wrapper = factory.call::<mlua::Function>((
                            valid,
                            tree::load_function(lua, f)?,
                            argument,
                        ))?;
                        let wrapper = Rc::new(lua.create_registry_value(wrapper)?);
                        context.queue_component(lua, owner, Some(key.clone()), &wrapper, ())?;
                    }
                }
                if let Some(i) = &c.node.input {
                    i.state.bind(
                        lua,
                        owner,
                        c.identity,
                        crate::lua::state::BindingType::String,
                    )?;
                }
            }
            for c in &node.children {
                visit(context, lua, owner, c, components, old)?;
            }
            Ok(())
        }
        self.ui
            .borrow_mut()
            .windows
            .iter_mut()
            .find(|w| w.owner == owner)
            .unwrap()
            .synchronized = true;
        visit(self, lua, owner, &root, &components, &old)
    }
}
