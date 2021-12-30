use std::collections::HashMap;
use std::marker::PhantomData;

use crate::ast::value::{Expression, LiteralValue};
use crate::ast::Statement;
use crate::xurvm::var::{CallableHanderF, Enviroment, Value};
use anyhow::{anyhow, bail, Context};
use tracing::{debug, info};

#[derive(Debug)]
pub struct XurVM {
    global_state: HashMap<String, Value>,
}

impl Enviroment for XurVM {
    fn get_value(&self, name: &str) -> Option<&Value> {
        self.global_state.get(name)
    }

    fn resolve_expression(&mut self, ex: Expression) -> anyhow::Result<Value> {
        self.get_value(ex)
    }
    fn resolve_args(&mut self, args: Vec<Expression>) -> anyhow::Result<Vec<Value>> {
        let values: anyhow::Result<Vec<Value>> = args
            .into_iter()
            .flat_map(|x| match x {
                Expression::ParenList(ex) => ex,
                x => vec![x],
            })
            .map(|x| self.get_value(x))
            .collect();
        Ok(values?)
    }
}

impl XurVM {
    pub fn new() -> Self {
        Self {
            global_state: HashMap::new(),
        }
        .add_builtins()
    }
    pub fn add_fn(&mut self, name: &str, arg_n: usize, f: CallableHanderF) {
        self.global_state
            .insert(name.into(), Value::create_fn(name, arg_n, f));
    }

    fn add_builtins(mut self) -> Self {
        self.add_fn("+", 2, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_number()?;
            let value_2 = args.pop().context("Failed to pop arg2")?.get_number()?;
            Ok(Value::Number(value_1 + value_2))
        });
        self.add_fn("%", 1, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_string()?;

            let binding = env
                .get_value(value_1.as_ref())
                .context(format!("Failed to get binding for \"{}\"", &value_1))?
                .clone();

            let fn_ref = binding.get_callable().context(format!(
                "Faiiled to convert \"{}\" bind to callable.",
                &value_1
            ))?;

            Ok(Value::Function(fn_ref))
        });
        self.add_fn(".", 2, |env, mut args| {
            let value_1 = args.pop().context("Failed to pop arg1")?;
            let value_2 = args.pop().context("Failed to pop arg2")?;
            println!(".call {:?} {:?}", value_1, value_2);

            let expr = match value_1 {
                Expression::Call { callie, mut args } => {
                    args.insert(0, value_2);
                    Expression::Call { callie, args }
                }
                x => x,
            };

            env.resolve_expression(expr)
        });
        self.add_fn("x2", 1, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_number()?;
            Ok(Value::Number(value_1 * 2))
        });
        self.add_fn("to_int", 2, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_number()?;
            let value_2 = args.pop().context("Failed to pop arg2")?.get_string()?;
            let i = u128::from_str_radix(&value_2, value_1 as u32)?;
            Ok(Value::Number(i))
        });
        self.add_fn("__get_symbol_bind__", 1, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_string()?;

            let binding = env
                .get_value(value_1.as_ref())
                .context(format!("Failed to get binding for \"{}\"", &value_1))?
                .clone();

            let fn_ref = binding.get_callable().context(format!(
                "Faiiled to convert \"{}\" bind to callable.",
                &value_1
            ))?;

            return Ok(Value::Function(fn_ref));
            Ok(Value::create_fn(
                &format!("__sym_bind_@{}", value_1),
                1,
                |env, args| {
                    let mut args = env.resolve_args(args)?;
                    let value_1 = args.pop().context("Failed to pop arg1")?.get_string()?;
                    println!("get fn {}", value_1);
                    Ok(Value::Number(10))
                },
            ))
        });

        self.add_fn("map", 1, |env, args| {
            let mut args = env.resolve_args(args)?;
            let value_1 = args.pop().context("Failed to pop arg1")?.get_number()?;
            Ok(Value::Number(value_1 * 2))
        });
        //
        self
    }

    #[tracing::instrument(skip(self))]
    pub fn execute(&mut self, s: Statement) -> anyhow::Result<Value> {
        debug!("executing");
        match s {
            Statement::Expr(e) => self.run_expr(e),
            Statement::Empty => Ok(Value::None),
            x => bail!("Statement {:?} not supported", x),
        }
    }

    #[tracing::instrument(skip(self, ex))]
    fn get_value(&mut self, ex: Expression) -> anyhow::Result<Value> {
        debug!("get_value {}", ex.short_name());
        match ex {
            Expression::Variable(s) => {
                let v = self
                    .global_state
                    .get(&s)
                    .context(format!("\"{}\" is not defined.", s))?;
                Ok(v.clone())
            }
            Expression::Literal(s) => match s {
                LiteralValue::Number(n) => Ok(Value::Number(n)),
                LiteralValue::String(s) => Ok(Value::String(s)),
                _ => bail!("Value type {:?} not supported yet", s),
            },
            Expression::Array(a) => {
                let values: anyhow::Result<Vec<Value>> =
                    a.into_iter().map(|e| self.get_value(e)).collect();
                values.map(|v| Value::Array(v))
            }
            x => self.run_expr(x),
        }
    }

    #[tracing::instrument(skip(self, ex))]
    fn run_expr(&mut self, ex: Expression) -> anyhow::Result<Value> {
        debug!("run_expr {}", ex.short_name());
        match ex {
            Expression::Call { callie, args } => {
                let mut fn_value = self.get_value(*callie)?.get_callable()?;

                fn_value.call(self, args)
            }
            x => bail!("Cant run expression {:?}", x),
        }
    }
}
