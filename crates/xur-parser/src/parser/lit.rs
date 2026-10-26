//! Turning literal tokens into values, like rustc's `LitKind::from_token_lit`.
//!
//! A literal that fails to decode is reported here and becomes
//! [`LitKind::Err`], so parsing carries on as if it were valid.

use xur_ast::{Lit, LitKind};
use xur_errors::Diag;
use xur_span::{Span, Symbol};

use super::expected::Expected;
use super::{PResult, Parser};
use crate::token::{Keyword, TokenKind};
use crate::unescape::unescape_str;

impl Parser<'_> {
    pub fn parse_lit(&mut self) -> PResult<Lit> {
        match self.parse_opt_lit() {
            Some(lit) => Ok(lit),
            None => {
                self.expected(Expected::Lit);
                Err(self.unexpected())
            }
        }
    }

    /// Parses a literal if the current token is one.
    pub(super) fn parse_opt_lit(&mut self) -> Option<Lit> {
        let span = self.token.span;
        let kind = match self.token.kind {
            TokenKind::IntLit(text) => self.decode_int(text, span),
            TokenKind::FloatLit(text) => LitKind::Float(Symbol::intern(&text.replace('_', ""))),
            TokenKind::StringLit(text) => self.decode_str(text, span),
            TokenKind::Keyword(Keyword::True) => LitKind::Bool(true),
            TokenKind::Keyword(Keyword::False) => LitKind::Bool(false),
            _ => return None,
        };
        self.bump();
        Some(Lit { kind, span })
    }

    fn decode_int(&mut self, text: &str, span: Span) -> LitKind {
        match text.replace('_', "").parse() {
            Ok(value) => LitKind::Int(value),
            Err(_) => {
                self.report(
                    Diag::error("integer literal is too large", span)
                        .with_label(span, "does not fit in 64 bits"),
                );
                LitKind::Err
            }
        }
    }

    /// Decodes the contents of a string literal token spanning `span`,
    /// reporting every invalid escape.
    fn decode_str(&mut self, contents: &str, span: Span) -> LitKind {
        let contents_start = span.start + 1; // skip the opening quote
        let mut value = String::with_capacity(contents.len());
        let mut valid = true;
        unescape_str(contents, |range, result| match result {
            Ok(c) => value.push(c),
            Err(error) => {
                valid = false;
                let span = Span::new(contents_start + range.start, contents_start + range.end);
                self.diags
                    .push(Diag::error(error.to_string(), span).with_label(span, error.label()));
            }
        });
        if valid {
            LitKind::Str(Symbol::intern(&value))
        } else {
            LitKind::Err
        }
    }
}
