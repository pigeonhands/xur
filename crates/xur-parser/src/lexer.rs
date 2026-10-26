//! Lexer / tokeniser for the Xur language.
//!
//! Designed after `rustc_lexer`:
//! - Zero-copy tokens borrowing slices from the original `&'src str`.
//! - Lookahead via `Chars<'src>` cloning (pointer copying).
//! - Unicode identifier support via UAX #31 (`unicode-ident`).
//! - Nested block comment support (`/* ... /* ... */ ... */`).
//! - `eat_while` cursor predicate helper.
//! - `Iterator<Item = Token<'src>>` implementation for streaming and resilient lexing.

use std::str::Chars;

use crate::error::LexError;
use crate::token::{Keyword, Span, Token, TokenKind};

pub struct Lexer<'src> {
    src: &'src str,
    chars: Chars<'src>,
    line: u32,
    col: u32,
    hit_eof: bool,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            src,
            chars: src.chars(),
            line: 1,
            col: 1,
            hit_eof: false,
        }
    }

    fn remaining(&self) -> &str {
        self.chars.as_str()
    }

    fn pos(&self) -> usize {
        self.src.len() - self.remaining().len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut iter = self.chars.clone();
        iter.next();
        iter.next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn eat_if(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) {
        while matches!(self.peek(), Some(c) if predicate(c)) {
            self.advance();
        }
    }

    fn span_from(&self, start: usize, line: u32, col: u32) -> Span {
        Span::new(start, self.pos(), line, col)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.eat_while(|c| c.is_whitespace());

            if self.remaining().starts_with("//") {
                self.eat_while(|c| c != '\n');
                continue;
            }

            if self.remaining().starts_with("/*") {
                self.advance(); // '/'
                self.advance(); // '*'
                let mut depth: u32 = 1;
                while depth > 0 {
                    if self.remaining().starts_with("/*") {
                        self.advance();
                        self.advance();
                        depth += 1;
                    } else if self.remaining().starts_with("*/") {
                        self.advance();
                        self.advance();
                        depth -= 1;
                    } else if self.advance().is_none() {
                        break;
                    }
                }
                continue;
            }

            break;
        }
    }

    fn lex_string(&mut self) -> TokenKind<'src> {
        let content_start = self.pos();
        let mut terminated = false;

        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.advance();
            if c == '"' {
                terminated = true;
                break;
            }
            if c == '\\' {
                self.advance();
            }
        }

        if terminated {
            let content = &self.src[content_start..self.pos() - 1];
            TokenKind::StringLit(content)
        } else {
            TokenKind::UnterminatedString
        }
    }

    fn lex_number(&mut self, start: usize) -> TokenKind<'src> {
        self.eat_while(|c| c.is_ascii_digit() || c == '_');

        let mut is_float = false;
        if self.peek() == Some('.') && matches!(self.peek2(), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.advance(); // consume '.'
            self.eat_while(|c| c.is_ascii_digit() || c == '_');
        }

        let raw = &self.src[start..self.pos()];

        if is_float {
            let filtered: String = raw.chars().filter(|&c| c != '_').collect();
            filtered
                .parse::<f64>()
                .map(TokenKind::FloatLit)
                .unwrap_or(TokenKind::Unknown('0'))
        } else {
            let filtered: String = raw.chars().filter(|&c| c != '_').collect();
            filtered
                .parse::<i64>()
                .map(TokenKind::IntLit)
                .unwrap_or(TokenKind::Unknown('0'))
        }
    }

    fn lex_ident(&mut self, start: usize) -> TokenKind<'src> {
        self.eat_while(unicode_ident::is_xid_continue);
        let name = &self.src[start..self.pos()];
        Keyword::try_from(name)
            .map(TokenKind::Keyword)
            .unwrap_or(TokenKind::Ident(name))
    }

    /// Fetches the next token from source without bailing on errors (resilient).
    pub fn next_token(&mut self) -> Token<'src> {
        self.skip_whitespace_and_comments();

        let start = self.pos();
        let line = self.line;
        let col = self.col;

        let ch = match self.advance() {
            None => return Token::new(TokenKind::Eof, self.span_from(start, line, col)),
            Some(c) => c,
        };

        let kind = match ch {
            '"' => self.lex_string(),

            c if c.is_ascii_digit() => self.lex_number(start),

            c if unicode_ident::is_xid_start(c) || c == '_' => self.lex_ident(start),

            ':' if self.eat_if(':') => {
                if self.eat_if('<') {
                    TokenKind::ColonColonLt
                } else {
                    TokenKind::ColonColon
                }
            }
            '#' => TokenKind::Hash,
            '-' if self.eat_if('>') => TokenKind::Arrow,
            '=' if self.eat_if('=') => TokenKind::EqEq,
            '!' if self.eat_if('=') => TokenKind::BangEq,
            '<' if self.eat_if('=') => TokenKind::LtEq,
            '>' if self.eat_if('=') => TokenKind::GtEq,
            '&' if self.eat_if('&') => TokenKind::AmpAmp,
            '|' if self.eat_if('|') => TokenKind::PipePipe,

            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            '.' if self.eat_if('.') => {
                if self.eat_if('=') {
                    TokenKind::DotDotEq
                } else {
                    TokenKind::DotDot
                }
            }
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            '=' => TokenKind::Eq,
            '!' => TokenKind::Bang,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,

            other => TokenKind::Unknown(other),
        };

        Token::new(kind, self.span_from(start, line, col))
    }

    /// Tokenise the entire source, returning all tokens or the first `LexError`.
    pub fn tokenise(mut self) -> Result<Vec<Token<'src>>, LexError> {
        let mut tokens = Vec::new();
        for tok in &mut self {
            match tok.kind {
                TokenKind::Unknown(c) => {
                    return Err(LexError::new(format!("unexpected character `{c}`"), tok.span));
                }
                TokenKind::UnterminatedString => {
                    return Err(LexError::new("unterminated string literal", tok.span));
                }
                _ => tokens.push(tok),
            }
        }
        Ok(tokens)
    }
}

