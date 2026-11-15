use super::tree::LuaFunction;
use crate::lua::state::StateHandle;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) struct Input {
    pub state: StateHandle,
    pub placeholder: String,
    pub changed: Option<LuaFunction>,
    pub submit: Option<LuaFunction>,
    pub keys: BTreeMap<String, LuaFunction>,
}
