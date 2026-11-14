use mlua::{Function, Lua, MultiValue, Table, Value};

use super::{Access, CallbackScope, LuaContext};

/// Shared lexical normalization for app, window and input key tables.
pub(crate) fn normalize(text: &str) -> Result<String, String> {
    let mut rest = text;
    let mut modifiers = [false; 4];
    while rest != "-" {
        let Some((modifier, next)) = rest.split_once('-') else {
            break;
        };
        let index = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" => 0,
            "shift" => 1,
            "alt" => 2,
            "super" => 3,
            _ => return Err("key: unknown modifier or key sequence".into()),
        };
        if modifiers[index] {
            return Err("key: duplicate modifier".into());
        }
        modifiers[index] = true;
        rest = next;
    }
    let lower = rest.to_ascii_lowercase();
    let named = matches!(
        lower.as_str(),
        "enter"
            | "escape"
            | "tab"
            | "space"
            | "backspace"
            | "delete"
            | "insert"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "left"
            | "right"
            | "up"
            | "down"
    ) || (1..=24).any(|number| lower == format!("f{number}"));
    let key = if named {
        lower
    } else {
        let mut chars = rest.chars();
        let Some(ch) = chars.next() else {
            return Err("key: expected one key".into());
        };
        if chars.next().is_some()
            || ch.is_whitespace()
            || ch.is_control()
            || ch.is_ascii_uppercase()
        {
            return Err("key: expected one key (use shift for ASCII uppercase)".into());
        }
        rest.to_owned()
    };
    let mut normalized = String::new();
    for (present, name) in modifiers.into_iter().zip(["ctrl", "shift", "alt", "super"]) {
        if present {
            normalized.push_str(name);
            normalized.push('-');
        }
    }
    normalized.push_str(&key);
    Ok(normalized)
}

fn string(value: Value) -> mlua::Result<String> {
    match value {
        Value::String(value) => Ok(value.to_str()?.to_owned()),
        _ => Err(mlua::Error::runtime("key: expected string")),
    }
}

fn options(value: Option<Value>) -> mlua::Result<()> {
    match value {
        None | Some(Value::Nil) => Ok(()),
        Some(Value::Table(table)) => {
            for pair in table.pairs::<Value, Value>() {
                let (key, value) = pair?;
                if string(key)? != "mode" || string(value)? != "normal" {
                    return Err(mlua::Error::runtime(
                        "keymap: only mode='normal' is supported",
                    ));
                }
            }
            Ok(())
        }
        _ => Err(mlua::Error::runtime("keymap: expected options table")),
    }
}

/// Validate the whole keys table before its caller installs any registrations.
/// Returns canonical keys and rejects collisions after normalization.
pub(crate) fn parse_keys(table: Table) -> mlua::Result<Vec<(String, Function)>> {
    let mut keys = std::collections::BTreeMap::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, callback) = pair?;
        let key = normalize(&string(key)?).map_err(mlua::Error::runtime)?;
        let Value::Function(callback) = callback else {
            return Err(mlua::Error::runtime("keys: expected function"));
        };
        if keys.insert(key, callback).is_some() {
            return Err(mlua::Error::runtime("keys: duplicate normalized key"));
        }
    }
    Ok(keys.into_iter().collect())
}

pub(super) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let keymap = lua.create_table()?;
    let ctx = context.clone();
    keymap.set(
        "set",
        lua.create_function(move |lua, args: MultiValue| {
            let result = (|| {
                ctx.check(Access::InitMutation)?;
                if !(2..=3).contains(&args.len()) {
                    return Err(mlua::Error::runtime(
                        "keymap.set: expected key, callback, optional options",
                    ));
                }
                let mut args = args.into_iter();
                let key =
                    normalize(&string(args.next().unwrap())?).map_err(mlua::Error::runtime)?;
                let Value::Function(function) = args.next().unwrap() else {
                    return Err(mlua::Error::runtime("keymap.set: expected function"));
                };
                options(args.next())?;
                let callback = ctx
                    .register(lua, function, CallbackScope::app())?
                    .expect("app callback");
                let old = ctx.shared.borrow_mut().keys.insert(key, callback);
                if let Some(old) = old {
                    ctx.cancel(old);
                }
                Ok(true)
            })();
            ctx.registration_result(result)
        })?,
    )?;
    let ctx = context.clone();
    keymap.set(
        "remove",
        lua.create_function(move |_, args: MultiValue| {
            let result = (|| {
                ctx.check(Access::InitMutation)?;
                if !(1..=2).contains(&args.len()) {
                    return Err(mlua::Error::runtime(
                        "keymap.remove: expected key, optional options",
                    ));
                }
                let mut args = args.into_iter();
                let key =
                    normalize(&string(args.next().unwrap())?).map_err(mlua::Error::runtime)?;
                options(args.next())?;
                let old = ctx.shared.borrow_mut().keys.remove(&key);
                if let Some(old) = old {
                    ctx.cancel(old);
                }
                Ok(old.is_some())
            })();
            ctx.registration_result(result)
        })?,
    )?;
    api.set("keymap", keymap)
}
