use crate::lexer::{Token, TokenKind};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum LiteralValue {
    String(Rc<String>),
    Number(u128),
    Boolean(bool),
    None,
}

#[derive(Clone, Debug)]
pub enum Expression {
    Variable(String),
    Literal(LiteralValue),
    Call {
        callie: Box<Expression>,
        args: Vec<Expression>,
    },
    Array(Vec<Expression>),
    ParenList(Vec<Expression>),
    None,
}

impl Expression {
    pub fn short_name(&self) -> String {
        match self {
            Expression::Variable(s) => format!("Expression(Var({}))", s),
            Expression::Literal(s) => format!("Expression({:?})", s),
            Expression::Call { callie, .. } => format!("Expression(Call({:?}))", callie),
            Expression::Array(s) => format!("Expression(Array({:?}))", s),
            Expression::ParenList(s) => format!("Expression(ParenList({:?}))", s),
            Expression::None => format!("Expression(None)"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Statement {
    Expr(Expression),
    Empty,
}
