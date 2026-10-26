use xur_ast::{Pat, PatKind};

use super::expected::Expected;
use super::{PResult, Parser, PathStyle};
use crate::token::TokenKind;

impl Parser<'_> {
    pub fn parse_pat(&mut self) -> PResult<Pat> {
        let lo = self.token.span;
        let kind = if self.eat_noexpect(TokenKind::Underscore) {
            PatKind::Wild
        } else if let Some(lit) = self.parse_opt_lit() {
            PatKind::Lit(lit)
        } else if self.eat_noexpect(TokenKind::LParen) {
            let (mut pats, trailing_comma) =
                self.parse_comma_seq(TokenKind::RParen, Self::parse_pat)?;
            if pats.len() == 1 && !trailing_comma {
                PatKind::Paren(Box::new(pats.remove(0)))
            } else {
                PatKind::Tuple(pats)
            }
        } else if matches!(self.token.kind, TokenKind::Ident(_)) {
            self.parse_pat_path()?
        } else {
            self.expected(Expected::Pat);
            return Err(self.unexpected());
        };
        Ok(Pat {
            id: self.next_id(),
            kind,
            span: lo.to(self.prev_token.span),
        })
    }

    /// Parses a pattern starting with a path: `x`, `None`, `Ok(v)`.
    fn parse_pat_path(&mut self) -> PResult<PatKind> {
        let path = self.parse_path(PathStyle::Expr)?;
        if self.eat(TokenKind::LParen) {
            let fields = self.parse_delimited(TokenKind::RParen, Self::parse_pat)?;
            return Ok(PatKind::TupleStruct(path, fields));
        }
        Ok(match path.as_ident() {
            Some(ident) => PatKind::Ident(ident),
            None => PatKind::Path(path),
        })
    }
}
