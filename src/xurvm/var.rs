use std::rc::Rc;
use std::{collections::VecDeque, ops::Range};

use crate::ast::value::Expression;
use anyhow::{anyhow, bail, Context};
use tracing::{debug, info};

pub trait Enviroment {
    fn get_value(&self, name: &str) -> Option<&Value>;
    fn resolve_expression(&mut self, ex: Expression) -> anyhow::Result<Value>;
    fn resolve_args(&mut self, args: Vec<Expression>) -> anyhow::Result<Vec<Value>>;
}

pub type CallableHanderF = fn(&mut dyn Enviroment, Vec<Expression>) -> anyhow::Result<Value>;

#[derive(Clone)]
pub struct Callable {
    id: String,
    expected_args: (usize, usize),
    target: CallableHanderF,
    binded_args: Vec<Expression>,
}

impl std::fmt::Debug for Callable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Callable(args: {:?})", self.expected_args)
    }
}

fn transfer_expression_list(destination: &mut Vec<Expression>, source: Vec<Expression>, fill_list: &mut VecDeque<Expression>) {
    for ex in source.into_iter() {
        match ex {
            Expression::None if !fill_list.is_empty() 
            => destination.push(fill_list.pop_front().expect("Failed to pop arg in call")),
            x => destination.push(x),
        }
    }
}

impl Callable {
    pub fn args_fit(&self, n: usize) -> bool {
        n >= self.expected_args.0 && n <= self.expected_args.1
    }
    pub fn min_args(&self) -> usize {
        self.expected_args.0.min(self.expected_args.1)
    }
    pub fn name(&self) -> &str {
        &self.id
    }


    #[tracing::instrument(skip(self, env))]
    pub fn call(
        mut self,
        env: &mut dyn Enviroment,
        args: Vec<Expression>,
    ) -> anyhow::Result<Value> {
        println!("Inargs: {:?}", &args);

        if !self.binded_args.is_empty() {
            let mut new_args = Vec::new();
            let mut v = VecDeque::from(args);

            for e in self.binded_args.into_iter() {
                match e {
                    Expression::None if !v.is_empty() => {
                        new_args.push(v.pop_front().context("Failed to pop arg in call")?)
                    }
                    Expression::ParenList(p) => {
                        transfer_expression_list(&mut new_args, p, &mut v);
                    }
                    x => new_args.push(x),
                }
            }
            for e in v.into_iter() {
                match e {
                    Expression::ParenList(mut p) => new_args.append(&mut p),
                    x => new_args.push(x),
                }
            }
            self.binded_args = new_args;
        } else {
            self.binded_args.clear();
            let mut v = VecDeque::from(args);
            while let Some(ex) = v.pop_front() {
                match ex {
                    Expression::ParenList(p) => {
                        transfer_expression_list(&mut self.binded_args, p, &mut v);
                    },
                    x => self.binded_args.push(x)
                }
            }
        };

        let binded_arg_count = self
            .binded_args
            .iter()
            .filter(|a| !matches!(a, Expression::None))
            .count();

        println!("Callinf {} with {:?}", &self.name(), &self.binded_args);

        if !self.args_fit(binded_arg_count) {
            info!(
                "\"{}\" expected {:?} args, got {}",
                &self.name(),
                self.min_args(),
                binded_arg_count
            );
            return Ok(Value::Function(self));
        }

        debug!("Calling {}", self.name());

        let ctx_name = self.name().to_string();
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
    None,
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Value({})", n),
            Value::String(n) => write!(f, "Value({})", n),
            Value::Array(n) => write!(f, "Value({:?})", n),
            Value::None => write!(f, "Value(None)"),
            _ => write!(f, "Value(?)"),
        }
    }
}

impl Value {
    pub fn create_fn(name: &str, arg_n: usize, f: CallableHanderF) -> Value {
        Value::Function(Callable {
            id: name.into(),
            target: f,
            expected_args: (arg_n, arg_n),
            binded_args: Vec::new(),
        })
    }
    pub fn get_callable(self) -> anyhow::Result<Callable> {
        match self {
            Value::Function(f) => Ok(f),
            x => bail!("Value {:?} is not callable", x),
        }
    }

    pub fn get_number(self) -> anyhow::Result<u128> {
        match self {
            Value::Number(f) => Ok(f),
            x => bail!("Value {:?} is not number", x),
        }
    }
    pub fn get_string(self) -> anyhow::Result<Rc<String>> {
        match self {
            Value::String(f) => Ok(f.clone()),
            x => bail!("Value {:?} is not a string", x),
        }
    }
}
