use super::value::{Expression, Statement};
use crate::ast::value::Statement::Expr;
use crate::ast::value::LiteralValue;
use crate::{Token, TokenKind};
use anyhow::{anyhow, bail, Context};
use std::collections::VecDeque;

pub struct Parser {
    tokens: VecDeque<Token>,
}

impl Iterator for Parser {
    type Item = anyhow::Result<Statement>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_eof() {
            None
        } else {
            Some(self.next_statement())
        }
    }
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: VecDeque::from(tokens),
        }
    }

    pub fn next_statement(&mut self) -> anyhow::Result<Statement> {
        let stmnt = match self.consume().context("Unexpected end of statement")? {
            TokenKind::Identifier(id) => self.ident(id)?,
            TokenKind::OpenBracket => Statement::Expr(self.array()?),
            TokenKind::String(s) => {
                let ex= Expression::Literal(LiteralValue::String(s.into()));
                Statement::Expr(self.join_expr_modifiers(ex)?)
            },
            TokenKind::Numeric(s) => {
                let ex = Expression::Literal(LiteralValue::Number(s));
                Statement::Expr(self.join_expr_modifiers(ex)?)
            },
            TokenKind::OpenParen => Statement::Expr(self.paren_list()?),
            TokenKind::Symbol(s) => {
                let c = Expression::Call {
                    callie: Box::from(Expression::Variable(
                        String::from("__get_symbol_bind__")
                    )),
                    args: vec![Expression::Literal(LiteralValue::String(s.into()))]
                };
                Statement::Expr(self.join_expr_modifiers(c)?)
            },
            TokenKind::Semicolon => Statement::Empty,
            t => bail!("Unexpected token {:?}", t),
        };
        Ok(stmnt)
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.front().map(|t| &t.kind)
    }
    fn is_eof(&self) -> bool {
        self.tokens.is_empty()
    }
    fn consume(&mut self) -> Option<TokenKind> {
        self.tokens.pop_front().map(|t| t.kind)
    }
    // x mod 3
    // `mod` is the inline function
    fn inline_fn_call(&mut self, ident: String, lp: Expression) -> anyhow::Result<Expression> {
        let fn_ex = match self.next_statement() {
            Ok(Statement::Expr(e)) => Expression::Call {
                callie: Box::from(Expression::Variable(ident)),
                args: vec![lp, e],
            },
            e => {
                bail!(
                    "Expected expression for right argument of {}! got {:?}",
                    ident,
                    e?
                )
            }
        };

        let ex = self.join_expr_modifiers(fn_ex)?;
        Ok(ex)
    }

    fn paren_list_fn_call(&mut self,lp: Expression) -> anyhow::Result<Expression>{
        let exprs = self.comma_seprated_list(&TokenKind::CloseParen)?;

        let fn_ex = Expression::Call {
            callie: Box::from(lp),
            args: exprs,
        };

        let ex = self.join_expr_modifiers(fn_ex)?;
        Ok(ex)
    }

    fn join_expr_modifiers(&mut self, ex: Expression) -> anyhow::Result<Expression> {
        let mut ex = ex;
        loop {
            let peek_value = match self.peek() {
                Some(v) => v,
                None => break,
            };

            match peek_value {
                TokenKind::Identifier(id) => {
                    let ident = match self.consume().context("Failed to read identifier")? {
                        TokenKind::Identifier(s) => s,
                        _ => bail!("failed to read identifier"),
                    };
                    ex = self.inline_fn_call(ident, ex)?;
                },
                TokenKind::OpenParen => {
                    self.consume();
                    ex = self.paren_list_fn_call(ex)?;
                },
                _ => break,
            };
        }
        Ok(ex)
    }

    fn ident(&mut self, id: String) -> anyhow::Result<Statement> {
        let ex = self.join_expr_modifiers(Expression::Variable(id))?;
        Ok(Statement::Expr(ex))
    }

    fn comma_seprated_list(&mut self, terminator: &TokenKind) -> anyhow::Result<Vec<Expression>> {
        let mut exprs : Vec<Expression> = Vec::new();
        let mut last_expression =None;

        loop {
            let peek_value = self.peek().context("eof on list")?;
            match peek_value {
                v if v == terminator => {
                    if let Some(e) = last_expression{
                        exprs.push(e);
                    }
                    self.consume();
                    break;
                }
                TokenKind::Comma => {
                    if let Some(e) = std::mem::replace(&mut last_expression, Some(Expression::None)){
                        exprs.push(e);
                    }

                    self.consume();
                    continue;
                }
                _ => {}
            }
            match self
                .next_statement()
                .context(format!("Unexpected end of {:?} terminated list. exprs({:?})", terminator, exprs))?
            {
                Statement::Expr(e) => last_expression = Some(e),
                s => bail!("Invalid value in array! {:?}", s),
            };
        }
        Ok(exprs)
    }
    fn array(&mut self) -> anyhow::Result<Expression> {
        let exprs =
            self.comma_seprated_list(&TokenKind::CloseBracket)
                .context("Failed to read list")?;

        let ex = self.join_expr_modifiers(Expression::Array(exprs))
            .context(format!("Failed to join exprs for list"))?;
        Ok(ex)
    }
    fn paren_list(&mut self) -> anyhow::Result<Expression> {
        let exprs = self.comma_seprated_list(&TokenKind::CloseParen)
            .context("Failed to read parens")?;

        let ex = self.join_expr_modifiers(Expression::ParenList(exprs))
            .context("Failed to join exprs for paren")?;
        Ok(ex)
    }
}
