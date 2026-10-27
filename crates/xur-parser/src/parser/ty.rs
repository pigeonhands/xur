use xur_ast::{Ty, TyKind};

use super::expected::Expected;
use super::{PResult, Parser, PathStyle};
use crate::token::TokenKind;

impl Parser<'_> {
    pub fn parse_ty(&mut self) -> PResult<Ty> {
        let lo = self.token.span;
        let kind = if self.eat_noexpect(TokenKind::LParen) {
            let (mut tys, trailing_comma) =
                self.parse_comma_seq(TokenKind::RParen, Self::parse_ty)?;
            if tys.len() == 1 && !trailing_comma {
                TyKind::Paren(Box::new(tys.remove(0)))
            } else {
                TyKind::Tuple(tys)
            }
        } else if self.eat_noexpect(TokenKind::LBracket) {
            let element = self.parse_ty()?;
            self.expect(TokenKind::RBracket)?;
            TyKind::List(Box::new(element))
        } else if self.eat_noexpect(TokenKind::At) {
            TyKind::Record
        } else if self.eat_noexpect(TokenKind::Underscore) {
            TyKind::Infer
        } else if matches!(self.token.kind, TokenKind::Ident(_)) {
            TyKind::Path(self.parse_path(PathStyle::Type)?)
        } else {
            self.expected(Expected::Type);
            return Err(self.unexpected());
        };
        Ok(Ty {
            kind,
            span: lo.to(self.prev_token.span),
        })
    }
}
