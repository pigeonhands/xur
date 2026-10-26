//! The Xur abstract syntax tree.
//!
//! Modelled on `rustc_ast`: every node is a struct pairing a `kind` enum with
//! the [`Span`] it was parsed from, names are interned [`Ident`]s, and
//! recursive children are boxed. The tree is a faithful record of the source
//! (parentheses are kept, nothing is desugared), so later passes and a pretty
//! printer can reproduce what the user wrote.
//!
//! [`Span`]: xur_span::Span
//! [`Ident`]: xur_span::Ident

mod attr;
mod expr;
mod item;
mod lit;
mod node_id;
mod op;
mod pat;
mod path;
mod stmt;
mod ty;

pub use attr::{Attribute, MetaItem, MetaItemInner, MetaItemKind};
pub use expr::{Arm, Closure, Expr, ExprKind, MethodCall, RangeLimits};
pub use item::{
    ConstItem, EnumDef, FieldDef, Fn, FnDecl, FnRetTy, GenericParam, Generics, Item, ItemKind,
    Module, Param, UseTree, Variant, VariantData, Visibility,
};
pub use lit::{Lit, LitKind};
pub use node_id::{NodeId, NodeIdGen};
pub use op::{BinOp, BinOpKind, Fixity, Precedence, UnOp};
pub use pat::{Pat, PatKind};
pub use path::{GenericArgs, Path, PathSegment};
pub use stmt::{Block, Local, Stmt, StmtKind};
pub use ty::{Ty, TyKind};
