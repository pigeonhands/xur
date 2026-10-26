//! Recursive-descent parser producing the AST, modelled on `rustc_parse`.
//!
//! - The parser walks the token list keeping `token` (current) and
//!   `prev_token`; a node's span is `lo.to(self.prev_token.span)`.
//! - `check`, `eat` and `expect` record what would have been accepted at the
//!   current position, so errors read "expected one of `,` or `)`, found `;`".
//! - Parsing functions return [`PResult`]. Recovery points (items and
//!   statements) report the error, skip ahead and carry on, leaving an `Err`
//!   node in the tree.

mod expected;
mod expr;
mod item;
mod lit;
mod pat;
mod path;
mod stmt;
mod ty;

#[cfg(test)]
mod tests;

use xur_ast::{Module, NodeId, NodeIdGen};
use xur_errors::Diag;
use xur_span::{Ident, Span, Symbol};

use self::expected::Expected;
use crate::lexer::Lexer;
use crate::token::{Keyword, Token, TokenKind};

pub use self::path::PathStyle;

/// Lexes and parses a whole source file, returning the module (complete
/// even if there were errors, with `Err` nodes where parsing failed) and
/// every diagnostic. Node ids are taken from `ids`.
pub fn parse_module(src: &str, ids: &mut NodeIdGen) -> (Module, Vec<Diag>) {
    let mut parser = Parser::new(src);
    parser.ids = std::mem::take(ids);
    let module = parser.parse_module();
    *ids = std::mem::take(&mut parser.ids);
    (module, parser.finish())
}

/// The result of a parsing function. The error has not been reported yet:
/// the caller either reports it or propagates it to a recovery point.
pub type PResult<T> = Result<T, Diag>;

pub struct Parser<'src> {
    src: &'src str,
    /// Always ends with `Eof`.
    tokens: Vec<Token<'src>>,
    /// Index of `token` in `tokens`.
    pos: usize,
    /// The current token.
    token: Token<'src>,
    /// The previously consumed token; its span ends the node being parsed.
    prev_token: Token<'src>,
    /// Everything checked for since the last `bump`.
    expected: Vec<Expected>,
    diags: Vec<Diag>,
    restrictions: Restrictions,
    ids: NodeIdGen,
}

/// Context that changes how an expression is parsed (rustc's
/// `Restrictions`).
#[derive(Debug, Clone, Copy, Default)]
struct Restrictions {
    /// Parsing an expression statement: a block-like expression such as `if`
    /// ends the statement, so `if c {} -1` is two statements, not a
    /// subtraction.
    stmt_expr: bool,
    /// Parsing the head of an `if`, `while`, `match` or `for`, where a `{`
    /// begins the body: `for i in 0.. {}` is an open-ended range.
    before_block: bool,
}

impl<'src> Parser<'src> {
    /// Lexes `src` and prepares to parse it. Lex errors are kept and returned
    /// by [`Parser::finish`] along with any parse errors.
    pub fn new(src: &'src str) -> Self {
        let (tokens, lex_errors) = Lexer::new(src).tokenise();
        let token = tokens[0];
        Self {
            src,
            tokens,
            pos: 0,
            token,
            prev_token: Token::new(TokenKind::Eof, Span::default()),
            expected: Vec::new(),
            diags: lex_errors.into_iter().map(Diag::from).collect(),
            restrictions: Restrictions::default(),
            ids: NodeIdGen::default(),
        }
    }

    /// Returns every diagnostic reported while lexing and parsing.
    pub fn finish(self) -> Vec<Diag> {
        self.diags
    }

    /// Whether all input has been consumed.
    pub fn is_at_end(&self) -> bool {
        self.token.kind == TokenKind::Eof
    }

    /// Errors unless all input has been consumed.
    pub fn expect_end(&mut self) -> PResult<()> {
        self.expect(TokenKind::Eof)
    }

    fn next_id(&mut self) -> NodeId {
        self.ids.next_id()
    }

    fn report(&mut self, diag: Diag) {
        self.diags.push(diag);
    }

    /// Advances to the next token. `Eof` is never advanced past.
    fn bump(&mut self) {
        self.prev_token = self.token;
        self.pos = (self.pos + 1).min(self.tokens.len() - 1);
        self.token = self.tokens[self.pos];
        self.expected.clear();
    }

