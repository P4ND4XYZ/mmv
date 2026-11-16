//! Viewer-owned Markdown typography requests; never bound to a document identity.
use mlua::{Lua, MultiValue, Table, Value};

use super::{Access, LuaContext, ui::tree};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Request {
    Set(f32),
    Adjust(f64),
    Reset,
}

impl Request {
    pub(crate) fn apply(self, current: f32, initial: f32) -> f32 {
        match self {
            Self::Set(size) => size,
            Self::Adjust(delta) => (f64::from(current) + delta).clamp(8.0, 48.0) as f32,
            Self::Reset => initial,
        }
    }
}

fn number(value: Value) -> mlua::Result<f64> {
    let value = match value {
        Value::Integer(value) => value as f64,
        Value::Number(value) => value,
        _ => return Err(tree::err("expected finite number")),
    };
    if !value.is_finite() {
        return Err(tree::err("expected finite number"));
    }
    Ok(value)
}

pub(crate) fn font_size(value: Value) -> mlua::Result<f32> {
    let size = number(value)?;
    if !(8.0..=48.0).contains(&size) {
        return Err(tree::err("font_size: expected number in 8..48 logical px"));
    }
    Ok(size as f32)
}

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let markdown = lua.create_table()?;
    for name in [
        "set_font_size",
        "adjust_font_size",
        "reset_font_size",
        "get_font_size",
    ] {
        let ctx = context.clone();
        markdown.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let getter = name == "get_font_size";
                ctx.check(if getter {
                    Access::Read
                } else {
                    Access::Mutation
                })?;
                let count = usize::from(matches!(name, "set_font_size" | "adjust_font_size"));
                if args.len() != count {
                    return Err(tree::err("unexpected number of arguments"));
                }
                if getter {
                    return Ok(ctx
                        .markdown
                        .borrow()
                        .map_or(Value::Nil, |size| Value::Number(f64::from(size))));
                }
                let request = match name {
                    "set_font_size" => Request::Set(font_size(args.front().unwrap().clone())?),
                    "adjust_font_size" => Request::Adjust(number(args.front().unwrap().clone())?),
                    _ => Request::Reset,
                };
                if ctx.markdown.borrow().is_none() {
                    return Ok(Value::Boolean(false));
                }
                ctx.request(request)?;
                Ok(Value::Boolean(true))
            })?,
        )?;
    }
    api.set("markdown", markdown)
}

#[cfg(test)]
mod tests;
