use std::{cell::RefCell, rc::Rc};

use mlua::{Lua, MultiValue, Table, Value};

use super::{Color, ColorScheme, Config, Group, Highlight, Position};

pub(super) fn execute(source: &str, name: &str) -> Result<Config, String> {
    let lua = Lua::new();
    let state = Rc::new(RefCell::new((Config::default(), None::<String>)));
    let api = lua.create_table().map_err(|e| e.to_string())?;
    for (name, action) in [
        (
            "setup",
            setup as fn(&mut Config, MultiValue) -> Result<(), String>,
        ),
        ("set_hl", set_hl),
    ] {
        let state = state.clone();
        let callback = lua
            .create_function(move |_, args: MultiValue| {
                let mut state = state.borrow_mut();
                match action(&mut state.0, args) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        if state.1.is_none() {
                            state.1 = Some(error.clone());
                        }
                        Err(mlua::Error::runtime(error))
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        api.set(name, callback).map_err(|e| e.to_string())?;
    }
    lua.register_module("mmv", api).map_err(|e| e.to_string())?;
    let result = lua.load(source).set_name(name).exec();
    let state = state.borrow();
    if let Some(error) = &state.1 {
        return Err(error.clone());
    }
    result.map_err(|e| e.to_string())?;
    Ok(state.0.clone())
}

fn string(value: Value, label: &str) -> Result<String, String> {
    match value {
        Value::String(value) => value
            .to_str()
            .map(|s| s.to_owned())
            .map_err(|_| format!("{label}: invalid UTF-8")),
        _ => Err(format!("{label}: expected string")),
    }
}

fn table(value: Value, label: &str) -> Result<Table, String> {
    match value {
        Value::Table(value) => Ok(value),
        _ => Err(format!("{label}: expected table")),
    }
}

fn setup(config: &mut Config, args: MultiValue) -> Result<(), String> {
    if args.len() != 1 {
        return Err("setup: expected one argument".into());
    }
    let options = table(args.into_iter().next().unwrap(), "setup")?;
    for pair in options.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| e.to_string())?;
        match string(key, "setup key")?.as_str() {
            "view" => {
                for pair in table(value, "view")?.pairs::<Value, Value>() {
                    let (key, value) = pair.map_err(|e| e.to_string())?;
                    match string(key, "view key")?.as_str() {
                        "position" => {
                            config.position = match string(value, "position")?.as_str() {
                                "center" => Position::Center,
                                "left" => Position::Left,
                                _ => return Err("position: expected center or left".into()),
                            }
                        }
                        "max_width" => {
                            let n = match value {
                                Value::Integer(n) => n as f64,
                                Value::Number(n) => n,
                                _ => return Err("max_width: expected number".into()),
                            };
                            if !n.is_finite() || n <= 0.0 {
                                return Err("max_width: expected positive finite number".into());
                            }
                            config.max_width = n;
                        }
                        key => return Err(format!("unknown view key: {key}")),
                    }
                }
            }
            "colorscheme" => {
                config.colorscheme = match string(value, "colorscheme")?.as_str() {
                    "dark" => ColorScheme::Dark,
                    "light" => ColorScheme::Light,
                    _ => return Err("colorscheme: expected dark or light".into()),
                }
            }
            key => return Err(format!("unknown setup key: {key}")),
        }
    }
    Ok(())
}

fn set_hl(config: &mut Config, args: MultiValue) -> Result<(), String> {
    if args.len() != 2 {
        return Err("set_hl: expected two arguments".into());
    }
    let mut args = args.into_iter();
    let name = string(args.next().unwrap(), "highlight group")?;
    let group = match name.as_str() {
        "Normal" => Group::Normal,
        "Border" => Group::Border,
        "Search" => Group::Search,
        "SearchCurrent" => Group::SearchCurrent,
        "StatusLine" => Group::StatusLine,
        "CommandLine" => Group::CommandLine,
        "ModeNormal" => Group::ModeNormal,
        "ModeSearch" => Group::ModeSearch,
        "ModeCommand" => Group::ModeCommand,
        "Heading1" => Group::Heading1,
        "Heading2" => Group::Heading2,
        "Heading3" => Group::Heading3,
        "Heading4" => Group::Heading4,
        "Heading5" => Group::Heading5,
        "Heading6" => Group::Heading6,
        "Link" => Group::Link,
        "CodeBlock" => Group::CodeBlock,
        "Quote" => Group::Quote,
        "Panel" => Group::Panel,
        "PanelSelected" => Group::PanelSelected,
        "Error" => Group::Error,
        "Warning" => Group::Warning,
        _ => return Err(format!("unknown highlight group: {name}")),
    };
    let mut highlight = config.highlights.get(&group).copied().unwrap_or_default();
    for pair in table(args.next().unwrap(), "highlight attributes")?.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| e.to_string())?;
        match string(key, "highlight attribute")?.as_str() {
            "fg" if matches!(group, Group::Search | Group::SearchCurrent) => {
                return Err(format!("{name}: fg is not supported"));
            }
            "bg" if group == Group::Border => {
                return Err(format!("{name}: bg is not supported"));
            }
            "fg" => highlight.fg = Some(color(value)?),
            "bg" => highlight.bg = Some(color(value)?),
            key => return Err(format!("unknown highlight attribute: {key}")),
        }
    }
    if highlight != Highlight::default() {
        config.highlights.insert(group, highlight);
    }
    Ok(())
}

fn color(value: Value) -> Result<Color, String> {
    let text = string(value, "color")?;
    let bytes = text.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err("color: expected #RRGGBB".into());
    }
    let byte = |i| u8::from_str_radix(&text[i..i + 2], 16).expect("validated hex");
    Ok(Color(byte(1), byte(3), byte(5)))
}
