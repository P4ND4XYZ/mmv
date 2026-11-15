//! Immutable document snapshots and owner-scoped replacement subscriptions.
use std::{
    cell::OnceCell,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use mlua::{
    AnyUserData, Lua, MetaMethod, MultiValue, Table, UserData, UserDataFields, UserDataMethods,
    Value,
};

use super::{
    Access, CallbackScope, LuaContext,
    callbacks::CallbackId,
    ui::{tree, window::WindowHandle},
};
use crate::core::{
    LinkTarget,
    links::{LinkCandidate, LinkKind},
};

type ExtractLinks = fn(&Path, &str) -> Result<Vec<LinkCandidate>, String>;

struct Snapshot {
    path: PathBuf,
    source: Arc<String>,
    links: OnceCell<Result<Vec<LinkCandidate>, String>>,
    extract: ExtractLinks,
}

#[derive(Clone)]
pub(crate) struct DocumentHandle(Rc<Snapshot>);
#[derive(Clone)]
pub(crate) struct SourceHandle(Rc<Snapshot>);
#[derive(Clone)]
pub(crate) struct RangeHandle {
    source: SourceHandle,
    range: Range<usize>,
}
#[derive(Clone)]
pub(crate) struct TargetHandle {
    document: DocumentHandle,
    pub(crate) target: LinkTarget,
}

#[derive(Default)]
pub(crate) struct Documents {
    pub(crate) current: Option<DocumentHandle>,
}

impl DocumentHandle {
    pub(crate) fn new(path: PathBuf, source: Arc<String>, extract: ExtractLinks) -> Self {
        Self(Rc::new(Snapshot {
            path,
            source,
            links: OnceCell::new(),
            extract,
        }))
    }

    pub(crate) fn same_snapshot(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    fn links(&self, lua: &Lua) -> mlua::Result<Table> {
        let links = self
            .0
            .links
            .get_or_init(|| (self.0.extract)(&self.0.path, &self.0.source));
        let links = links
            .as_ref()
            .map_err(|message| tree::err(message.clone()))?;
        let result = lua.create_table()?;
        for (index, candidate) in links.iter().enumerate() {
            let item = lua.create_table()?;
            item.set("id", candidate.id.0 as i64)?;
            item.set("display_name", candidate.display_name.clone())?;
            item.set("destination", candidate.destination.clone())?;
            item.set(
                "kind",
                match candidate.kind {
                    LinkKind::Heading => "heading",
                    LinkKind::LocalMarkdown => "markdown",
                    LinkKind::LocalMarkdownHeading => "markdown_heading",
                    LinkKind::Web => "web",
                },
            )?;
            // Resolve fragments now, never against a later current document.
            let target = match &candidate.target {
                LinkTarget::Heading(id) => LinkTarget::File {
                    path: self.0.path.clone(),
                    fragment: Some(id.clone()),
                },
                target => target.clone(),
            };
            item.set(
                "target",
                TargetHandle {
                    document: self.clone(),
                    target,
                },
            )?;
            let source = SourceHandle(self.0.clone());
            let range = candidate.source_range.clone().map(|range| RangeHandle {
                source: source.clone(),
                range,
            });
            if let Some(range) = &range {
                range.for_source(&source)?;
            }
            item.set("source_range", range)?;
            result.raw_set(index + 1, item)?;
        }
        Ok(result)
    }
}

impl SourceHandle {
    /// D shares these bytes; obtaining a source never copies the document body.
    pub(crate) fn text(&self) -> Arc<String> {
        self.0.source.clone()
    }
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl RangeHandle {
    /// D must validate every highlight/scroll range through this boundary.
    pub(crate) fn for_source(&self, source: &SourceHandle) -> mlua::Result<Range<usize>> {
        if !self.source.same_source(source) {
            return Err(tree::err("range belongs to another source"));
        }
        if source.text().get(self.range.clone()).is_none() {
            return Err(tree::err("invalid source range"));
        }
        Ok(self.range.clone())
    }
}
impl TargetHandle {
    pub(crate) fn is_current(&self, context: &LuaContext) -> bool {
        context
            .documents
            .borrow()
            .current
            .as_ref()
            .is_some_and(|doc| doc.same_snapshot(&self.document))
    }
}

fn no_args(args: MultiValue) -> mlua::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(tree::err("expected no arguments"))
    }
}
impl UserData for DocumentHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("source", |_, this, args: MultiValue| {
            no_args(args)?;
            Ok(SourceHandle(this.0.clone()))
        });
        methods.add_method("links", |lua, this, args: MultiValue| {
            no_args(args)?;
            this.links(lua)
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other
                .borrow::<Self>()
                .is_ok_and(|other| this.same_snapshot(&other)))
        });
    }
}
impl UserData for SourceHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other
                .borrow::<Self>()
                .is_ok_and(|other| this.same_source(&other)))
        });
    }
}
impl UserData for RangeHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("start", |_, this| Ok(this.range.start));
        fields.add_field_method_get("stop", |_, this| Ok(this.range.end));
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other.borrow::<Self>().is_ok_and(|other| {
                this.source.same_source(&other.source) && this.range == other.range
            }))
        });
    }
}
impl UserData for TargetHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other.borrow::<Self>().is_ok_and(|other| {
                this.document.same_snapshot(&other.document) && this.target == other.target
            }))
        });
    }
}

