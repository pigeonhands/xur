//! Modules implemented in Rust, as name resolution sees them.
//!
//! The resolver only needs names; the runtime holds the implementations and
//! numbers them, and resolved paths refer to native items by those numbers.

/// Identifies a function implemented in Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeFnId(pub u32);

/// Identifies a type implemented in Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeTypeId(pub u32);

/// Supplies the native modules: their declarations up front, and the
/// contents of lazy modules once a program looks inside them.
///
/// Providers are shared with a running program, which may import modules,
/// and so expand lazy native modules, from any thread.
pub trait NativeProvider: Send + Sync {
    fn modules(&self) -> Vec<NativeModule>;

    /// The contents of the lazy module at `path` (such as `["std", "net"]`),
    /// which is now needed. Its own submodules may be lazy in turn.
    fn expand(&self, path: &[String]) -> NativeModule;
}

impl<T: NativeProvider + ?Sized> NativeProvider for std::sync::Arc<T> {
    fn modules(&self) -> Vec<NativeModule> {
        (**self).modules()
    }

    fn expand(&self, path: &[String]) -> NativeModule {
        (**self).expand(path)
    }
}

/// Native modules that are all declared up front.
impl NativeProvider for Vec<NativeModule> {
    fn modules(&self) -> Vec<NativeModule> {
        self.clone()
    }

    fn expand(&self, path: &[String]) -> NativeModule {
        panic!("`{}` was not declared lazy", path.join("::"))
    }
}

/// A module of native items, such as `std` or one of its submodules.
#[derive(Debug, Clone, Default)]
pub struct NativeModule {
    pub name: String,
    /// Whether the module's contents are only provided when first needed,
    /// by [`NativeProvider::expand`]. A lazy module is declared empty.
    pub lazy: bool,
    pub functions: Vec<NativeFnDecl>,
    pub types: Vec<NativeTypeDecl>,
    pub modules: Vec<NativeModule>,
}

#[derive(Debug, Clone)]
pub struct NativeFnDecl {
    pub name: String,
    pub id: NativeFnId,
}

/// A native type and its methods and associated functions, which are in
/// its scope as `Type::name`.
#[derive(Debug, Clone)]
pub struct NativeTypeDecl {
    pub name: String,
    pub id: NativeTypeId,
    pub functions: Vec<NativeFnDecl>,
}
