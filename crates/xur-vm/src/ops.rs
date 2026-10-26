//! Operators on values, checked at runtime.
//!
//! As in Rust, integers and floats don't mix, and integer overflow and
//! division by zero are errors rather than wrapping. Strings can be
//! concatenated with `+` and compared.

use std::cmp::Ordering;

use xur_ast::{BinOpKind, UnOp};
use xur_runtime::{RunResult, RuntimeError, Value, lock};

use crate::interp::Interp;

pub(crate) fn unary(op: UnOp, value: &Value) -> RunResult<Value> {
    match (op, value) {
        (UnOp::Neg, Value::Int(i)) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        _ => Err(RuntimeError::new(format!(
            "cannot apply `{}` to {}",
            op.as_str(),
            value.kind()
        ))),
    }
}

impl Interp<'_> {
    pub(crate) fn binary(&self, op: BinOpKind, lhs: &Value, rhs: &Value) -> RunResult<Value> {
        use BinOpKind::*;
        let result = match (op, lhs, rhs) {
            (Eq, _, _) => Some(Value::Bool(self.values_eq(lhs, rhs))),
            (Ne, _, _) => Some(Value::Bool(!self.values_eq(lhs, rhs))),
            (Add | Sub | Mul | Div | Rem, Value::Int(a), Value::Int(b)) => {
                Some(int_op(op, *a, *b)?)
            }
            (Add | Sub | Mul | Div | Rem, Value::Float(a), Value::Float(b)) => {
                Some(Value::Float(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    _ => a % b,
                }))
            }
            (Add, Value::Str(a), Value::Str(b)) => Some(Value::str(&format!("{a}{b}"))),
            (Lt | Le | Gt | Ge, _, _) => compare(lhs, rhs).map(|ordering| {
                Value::Bool(match op {
                    Lt => ordering.is_lt(),
                    Le => ordering.is_le(),
                    Gt => ordering.is_gt(),
                    _ => ordering.is_ge(),
                })
            }),
            (And | Or, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(match op {
                And => *a && *b,
                _ => *a || *b,
            })),
            _ => None,
        };
        result.ok_or_else(|| {
            RuntimeError::new(format!(
                "cannot apply `{op}` to {} and {}",
                lhs.kind(),
                rhs.kind()
            ))
        })
    }

    /// `==`: structural, with native objects compared by their Rust
    /// `PartialEq`.
    pub(crate) fn values_eq(&self, lhs: &Value, rhs: &Value) -> bool {
        match (lhs, rhs) {
            (Value::Native(a), Value::Native(b)) => self.shared.registry.native_eq(a, b),
            _ => lhs == rhs,
        }
    }
}

fn int_op(op: BinOpKind, a: i64, b: i64) -> RunResult<Value> {
    let result = match op {
        BinOpKind::Add => a.checked_add(b),
        BinOpKind::Sub => a.checked_sub(b),
        BinOpKind::Mul => a.checked_mul(b),
        BinOpKind::Div | BinOpKind::Rem if b == 0 => {
            return Err(RuntimeError::new("attempt to divide by zero"));
        }
        BinOpKind::Div => a.checked_div(b),
        _ => a.checked_rem(b),
    };
    result.map(Value::Int).ok_or_else(overflow)
}

fn overflow() -> RuntimeError {
    RuntimeError::new("arithmetic overflow")
}

fn compare(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

pub(crate) fn index(base: &Value, index: &Value) -> RunResult<Value> {
    match (base, index) {
        (Value::List(items), Value::Int(i)) => {
            let items = lock(items);
            position(*i, items.len()).map(|i| items[i].clone())
        }
        (Value::Tuple(items), Value::Int(i)) => position(*i, items.len()).map(|i| items[i].clone()),
        _ => Err(RuntimeError::new(format!(
            "cannot index {} with {}",
            base.kind(),
            index.kind()
        ))),
    }
}

pub(crate) fn set_index(base: &Value, index: &Value, value: Value) -> RunResult<()> {
    match (base, index) {
        (Value::List(items), Value::Int(i)) => {
            let mut items = lock(items);
            let i = position(*i, items.len())?;
            items[i] = value;
            Ok(())
        }
        _ => Err(RuntimeError::new(format!(
            "cannot assign to an index of {}",
            base.kind()
        ))),
    }
}

fn position(index: i64, len: usize) -> RunResult<usize> {
    usize::try_from(index)
        .ok()
        .filter(|&i| i < len)
        .ok_or_else(|| RuntimeError::new(format!("index {index} is out of range for length {len}")))
}
