//! Acceptance-only PDF commands. Targets are display identities, never file paths.
use std::{cell::Cell, rc::Rc};

use mlua::{Lua, MultiValue, Table, Value};

use super::{Access, LuaContext, ui::tree};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Operation {
    SetZoom(f32),
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
}

/// Shared read model; only the actual PDF adapter publishes applied values.
#[derive(Clone, Default)]
pub(crate) struct Target(Rc<Applied>);

#[derive(Default)]
struct Applied {
    zoom: Cell<Option<f32>>,
    released: Cell<bool>,
}

impl Target {
    pub(crate) fn same_display(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn zoom(&self) -> Option<f32> {
        if self.0.released.get() {
            None
        } else {
            self.0.zoom.get()
        }
    }
    pub(crate) fn is_released(&self) -> bool {
        self.0.released.get()
    }
    pub(crate) fn publish(&self, zoom: f32) {
        self.0.zoom.set(Some(zoom));
    }
    pub(crate) fn release(&self) {
        self.0.released.set(true);
        self.0.zoom.set(None);
    }
}

pub(crate) struct Request {
    pub target: Target,
    pub operation: Operation,
}

fn no_args(args: &MultiValue) -> mlua::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(tree::err("expected no arguments"))
    }
}

pub(crate) fn install(lua: &Lua, api: &Table, context: &LuaContext) -> mlua::Result<()> {
    let pdf = lua.create_table()?;
    let ctx = context.clone();
    pdf.set(
        "set_zoom",
        lua.create_function(move |_, args: MultiValue| {
            ctx.check(Access::Mutation)?;
            if args.len() != 1 {
                return Err(tree::err("set_zoom expects one numeric factor"));
            }
            let factor = match args.front().unwrap() {
                Value::Integer(n) => *n as f64,
                Value::Number(n) => *n,
                _ => {
                    return Err(tree::err(
                        "factor: expected finite number greater than zero",
                    ));
                }
            };
            if !factor.is_finite() || factor <= 0.0 {
                return Err(tree::err(
                    "factor: expected finite number greater than zero",
                ));
            }
            // Clamp before narrowing, including positive subnormal/very large f64 input.
            accept(&ctx, Operation::SetZoom(factor.clamp(0.01, 3.0) as f32))
        })?,
    )?;
    for (name, operation) in [
        ("zoom_in", Operation::ZoomIn),
        ("zoom_out", Operation::ZoomOut),
        ("fit_width", Operation::FitWidth),
        ("fit_page", Operation::FitPage),
    ] {
        let ctx = context.clone();
        pdf.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                ctx.check(Access::Mutation)?;
                no_args(&args)?;
                accept(&ctx, operation)
            })?,
        )?;
    }
    let ctx = context.clone();
    pdf.set(
        "get_zoom",
        lua.create_function(move |_, args: MultiValue| {
            ctx.check(Access::Read)?;
            no_args(&args)?;
            Ok(ctx.pdf.borrow().as_ref().and_then(Target::zoom))
        })?,
    )?;
    api.set("pdf", pdf)
}

fn accept(context: &LuaContext, operation: Operation) -> mlua::Result<bool> {
    let target = context.pdf.borrow().clone();
    let Some(target) = target.filter(|target| !target.is_released()) else {
        return Ok(false);
    };
    context.request(Request { target, operation })?;
    Ok(true)
}

#[cfg(test)]
mod tests;
