//! Loading modules and resolving names, after `rustc_resolve`.

mod loader;

pub use loader::{Crate, LoadedModule, ModuleIdx, load_crate};
