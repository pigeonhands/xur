//! Lexer for the Xur language.
//!
//! - Zero-copy: tokens borrow slices from the original `&'src str`.
//! - Unicode identifiers via UAX #31 (`unicode-ident`).
//! - Nested block comments (`/* ... /* ... */ ... */`).
//! - Resilient: like rustc's lexer, errors are recorded on the side and a
//!   best-effort token is still produced (an unknown character is skipped,
//!   an unterminated string still becomes a string), so the parser never
//!   sees holes and every error in a file is reported in one pass.
//! - Literals keep their source text; turning them into values (and
//!   reporting overflow or bad escapes) is the parser's job.

mod cursor;
#[cfg(test)]
mod tests;

use crate::error::{LexError, LexErrorKind};
use crate::token::{Keyword, Token, TokenKind};
use cursor::Cursor;
use xur_span::{SourceFile, Span};

pub struct Lexer<'src> {
    cursor: Cursor<'src>,
    /// Global position of the start of the source, added to every span.
    offset: usize,
    errors: Vec<LexError>,
}

impl<'src> Lexer<'src> {
    /// Lexes a standalone string; spans start at position 0.
    pub fn new(src: &'src str) -> Self {
        Self::with_offset(src, 0)
    }

    /// Lexes a file from the source map, with spans in its global positions.
    pub fn for_file(file: &'src SourceFile) -> Self {
        Self::with_offset(&file.src, file.start)
    }

    fn with_offset(src: &'src str, offset: usize) -> Self {
        Self {
            cursor: Cursor::new(src),
            offset,
            errors: Vec::new(),
        }
    }

    /// Lexes the whole source.
    ///
    /// Returns every token (always ending with `Eof`) and every error found.
    /// Errors never leave gaps in the token stream.
    pub fn tokenise(mut self) -> (Vec<Token<'src>>, Vec<LexError>) {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token();
            tokens.push(token);
            if token.kind == TokenKind::Eof {
                return (tokens, self.errors);
            }
        }
    }

    fn next_token(&mut self) -> Token<'src> {
        loop {
            self.skip_trivia();

            let start = self.cursor.pos();
            let Some(first) = self.cursor.bump() else {
                return Token::new(TokenKind::Eof, self.span_from(start));
            };
            match self.lex_token(first, start) {
                Some(kind) => return Token::new(kind, self.span_from(start)),
                None => self.error(LexErrorKind::UnexpectedChar(first), start),
            }
        }
    }

    /// The span from cursor position `start` to the current position.
    fn span_from(&self, start: usize) -> Span {
        Span::new(self.offset + start, self.offset + self.cursor.pos())
    }

    /// Records an error covering `start` up to the current position.
    fn error(&mut self, kind: LexErrorKind, start: usize) {
        let span = self.span_from(start);
        self.errors.push(LexError { kind, span });
    }

    /// Lexes the token starting with the already-consumed `first`, or
    /// returns `None` if no token can start with it.
    fn lex_token(&mut self, first: char, start: usize) -> Option<TokenKind<'src>> {
        let kind = match first {
            '"' => self.lex_string(start),
            c if c.is_ascii_digit() => self.lex_number(start),
            c if is_ident_start(c) => self.lex_ident(start),
            c => return self.lex_punct(c),
        };
        Some(kind)
    }

    /// Lexes punctuation and operators, preferring the longest match.
    fn lex_punct(&mut self, first: char) -> Option<TokenKind<'src>> {
        let c = &mut self.cursor;
        let kind = match first {
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
            '?' => TokenKind::Question,

            ':' if c.eat(':') => TokenKind::ColonColon,
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
            '=' if c.eat('>') => TokenKind::FatArrow,
            '=' => TokenKind::Eq,
            '!' if c.eat('=') => TokenKind::BangEq,
            '!' => TokenKind::Bang,
            '<' if c.eat('=') => TokenKind::LtEq,
            '<' => TokenKind::Lt,
            '>' if c.eat('=') => TokenKind::GtEq,
            '>' => TokenKind::Gt,
            '&' if c.eat('&') => TokenKind::AmpAmp,
            '|' if c.eat('|') => TokenKind::PipePipe,
            '|' => TokenKind::Pipe,
            '@' => TokenKind::At,

            _ => return None,
        };
        Some(kind)
    }

    /// Skips whitespace, line comments and (nested) block comments.
    ///
    fn skip_trivia(&mut self) {
        loop {
            self.cursor.eat_while(char::is_whitespace);

            let start = self.cursor.pos();
            if self.cursor.eat_str("//") {
                self.cursor.eat_while(|c| c != '\n');
            } else if self.cursor.eat_str("/*") {
                if !self.skip_block_comment() {
                    self.error(LexErrorKind::UnterminatedBlockComment, start);
                }
            } else {
                return;
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
    ///
    /// An unterminated string ends at the end of its line.
    fn lex_string(&mut self, start: usize) -> TokenKind<'src> {
        let content_start = self.cursor.pos();
        loop {
            match self.cursor.peek() {
                None | Some('\n') => {
                    self.error(LexErrorKind::UnterminatedString, start);
                    return TokenKind::StringLit(self.cursor.slice_from(content_start));
                }
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
        // `0x1F`, `0o17`, `0b101`: the digits are checked when decoded, so
        // a bad digit is reported as such rather than ending the number.
        if self.cursor.slice_from(start) == "0"
            && matches!(self.cursor.peek(), Some('x' | 'o' | 'b'))
        {
            self.cursor.bump();
            self.cursor
                .eat_while(|c| c.is_ascii_alphanumeric() || c == '_');
            return TokenKind::IntLit(self.cursor.slice_from(start));
        }
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

        let text = self.cursor.slice_from(start);
        if is_float {
            TokenKind::FloatLit(text)
        } else {
            TokenKind::IntLit(text)
        }
    }

    fn lex_ident(&mut self, start: usize) -> TokenKind<'src> {
        self.cursor.eat_while(unicode_ident::is_xid_continue);
        match self.cursor.slice_from(start) {
            "_" => TokenKind::Underscore,
            "b" if self.cursor.eat('"') => match self.lex_string(start) {
                TokenKind::StringLit(content) => TokenKind::ByteStrLit(content),
                other => other,
            },
            name => Keyword::try_from(name).map_or(TokenKind::Ident(name), TokenKind::Keyword),
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}
