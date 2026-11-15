use super::tree::LuaFunction;
use crate::lua::state::StateHandle;
use std::collections::BTreeMap;

/// Keys delegated to the native editor must not fall through to app mappings.
pub(crate) fn native_key(key: &str) -> bool {
    let base = key.rsplit('-').next().unwrap_or(key);
    let text_key = (base.chars().count() == 1 || key == "-" || key == "shift--")
        && !key.starts_with("ctrl-")
        && !key.starts_with("alt-")
        && !key.starts_with("super-");
    text_key
        || matches!(
            base,
            "left"
                | "right"
                | "up"
                | "down"
                | "home"
                | "end"
                | "backspace"
                | "delete"
                | "insert"
                | "space"
        )
        || matches!(
            key,
            "ctrl-a" | "ctrl-c" | "ctrl-x" | "ctrl-v" | "ctrl-z" | "ctrl-y" | "ctrl-shift-z"
        )
}

#[derive(Clone)]
pub(crate) struct Input {
    pub state: StateHandle,
    pub placeholder: String,
    pub changed: Option<LuaFunction>,
    pub submit: Option<LuaFunction>,
    pub keys: BTreeMap<String, LuaFunction>,
}
