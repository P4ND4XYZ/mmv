use super::tree;
use crate::lua::{
    Access, LuaContext, OwnerId,
    document::{RangeHandle, SourceHandle},
};
use mlua::{MultiValue, UserData, UserDataMethods, Value};
use std::{cell::RefCell, ops::Range, rc::Rc};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Align {
    Start,
    Center,
    End,
    Nearest,
}
#[derive(Clone, Debug)]
pub(crate) enum Motion {
    Pages(f64),
    Lines(f64),
    Edge(bool),
    Range(Range<usize>, Align),
}
#[derive(Clone)]
pub(crate) struct Source {
    pub source: SourceHandle,
    pub highlight: Option<Range<usize>>,
    pub motions: Rc<RefCell<Vec<Motion>>>,
}
#[derive(Clone)]
pub(crate) struct SourceControl {
    pub owner: OwnerId,
    pub identity: u64,
    pub context: LuaContext,
}
impl SourceControl {
    pub(crate) fn source(&self) -> Option<Source> {
        self.context
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == self.owner)?
            .components
            .values()
            .find(|c| c.identity == self.identity)?
            .node
            .source
            .clone()
    }
    pub(crate) fn motion(&self, motion: Motion) -> mlua::Result<bool> {
        let Some(s) = self.source() else {
            return Ok(false);
        };
        self.context.check(Access::Mutation)?;
        s.motions.borrow_mut().push(motion);
        Ok(true)
    }
}
impl UserData for SourceControl {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("scroll", |_, this, args: MultiValue| {
            if this.source().is_none() {
                return Ok(false);
            }
            this.context.check(Access::Mutation)?;
            if args.len() != 1 {
                return Err(tree::err("scroll requires options"));
            }
            let t = tree::table(args.into_iter().next().unwrap(), "scroll")?;
            tree::fields(&t, &["pages"])?;
            this.motion(Motion::Pages(tree::number(t.raw_get("pages")?, "pages")?))
        });
        m.add_method("scroll_to", |_, this, args: MultiValue| {
            let Some(s) = this.source() else {
                return Ok(false);
            };
            this.context.check(Access::Mutation)?;
            if args.is_empty() || args.len() > 2 {
                return Err(tree::err("scroll_to expects range and optional options"));
            }
            let mut args = args.into_iter();
            let Value::UserData(data) = args.next().unwrap() else {
                return Err(tree::err("expected source range handle"));
            };
            let range = data
                .borrow::<RangeHandle>()
                .map_err(|_| tree::err("expected source range handle"))?
                .for_source(&s.source)?;
            let options = match args.next().unwrap_or(Value::Nil) {
                Value::Nil => None,
                v => Some(tree::table(v, "scroll_to")?),
            };
            let align = if let Some(t) = options {
                tree::fields(&t, &["align"])?;
                tree::optional_string(&t, "align")?
            } else {
                None
            };
            let align = match align.as_deref().unwrap_or("nearest") {
                "start" => Align::Start,
                "center" => Align::Center,
                "end" => Align::End,
                "nearest" => Align::Nearest,
                _ => return Err(tree::err("invalid scroll alignment")),
            };
            this.motion(Motion::Range(range, align))
        });
    }
}
pub(crate) fn source(value: Value) -> mlua::Result<SourceHandle> {
    match value {
        Value::UserData(d) => Ok(d
            .borrow::<SourceHandle>()
            .map_err(|_| tree::err("expected source handle"))?
            .clone()),
        _ => Err(tree::err("expected source handle")),
    }
}
pub(crate) fn range(value: Value, s: &SourceHandle) -> mlua::Result<Option<Range<usize>>> {
    match value {
        Value::Nil => Ok(None),
        Value::UserData(d) => d
            .borrow::<RangeHandle>()
            .map_err(|_| tree::err("expected range handle"))?
            .for_source(s)
            .map(Some),
        _ => Err(tree::err("expected range handle")),
    }
}
