//! Lexer for the Xur language.
//!
//! - Zero-copy: tokens borrow slices from the original `&'src str`.
//! - Unicode identifiers via UAX #31 (`unicode-ident`).
//! - Nested block comments (`/* ... /* ... */ ... */`).
//! - Resilient: [`Lexer`] is an iterator that never stops on bad input;
//!   [`Lexer::tokenise`] is the strict entry point.

mod cursor;
#[cfg(test)]
mod tests;

use crate::error::LexError;
use crate::token::{Keyword, Span, Token, TokenKind};
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

    /// Tokenise the entire source, returning all tokens or the first `LexError`.
    pub fn tokenise(self) -> Result<Vec<Token<'src>>, LexError> {
        let mut tokens = Vec::new();
        for tok in self {
            match tok.kind {
                TokenKind::Unknown(c) => {
                    return Err(LexError::new(
                        format!("unexpected character `{c}`"),
                        tok.span,
                    ));
                }
                TokenKind::UnterminatedString => {
                    return Err(LexError::new("unterminated string literal", tok.span));
                }
                _ => tokens.push(tok),
            }
        }
        Ok(tokens)
    }

    /// Lexes the next token. Returns `Eof` forever once the input is exhausted.
    pub fn next_token(&mut self) -> Token<'src> {
        self.skip_trivia();

        let start = self.cursor.pos();
        let (line, col) = (self.cursor.line(), self.cursor.col());

        let kind = match self.cursor.bump() {
            None => TokenKind::Eof,
            Some(c) => self.lex_token(c, start),
        };

        Token::new(kind, Span::new(start, self.cursor.pos(), line, col))
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

            other => TokenKind::Unknown(other),
        }
    }

    /// Skips whitespace, line comments and (nested) block comments.
    fn skip_trivia(&mut self) {
        loop {
            self.cursor.eat_while(char::is_whitespace);

            if self.cursor.eat_str("//") {
                self.cursor.eat_while(|c| c != '\n');
            } else if self.cursor.eat_str("/*") {
                self.skip_block_comment();
            } else {
                break;
            }
        }
    }

    /// Skips the rest of a block comment whose opening `/*` was already consumed.
    fn skip_block_comment(&mut self) {
        let mut depth = 1u32;
        while depth > 0 {
            if self.cursor.eat_str("/*") {
                depth += 1;
            } else if self.cursor.eat_str("*/") {
                depth -= 1;
            } else if self.cursor.bump().is_none() {
                break;
            }
        }
    }

    /// Lexes a string literal whose opening `"` was already consumed.
    fn lex_string(&mut self) -> TokenKind<'src> {
        let content_start = self.cursor.pos();
        loop {
            match self.cursor.peek() {
                None | Some('\n') => return TokenKind::UnterminatedString,
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
        let parsed = if is_float {
            digits.parse().map(TokenKind::FloatLit).ok()
        } else {
            digits.parse().map(TokenKind::IntLit).ok()
        };
        parsed.unwrap_or(TokenKind::Unknown('0'))
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
