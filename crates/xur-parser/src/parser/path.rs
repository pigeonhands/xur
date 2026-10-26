use xur_ast::{GenericArgs, Path, PathSegment};

use super::{PResult, Parser};
use crate::token::TokenKind;

/// Where a path appears, which decides how generic arguments are written
/// (rustc's `PathStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// In expressions and patterns, generic arguments need the turbofish,
    /// `type_name::<T>`, since a bare `<` would be a comparison.
    Expr,
    /// In types, generic arguments follow directly: `Result<u32, E>`.
    Type,
    /// In `use` and attributes: no generic arguments at all.
    Mod,
}

impl Parser<'_> {
    pub fn parse_path(&mut self, style: PathStyle) -> PResult<Path> {
        let lo = self.token.span;
        let mut segments = Vec::new();
        loop {
            let ident = self.parse_ident()?;
            let args = self.parse_opt_generic_args(style)?;
            segments.push(PathSegment { ident, args });
            if !self.eat(TokenKind::ColonColon) {
                break;
            }
        }
        Ok(Path {
            segments,
            span: lo.to(self.prev_token.span),
        })
    }

    pub(super) fn parse_opt_generic_args(
        &mut self,
        style: PathStyle,
    ) -> PResult<Option<GenericArgs>> {
        let has_args = match style {
            PathStyle::Type => self.check(TokenKind::Lt),
            PathStyle::Expr => {
                let turbofish =
                    self.token.kind == TokenKind::ColonColon && self.look_ahead(1) == TokenKind::Lt;
                if turbofish {
                    self.bump(); // `::`
                }
                turbofish
            }
            PathStyle::Mod => false,
        };
        if !has_args {
            return Ok(None);
        }

        let lo = self.token.span;
        self.expect(TokenKind::Lt)?;
        let args = self.parse_delimited(TokenKind::Gt, Self::parse_ty)?;
        Ok(Some(GenericArgs {
            args,
            span: lo.to(self.prev_token.span),
        }))
    }
}