    /// The kind of the token `n` places after the current one.
    fn look_ahead(&self, n: usize) -> TokenKind<'src> {
        let index = (self.pos + n).min(self.tokens.len() - 1);
        self.tokens[index].kind
    }

    /// Runs `parse` with `restrictions` in effect.
    fn with_res<T>(&mut self, restrictions: Restrictions, parse: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.restrictions, restrictions);
        let result = parse(self);
        self.restrictions = saved;
        result
    }

    /// Whether the current token is `kind`; if not, `kind` is recorded as
    /// expected.
    fn check(&mut self, kind: TokenKind<'static>) -> bool {
        let found = self.token.kind == kind;
        if !found {
            self.expected.push(Expected::Token(kind));
        }
        found
    }

    /// Consumes the current token if it is `kind`.
    fn eat(&mut self, kind: TokenKind<'static>) -> bool {
        let found = self.check(kind);
        if found {
            self.bump();
        }
        found
    }

    fn expect(&mut self, kind: TokenKind<'static>) -> PResult<()> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        self.eat(TokenKind::Keyword(keyword))
    }

    /// Consumes the current token if it is `kind`, without recording it as
    /// expected. Used for tokens that begin a category already named in the
    /// error, so it reads "expected type" rather than listing `(`, `_`, ...
    fn eat_noexpect(&mut self, kind: TokenKind<'static>) -> bool {
        let found = self.token.kind == kind;
        if found {
            self.bump();
        }
        found
    }

    /// Records that a whole category of syntax would have been accepted here.
    fn expected(&mut self, expected: Expected) {
        self.expected.push(expected);
    }

    fn parse_ident(&mut self) -> PResult<Ident> {
        match self.token.kind {
            TokenKind::Ident(name) => {
                let ident = Ident::new(Symbol::intern(name), self.token.span);
                self.bump();
                Ok(ident)
            }
            _ => {
                self.expected(Expected::Ident);
                Err(self.unexpected())
            }
        }
    }

    /// Parses `item (, item)* ,? close`, the opening delimiter having already
    /// been consumed. Also returns whether there was a trailing comma, which
    /// distinguishes `(a)` from the one-element tuple `(a,)`.
    fn parse_comma_seq<T>(
        &mut self,
        close: TokenKind<'static>,
        mut parse: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<(Vec<T>, bool)> {
        let mut items = Vec::new();
        let mut trailing_comma = false;
        while !self.eat(close) {
            items.push(parse(self)?);
            trailing_comma = self.eat(TokenKind::Comma);
            if !trailing_comma {
                self.expect(close)?;
                break;
            }
        }
        Ok((items, trailing_comma))
    }

    /// Like [`Self::parse_comma_seq`], when trailing commas don't matter.
    fn parse_delimited<T>(
        &mut self,
        close: TokenKind<'static>,
        parse: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<T>> {
        self.parse_comma_seq(close, parse).map(|(items, _)| items)
    }

    /// Builds an "expected ..., found ..." error for the current token from
    /// everything checked for since the last `bump`.
    fn unexpected(&self) -> Diag {
        let alternatives = expected::alternatives(&self.expected);
        let expected = expected::join(&alternatives);
        let found = self.describe_token(self.token);
        let (message, label) = match alternatives.len() {
            0 => (format!("unexpected {found}"), "unexpected token".to_owned()),
            1 => (
                format!("expected {expected}, found {found}"),
                format!("expected {expected}"),
            ),
            n => (
                format!("expected {expected}, found {found}"),
                format!("expected one of {n} possible tokens"),
            ),
        };

        // As in rustc: if the unexpected token starts a new line, the problem
        // is most likely something missing at the end of the previous one.
        if self.token_starts_line() {
            let missing = self.prev_token.span.shrink_to_hi();
            Diag::error(message, missing)
                .with_label(missing, label)
                .with_label(self.token.span, "unexpected token")
        } else {
            Diag::error(message, self.token.span).with_label(self.token.span, label)
        }
    }

    /// Whether a line break separates the current token from the previous one.
    fn token_starts_line(&self) -> bool {
        let gap = &self.src[self.prev_token.span.end..self.token.span.start];
        self.prev_token.span != Span::default() && gap.contains('\n')
    }

    /// Expects a `;`. As in rustc, a `;` missing at the end of a line is
    /// reported but otherwise treated as present, so parsing continues with
    /// the next line instead of skipping it.
    fn expect_semi(&mut self) -> PResult<()> {
        if self.eat(TokenKind::Semi) {
            return Ok(());
        }
        let diag = self.unexpected();
        if self.token_starts_line() {
            self.report(diag);
            Ok(())
        } else {
            Err(diag)
        }
    }

    /// How a token is named in "found ..." messages.
    fn describe_token(&self, token: Token<'_>) -> String {
        match token.kind {
            TokenKind::Eof => "end of file".to_owned(),
            TokenKind::Keyword(keyword) => format!("keyword `{keyword}`"),
            _ => format!("`{}`", token.span.slice(self.src)),
        }
    }
}
