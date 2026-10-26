//! Character cursor over the source text.
//!
//! Lookahead is done by cloning the underlying `Chars` iterator, which is just
//! a pointer copy, so peeking is cheap.

use std::str::Chars;

pub(super) struct Cursor<'src> {
    src: &'src str,
    chars: Chars<'src>,
}

impl<'src> Cursor<'src> {
    pub(super) fn new(src: &'src str) -> Self {
        Self {
            src,
            chars: src.chars(),
        }
    }

    /// Byte offset of the next character.
    pub(super) fn pos(&self) -> usize {
        self.src.len() - self.chars.as_str().len()
    }

    /// The source text from `start` up to the current position.
    pub(super) fn slice_from(&self, start: usize) -> &'src str {
        &self.src[start..self.pos()]
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    pub(super) fn peek_second(&self) -> Option<char> {
        let mut chars = self.chars.clone();
        chars.next();
        chars.next()
    }

    pub(super) fn bump(&mut self) -> Option<char> {
        self.chars.next()
    }

    /// Consumes the next character if it is `expected`.
    pub(super) fn eat(&mut self, expected: char) -> bool {
        let matched = self.peek() == Some(expected);
        if matched {
            self.bump();
        }
        matched
    }

    /// Consumes `prefix` if the remaining input starts with it.
    pub(super) fn eat_str(&mut self, prefix: &str) -> bool {
        match self.chars.as_str().strip_prefix(prefix) {
            Some(rest) => {
                self.chars = rest.chars();
                true
            }
            None => false,
        }
    }

    pub(super) fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) {
        while self.peek().is_some_and(&mut predicate) {
            self.bump();
        }
    }
}
