use super::{
    Component, FocusTarget,
    tree::{self, Dimension, Kind, LuaFunction, Node},
};
use crate::lua::{Access, LuaContext, OwnerId, callbacks::CloseReason};
use mlua::{Lua, MultiValue, UserData, UserDataMethods, Value};
use std::{collections::BTreeMap, rc::Rc};

pub(crate) struct WindowData {
    pub owner: OwnerId,
    pub width: Dimension,
    pub height: Dimension,
    pub modal: bool,
    pub escape: bool,
    pub outside_click: bool,
    pub render: LuaFunction,
    pub keys: BTreeMap<String, LuaFunction>,
    pub root: Rc<Node>,
    pub components: BTreeMap<String, Component>,
    pub focus_order: Vec<u64>,
    pub previous_focus: Option<FocusTarget>,
    pub synchronized: bool,
    pub native_pending: bool,
}

#[derive(Clone)]
pub(crate) struct WindowHandle {
    owner: OwnerId,
    context: LuaContext,
}

impl WindowHandle {
    /// E extracts ownership by concrete Rust userdata type, never by a Lua field.
    pub(crate) fn owner(&self) -> OwnerId {
        self.owner
    }
}

impl UserData for WindowHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        for (name, kind) in [("list", Kind::List), ("source", Kind::Source)] {
            methods.add_method(name, move |lua, this, args: MultiValue| {
                if !this.context.is_open(this.owner) {
                    return Ok(Value::Boolean(false));
                }
                this.context.check(Access::Mutation)?;
                if args.len() != 1 {
                    return Err(tree::err("expected one component key"));
                }
                let key = tree::string(args.into_iter().next().unwrap(), "component key")?;
                let ui = this.context.ui.borrow();
                let c = ui
                    .windows
                    .iter()
                    .find(|w| w.owner == this.owner)
                    .and_then(|w| w.components.get(&key))
                    .ok_or_else(|| tree::err("unknown component key"))?;
                if c.kind != kind {
                    return Err(tree::err("component type mismatch"));
                }
                let identity = c.identity;
                drop(ui);
                if kind == Kind::List {
                    Ok(Value::UserData(lua.create_userdata(
                        super::list::ListHandle {
                            owner: this.owner,
                            identity,
                            context: this.context.clone(),
                        },
                    )?))
                } else {
                    Ok(Value::UserData(lua.create_userdata(
                        super::source::SourceControl {
                            owner: this.owner,
                            identity,
                            context: this.context.clone(),
                        },
                    )?))
                }
            });
        }
        methods.add_method("is_open", |_, this, ()| {
            if !this.context.is_open(this.owner) {
                return Ok(false);
            }
            this.context.check(Access::Read)?;
            Ok(true)
        });
        methods.add_method("close", |_, this, _: MultiValue| {
            this.context
                .close_owner(this.owner, CloseReason::Programmatic)
        });
        methods.add_method("focus", |_, this, args: MultiValue| {
            if !this.context.is_open(this.owner) {
                return Ok(false);
            }
            if args.len() != 1 {
                return Err(tree::err("focus expects one key"));
            }
            this.context
                .focus_window(this.owner, args.into_iter().next().unwrap())
        });
    }
}

pub(crate) fn render_result(
    lua: &Lua,
    context: &LuaContext,
    values: MultiValue,
) -> mlua::Result<Rc<Node>> {
    if values.len() != 1 {
        return Err(tree::err(
            "render must return exactly one display description",
        ));
    }
    let value = values.into_iter().next().unwrap();
    let root = tree::rooted_description(lua, context, value)?;
    tree::validate_keys(&root)?;
    Ok(root)
}

