use std::ops::Range;
use std::rc::Rc;

use anyhow::{anyhow, bail, Context};
use tracing::{debug, info};
use crate::ast::value::Expression;

pub trait Enviroment {
    fn get_value(&self, name: &str) -> Option<&Value>;
    fn resolve_expression(&mut self, ex: Expression)  -> anyhow::Result<Value>;
    fn resolve_args(&mut self, args: Vec<Expression>) -> anyhow::Result<Vec<Value>>;
}

pub type CallableHanderF = fn(&mut dyn Enviroment, Vec<Expression>) -> anyhow::Result<Value>;

#[derive(Clone)]
pub struct Callable {
    id: String,
    expected_args: (usize,usize),
    target: CallableHanderF,
    binded_args: Vec<Expression>
}

impl std::fmt::Debug for Callable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Callable(args: {:?})", self.expected_args)
    }
}



impl Callable {
    pub fn args_fit(&self, n: usize) -> bool {
        let total_n = self.binded_args.len() + n;
        total_n >= self.expected_args.0 && total_n <= self.expected_args.1
    }
    pub fn min_args(&self) -> usize {
        self.expected_args.0.min(self.expected_args.1)
    }
    pub fn name(&self) -> &str {
        &self.id
    }

    #[tracing::instrument(skip(self, env))]
    pub fn call(mut self, env: &mut dyn Enviroment, mut args: Vec<Expression>) -> anyhow::Result<Value> {
        if !self.args_fit(args.len()) {
            info!("\"{}\" expected {:?} args, got {}", &self.name(), self.min_args(), args.len());
            self.binded_args = args;
            return Ok(Value::Function(self));
        }
        debug!("Calling {}", self.name());

        let ctx_name = self.name().to_string();
        self.binded_args.append(&mut args);
        let r = (&self.target)(env, self.binded_args);
        r.context(format!("Fn \"{}\"", ctx_name))
    }

}

#[derive(Clone)]
pub enum Value {
    Number(u128),
    String(Rc<String>),
    Function(Callable),
    Array(Vec<Value>),
    None
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Number(n) =>  write!(f, "Value({})", n),
            Value::String(n) =>  write!(f, "Value({})", n),
            Value::Array(n) =>  write!(f, "Value({:?})", n),
            Value::None =>  write!(f, "Value(None)"),
            _ => write!(f, "Value(?)"),
        }

    }
}


impl Value {
    pub fn create_fn (name: &str, arg_n: usize, f: CallableHanderF) -> Value {
        Value::Function(Callable{
            id: name.into(),
            target: f,
            expected_args: (arg_n, arg_n),
            binded_args: Vec::new()
        })
    }
    pub fn get_callable(self) -> anyhow::Result<Callable> {
        match self {
            Value::Function(f) => Ok(f),
            x => bail!("Value {:?} is not callable", x)
        }
    }

    pub fn get_number(self) -> anyhow::Result<u128> {
        match self {
            Value::Number(f) => Ok(f),
            x => bail!("Value {:?} is not number", x)
        }
    }
    pub fn get_string(self) -> anyhow::Result<Rc<String>> {
        match self {
            Value::String(f) => Ok(f.clone()),
            x => bail!("Value {:?} is not a string", x)
        }
    }
}