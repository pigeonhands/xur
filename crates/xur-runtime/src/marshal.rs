//! Traits for converting between Rust types and Xur [`Value`]s.
//!
//! [`FromValue`] and [`IntoXurValue`] decouple the typed function API
//! from [`Facet`]: any type that implements `Facet` automatically gets
//! both traits via blanket impls, while types like `Value`, `XurList`
//! and `XurBytes` provide their own custom conversions.

use facet::{Facet, Shape};

use crate::convert::{self, ConvertCx};
use crate::{RunResult, Value};

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

// ---- Blanket impls over Facet ----

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