impl<'src> Iterator for Lexer<'src> {
    type Item = Token<'src>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.hit_eof {
            return None;
        }
        let tok = self.next_token();
        if tok.kind == TokenKind::Eof {
            self.hit_eof = true;
        }
        Some(tok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{Keyword, TokenKind};

    fn lex(src: &str) -> Vec<TokenKind<'_>> {
        Lexer::new(src)
            .tokenise()
            .expect("lex failed")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    fn kw<'a>(k: Keyword) -> TokenKind<'a> {
        TokenKind::Keyword(k)
    }
    fn id<'a>(s: &'a str) -> TokenKind<'a> {
        TokenKind::Ident(s)
    }

    #[test]
    fn test_use_statement() {
        let kinds = lex("use std;");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Use),
                id("std"),
                TokenKind::Semi,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_path_access() {
        let kinds = lex("std::io");
        assert_eq!(
            kinds,
            vec![
                id("std"),
                TokenKind::ColonColon,
                id("io"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_dot_access() {
        let kinds = lex("math.add");
        assert_eq!(
            kinds,
            vec![
                id("math"),
                TokenKind::Dot,
                id("add"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_fn_signature() {
        let kinds = lex("pub fn add(a: u32, b: u32) -> u32");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Pub),
                kw(Keyword::Fn),
                id("add"),
                TokenKind::LParen,
                id("a"),
                TokenKind::Colon,
                id("u32"),
                TokenKind::Comma,
                id("b"),
                TokenKind::Colon,
                id("u32"),
                TokenKind::RParen,
                TokenKind::Arrow,
                id("u32"),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_string_literal() {
        let kinds = lex(r#""Hello, World""#);
        assert_eq!(
            kinds,
            vec![
                TokenKind::StringLit("Hello, World"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_string_interpolation_holes_pass_through() {
        let kinds = lex(r#""1 + 2 = {}""#);
        assert_eq!(
            kinds,
            vec![
                TokenKind::StringLit("1 + 2 = {}"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_number_literals() {
        let kinds = lex("42 3.14 1_000_000");
        assert_eq!(
            kinds,
            vec![
                TokenKind::IntLit(42),
                TokenKind::FloatLit(3.14),
                TokenKind::IntLit(1_000_000),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_line_comment_skipped() {
        let kinds = lex("let x // this is a comment\nlet y");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Let),
                id("x"),
                kw(Keyword::Let),
                id("y"),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_nested_block_comments() {
        let kinds = lex("let a; /* outer /* inner */ still outer */ let b;");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Let),
                id("a"),
                TokenKind::Semi,
                kw(Keyword::Let),
                id("b"),
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_unicode_identifiers() {
        let kinds = lex("let α_beta = 10;");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Let),
                id("α_beta"),
                TokenKind::Eq,
                TokenKind::IntLit(10),
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_iterator_streaming_and_resilient_recovery() {
        let mut lexer = Lexer::new("let 42? // unknown char ?");
        let t1 = lexer.next().unwrap();
        assert_eq!(t1.kind, kw(Keyword::Let));
        let t2 = lexer.next().unwrap();
        assert_eq!(t2.kind, TokenKind::IntLit(42));
        let t3 = lexer.next().unwrap();
        assert_eq!(t3.kind, TokenKind::Unknown('?'));
        let t4 = lexer.next().unwrap();
        assert_eq!(t4.kind, TokenKind::Eof);
        assert!(lexer.next().is_none());
    }

    #[test]
    fn test_const_import() {
        let kinds = lex(r#"const math = std::import("./math.xur");"#);
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::Const),
                id("math"),
                TokenKind::Eq,
                id("std"),
                TokenKind::ColonColon,
                id("import"),
                TokenKind::LParen,
                TokenKind::StringLit("./math.xur"),
                TokenKind::RParen,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_loop_constructs() {
        let kinds = lex("for i in 0..10 { if i == 5 { break; } else { continue; } }");
        assert_eq!(
            kinds,
            vec![
                kw(Keyword::For),
                id("i"),
                kw(Keyword::In),
                TokenKind::IntLit(0),
                TokenKind::DotDot,
                TokenKind::IntLit(10),
                TokenKind::LBrace,
                kw(Keyword::If),
                id("i"),
                TokenKind::EqEq,
                TokenKind::IntLit(5),
                TokenKind::LBrace,
                kw(Keyword::Break),
                TokenKind::Semi,
                TokenKind::RBrace,
                kw(Keyword::Else),
                TokenKind::LBrace,
                kw(Keyword::Continue),
                TokenKind::Semi,
                TokenKind::RBrace,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_range_operators() {
        let kinds = lex("0..10 1..=5");
        assert_eq!(
            kinds,
            vec![
                TokenKind::IntLit(0),
                TokenKind::DotDot,
                TokenKind::IntLit(10),
                TokenKind::IntLit(1),
                TokenKind::DotDotEq,
                TokenKind::IntLit(5),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_turbofish() {
        let kinds = lex("print_name::<ZeroPacked>();");
        assert_eq!(
            kinds,
            vec![
                id("print_name"),
                TokenKind::ColonColonLt,
                id("ZeroPacked"),
                TokenKind::Gt,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_attributes_and_struct() {
        let kinds = lex("#[repr(C)] struct Foo { a: u32 }");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Hash,
                TokenKind::LBracket,
                id("repr"),
                TokenKind::LParen,
                id("C"),
                TokenKind::RParen,
                TokenKind::RBracket,
                kw(Keyword::Struct),
                id("Foo"),
                TokenKind::LBrace,
                id("a"),
                TokenKind::Colon,
                id("u32"),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_keyword_display() {
        assert_eq!(TokenKind::Keyword(Keyword::Fn).to_string(), "fn");
        assert_eq!(TokenKind::Keyword(Keyword::Use).to_string(), "use");
        assert_eq!(TokenKind::Keyword(Keyword::Struct).to_string(), "struct");
        assert_eq!(TokenKind::Ident("foo").to_string(), "<ident>");
        assert_eq!(TokenKind::ColonColon.to_string(), "::");
        assert_eq!(TokenKind::ColonColonLt.to_string(), "::<");
        assert_eq!(TokenKind::Hash.to_string(), "#");
        assert_eq!(TokenKind::Arrow.to_string(), "->");
    }

    #[test]
    fn test_example_main_file() {
        let content = std::fs::read_to_string("../../example/main.xur")
            .expect("read example/main.xur");
        let toks = Lexer::new(&content).tokenise().expect("should lex example/main.xur cleanly");
        assert!(!toks.is_empty());
        assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
    }

    #[test]
    fn test_full_main() {
        let src = r#"
            use std;
            const math = std::import("./math.xur");
            fn main() {
                let io = std::io;
                io::println("Hello, World");
                io::println("1 + 2 = {}", math.add(1, 2));
            }
        "#;
        let toks = Lexer::new(src).tokenise().expect("should lex cleanly");
        assert!(!toks.is_empty());
        assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
    }
}
