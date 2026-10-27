//! Expressions, parsed by precedence climbing as in rustc's
//! `parse_expr_assoc_with`.
//!
//! Binding strength, weakest first: closures and jumps, `=`, `..`, `||`,
//! `&&`, comparisons, `+ -`, `* / %`, prefix `- !`, then postfix calls,
//! method calls, field access and `?`.

use std::ops::Bound;

use xur_ast::{
    Arm, BinOp, BinOpKind, Closure, Expr, ExprField, ExprKind, Fixity, FnDecl, FnRetTy, MethodCall,
    Param, Path, PathSegment, Precedence, RangeLimits, StructExpr, Ty, TyKind, UnOp,
};
use xur_errors::Diag;
use xur_span::{Ident, Span, Symbol};

use super::expected::Expected;
use super::{PResult, Parser, PathStyle, Restrictions};
use crate::token::{Keyword, TokenKind};

/// An infix operator, as in rustc's `AssocOp`.
#[derive(Debug, Clone, Copy)]
enum AssocOp {
    Binary(BinOpKind),
    Assign,
    Range(RangeLimits),
}

impl AssocOp {
    fn from_token(kind: TokenKind<'_>) -> Option<Self> {
        let binary = |op| Some(Self::Binary(op));
        match kind {
            TokenKind::Plus => binary(BinOpKind::Add),
            TokenKind::Minus => binary(BinOpKind::Sub),
            TokenKind::Star => binary(BinOpKind::Mul),
            TokenKind::Slash => binary(BinOpKind::Div),
            TokenKind::Percent => binary(BinOpKind::Rem),
            TokenKind::AmpAmp => binary(BinOpKind::And),
            TokenKind::PipePipe => binary(BinOpKind::Or),
            TokenKind::EqEq => binary(BinOpKind::Eq),
            TokenKind::BangEq => binary(BinOpKind::Ne),
            TokenKind::Lt => binary(BinOpKind::Lt),
            TokenKind::LtEq => binary(BinOpKind::Le),
            TokenKind::Gt => binary(BinOpKind::Gt),
            TokenKind::GtEq => binary(BinOpKind::Ge),
            TokenKind::Eq => Some(Self::Assign),
            TokenKind::DotDot => Some(Self::Range(RangeLimits::HalfOpen)),
            TokenKind::DotDotEq => Some(Self::Range(RangeLimits::Closed)),
            _ => None,
        }
    }

    fn precedence(self) -> Precedence {
        match self {
            Self::Binary(op) => op.precedence(),
            Self::Assign => Precedence::Assign,
            Self::Range(_) => Precedence::Range,
        }
    }

    fn fixity(self) -> Fixity {
        match self {
            Self::Binary(op) => op.fixity(),
            Self::Assign => Fixity::Right,
            Self::Range(_) => Fixity::None,
        }
    }
}

/// Whether an operator of precedence `prec` may continue an expression
/// whose operators must bind at least as tightly as `min`.
fn binds_within(prec: Precedence, min: Bound<Precedence>) -> bool {
    match min {
        Bound::Unbounded => true,
        Bound::Included(min) => prec >= min,
        Bound::Excluded(min) => prec > min,
    }
}

fn can_begin_expr(kind: TokenKind<'_>) -> bool {
    match kind {
        TokenKind::Ident(_)
        | TokenKind::IntLit(_)
        | TokenKind::FloatLit(_)
        | TokenKind::StringLit(_)
        | TokenKind::ByteStrLit(_)
        | TokenKind::LParen
        | TokenKind::LBracket
        | TokenKind::LBrace
        | TokenKind::Minus
        | TokenKind::Bang
        | TokenKind::Pipe
        | TokenKind::PipePipe
        | TokenKind::DotDot
        | TokenKind::DotDotEq => true,
        TokenKind::Keyword(keyword) => matches!(
            keyword,
            Keyword::True
                | Keyword::False
                | Keyword::If
                | Keyword::Match
                | Keyword::For
                | Keyword::While
                | Keyword::Loop
                | Keyword::Return
                | Keyword::Break
                | Keyword::Continue
        ),
        _ => false,
    }
}

