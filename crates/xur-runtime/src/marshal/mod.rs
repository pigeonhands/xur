//! Traits for converting between Rust types and Xur [`Value`]s.
//!
//! [`FromValue`] and [`IntoXurValue`] decouple the typed function API
//! from [`Facet`]: any type that implements `Facet` automatically gets
//! both traits via blanket impls, while types like `Value`, `XurList`
//! and `XurBytes` provide their own custom conversions.

mod wrappers;

use facet::{Facet, Shape};

use crate::convert::{self, ConvertCx};
use crate::RunResult;
use crate::Value;

pub use wrappers::{XurBytes, XurList, XurStr};

/// Extract a Rust value from a Xur [`Value`].
pub trait FromValue: Sized + Send + 'static {
    const SHAPE: Option<&'static Shape> = None;
    fn from_value(cx: &dyn ConvertCx, value: &Value) -> RunResult<Self>;
}

/// Convert a Rust value into a Xur [`Value`].
pub trait IntoXurValue: Send + 'static {
    const SHAPE: Option<&'static Shape> = None;
    fn into_xur_value(self, cx: &dyn ConvertCx) -> RunResult<Value>;
}

impl<T: Facet<'static> + Send + 'static> FromValue for T {
    const SHAPE: Option<&'static Shape> = Some(T::SHAPE);
    fn from_value(cx: &dyn ConvertCx, value: &Value) -> RunResult<Self> {
        convert::from_value::<T>(cx, value)
    }
}

impl<T: Facet<'static> + Send + 'static> IntoXurValue for T {
    const SHAPE: Option<&'static Shape> = Some(T::SHAPE);
    fn into_xur_value(self, cx: &dyn ConvertCx) -> RunResult<Value> {
        convert::into_value(cx, self)
    }
}

impl FromValue for Value {
    fn from_value(_cx: &dyn ConvertCx, value: &Value) -> RunResult<Self> {
        Ok(value.clone())
    }
}

impl IntoXurValue for Value {
    fn into_xur_value(self, _cx: &dyn ConvertCx) -> RunResult<Value> {
        Ok(self)
    }
}

/// A typed-function return that **raises** on `Err` (its `RuntimeError`
/// propagates) rather than becoming a Xur `Result` value, so a typed function
/// can fail like a raw one. (A bare `Result<T, RuntimeError>` can't have its own
/// `IntoXurValue` impl — it collides with the `Facet` blanket — so this newtype
/// carries it.) The declared return type is `T`'s.
pub struct Raise<T>(pub RunResult<T>);

impl<T: IntoXurValue> IntoXurValue for Raise<T> {
    const SHAPE: Option<&'static Shape> = T::SHAPE;
    fn into_xur_value(self, cx: &dyn ConvertCx) -> RunResult<Value> {
        self.0?.into_xur_value(cx)
    }
}

impl<T> From<RunResult<T>> for Raise<T> {
    fn from(result: RunResult<T>) -> Self {
        Raise(result)
    }
}

/// A typed-function return of an optional **dynamic** value, built as the
/// prelude's `Some`/`None`. `Option<T>` works directly when `T: Facet`, but
/// `Option<Value>` can't (same coherence collision), so this newtype carries it
/// — e.g. `list.get` -> `Opt<Value>`.
pub struct Opt<T>(pub Option<T>);

impl<T: IntoXurValue> IntoXurValue for Opt<T> {
    fn into_xur_value(self, cx: &dyn ConvertCx) -> RunResult<Value> {
        let lang = cx.lang();
        Ok(match self.0 {
            Some(value) => Value::adt(lang.some, vec![value.into_xur_value(cx)?]),
            None => Value::adt(lang.none, Vec::new()),
        })
    }
}

impl<T> From<Option<T>> for Opt<T> {
    fn from(option: Option<T>) -> Self {
        Opt(option)
    }
}

