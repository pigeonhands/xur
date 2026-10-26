use xur_span::{Ident, Span};

use crate::{BinOp, Block, FnDecl, Lit, Pat, Path, PathSegment, Precedence, UnOp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    /// How tightly this expression binds, used to decide where parentheses
    /// are needed.
    pub fn precedence(&self) -> Precedence {
        match &self.kind {
            ExprKind::Closure(_) | ExprKind::Break(_) | ExprKind::Ret(_) => Precedence::Jump,
            ExprKind::Assign(..) => Precedence::Assign,
            ExprKind::Range(..) => Precedence::Range,
            ExprKind::Binary(op, ..) => op.kind.precedence(),
            ExprKind::Unary(..) => Precedence::Prefix,
            _ => Precedence::Unambiguous,
        }
    }

    /// Whether this expression ends in a block, so that as a statement it
    /// needs no trailing `;` (rustc's `expr_requires_semi_to_be_stmt`).
    pub fn is_block_like(&self) -> bool {
        matches!(
            self.kind,
            ExprKind::Block(_)
                | ExprKind::If(..)
                | ExprKind::Match(..)
                | ExprKind::ForLoop { .. }
                | ExprKind::While(..)
                | ExprKind::Loop(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    /// `1`, `"text"`, `true`
    Lit(Lit),
    /// `x`, `std::io`, `type_name::<T>`
    Path(Path),
    /// `(expr)`, kept so the tree records exactly what was written.
    Paren(Box<Expr>),
    /// `(a, b)`; `()` is the unit value.
    Tuple(Vec<Expr>),
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `-x`, `!x`
    Unary(UnOp, Box<Expr>),
    /// `a + b`
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `place = value`; the span is that of the `=`.
    Assign(Box<Expr>, Box<Expr>, Span),
    /// `a..b`, `a..`, `..b`, `a..=b`
    Range(Option<Box<Expr>>, Option<Box<Expr>>, RangeLimits),
    /// `callee(args)`
    Call(Box<Expr>, Vec<Expr>),
    /// `receiver.method(args)`
    MethodCall(Box<MethodCall>),
    /// `expr.field`
    Field(Box<Expr>, Ident),
    /// `expr?`
    Try(Box<Expr>),
    /// `list[index]`
    Index(Box<Expr>, Box<Expr>),
    /// `{ ... }`
    Block(Box<Block>),
    /// `if cond { ... } else ...`; the else branch is a block or another `if`.
    If(Box<Expr>, Box<Block>, Option<Box<Expr>>),
    /// `match scrutinee { arms }`
    Match(Box<Expr>, Vec<Arm>),
    /// `for pat in iter { body }`
    ForLoop {
        pat: Box<Pat>,
        iter: Box<Expr>,
        body: Box<Block>,
    },
    /// `while cond { body }`
    While(Box<Expr>, Box<Block>),
    /// `loop { body }`
    Loop(Box<Block>),
    /// `|params| body`
    Closure(Box<Closure>),
    /// `break`, `break value`
    Break(Option<Box<Expr>>),
    /// `continue`
    Continue,
    /// `return`, `return value`
    Ret(Option<Box<Expr>>),
    /// An expression that failed to parse; the error has already been reported.
    Err,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodCall {
    /// The method name, with any turbofish: `parse::<u32>`.
    pub seg: PathSegment,
    pub receiver: Expr,
    pub args: Vec<Expr>,
    /// From the method name to the closing `)`.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closure {
    /// Parameters without a written type get [`TyKind::Infer`](crate::TyKind::Infer).
    pub decl: FnDecl,
    pub body: Box<Expr>,
    /// The `|params| -> ret` part.
    pub decl_span: Span,
}

/// `pat => body`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    pub pat: Pat,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeLimits {
    /// `a..b`
    HalfOpen,
    /// `a..=b`
    Closed,
}
