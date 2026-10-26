use std::fmt;

use xur_span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinOp {
    pub kind: BinOpKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::And => "&&",
            Self::Or => "||",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }

    pub fn precedence(self) -> Precedence {
        match self {
            Self::Mul | Self::Div | Self::Rem => Precedence::Product,
            Self::Add | Self::Sub => Precedence::Sum,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge => Precedence::Compare,
            Self::And => Precedence::And,
            Self::Or => Precedence::Or,
        }
    }

    pub fn fixity(self) -> Fixity {
        match self.precedence() {
            // `a == b == c` is rejected rather than silently grouped.
            Precedence::Compare => Fixity::None,
            _ => Fixity::Left,
        }
    }
}

impl fmt::Display for BinOpKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
}

impl UnOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
        }
    }
}

/// How tightly an expression binds, weakest first; follows rustc's
/// `ExprPrecedence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    /// `return`, `break`, closures: extend as far right as possible.
    Jump,
    /// `a = b`
    Assign,
    /// `a..b`
    Range,
    /// `a || b`
    Or,
    /// `a && b`
    And,
    /// `a == b`, `a < b`, ...
    Compare,
    /// `a + b`, `a - b`
    Sum,
    /// `a * b`, `a / b`, `a % b`
    Product,
    /// `-a`, `!a`
    Prefix,
    /// Calls, field access, `?`, literals, paths, blocks, ...
    Unambiguous,
}

/// Which way a chain of same-precedence operators groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixity {
    /// `a - b - c` is `(a - b) - c`.
    Left,
    /// `a = b = c` is `a = (b = c)`.
    Right,
    /// `a == b == c` is an error.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_binds_tighter_than_sum_than_compare() {
        assert!(BinOpKind::Mul.precedence() > BinOpKind::Add.precedence());
        assert!(BinOpKind::Add.precedence() > BinOpKind::Lt.precedence());
        assert!(BinOpKind::Lt.precedence() > BinOpKind::And.precedence());
        assert!(BinOpKind::And.precedence() > BinOpKind::Or.precedence());
        assert!(BinOpKind::Or.precedence() > Precedence::Range);
        assert!(Precedence::Range > Precedence::Assign);
    }

    #[test]
    fn comparisons_do_not_associate() {
        assert_eq!(BinOpKind::Eq.fixity(), Fixity::None);
        assert_eq!(BinOpKind::Sub.fixity(), Fixity::Left);
    }
}
