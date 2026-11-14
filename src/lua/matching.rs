use super::{Access, LuaContext, ui::tree};
use mlua::{Lua, MultiValue, Table, Value};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let matcher = Rc::new(RefCell::new(crate::core::links::LinkMatcher::default()));
    let matching = lua.create_table()?;
    let ctx = context.clone();
    matching.set(
        "fuzzy",
        lua.create_function(move |lua, args: MultiValue| {
            ctx.check(Access::Pure)?;
            if args.len() != 2 {
                return Err(tree::err("fuzzy expects query and text"));
            }
            let mut args = args.into_iter();
            let query = tree::string(args.next().unwrap(), "query")?;
            let text = tree::string(args.next().unwrap(), "text")?;
            let Some((score, ranges)) = matcher.borrow_mut().fuzzy(&query, &text) else {
                return Ok(Value::Nil);
            };
            let result = lua.create_table()?;
            result.set("score", score)?;
            let output = lua.create_table()?;
            for (index, range) in ranges.into_iter().enumerate() {
                let item = lua.create_table()?;
                item.set("start", range.start)?;
                item.set("stop", range.end)?;
                output.raw_set(index + 1, item)?;
            }
            result.set("ranges", output)?;
            Ok(Value::Table(result))
        })?,
    )?;
    api.set("match", matching)
}
