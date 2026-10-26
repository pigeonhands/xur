//! Blocks and statements, with statement-level error recovery.

use xur_ast::{Block, Expr, ExprKind, Local, Stmt, StmtKind};
use xur_errors::Diag;

use super::{PResult, Parser, Restrictions};
use crate::token::{Keyword, TokenKind};

impl Parser<'_> {
    /// `{ stmts }`. A statement that fails to parse is reported, skipped and
    /// replaced by an error expression, so one mistake doesn't lose the
    /// whole block.
    pub fn parse_block(&mut self) -> PResult<Block> {
        let lo = self.token.span;
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.eat(TokenKind::RBrace) {
            if self.is_at_end() {
                return Err(Diag::error(
                    "this file contains an unclosed delimiter",
                    self.token.span,
                )
                .with_label(lo, "unclosed delimiter"));
            }
            let stmt_lo = self.token.span;
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(diag) => {
                    self.report(diag);
                    self.recover_stmt();
                    let span = stmt_lo.to(self.prev_token.span);
                    let err = Box::new(Expr {
                        kind: ExprKind::Err,
                        span,
                    });
                    stmts.push(Stmt {
                        kind: StmtKind::Expr(err),
                        span,
                    });
                }
            }
        }
        Ok(Block {
            stmts,
            span: lo.to(self.prev_token.span),
        })
    }

    pub fn parse_stmt(&mut self) -> PResult<Stmt> {
        let lo = self.token.span;
        let kind = if self.eat(TokenKind::Semi) {
            StmtKind::Empty
        } else if self.eat_keyword(Keyword::Let) {
            StmtKind::Let(Box::new(self.parse_local()?))
        } else if self.is_item_start() {
            StmtKind::Item(Box::new(self.parse_item()?))
        } else {
            self.parse_stmt_expr()?
        };
        Ok(Stmt {
            kind,
            span: lo.to(self.prev_token.span),
        })
    }

    /// `pat: ty = init;`, after the `let`.
    fn parse_local(&mut self) -> PResult<Local> {
        let lo = self.prev_token.span;
        let pat = self.parse_pat()?;
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_ty()?)
        } else {
            None
        };
        let init = if self.eat(TokenKind::Eq) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect_semi()?;
        Ok(Local {
            pat,
            ty,
            init,
            span: lo.to(self.prev_token.span),
        })
    }

    /// An expression statement. It needs a `;` unless it is block-like or
    /// the block's trailing value.
    fn parse_stmt_expr(&mut self) -> PResult<StmtKind> {
        let expr = Box::new(self.parse_expr_res(Restrictions {
            stmt_expr: true,
            ..Restrictions::default()
        })?);
        if self.eat(TokenKind::Semi) {
            return Ok(StmtKind::Semi(expr));
        }
        if expr.is_block_like() || self.check(TokenKind::RBrace) {
            return Ok(StmtKind::Expr(expr));
        }
        self.expect_semi()?;
        Ok(StmtKind::Semi(expr))
    }

    /// Skips to the end of the current statement: past the next `;`, or up
    /// to the `}` closing the enclosing block. Nested delimiters are skipped
    /// as a whole.
    fn recover_stmt(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.token.kind {
                TokenKind::Eof => return,
                TokenKind::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                TokenKind::RBrace if depth == 0 => return,
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
        }
    }
}
