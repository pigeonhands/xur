use xur_span::{Ident, Span};

use crate::{Attribute, Block, Expr, NodeId, Pat, Path, Ty};

/// A whole source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: NodeId,
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// `pub`, with the keyword's span.
    Public(Span),
    /// No visibility was written.
    Inherited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// `use std;`, or `use std::{io, fs::File};`, which imports each path
    /// in the braces: one tree per imported path, prefix included.
    Use(Vec<UseTree>),
    /// `const name: ty = expr;`
    Const(Box<ConstItem>),
    /// `fn name<T>(params) -> ret { body }`
    Fn(Box<Fn>),
    /// `struct Name<T> { fields }`
    Struct(Ident, Generics, VariantData),
    /// `enum Name<T> { variants }`
    Enum(Ident, Generics, EnumDef),
    /// `impl<T> Type<T> { fns }`
    Impl(Box<Impl>),
}

/// An `impl` block, adding functions to a struct or enum: methods, which
/// take `self` first, and associated functions, which don't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impl {
    pub generics: Generics,
    pub self_ty: Ty,
    /// Always function items.
    pub items: Vec<Item>,
}

/// One path a `use` imports, such as `std::fs::File`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseTree {
    pub path: Path,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstItem {
    pub ident: Ident,
    /// Unlike Rust, the type may be left for inference: `const math = ...;`.
    pub ty: Option<Ty>,
    pub expr: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fn {
    pub ident: Ident,
    pub generics: Generics,
    pub decl: FnDecl,
    pub body: Box<Block>,
}

/// The parameters and return type of a function or closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDecl {
    pub inputs: Vec<Param>,
    pub output: FnRetTy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub pat: Pat,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnRetTy {
    /// No `-> ty` was written; the span is where it would go.
    Default(Span),
    Ty(Box<Ty>),
}

/// `<T, U>` on a declaration. Empty, with an empty span, when absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Generics {
    pub params: Vec<GenericParam>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParam {
    pub id: NodeId,
    pub ident: Ident,
    /// `T!`: the caller must provide a concrete type, not `_`.
    pub required: bool,
    /// `E = str`: the type to use when the caller omits this parameter.
    pub default: Option<Ty>,
}

/// The fields of a struct or enum variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantData {
    /// `{ a: u32 b: u32 }`
    Struct(Vec<FieldDef>),
    /// `(u32, u32)`
    Tuple(Vec<FieldDef>),
    /// No fields.
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    /// `None` for tuple fields.
    pub ident: Option<Ident>,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDef {
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub id: NodeId,
    pub attrs: Vec<Attribute>,
    pub ident: Ident,
    pub data: VariantData,
    pub span: Span,
}
