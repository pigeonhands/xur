//! Running Xur programs.

pub mod convert;
mod error;
pub mod registry;
mod value;

pub use error::{RunResult, RuntimeError};
pub use registry::{Context, Registry};
pub use value::{Adt, BuiltinType, Closure, NativeObject, Range, Record, Value, ValueIter, lock};
pub use xur_resolve::LangItems;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