fn mk_expr(kind: ExprKind, span: Span) -> Expr {
    Expr { kind, span }
}

impl Parser<'_> {
    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_expr_res(Restrictions::default())
    }

    pub(super) fn parse_expr_res(&mut self, restrictions: Restrictions) -> PResult<Expr> {
        self.with_res(restrictions, |this| {
            this.parse_expr_assoc_with(Bound::Unbounded)
        })
    }

    /// Parses the head of an `if` or `while`, reporting a missing condition
    /// rather than mistaking the body for it.
    fn parse_expr_cond(&mut self, keyword: Keyword) -> PResult<Expr> {
        if self.token.kind == TokenKind::LBrace {
            let span = self.prev_token.span.shrink_to_hi();
            self.report(
                Diag::error(
                    format!("missing condition for `{keyword}` expression"),
                    span,
                )
                .with_label(span, "expected condition here"),
            );
            return Ok(mk_expr(ExprKind::Err, span));
        }
        self.parse_expr_before_block()
    }

    fn parse_expr_before_block(&mut self) -> PResult<Expr> {
        self.parse_expr_res(Restrictions {
            before_block: true,
            ..Restrictions::default()
        })
    }

    /// Parses an expression whose operators bind at least as tightly as `min`.
    fn parse_expr_assoc_with(&mut self, min: Bound<Precedence>) -> PResult<Expr> {
        let lhs = if matches!(self.token.kind, TokenKind::DotDot | TokenKind::DotDotEq) {
            self.parse_expr_prefix_range()?
        } else {
            self.parse_expr_prefix()?
        };
        self.parse_expr_assoc_rest(min, lhs)
    }

    /// Continues `lhs` with infix operators binding at least as tightly as `min`.
    fn parse_expr_assoc_rest(&mut self, min: Bound<Precedence>, mut lhs: Expr) -> PResult<Expr> {
        loop {
            if self.restrictions.stmt_expr && lhs.is_block_like() {
                return Ok(lhs);
            }
            let Some(op) = AssocOp::from_token(self.token.kind) else {
                self.expected(Expected::Operator);
                return Ok(lhs);
            };
            let prec = op.precedence();
            if !binds_within(prec, min) {
                return Ok(lhs);
            }
            if op.fixity() == Fixity::None && lhs.precedence() == prec {
                self.report_chained_operator(op);
            }

            let lhs_span = lhs.span;
            let op_span = self.token.span;
            self.bump();
            let rhs_min = match op.fixity() {
                Fixity::Right => Bound::Included(prec),
                Fixity::Left | Fixity::None => Bound::Excluded(prec),
            };
            // The right-hand side is a new expression, not a statement.
            let rhs_res = Restrictions {
                stmt_expr: false,
                ..self.restrictions
            };

            let kind = match op {
                AssocOp::Range(limits) => {
                    let end = self.with_res(rhs_res, |this| {
                        this.parse_range_end(rhs_min, limits, op_span)
                    })?;
                    ExprKind::Range(Some(Box::new(lhs)), end, limits)
                }
                AssocOp::Assign => {
                    let rhs = self.with_res(rhs_res, |this| this.parse_expr_assoc_with(rhs_min))?;
                    ExprKind::Assign(Box::new(lhs), Box::new(rhs), op_span)
                }
                AssocOp::Binary(kind) => {
                    let rhs = self.with_res(rhs_res, |this| this.parse_expr_assoc_with(rhs_min))?;
                    let op = BinOp {
                        kind,
                        span: op_span,
                    };
                    ExprKind::Binary(op, Box::new(lhs), Box::new(rhs))
                }
            };
            lhs = mk_expr(kind, lhs_span.to(self.prev_token.span));
        }
    }

    fn report_chained_operator(&mut self, op: AssocOp) {
        let span = self.token.span;
        let message = match op {
            AssocOp::Range(_) => "range operators cannot be chained",
            _ => "comparison operators cannot be chained",
        };
        self.report(
            Diag::error(message, span)
                .with_label(span, "chained here")
                .with_note("add parentheses to make the grouping explicit"),
        );
    }

    /// `..end`, `..=end` or `..` with no left operand.
    fn parse_expr_prefix_range(&mut self) -> PResult<Expr> {
        let lo = self.token.span;
        let limits = match self.token.kind {
            TokenKind::DotDotEq => RangeLimits::Closed,
            _ => RangeLimits::HalfOpen,
        };
        self.bump();
        let end = self.parse_range_end(Bound::Excluded(Precedence::Range), limits, lo)?;
        Ok(mk_expr(
            ExprKind::Range(None, end, limits),
            lo.to(self.prev_token.span),
        ))
    }

    /// Parses the end of a range if there is one; `a..=` must have one.
    fn parse_range_end(
        &mut self,
        min: Bound<Precedence>,
        limits: RangeLimits,
        op_span: Span,
    ) -> PResult<Option<Box<Expr>>> {
        let has_end = can_begin_expr(self.token.kind)
            && !(self.token.kind == TokenKind::LBrace && self.restrictions.before_block);
        if has_end {
            return Ok(Some(Box::new(self.parse_expr_assoc_with(min)?)));
        }
        if limits == RangeLimits::Closed {
            self.report(
                Diag::error("inclusive range with no end", op_span)
                    .with_label(op_span, "an inclusive range needs an end; use `..` instead"),
            );
        }
        Ok(None)
    }

    /// Prefix operators: `-x`, `!x`.
    fn parse_expr_prefix(&mut self) -> PResult<Expr> {
        let lo = self.token.span;
        let op = match self.token.kind {
            TokenKind::Minus => UnOp::Neg,
            TokenKind::Bang => UnOp::Not,
            _ => return self.parse_expr_dot_or_call(),
        };
        self.bump();
        let operand = self.parse_expr_prefix()?;
        Ok(mk_expr(
            ExprKind::Unary(op, Box::new(operand)),
            lo.to(self.prev_token.span),
        ))
    }

    /// A primary expression followed by any postfix operators.
    fn parse_expr_dot_or_call(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_expr_bottom()?;
        loop {
            let lo = expr.span;
            let kind = if self.eat(TokenKind::Question) {
                ExprKind::Try(Box::new(expr))
            } else if self.eat(TokenKind::Dot) {
                self.parse_dot_suffix(expr)?
            } else if self.restrictions.stmt_expr && expr.is_block_like() {
                // `if c {} (a, b)` is an `if` statement followed by a tuple,
                // and `if c {} [a]` one followed by a list.
                return Ok(expr);
            } else if self.eat(TokenKind::LParen) {
                let args = self.parse_delimited(TokenKind::RParen, Self::parse_expr)?;
                ExprKind::Call(Box::new(expr), args)
            } else if self.eat(TokenKind::LBracket) {
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket)?;
                ExprKind::Index(Box::new(expr), Box::new(index))
            } else if self.eat(TokenKind::Keyword(Keyword::As)) {
                let ty = self.parse_ty()?;
                ExprKind::Cast(Box::new(expr), ty)
            } else if self.eat(TokenKind::Keyword(Keyword::Is)) {
                let pat = self.parse_pat()?;
                ExprKind::Is(Box::new(expr), Box::new(pat))
            } else {
                return Ok(expr);
            };
            expr = mk_expr(kind, lo.to(self.prev_token.span));
        }
    }

    /// `.field`, `.0` or `.method::<T>(args)`, after the `.`.
    fn parse_dot_suffix(&mut self, receiver: Expr) -> PResult<ExprKind> {
        let receiver = match self.parse_tuple_field(receiver) {
            Ok(kind) => return Ok(kind),
            Err(receiver) => receiver,
        };
        let ident = self.parse_ident()?;
        let args = self.parse_opt_generic_args(PathStyle::Expr)?;
        if self.eat(TokenKind::LParen) {
            let call_args = self.parse_delimited(TokenKind::RParen, Self::parse_expr)?;
            return Ok(ExprKind::MethodCall(Box::new(MethodCall {
                seg: PathSegment { ident, args },
                receiver,
                args: call_args,
                span: ident.span.to(self.prev_token.span),
            })));
        }
        if let Some(args) = args {
            self.report(
                Diag::error("field expressions cannot have generic arguments", args.span)
                    .with_label(args.span, "remove these generic arguments"),
            );
        }
        Ok(ExprKind::Field(Box::new(receiver), ident))
    }

    /// `.0`, the element of a tuple by number, or `.0.1`, which the lexer
    /// sees as the float `0.1` and is split in two, as rustc does. Gives the
    /// receiver back if the next token is not one.
    fn parse_tuple_field(&mut self, receiver: Expr) -> Result<ExprKind, Expr> {
        let span = self.token.span;
        let field = |text: &str, start: usize| {
            let end = start + text.len();
            Ident::new(Symbol::intern(text), Span::new(start, end))
        };
        let kind = match self.token.kind {
            TokenKind::IntLit(text) if is_tuple_index(text) => {
                ExprKind::Field(Box::new(receiver), field(text, span.start))
            }
            TokenKind::FloatLit(text) => match text.split_once('.') {
                Some((first, second)) if is_tuple_index(first) && is_tuple_index(second) => {
                    let first = field(first, span.start);
                    let inner_span = receiver.span.to(first.span);
                    let inner = mk_expr(ExprKind::Field(Box::new(receiver), first), inner_span);
                    ExprKind::Field(Box::new(inner), field(second, first.span.end + 1))
                }
                _ => return Err(receiver),
            },
            _ => return Err(receiver),
        };
        self.bump();
        Ok(kind)
    }

    /// Literals, paths, and expressions introduced by a token or keyword.
    fn parse_expr_bottom(&mut self) -> PResult<Expr> {
        let lo = self.token.span;
        if let Some(lit) = self.parse_opt_lit() {
            return Ok(mk_expr(ExprKind::Lit(lit), lo));
        }
        let kind = match self.token.kind {
            TokenKind::Ident(_) => {
                let path = self.parse_path(PathStyle::Expr)?;
                if self.at_struct_literal() {
                    self.parse_struct_literal(path)?
                } else {
                    ExprKind::Path(path)
                }
            }
            TokenKind::LParen => self.parse_expr_paren_or_tuple()?,
            TokenKind::LBracket => {
                self.bump();
                ExprKind::List(self.parse_delimited(TokenKind::RBracket, Self::parse_expr)?)
            }
            TokenKind::LBrace => ExprKind::Block(Box::new(self.parse_block()?)),
            TokenKind::At if self.look_ahead(1) == TokenKind::LBrace => {
                self.bump(); // `@`
                self.parse_record_literal()?
            }
            TokenKind::Pipe | TokenKind::PipePipe => self.parse_expr_closure()?,
            TokenKind::Keyword(Keyword::If) => {
                self.bump();
                self.parse_expr_if()?
            }
            TokenKind::Keyword(Keyword::Match) => self.parse_expr_match()?,
            TokenKind::Keyword(Keyword::For) => self.parse_expr_for()?,
            TokenKind::Keyword(Keyword::While) => {
                self.bump();
                let cond = self.parse_expr_cond(Keyword::While)?;
                ExprKind::While(Box::new(cond), Box::new(self.parse_block()?))
            }
            TokenKind::Keyword(Keyword::Loop) => {
                self.bump();
                ExprKind::Loop(Box::new(self.parse_block()?))
            }
            TokenKind::Keyword(Keyword::Return) => {
                self.bump();
                ExprKind::Ret(self.parse_opt_jump_value()?)
            }
            TokenKind::Keyword(Keyword::Break) => {
                self.bump();
                ExprKind::Break(self.parse_opt_jump_value()?)
            }
            TokenKind::Keyword(Keyword::Continue) => {
                self.bump();
                ExprKind::Continue
            }
            TokenKind::Keyword(Keyword::Fn) => {
                let ty = self.parse_ty()?;
                ExprKind::TypeLiteral(ty)
            }
            _ => {
                self.expected(Expected::Expr);
                return Err(self.unexpected());
            }
        };
        Ok(mk_expr(kind, lo.to(self.prev_token.span)))
    }

    /// Whether a `{` after a path starts a struct literal: not in the head
    /// of an `if`, `while`, `match` or `for`, where it starts the body, and
    /// only if it looks like one (`{}`, `{ name:`, `{ name,` or `{ name }`).
    fn at_struct_literal(&self) -> bool {
        if self.token.kind != TokenKind::LBrace || self.restrictions.before_block {
            return false;
        }
        matches!(
            (self.look_ahead(1), self.look_ahead(2)),
            (TokenKind::RBrace, _)
                | (
                    TokenKind::Ident(_),
                    TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace
                )
        )
    }

    /// `{ name: expr, name }` after the struct's path.
    fn parse_struct_literal(&mut self, path: Path) -> PResult<ExprKind> {
        self.expect(TokenKind::LBrace)?;
        let fields = self.parse_struct_fields()?;
        Ok(ExprKind::Struct(Box::new(StructExpr { path, fields })))
    }

    /// `@{ name: expr, name }` — an anonymous record literal.
    fn parse_record_literal(&mut self) -> PResult<ExprKind> {
        self.expect(TokenKind::LBrace)?;
        let fields = self.parse_struct_fields()?;
        Ok(ExprKind::Record(fields))
    }

    fn parse_struct_fields(&mut self) -> PResult<Vec<ExprField>> {
        self.parse_delimited(TokenKind::RBrace, |this| {
            let lo = this.token.span;
            let ident = this.parse_ident()?;
            let (expr, shorthand) = if this.eat(TokenKind::Colon) {
                (this.parse_expr()?, false)
            } else {
                let path = Path::single(this.next_id(), ident);
                let expr = mk_expr(ExprKind::Path(path), ident.span);
                (expr, true)
            };
            Ok(ExprField {
                ident,
                expr,
                shorthand,
                span: lo.to(this.prev_token.span),
            })
        })
    }

    /// The optional value of `return` or `break`.
    fn parse_opt_jump_value(&mut self) -> PResult<Option<Box<Expr>>> {
        if can_begin_expr(self.token.kind) {
            Ok(Some(Box::new(self.parse_expr_jump_operand()?)))
        } else {
            Ok(None)
        }
    }

    /// Like closure bodies, jump operands extend as far right as possible
    /// but are not statements themselves.
    fn parse_expr_jump_operand(&mut self) -> PResult<Expr> {
        let restrictions = Restrictions {
            stmt_expr: false,
            ..self.restrictions
        };
        self.parse_expr_res(restrictions)
    }

    /// `()`, `(expr)` or `(a, b, ...)`.
    fn parse_expr_paren_or_tuple(&mut self) -> PResult<ExprKind> {
        self.bump(); // `(`
        let (mut exprs, trailing_comma) =
            self.parse_comma_seq(TokenKind::RParen, Self::parse_expr)?;
        Ok(if exprs.len() == 1 && !trailing_comma {
            ExprKind::Paren(Box::new(exprs.remove(0)))
        } else {
            ExprKind::Tuple(exprs)
        })
    }

    /// `if cond { ... } else ...`, after the `if`.
    fn parse_expr_if(&mut self) -> PResult<ExprKind> {
        let cond = self.parse_expr_cond(Keyword::If)?;
        let then = self.parse_block()?;
        let els = if self.eat_keyword(Keyword::Else) {
            let lo = self.token.span;
            let kind = if self.eat_keyword(Keyword::If) {
                self.parse_expr_if()?
            } else {
                ExprKind::Block(Box::new(self.parse_block()?))
            };
            Some(Box::new(mk_expr(kind, lo.to(self.prev_token.span))))
        } else {
            None
        };
        Ok(ExprKind::If(Box::new(cond), Box::new(then), els))
    }

    /// `match scrutinee { pat => expr, ... }`
    fn parse_expr_match(&mut self) -> PResult<ExprKind> {
        self.bump(); // `match`
        let scrutinee = self.parse_expr_before_block()?;
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.eat(TokenKind::RBrace) {
            arms.push(self.parse_arm()?);
        }
        Ok(ExprKind::Match(Box::new(scrutinee), arms))
    }

    fn parse_arm(&mut self) -> PResult<Arm> {
        let lo = self.token.span;
        let pat = self.parse_pat()?;
        self.expect(TokenKind::FatArrow)?;
        // As in rustc, a block-like arm body ends the arm and needs no comma.
        let body = self.parse_expr_res(Restrictions {
            stmt_expr: true,
            ..Restrictions::default()
        })?;
        let span = lo.to(self.prev_token.span);
        let needs_comma = !body.is_block_like() && !self.check(TokenKind::RBrace);
        if needs_comma {
            self.expect(TokenKind::Comma)?;
        } else {
            self.eat(TokenKind::Comma);
        }
        Ok(Arm { pat, body, span })
    }

    /// `for pat in iter { body }`
    fn parse_expr_for(&mut self) -> PResult<ExprKind> {
        self.bump(); // `for`
        let pat = self.parse_pat()?;
        self.expect(TokenKind::Keyword(Keyword::In))?;
        let iter = self.parse_expr_before_block()?;
        let body = self.parse_block()?;
        Ok(ExprKind::ForLoop {
            pat: Box::new(pat),
            iter: Box::new(iter),
            body: Box::new(body),
        })
    }

    /// `|params| body`, `|params| -> ty { body }` or `|| body`.
    fn parse_expr_closure(&mut self) -> PResult<ExprKind> {
        let lo = self.token.span;
        let inputs = if self.eat_noexpect(TokenKind::PipePipe) {
            Vec::new()
        } else {
            self.bump(); // `|`
            self.parse_delimited(TokenKind::Pipe, Self::parse_closure_param)?
        };
        let output = if self.eat(TokenKind::Arrow) {
            FnRetTy::Ty(Box::new(self.parse_ty()?))
        } else {
            FnRetTy::Default(self.prev_token.span.shrink_to_hi())
        };
        let decl_span = lo.to(self.prev_token.span);

        // With an explicit return type the body must be a block, as in Rust.
        let body = match output {
            FnRetTy::Ty(_) => {
                let block = self.parse_block()?;
                let span = block.span;
                mk_expr(ExprKind::Block(Box::new(block)), span)
            }
            FnRetTy::Default(_) => self.parse_expr_jump_operand()?,
        };
        Ok(ExprKind::Closure(Box::new(Closure {
            decl: FnDecl { inputs, output },
            body: Box::new(body),
            decl_span,
        })))
    }

    fn parse_closure_param(&mut self) -> PResult<Param> {
        let lo = self.token.span;
        let pat = self.parse_pat()?;
        let ty = if self.eat(TokenKind::Colon) {
            self.parse_ty()?
        } else {
            Ty {
                kind: TyKind::Infer,
                span: self.prev_token.span.shrink_to_hi(),
            }
        };
        Ok(Param {
            pat,
            ty,
            span: lo.to(self.prev_token.span),
        })
    }
}

/// Whether `text` numbers a tuple's element: decimal digits, without a
/// suffix or leading zeros.
fn is_tuple_index(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}
