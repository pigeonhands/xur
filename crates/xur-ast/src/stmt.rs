use xur_span::Span;

use crate::{Expr, Item, Pat, Ty};

/// `{ stmts }`. The block's value is its trailing [`StmtKind::Expr`], if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind {
    /// `let pat: ty = init;`
    Let(Box<Local>),
    /// An item nested in a block, e.g. a helper `fn`.
    Item(Box<Item>),
    /// An expression without a trailing `;`: either the block's value or a
    /// block-like expression such as `if` or `for` used as a statement.
    Expr(Box<Expr>),
    /// `expr;`
    Semi(Box<Expr>),
    /// A lone `;`.
    Empty,
}

/// A `let` binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub pat: Pat,
    pub ty: Option<Ty>,
    pub init: Option<Box<Expr>>,
    pub span: Span,
}
