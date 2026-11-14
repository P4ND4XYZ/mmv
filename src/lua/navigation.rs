use super::{
    Access, CallbackScope, LuaContext,
    callbacks::CallbackId,
    document::{self, TargetHandle},
    notify,
    ui::tree,
};
use mlua::{Lua, MultiValue, Table, Value};

pub(crate) struct Delivery {
    pub callback: Option<CallbackId>,
    // Remains true even when a closed owner prevents callback registration.
    pub handler: bool,
}
pub(crate) struct OpenRequest {
    pub target: TargetHandle,
    pub delivery: Delivery,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detail {
    pub code: &'static str,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub ok: bool,
    pub detail: Option<Detail>,
}
impl Outcome {
    pub(crate) fn success() -> Self {
        Self {
            ok: true,
            detail: None,
        }
    }
    pub(crate) fn failure(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: Some(Detail {
                code,
                message: message.into(),
            }),
        }
    }
    pub(crate) fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: Some(Detail {
                code,
                message: message.into(),
            }),
        }
    }
    pub(crate) fn value(&self, lua: &Lua) -> mlua::Result<Table> {
        let result = lua.create_table()?;
        result.set("ok", self.ok)?;
        if let Some(detail) = &self.detail {
            let table = if self.ok {
                lua.create_table()?
            } else {
                result.clone()
            };
            table.set("code", detail.code)?;
            table.set("message", detail.message.clone())?;
            if self.ok {
                result.set("warning", table)?;
            }
        }
        Ok(result)
    }
}

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let navigation = lua.create_table()?;
    let ctx = context.clone();
    navigation.set(
        "open",
        lua.create_function(move |lua, args: MultiValue| {
            ctx.check(Access::Mutation)?;
            if args.is_empty() || args.len() > 2 {
                return Err(tree::err("open expects target and optional options"));
            }
            let mut args = args.into_iter();
            let target = match args.next().unwrap() {
                Value::UserData(data) => data
                    .borrow::<TargetHandle>()
                    .map(|target| target.clone())
                    .map_err(|_| tree::err("expected navigation target"))?,
                _ => return Err(tree::err("expected navigation target")),
            };
            let options = notify::options(lua, args.next().unwrap_or(Value::Nil))?;
            tree::fields(&options, &["owner", "on_complete"])?;
            let owner = document::owner(options.raw_get("owner")?, false)?;
            let function = match options.raw_get("on_complete")? {
                Value::Nil => None,
                Value::Function(f) => Some(f),
                _ => return Err(tree::err("on_complete: expected function")),
            };
            let handler = function.is_some();
            let callback = function
                .map(|function| {
                    ctx.register_once(
                        lua,
                        function,
                        owner.map_or_else(CallbackScope::app, |owner| {
                            CallbackScope::window(owner, None)
                        }),
                    )
                })
                .transpose()?
                .flatten();
            let stale = !target.is_current(&ctx);
            ctx.request(OpenRequest {
                target,
                delivery: Delivery { callback, handler },
                stale,
            })?;
            Ok(())
        })?,
    )?;
    api.set("navigation", navigation)
}

#[cfg(test)]
mod tests;
