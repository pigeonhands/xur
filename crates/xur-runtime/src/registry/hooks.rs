//! Operations on registered types that need the concrete Rust type.

use std::any::Any;

use facet::{Facet, Peek};

use crate::convert::{Builder, ConvertCx, peek_to_value, reflect_error};
use crate::{RunResult, Value};

/// Operations on values of one registered type, instantiated where the type
/// is still known (in [`ModuleBuilder::ty`]) and used where it is erased.
pub(super) struct TypeHooks {
    /// Clones the value behind a peek, for a registered value nested inside
    /// another, like the elements of a `Vec<Point>`.
    pub(super) clone_peek: fn(Peek<'_, 'static>) -> Option<Box<dyn Any + Send>>,
    /// Writes a clone of the value into a builder for the type.
    pub(super) set: fn(Builder, &dyn Any) -> RunResult<Builder>,
    /// Reads a field by name.
    pub(super) field: fn(&dyn ConvertCx, &dyn Any, &str) -> Option<RunResult<Value>>,
    pub(super) display: fn(&dyn Any) -> String,
    pub(super) eq: fn(&dyn Any, &dyn Any) -> bool,
}

impl TypeHooks {
    pub(super) fn new<T: Facet<'static> + Clone + Send + 'static>() -> Self {
        Self {
            clone_peek: clone_peek::<T>,
            set: set::<T>,
            field: field::<T>,
            display: display::<T>,
            eq: eq::<T>,
        }
    }
}

fn downcast<T: 'static>(value: &dyn Any) -> &T {
    value
        .downcast_ref()
        .expect("type hooks are only used with their own type")
}

fn clone_peek<T: Facet<'static> + Clone + Send>(
    peek: Peek<'_, 'static>,
) -> Option<Box<dyn Any + Send>> {
    let value: &T = peek.get().ok()?;
    Some(Box::new(value.clone()))
}

fn set<T: Facet<'static> + Clone>(partial: Builder, value: &dyn Any) -> RunResult<Builder> {
    partial
        .set(downcast::<T>(value).clone())
        .map_err(reflect_error)
}

fn field<T: Facet<'static>>(
    cx: &dyn ConvertCx,
    value: &dyn Any,
    name: &str,
) -> Option<RunResult<Value>> {
    let fields = Peek::new(downcast::<T>(value)).into_struct().ok()?;
    let field = fields.field_by_name(name).ok()?;
    Some(peek_to_value(cx, field))
}

/// Uses `Display`, or `Debug` if the type has no `Display`.
fn display<T: Facet<'static>>(value: &dyn Any) -> String {
    let peek = Peek::new(downcast::<T>(value));
    let displayed = peek.to_string();
    // Without a `Display`, a peek shows the type name as `⟨Type⟩`.
    if displayed == format!("⟨{}⟩", T::SHAPE) {
        format!("{peek:?}")
    } else {
        displayed
    }
}

fn eq<T: Facet<'static>>(a: &dyn Any, b: &dyn Any) -> bool {
    Peek::new(downcast::<T>(a)) == Peek::new(downcast::<T>(b))
}
