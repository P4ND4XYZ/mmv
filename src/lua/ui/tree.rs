use std::{collections::BTreeSet, rc::Rc};

use crate::config::Group;
use mlua::{Function, Lua, RegistryKey, Table, UserData, Value};

pub(crate) type LuaFunction = Rc<RegistryKey>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Dimension {
    Auto,
    Pixels(f64),
    Ratio(f64),
}

impl Dimension {
    pub(crate) fn resolve(self, available: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Pixels(n) => Some(n.min(f64::from(f32::MAX)) as f32),
            Self::Ratio(n) => Some((f64::from(available) * n) as f32),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Row,
    Column,
    Text,
    Button,
    List,
    Input,
    Source,
}
impl Kind {
    pub(crate) fn focusable(self, enabled: bool) -> bool {
        matches!(self, Self::List | Self::Input | Self::Source) || (self == Self::Button && enabled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone)]
pub(crate) struct Highlight {
    pub range: std::ops::Range<usize>,
    pub bold: Option<bool>,
    pub underline: Option<bool>,
    pub hl: Option<Group>,
}

#[derive(Clone)]
pub(crate) struct Node {
    pub kind: Kind,
    pub key: Option<String>,
    pub width: Dimension,
    pub height: Dimension,
    pub grow: f32,
    pub padding: f32,
    pub gap: f32,
    pub align: Align,
    pub hl: Option<Group>,
    pub children: Vec<Rc<Node>>,
    pub text: String,
    pub wrap: bool,
    pub highlights: Vec<Highlight>,
    pub enabled: bool,
    // Only materialized host trees own callback roots. Description metadata
    // always leaves this empty; its Lua references live in userdata user values.
    pub on_click: Option<LuaFunction>,
    pub list: Option<super::list::List>,
    pub input: Option<super::input::Input>,
    pub source: Option<super::source::Source>,
}

#[derive(Clone)]
pub(crate) struct Description(pub Rc<Node>);
impl UserData for Description {}

pub(crate) fn err(message: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(message.into())
}

pub(crate) fn table(value: Value, label: &str) -> mlua::Result<Table> {
    match value {
        Value::Table(table) if table.metatable().is_none() => Ok(table),
        _ => Err(err(format!("{label}: expected a plain table"))),
    }
}

pub(crate) fn fields(table: &Table, allowed: &[&str]) -> mlua::Result<()> {
    for entry in table.clone().pairs::<Value, Value>() {
        let (key, _) = entry?;
        let key = string(key, "option")?;
        if !allowed.contains(&key.as_str()) {
            return Err(err(format!("unknown option: {key}")));
        }
    }
    Ok(())
}

pub(crate) fn string(value: Value, label: &str) -> mlua::Result<String> {
    match value {
        Value::String(s) => s
            .to_str()
            .map(|s| s.to_string())
            .map_err(|_| err(format!("{label}: expected UTF-8 string"))),
        _ => Err(err(format!("{label}: expected string"))),
    }
}

pub(crate) fn optional_string(table: &Table, key: &str) -> mlua::Result<Option<String>> {
    match table.raw_get(key)? {
        Value::Nil => Ok(None),
        value => string(value, key).map(Some),
    }
}

pub(crate) fn boolean(table: &Table, key: &str, default: bool) -> mlua::Result<bool> {
    match table.raw_get(key)? {
        Value::Nil => Ok(default),
        Value::Boolean(value) => Ok(value),
        _ => Err(err(format!("{key}: expected boolean"))),
    }
}

pub(crate) fn function(
    lua: &Lua,
    table: &Table,
    key: &str,
    required: bool,
) -> mlua::Result<Option<LuaFunction>> {
    match table.raw_get(key)? {
        Value::Nil if !required => Ok(None),
        Value::Function(f) => Ok(Some(Rc::new(lua.create_registry_value(f)?))),
        _ => Err(err(format!("{key}: expected function"))),
    }
}

pub(crate) fn number(value: Value, label: &str) -> mlua::Result<f64> {
    let n = match value {
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => return Err(err(format!("{label}: expected number"))),
    };
    if !n.is_finite() {
        return Err(err(format!("{label}: expected finite number")));
    }
    Ok(n)
}

fn nonnegative(table: &Table, key: &str) -> mlua::Result<f32> {
    let value = match table.raw_get(key)? {
        Value::Nil => 0.,
        value => number(value, key)?,
    };
    if value < 0. {
        return Err(err(format!("{key}: expected nonnegative number")));
    }
    Ok(value.min(f64::from(f32::MAX)) as f32)
}

pub(crate) fn dimension(value: Value, auto: bool) -> mlua::Result<Dimension> {
    match value {
        Value::Nil if auto => Ok(Dimension::Auto),
        Value::String(ref s) if auto && s.as_bytes().as_ref() == b"auto" => Ok(Dimension::Auto),
        Value::Table(_) => {
            let options = table(value, "dimension")?;
            fields(&options, &["ratio"])?;
            let n = number(options.raw_get("ratio")?, "ratio")?;
            if n <= 0. || n > 1. {
                return Err(err("ratio must be > 0 and <= 1"));
            }
            Ok(Dimension::Ratio(n))
        }
        value => {
            let n = number(value, "dimension")?;
            if n <= 0. {
                return Err(err("dimension must be positive"));
            }
            Ok(Dimension::Pixels(n))
        }
    }
}

pub(crate) fn array(value: Value, label: &str) -> mlua::Result<Vec<Value>> {
    let table = table(value, label)?;
    let mut count = 0;
    let mut max = 0;
    for pair in table.clone().pairs::<Value, Value>() {
        let (key, _) = pair?;
        match key {
            Value::Integer(i) if i > 0 => {
                count += 1;
                max = max.max(i);
            }
            _ => return Err(err(format!("{label}: expected a dense array"))),
        }
    }
    if max != count {
        return Err(err(format!("{label}: expected a dense array")));
    }
    (1..=count).map(|i| table.raw_get(i)).collect()
}

pub(crate) fn group(name: &str) -> mlua::Result<Group> {
    use Group::*;
    Ok(match name {
        "Normal" => Normal,
        "Border" => Border,
        "Search" => Search,
        "SearchCurrent" => SearchCurrent,
        "StatusLine" => StatusLine,
        "CommandLine" => CommandLine,
        "ModeNormal" => ModeNormal,
        "ModeSearch" => ModeSearch,
        "ModeCommand" => ModeCommand,
        "Heading1" => Heading1,
        "Heading2" => Heading2,
        "Heading3" => Heading3,
        "Heading4" => Heading4,
        "Heading5" => Heading5,
        "Heading6" => Heading6,
        "Link" => Link,
        "CodeBlock" => CodeBlock,
        "Quote" => Quote,
        "Panel" => Panel,
        "PanelSelected" => PanelSelected,
        "Error" => Error,
        "Warning" => Warning,
        _ => return Err(err(format!("unknown highlight group: {name}"))),
    })
}
fn highlight_group(table: &Table) -> mlua::Result<Option<Group>> {
    optional_string(table, "hl")?
        .as_deref()
        .map(group)
        .transpose()
}

pub(crate) fn description(value: Value) -> mlua::Result<Rc<Node>> {
    match value {
        Value::UserData(data) => Ok(data
            .borrow::<Description>()
            .map_err(|_| err("expected display description"))?
            .0
            .clone()),
        _ => Err(err("expected one display description")),
    }
}

/// Materialize owner-lifetime host roots from Lua-GC-visible description edges.
/// Never store this rooted tree back inside a Description userdata.
pub(crate) fn rooted_description(
    lua: &Lua,
    context: &crate::lua::LuaContext,
    value: Value,
) -> mlua::Result<Rc<Node>> {
    let mut node = (*description(value.clone())?).clone();
    let Value::UserData(data) = value else {
        unreachable!()
    };
    let refs = data.user_value::<Table>()?;
    node.children = array(refs.raw_get("children")?, "description children")?
        .into_iter()
        .map(|child| rooted_description(lua, context, child))
        .collect::<mlua::Result<_>>()?;
    if node.kind == Kind::Button {
        node.on_click = Some(Rc::new(
            lua.create_registry_value(refs.raw_get::<Function>("on_click")?)?,
        ));
    }
    let callback =
        |name: &str| -> mlua::Result<Option<LuaFunction>> { function(lua, &refs, name, false) };
    if let Some(list) = &mut node.list {
        let key = node.key.clone().unwrap();
        let materialized = (|| -> mlua::Result<Vec<Rc<Node>>> {
            let old = context.render_component(&key);
            list.selected = list.normalize(
                old.as_ref().and_then(|c| c.node.list.as_ref()),
                super::list::Id::parse(list.state.get(lua)?)?,
            );
            let f = refs.raw_get::<Function>("render_item")?;
            let mut children = Vec::new();
            for (item, id) in array(refs.raw_get("items")?, "items")?
                .into_iter()
                .zip(&list.ids)
            {
                let item = crate::lua::state::copy(lua, context, item)?;
                let values = context.render_item::<mlua::MultiValue>(
                    key.clone(),
                    &f,
                    (item, list.selected.as_ref() == Some(id)),
                )?;
                if values.len() != 1 {
                    return Err(err(
                        "render_item must return exactly one display description",
                    ));
                }
                children.push(rooted_description(
                    lua,
                    context,
                    values.into_iter().next().unwrap(),
                )?);
            }
            if list.ids.is_empty() {
                let empty = refs.raw_get::<Value>("empty")?;
                if !matches!(empty, Value::Nil) {
                    children.push(rooted_description(lua, context, empty)?);
                }
            }
            Ok(children)
        })();
        node.children = materialized.map_err(|error| {
            mlua::Error::external(crate::lua::callbacks::RenderFailure { error, key })
        })?;
        list.changed = callback("on_selection_changed")?;
        list.confirm = callback("on_confirm")?;
    }
    if let Some(input) = &mut node.input {
        input.changed = callback("on_change")?;
        input.submit = callback("on_submit")?;
        input.keys = key_roots(lua, refs.raw_get("keys")?)?;
    }
    Ok(Rc::new(node))
}

pub(crate) fn key_values(value: Value) -> mlua::Result<Table> {
    let t = table(value, "keys")?;
    let mut seen = BTreeSet::new();
    for pair in t.clone().pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = crate::lua::keymap::normalize(&string(key, "key")?).map_err(err)?;
        if !matches!(value, Value::Function(_)) {
            return Err(err("keys values must be functions"));
        }
        if !seen.insert(key) {
            return Err(err("duplicate normalized key"));
        }
    }
    Ok(t)
}
pub(crate) fn key_roots(
    lua: &Lua,
    value: Value,
) -> mlua::Result<std::collections::BTreeMap<String, LuaFunction>> {
    let mut keys = std::collections::BTreeMap::new();
    if matches!(value, Value::Nil) {
        return Ok(keys);
    }
    for pair in key_values(value)?.pairs::<Value, Function>() {
        let (key, f) = pair?;
        keys.insert(
            crate::lua::keymap::normalize(&string(key, "key")?).map_err(err)?,
            Rc::new(lua.create_registry_value(f)?),
        );
    }
    Ok(keys)
}

pub(crate) fn build(
    lua: &Lua,
    context: &crate::lua::LuaContext,
    kind: Kind,
    value: Value,
) -> mlua::Result<mlua::AnyUserData> {
    let options = table(value, "component")?;
    let mut allowed = vec!["key", "width", "height", "grow", "padding", "hl"];
    match kind {
        Kind::Row | Kind::Column => allowed.extend(["children", "gap", "align"]),
        Kind::Text => allowed.extend(["text", "wrap", "highlights"]),
        Kind::Button => allowed.extend(["label", "enabled", "on_click"]),
        Kind::List => allowed.extend([
            "items",
            "selection",
            "render_item",
            "empty",
            "reset_on",
            "wrap_navigation",
            "on_selection_changed",
            "on_confirm",
        ]),
        Kind::Input => allowed.extend(["bind", "placeholder", "on_change", "on_submit", "keys"]),
        Kind::Source => allowed.extend(["source", "highlight", "wrap"]),
    }
    fields(&options, &allowed)?;
    let key = optional_string(&options, "key")?;
    if matches!(kind, Kind::Button | Kind::List | Kind::Input | Kind::Source) && key.is_none() {
        return Err(err("stateful component key is required"));
    }
    let child_values = if matches!(kind, Kind::Row | Kind::Column) {
        array(options.raw_get("children")?, "children")?
    } else {
        Vec::new()
    };
    let mut list = None;
    let mut input = None;
    let mut source = None;
    let refs = lua.create_table()?;
    let state = |name: &str| -> mlua::Result<crate::lua::state::StateHandle> {
        match options.raw_get::<Value>(name)? {
            Value::UserData(d) => Ok(d
                .borrow::<crate::lua::state::StateHandle>()
                .map_err(|_| err("expected state handle"))?
                .clone()),
            _ => Err(err("expected state handle")),
        }
    };
    let keep_callback = |name: &str, required: bool| -> mlua::Result<()> {
        let value = options.raw_get::<Value>(name)?;
        if !matches!(value, Value::Function(_)) && !(matches!(value, Value::Nil) && !required) {
            return Err(err(format!("{name}: expected function")));
        }
        refs.raw_set(name, value)
    };
    if kind == Kind::List {
        context.check(crate::lua::Access::RenderOnly)?;
        let selection = state("selection")?;
        let mut ids = Vec::new();
        let mut items = Vec::new();
        for item in array(options.raw_get("items")?, "items")? {
            let item = table(item, "item")?;
            let copied = crate::lua::state::copy(lua, context, Value::Table(item))?;
            let Value::Table(t) = &copied else {
                unreachable!()
            };
            let id = super::list::Id::parse(t.raw_get("id")?)?
                .ok_or_else(|| err("item.id is required"))?;
            if ids.contains(&id) {
                return Err(err("duplicate item ID"));
            }
            ids.push(id);
            items.push(copied);
        }
        let reset = options.raw_get::<Value>("reset_on")?;
        if !matches!(
            &reset,
            Value::Nil | Value::Boolean(_) | Value::String(_) | Value::Integer(_)
        ) && !matches!(&reset,Value::Number(n) if n.is_finite())
        {
            return Err(err("invalid reset_on"));
        }
        keep_callback("render_item", true)?;
        keep_callback("on_selection_changed", false)?;
        keep_callback("on_confirm", false)?;
        super::list::Id::parse(selection.get(lua)?)?;
        refs.raw_set("items", lua.create_sequence_from(items)?)?;
        let empty = options.raw_get::<Value>("empty")?;
        if !matches!(empty, Value::Nil) {
            description(empty.clone())?;
            refs.raw_set("empty", empty)?;
        }
        list = Some(super::list::List {
            state: selection,
            ids,
            selected: None,
            reset,
            wrap: boolean(&options, "wrap_navigation", true)?,
            reveal: Default::default(),
            notification_revision: Default::default(),
            notified: Default::default(),
            changed: None,
            confirm: None,
        });
    }
    if kind == Kind::Input {
        let bind = state("bind")?;
        string(bind.get(lua)?, "input bind")?;
        keep_callback("on_change", false)?;
        keep_callback("on_submit", false)?;
        let keys = options.raw_get::<Value>("keys")?;
        if !matches!(keys, Value::Nil) {
            refs.raw_set("keys", key_values(keys)?)?;
        }
        input = Some(super::input::Input {
            state: bind,
            placeholder: optional_string(&options, "placeholder")?.unwrap_or_default(),
            changed: None,
            submit: None,
            keys: Default::default(),
        });
    }
    if kind == Kind::Source {
        if !boolean(&options, "wrap", true)? {
            return Err(err("source.wrap must be true"));
        }
        let s = super::source::source(options.raw_get("source")?)?;
        source = Some(super::source::Source {
            highlight: super::source::range(options.raw_get("highlight")?, &s)?,
            source: s,
            motions: Default::default(),
        });
    }
    let children = child_values
        .iter()
        .cloned()
        .map(description)
        .collect::<mlua::Result<_>>()?;
    // Lua GC must see callback -> controller -> description -> callback cycles.
    // Child userdata, not only their Rust metadata, are part of that Lua graph.
    refs.raw_set("children", lua.create_sequence_from(child_values)?)?;
    if kind == Kind::Button {
        let Value::Function(callback) = options.raw_get("on_click")? else {
            return Err(err("on_click: expected function"));
        };
        refs.raw_set("on_click", callback)?;
    }
    let text = match kind {
        Kind::Text => string(options.raw_get("text")?, "text")?,
        Kind::Button => string(options.raw_get("label")?, "label")?,
        _ => String::new(),
    };
    let align = match optional_string(&options, "align")?
        .as_deref()
        .unwrap_or("stretch")
    {
        "start" => Align::Start,
        "center" => Align::Center,
        "end" => Align::End,
        "stretch" => Align::Stretch,
        _ => return Err(err("invalid align")),
    };
    let mut highlights = Vec::new();
    if let value @ Value::Table(_) = options.raw_get("highlights")? {
        for value in array(value, "highlights")? {
            let h = table(value, "highlight")?;
            fields(&h, &["range", "bold", "underline", "hl"])?;
            let r = table(h.raw_get("range")?, "range")?;
            fields(&r, &["start", "stop"])?;
            let index = |key| -> mlua::Result<usize> {
                match r.raw_get(key)? {
                    Value::Integer(n) if n >= 0 => {
                        usize::try_from(n).map_err(|_| err("highlight index out of range"))
                    }
                    _ => Err(err("highlight indices must be Lua integers")),
                }
            };
            let start = index("start")?;
            let stop = index("stop")?;
            if start > stop
                || stop > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(stop)
            {
                return Err(err("invalid UTF-8 highlight range"));
            }
            let optional_bool = |key| match h.raw_get(key)? {
                Value::Nil => Ok(None),
                Value::Boolean(b) => Ok(Some(b)),
                _ => Err(err("highlight decoration must be boolean")),
            };
            highlights.push(Highlight {
                range: start..stop,
                bold: optional_bool("bold")?,
                underline: optional_bool("underline")?,
                hl: highlight_group(&h)?,
            });
        }
    } else if !matches!(options.raw_get::<Value>("highlights")?, Value::Nil) {
        return Err(err("highlights must be an array"));
    }
    let result = lua.create_userdata(Description(Rc::new(Node {
        kind,
        key,
        width: dimension(options.raw_get("width")?, true)?,
        height: dimension(options.raw_get("height")?, true)?,
        grow: nonnegative(&options, "grow")?,
        padding: nonnegative(&options, "padding")?,
        gap: nonnegative(&options, "gap")?,
        align,
        hl: highlight_group(&options)?,
        children,
        text,
        wrap: boolean(&options, "wrap", false)?,
        highlights,
        enabled: boolean(&options, "enabled", true)?,
        on_click: None,
        list,
        input,
        source,
    })))?;
    result.set_user_value(refs)?;
    Ok(result)
}

pub(crate) fn validate_keys(root: &Node) -> mlua::Result<()> {
    fn visit(node: &Node, keys: &mut BTreeSet<String>) -> mlua::Result<()> {
        if let Some(key) = &node.key
            && !keys.insert(key.clone())
        {
            return Err(err(format!("duplicate component key: {key}")));
        }
        for child in &node.children {
            visit(child, keys)?;
        }
        Ok(())
    }
    visit(root, &mut BTreeSet::new())
}

pub(crate) fn load_function(lua: &Lua, key: &LuaFunction) -> mlua::Result<Function> {
    lua.registry_value(key)
}