pub(crate) fn open(
    lua: &Lua,
    context: &LuaContext,
    args: MultiValue,
) -> mlua::Result<WindowHandle> {
    context.check(Access::Mutation)?;
    if context.ui.borrow().shutting_down {
        return Err(tree::err("application is shutting down"));
    }
    if args.len() != 1 {
        return Err(tree::err("float.open expects one options table"));
    }
    let options = tree::table(args.into_iter().next().unwrap(), "float.open")?;
    tree::fields(
        &options,
        &[
            "width",
            "height",
            "placement",
            "modal",
            "focus",
            "render",
            "dismiss",
            "keys",
            "on_close",
            "on_error",
        ],
    )?;
    let width = tree::dimension(options.raw_get("width")?, false)?;
    let height = tree::dimension(options.raw_get("height")?, false)?;
    if tree::optional_string(&options, "placement")?.is_some_and(|p| p != "center") {
        return Err(tree::err("invalid placement"));
    }
    let modal = tree::boolean(&options, "modal", true)?;
    let focus = tree::optional_string(&options, "focus")?;
    let render = tree::function(lua, &options, "render", true)?.unwrap();
    let on_close = tree::function(lua, &options, "on_close", false)?;
    let on_error = tree::function(lua, &options, "on_error", false)?;
    let (escape, outside_click) = match options.raw_get("dismiss")? {
        Value::Nil => (true, true),
        value => {
            let dismiss = tree::table(value, "dismiss")?;
            tree::fields(&dismiss, &["escape", "outside_click"])?;
            (
                tree::boolean(&dismiss, "escape", true)?,
                tree::boolean(&dismiss, "outside_click", true)?,
            )
        }
    };
    let mut keys = BTreeMap::new();
    if !matches!(options.raw_get::<Value>("keys")?, Value::Nil) {
        for entry in tree::table(options.raw_get("keys")?, "keys")?.pairs::<Value, Value>() {
            let (key, value) = entry?;
            let key =
                crate::lua::keymap::normalize(&tree::string(key, "key")?).map_err(tree::err)?;
            let Value::Function(f) = value else {
                return Err(tree::err("keys values must be functions"));
            };
            if keys
                .insert(key, Rc::new(lua.create_registry_value(f)?))
                .is_some()
            {
                return Err(tree::err("duplicate normalized key"));
            }
        }
    }
    let owner = context.create_owner(
        lua,
        on_close
            .as_ref()
            .map(|f| tree::load_function(lua, f))
            .transpose()?,
        on_error
            .as_ref()
            .map(|f| tree::load_function(lua, f))
            .transpose()?,
    )?;
    let result = (|| {
        let root = context.build_ui(lua, owner, &render, focus.as_deref())?;
        let mut ui = context.ui.borrow_mut();
        // Validate explicit focus before publishing any part of the window.
        fn focusable(node: &Node, key: &str) -> bool {
            (node.key.as_deref() == Some(key) && node.kind.focusable(node.enabled))
                || node.children.iter().any(|c| focusable(c, key))
        }
        if focus.as_ref().is_some_and(|key| !focusable(&root, key)) {
            return Err(tree::err("invalid initial focus target"));
        }
        let previous_focus = ui.focus;
        ui.windows.push(WindowData {
            owner,
            width,
            height,
            modal,
            escape,
            outside_click,
            render,
            keys,
            root: root.clone(),
            components: BTreeMap::new(),
            focus_order: Vec::new(),
            previous_focus,
            synchronized: false,
            native_pending: false,
        });
        drop(ui);
        context.adopt_ui(lua, owner, root)?;
        let mut ui = context.ui.borrow_mut();
        if let Some(key) = focus {
            let identity = ui.windows.last().unwrap().components[&key].identity;
            ui.set_focus(Some(FocusTarget {
                owner,
                identity: Some(identity),
            }));
        } else if modal {
            ui.set_focus(Some(FocusTarget {
                owner,
                identity: None,
            }));
        }
        Ok(WindowHandle {
            owner,
            context: context.clone(),
        })
    })();
    if result.is_err() {
        context.discard_owner(owner);
    }
    result
}
