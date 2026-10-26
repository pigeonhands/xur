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

/// A module of native items, such as `std` or one of its submodules.
#[derive(Debug, Clone, Default)]
pub struct NativeModule {
    pub name: String,
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
