//! What the parser would have accepted, for "expected ..., found ..." errors.

use std::fmt;

use crate::token::TokenKind;

/// One thing the parser checked for at the current position, as in
/// rustc's `TokenType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Expected {
    /// A specific token, e.g. `;` or the keyword `fn`.
    Token(TokenKind<'static>),
    Ident,
    Lit,
    Type,
    Pat,
    Expr,
    Item,
    Operator,
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(kind) => write!(f, "`{kind}`"),
            Self::Ident => f.write_str("identifier"),
            Self::Lit => f.write_str("literal"),
            Self::Type => f.write_str("type"),
            Self::Pat => f.write_str("pattern"),
            Self::Expr => f.write_str("expression"),
            Self::Item => f.write_str("item"),
            Self::Operator => f.write_str("an operator"),
        }
    }
}

/// The distinct alternatives in `expected`, as display strings: specific
/// tokens sorted first, then categories such as "expression".
pub(super) fn alternatives(expected: &[Expected]) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut categories = Vec::new();
    for item in expected {
        let list = match item {
            Expected::Token(_) => &mut tokens,
            _ => &mut categories,
        };
        let text = item.to_string();
        if !list.contains(&text) {
            list.push(text);
        }
    }
    tokens.sort();
    tokens.extend(categories);
    tokens
}

/// Joins alternatives as rustc does: "`a`", "one of `a` or `b`",
/// "one of `a`, `b`, or `c`".
pub(super) fn join(alternatives: &[String]) -> String {
    match alternatives {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("one of {first} or {second}"),
        [init @ .., last] => format!("one of {}, or {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn describe(expected: &[Expected]) -> String {
        join(&alternatives(expected))
    }

    #[test]
    fn describe_lists() {
        let semi = Expected::Token(TokenKind::Semi);
        let comma = Expected::Token(TokenKind::Comma);
        let paren = Expected::Token(TokenKind::RParen);
        assert_eq!(describe(&[semi]), "`;`");
        assert_eq!(describe(&[semi, semi]), "`;`");
        assert_eq!(describe(&[semi, comma]), "one of `,` or `;`");
        assert_eq!(
            describe(&[Expected::Type, paren, comma]),
            "one of `)`, `,`, or type"
        );
    }
}
