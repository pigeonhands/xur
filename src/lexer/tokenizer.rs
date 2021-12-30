use super::token::{Token, TokenKind, EOF_CHAR};
use std::str::Chars;
use unicode_xid::UnicodeXID;

#[derive(Clone, Debug)]
pub struct Tokenizer<'a> {
    initial_len: usize,
    chars: Chars<'a>,
    prev: char,
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.reset_len_consumed();

        let kind = match self.bump()? {
            c if c.is_whitespace() => self.whitespace(),
            '@' => match self.first() {
                s if s.is_xid_start() => {
                    self.bump();
                    TokenKind::Symbol(self.ident())
                }
                '"' => {
                    self.bump();
                    TokenKind::Symbol(self.string())
                }
                _ => TokenKind::At,
            },
            c if c.is_numeric() => self.number(),
            c if c.is_xid_start() => TokenKind::Identifier(self.ident()),
            '"' => TokenKind::String(self.string()),

            ':' => TokenKind::Colon,
            '=' => TokenKind::Equals,
            ';' => TokenKind::Semicolon,
            '+' => TokenKind::Identifier("+".into()),//TokenKind::Plus,
            '.' => TokenKind::Identifier(".".into()),
            '(' => TokenKind::OpenParen,
            ')' => TokenKind::CloseParen,
            '[' => TokenKind::OpenBracket,
            ']' => TokenKind::CloseBracket,
            ',' => TokenKind::Comma,
            c => TokenKind::Unknown(c.into()),
        };
        Some(Token {
            kind,
            length: self.len_consumed(),
        })
    }
}

impl<'a> Tokenizer<'a> {
    pub fn new(s: &'a str) -> Self {
        Self {
            initial_len: s.len(),
            chars: s.chars(),
            prev: EOF_CHAR,
        }
    }
    pub fn nth(&self, i: usize) -> char {
        self.chars.clone().nth(i).unwrap_or(EOF_CHAR)
    }

    pub fn first(&self) -> char {
        self.chars.clone().next().unwrap_or(EOF_CHAR)
    }

    pub fn bump(&mut self) -> Option<char> {
        self.prev = self.chars.next()?;
        Some(self.prev)
    }

    pub fn second(&self) -> char {
        let mut iter = self.chars.clone();
        iter.next();
        iter.next().unwrap_or(EOF_CHAR)
    }

    pub fn is_eof(&self) -> bool {
        self.chars.clone().next().is_none()
    }

    pub fn len_consumed(&self) -> usize {
        self.initial_len - self.chars.as_str().len()
    }

    pub fn reset_len_consumed(&mut self) {
        self.initial_len = self.chars.as_str().len();
    }

    fn eat_while(
        &mut self,
        mut predicate: impl FnMut(char) -> bool,
        mut cb: Option<impl FnMut(char)>,
    ) {
        while predicate(self.first()) && !self.is_eof() {
            match (self.bump(), &mut cb) {
                (Some(c), Some(f)) => f(c),
                _ => continue,
            }
        }
    }

    pub fn whitespace(&mut self) -> TokenKind {
        debug_assert!(self.prev.is_whitespace());
        self.eat_while(char::is_whitespace, None::<fn(char)>);
        TokenKind::Whitespace
    }

    pub fn ident(&mut self) -> String {
        debug_assert!(self.prev.is_xid_start());
        let mut s = String::new();
        s.push(self.prev);
        self.eat_while(char::is_xid_continue, Some(|c| s.push(c)));
        s
    }
    pub fn number(&mut self) -> TokenKind {
        debug_assert!(self.prev.is_numeric());
        if self.prev == '0' && matches!(self.first(), 'x' | 'X') {
            self.bump();
            let mut s = String::new();
            s.push(self.prev);
            self.eat_while(|c| c.is_ascii_hexdigit(), Some(|c| s.push(c)));

            if let Ok(n) = u128::from_str_radix(&s, 16) {
                TokenKind::Numeric(n)
            } else {
                TokenKind::Unknown(s)
            }
        } else {
            let mut s = String::new();
            s.push(self.prev);
            self.eat_while(char::is_numeric, Some(|c| s.push(c)));

            if let Ok(n) = s.parse() {
                TokenKind::Numeric(n)
            } else {
                TokenKind::Unknown(s)
            }
        }
    }
    pub fn string(&mut self) -> String {
        debug_assert!(self.prev == '"');
        let mut s = String::new();

        while let Some(c) = self.bump() {
            match c {
                '\\' if self.first() == '"' => {
                    s.push(c);
                    if let Some(esc) = self.bump() {
                        s.push(esc);
                    }
                }
                '"' => break,
                _ => {
                    s.push(c);
                }
            }
        }
        s
    }
}
