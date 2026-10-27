//! Operations on registered types that need the concrete Rust type.

use std::any::Any;

use facet::{Facet, Peek};

use crate::convert::{Builder, ConvertCx, peek_to_value, reflect_error};
use crate::{RunResult, RuntimeError, Value};

/// Operations on values of one registered type, instantiated where the type
/// is still known (in [`ModuleBuilder::ty`]) and used where it is erased.
#[derive(Clone, Copy)]
pub(super) struct TypeHooks {
    /// Clones the value behind a peek, for a registered value nested inside
    /// another, like the elements of a `Vec<Point>`.
    pub(super) clone_peek: fn(Peek<'_, 'static>) -> Option<Box<dyn Any + Send>>,
    /// Writes a clone of the value into a builder for the type.
    pub(super) set: fn(Builder, &dyn Any) -> RunResult<Builder>,
    /// Reads a field by name.
    pub(super) field: fn(&dyn ConvertCx, &dyn Any, &str) -> Option<RunResult<Value>>,
    pub(super) display: fn(&dyn Any) -> Option<String>,
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

    /// Hooks for a type that is not `Clone`, such as one that borrows memory
    /// kept alive beside it. Its values can be read (fields, display, `==`)
    /// but not copied into another value or rebuilt from Xur.
    pub(super) fn borrowing<T: Facet<'static> + Send + 'static>() -> Self {
        Self {
            clone_peek: |_| None,
            set: |_, _| {
                Err(RuntimeError::new(format!(
                    "`{}` cannot be moved into a value",
                    T::SHAPE
                )))
            },
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

/// The value's `Display`, or its `Debug`, or `None` when the type has
/// neither (an opaque type), so the caller can fall back to its Xur name
/// rather than facet's `⟨Type⟩` placeholder.
fn display<T: Facet<'static>>(value: &dyn Any) -> Option<String> {
    let peek = Peek::new(downcast::<T>(value));
    let placeholder = format!("⟨{}⟩", T::SHAPE);
    // Without a `Display`, a peek's `Display` is the `⟨Type⟩` placeholder.
    let displayed = peek.to_string();
    if displayed != placeholder {
        return Some(displayed);
    }
    // Fall back to `Debug`, which is the same placeholder when absent.
    let debugged = format!("{peek:?}");
    (debugged != placeholder).then_some(debugged)
}

fn eq<T: Facet<'static>>(a: &dyn Any, b: &dyn Any) -> bool {
    Peek::new(downcast::<T>(a)) == Peek::new(downcast::<T>(b))
}
