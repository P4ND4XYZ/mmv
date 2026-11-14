use super::{Access, LuaContext, ui::tree};
use mlua::{Lua, MultiValue, Table, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Info,
    Warning,
    Error,
}
pub(crate) struct Notice {
    pub message: String,
    pub level: Level,
}

pub(crate) fn options(lua: &Lua, value: Value) -> mlua::Result<Table> {
    match value {
        Value::Nil => lua.create_table(),
        value => tree::table(value, "options"),
    }
}
pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let ctx = context.clone();
    api.set(
        "notify",
        lua.create_function(move |lua, args: MultiValue| {
            ctx.check(Access::Mutation)?;
            if args.is_empty() || args.len() > 2 {
                return Err(tree::err("notify expects message and optional options"));
            }
            let mut args = args.into_iter();
            let message = tree::string(args.next().unwrap(), "message")?;
            let options = options(lua, args.next().unwrap_or(Value::Nil))?;
            tree::fields(&options, &["level"])?;
            let level = match tree::optional_string(&options, "level")?
                .as_deref()
                .unwrap_or("info")
            {
                "info" => Level::Info,
                "warning" => Level::Warning,
                "error" => Level::Error,
                _ => return Err(tree::err("invalid notification level")),
            };
            ctx.request(Notice { message, level })?;
            Ok(true)
        })?,
    )
}
