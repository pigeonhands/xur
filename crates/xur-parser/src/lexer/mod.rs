//! Lexer for the Xur language.
//!
//! - Zero-copy: tokens borrow slices from the original `&'src str`.
//! - Unicode identifiers via UAX #31 (`unicode-ident`).
//! - Nested block comments (`/* ... /* ... */ ... */`).
//! - Resilient: malformed input becomes a [`TokenKind::Error`] token and
//!   lexing carries on, so every error in a file is reported in one pass.

mod cursor;
#[cfg(test)]
mod tests;

use crate::error::{LexError, LexErrorKind};
use crate::span::Span;
use crate::token::{Keyword, Token, TokenKind};
use cursor::Cursor;

pub struct Lexer<'src> {
    cursor: Cursor<'src>,
    finished: bool,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            cursor: Cursor::new(src),
            finished: false,
        }
    }

    /// Lexes the whole source.
    ///
    /// Returns the valid tokens (ending with `Eof`) and, separately, every
    /// error encountered. Error tokens are not included in the token list.
    pub fn tokenise(self) -> (Vec<Token<'src>>, Vec<LexError>) {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();
        for token in self {
            match token.kind {
                TokenKind::Error(kind) => errors.push(LexError {
                    kind,
                    span: token.span,
                }),
                _ => tokens.push(token),
            }
        }
        (tokens, errors)
    }

    /// Lexes the next token. Returns `Eof` forever once the input is exhausted.
    pub fn next_token(&mut self) -> Token<'src> {
        if let Err(error) = self.skip_trivia() {
            return error;
        }

        let start = self.cursor.pos();
        let kind = match self.cursor.bump() {
            None => TokenKind::Eof,
            Some(c) => self.lex_token(c, start),
        };
        Token::new(kind, self.span_from(start))
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.cursor.pos())
    }

    fn lex_token(&mut self, first: char, start: usize) -> TokenKind<'src> {
        match first {
            '"' => self.lex_string(),
            c if c.is_ascii_digit() => self.lex_number(start),
            c if is_ident_start(c) => self.lex_ident(start),
            c => self.lex_punct(c),
        }
    }

    /// Lexes punctuation and operators, preferring the longest match.
    fn lex_punct(&mut self, first: char) -> TokenKind<'src> {
        let c = &mut self.cursor;
        match first {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            '#' => TokenKind::Hash,
            '+' => TokenKind::Plus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,

            ':' if c.eat(':') => {
                if c.eat('<') {
                    TokenKind::ColonColonLt
                } else {
                    TokenKind::ColonColon
                }
            }
            ':' => TokenKind::Colon,
            '.' if c.eat('.') => {
                if c.eat('=') {
                    TokenKind::DotDotEq
                } else {
                    TokenKind::DotDot
                }
            }
            '.' => TokenKind::Dot,
            '-' if c.eat('>') => TokenKind::Arrow,
            '-' => TokenKind::Minus,
            '=' if c.eat('=') => TokenKind::EqEq,
            '=' => TokenKind::Eq,
            '!' if c.eat('=') => TokenKind::BangEq,
            '!' => TokenKind::Bang,
            '<' if c.eat('=') => TokenKind::LtEq,
            '<' => TokenKind::Lt,
            '>' if c.eat('=') => TokenKind::GtEq,
            '>' => TokenKind::Gt,
            '&' if c.eat('&') => TokenKind::AmpAmp,
            '|' if c.eat('|') => TokenKind::PipePipe,

            other => TokenKind::Error(LexErrorKind::UnexpectedChar(other)),
        }
    }

    /// Skips whitespace, line comments and (nested) block comments.
    ///
    /// A block comment still open at end of input is returned as an error token.
    fn skip_trivia(&mut self) -> Result<(), Token<'src>> {
        loop {
            self.cursor.eat_while(char::is_whitespace);

            let start = self.cursor.pos();
            if self.cursor.eat_str("//") {
                self.cursor.eat_while(|c| c != '\n');
            } else if self.cursor.eat_str("/*") {
                if !self.skip_block_comment() {
                    let kind = TokenKind::Error(LexErrorKind::UnterminatedBlockComment);
                    return Err(Token::new(kind, self.span_from(start)));
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Skips the rest of a block comment whose opening `/*` was already consumed.
    /// Returns `false` if the input ended before the comment was closed.
    fn skip_block_comment(&mut self) -> bool {
        let mut depth = 1u32;
        while depth > 0 {
            if self.cursor.eat_str("/*") {
                depth += 1;
            } else if self.cursor.eat_str("*/") {
                depth -= 1;
            } else if self.cursor.bump().is_none() {
                return false;
            }
        }
        true
    }

    /// Lexes a string literal whose opening `"` was already consumed.
    fn lex_string(&mut self) -> TokenKind<'src> {
        let content_start = self.cursor.pos();
        loop {
            match self.cursor.peek() {
                None | Some('\n') => return TokenKind::Error(LexErrorKind::UnterminatedString),
                Some('"') => break,
                Some('\\') => {
                    self.cursor.bump();
                    self.cursor.bump();
                }
                Some(_) => {
                    self.cursor.bump();
                }
            }
        }
        let content = self.cursor.slice_from(content_start);
        self.cursor.bump(); // closing '"'
        TokenKind::StringLit(content)
    }

    fn lex_number(&mut self, start: usize) -> TokenKind<'src> {
        let is_digit = |c: char| c.is_ascii_digit() || c == '_';
        self.cursor.eat_while(is_digit);

        let is_float = self.cursor.peek() == Some('.')
            && self
                .cursor
                .peek_second()
                .is_some_and(|c| c.is_ascii_digit());
        if is_float {
            self.cursor.bump(); // '.'
            self.cursor.eat_while(is_digit);
        }

        let digits = self.cursor.slice_from(start).replace('_', "");
        if is_float {
            // `digits.digits` always parses; values beyond `f64::MAX` become infinity.
            TokenKind::FloatLit(digits.parse().expect("lexed float is well-formed"))
        } else {
            digits.parse().map_or(
                TokenKind::Error(LexErrorKind::IntTooLarge),
                TokenKind::IntLit,
            )
        }
    }

    fn lex_ident(&mut self, start: usize) -> TokenKind<'src> {
        self.cursor.eat_while(unicode_ident::is_xid_continue);
        let name = self.cursor.slice_from(start);
        Keyword::try_from(name).map_or(TokenKind::Ident(name), TokenKind::Keyword)
    }
}

impl<'src> Iterator for Lexer<'src> {
    type Item = Token<'src>;

    /// Yields every token including a final `Eof`, then `None`.
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let tok = self.next_token();
        self.finished = tok.kind == TokenKind::Eof;
        Some(tok)
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}
