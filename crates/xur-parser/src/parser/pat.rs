use xur_ast::{Pat, PatField, PatKind, Path};
use xur_errors::Diag;

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

    /// Parses a pattern starting with a path: `x`, `None`, `Ok(v)`,
    /// `Point { x, y }`.
    fn parse_pat_path(&mut self) -> PResult<PatKind> {
        let path = self.parse_path(PathStyle::Expr)?;
        if self.eat(TokenKind::LParen) {
            let fields = self.parse_delimited(TokenKind::RParen, Self::parse_pat)?;
            return Ok(PatKind::TupleStruct(path, fields));
        }
        if self.eat(TokenKind::LBrace) {
            return self.parse_pat_struct(path);
        }
        Ok(match path.as_ident() {
            Some(ident) => PatKind::Ident(ident),
            None => PatKind::Path(path),
        })
    }

    /// The fields of a struct pattern, after the `{`; `..` may come last.
    fn parse_pat_struct(&mut self, path: Path) -> PResult<PatKind> {
        let mut rest = None;
        let fields = self.parse_delimited(TokenKind::RBrace, |this| {
            if this.eat(TokenKind::DotDot) {
                rest = Some(this.prev_token.span);
                return Ok(None);
            }
            if let Some(span) = rest {
                this.report(Diag::error(
                    "`..` must be the last field of a pattern",
                    span,
                ));
            }
            let lo = this.token.span;
            let ident = this.parse_ident()?;
            let (pat, shorthand) = if this.eat(TokenKind::Colon) {
                (this.parse_pat()?, false)
            } else {
                let pat = Pat {
                    id: this.next_id(),
                    kind: PatKind::Ident(ident),
                    span: ident.span,
                };
                (pat, true)
            };
            Ok(Some(PatField {
                ident,
                pat,
                shorthand,
                span: lo.to(this.prev_token.span),
            }))
        })?;
        Ok(PatKind::Struct {
            path,
            fields: fields.into_iter().flatten().collect(),
            rest: rest.is_some(),
        })
    }
}
