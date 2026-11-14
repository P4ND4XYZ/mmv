//! Only Rust may extend the set of state-safe immutable userdata types.
use std::{cell::RefCell, rc::Rc};

use mlua::{AnyUserData, UserData};

type TypeCheck = fn(&AnyUserData) -> bool;

#[derive(Clone, Default)]
pub(crate) struct ReadonlyTypes(Rc<RefCell<Vec<TypeCheck>>>);

impl ReadonlyTypes {
    /// The registering adapter must expose no mutation methods or mutable fields.
    pub(crate) fn register<T: UserData + 'static>(&self) {
        self.0.borrow_mut().push(AnyUserData::is::<T>);
    }

    pub(crate) fn contains(&self, value: &AnyUserData) -> bool {
        self.0.borrow().iter().any(|accept| accept(value))
    }
}