pub(crate) fn owner(value: Value, required: bool) -> mlua::Result<Option<super::OwnerId>> {
    match value {
        Value::Nil if !required => Ok(None),
        Value::UserData(data) => data
            .borrow::<WindowHandle>()
            .map(|handle| Some(handle.owner()))
            .map_err(|_| tree::err("owner: expected window")),
        _ => Err(tree::err("owner: expected window")),
    }
}

struct Subscription {
    callback: CallbackId,
    context: LuaContext,
}
impl UserData for Subscription {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cancel", |_, this, args: MultiValue| {
            if !this.context.has_callback(this.callback) {
                return Ok(false);
            }
            this.context.check(Access::Mutation)?;
            no_args(args)?;
            Ok(this.context.cancel(this.callback))
        });
    }
}

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    context.readonly.register::<DocumentHandle>();
    context.readonly.register::<SourceHandle>();
    context.readonly.register::<RangeHandle>();
    context.readonly.register::<TargetHandle>();
    let document = lua.create_table()?;
    let ctx = context.clone();
    document.set(
        "current",
        lua.create_function(move |_, args: MultiValue| {
            ctx.check(Access::Read)?;
            no_args(args)?;
            Ok(ctx.documents.borrow().current.clone())
        })?,
    )?;
    let ctx = context.clone();
    document.set(
        "on_replaced",
        lua.create_function(move |lua, args: MultiValue| {
            ctx.check(Access::Mutation)?;
            if args.len() != 2 {
                return Err(tree::err("on_replaced expects options and function"));
            }
            let mut args = args.into_iter();
            let options = tree::table(args.next().unwrap(), "on_replaced")?;
            tree::fields(&options, &["owner"])?;
            let owner = owner(options.raw_get("owner")?, true)?.unwrap();
            let Value::Function(function) = args.next().unwrap() else {
                return Err(tree::err("expected function"));
            };
            let Some(callback) = ctx.register(lua, function, CallbackScope::window(owner, None))?
            else {
                return Ok(Value::Boolean(false));
            };
            ctx.shared.borrow_mut().replaced.push(callback);
            Ok(Value::UserData(lua.create_userdata(Subscription {
                callback,
                context: ctx.clone(),
            })?))
        })?,
    )?;
    api.set("document", document)
}

impl LuaContext {
    /// Capture registrations at actual adoption, before dispatching any Lua.
    pub(crate) fn replace_document(
        &self,
        lua: &Lua,
        document: impl Into<Option<DocumentHandle>>,
    ) -> mlua::Result<()> {
        self.documents.borrow_mut().current = document.into();
        let subscribers = self.shared.borrow().replaced.clone();
        for callback in subscribers {
            self.enqueue(lua, callback, ())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
