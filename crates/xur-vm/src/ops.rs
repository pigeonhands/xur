//! Operators on values, checked at runtime.
//!
//! As in Rust, integers and floats don't mix, and integer overflow and
//! division by zero are errors rather than wrapping. Strings, lists and
//! bytes can be concatenated with `+`, and indexed with a range to slice
//! them.

use std::cmp::Ordering;

use xur_ast::{BinOpKind, UnOp};
use xur_runtime::{IntValue, MapKey, Range, RunResult, RuntimeError, Value, lock};

use crate::interp::Interp;

pub(crate) fn unary(op: UnOp, value: &Value) -> RunResult<Value> {
    match (op, value) {
        (UnOp::Neg, Value::Int(i)) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float((-f.as_f64()).into())),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (UnOp::BitNot, Value::Int(i)) => Ok(Value::Int(i.bit_not())),
        _ => Err(RuntimeError::new(format!(
            "cannot apply `{}` to {}",
            op.as_str(),
            value.kind()
        ))),
    }
}

impl Interp<'_> {
    pub(crate) fn coerce_repr(&self, value: &Value) -> Option<Value> {
        if let Value::Adt(adt) = value
            && adt.fields.is_empty()
        {
            let program = self.shared.program();
            return program
                .resolutions()
                .repr_values
                .get(&adt.ctor)
                .map(|&v| Value::int(v));
        }
        None
    }

    pub(crate) fn binary(&self, op: BinOpKind, lhs: &Value, rhs: &Value) -> RunResult<Value> {
        use BinOpKind::*;
        if op == Eq {
            return Ok(Value::Bool(self.values_eq(lhs, rhs)));
        }
        if op == Ne {
            return Ok(Value::Bool(!self.values_eq(lhs, rhs)));
        }
        let lhs_c = self.coerce_repr(lhs);
        let lhs = lhs_c.as_ref().unwrap_or(lhs);
        let rhs_c = self.coerce_repr(rhs);
        let rhs = rhs_c.as_ref().unwrap_or(rhs);
        let result = match (op, lhs, rhs) {
            (Add | Sub | Mul | Div | Rem, Value::Int(a), Value::Int(b)) => {
                Some(int_arith(op, *a, *b)?)
            }
            (BitAnd, Value::Int(a), Value::Int(b)) => Some(int_bitwise(*a, *b, |x, y| x & y)),
            (BitOr, Value::Int(a), Value::Int(b)) => Some(int_bitwise(*a, *b, |x, y| x | y)),
            (BitXor, Value::Int(a), Value::Int(b)) => Some(int_bitwise(*a, *b, |x, y| x ^ y)),
            (Shl, Value::Int(a), Value::Int(b)) => Some(shift_left(*a, *b)?),
            (Shr, Value::Int(a), Value::Int(b)) => Some(shift_right(*a, *b)?),
            (Add | Sub | Mul | Div | Rem, Value::Float(a), Value::Float(b)) => {
                let (a, b) = (a.as_f64(), b.as_f64());
                Some(Value::Float(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    _ => a % b,
                }.into()))
            }
            (Add, Value::Str(a), Value::Str(b)) => Some(Value::str(&format!("{a}{b}"))),
            (Add, Value::List(a), Value::List(b)) => {
                Some(Value::list([lock(a).clone(), lock(b).clone()].concat()))
            }
            (Add, Value::Bytes(a), Value::Bytes(b)) => {
                Some(Value::bytes([lock(a).clone(), lock(b).clone()].concat()))
            }
            (Add, Value::Ptr(p), Value::Int(n)) => {
                Some(Value::Ptr(p.wrapping_add(n.to_i128() as u64)))
            }
            (Sub, Value::Ptr(p), Value::Int(n)) => {
                Some(Value::Ptr(p.wrapping_sub(n.to_i128() as u64)))
            }
            (Sub, Value::Ptr(a), Value::Ptr(b)) => Some(Value::int((*a as i64).wrapping_sub(*b as i64))),
            (BitAnd, Value::Ptr(p), Value::Int(m)) => {
                Some(Value::Ptr(p & m.to_i128() as u64))
            }
            (BitOr, Value::Ptr(p), Value::Int(m)) => {
                Some(Value::Ptr(p | m.to_i128() as u64))
            }
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
            _ => {
                let lhs_c = self.coerce_repr(lhs);
                let rhs_c = self.coerce_repr(rhs);
                match (&lhs_c, &rhs_c) {
                    (Some(_), _) | (_, Some(_)) => {
                        let lhs = lhs_c.as_ref().unwrap_or(lhs);
                        let rhs = rhs_c.as_ref().unwrap_or(rhs);
                        lhs == rhs
                    }
                    _ => lhs == rhs,
                }
            }
        }
    }
}

fn int_arith(op: BinOpKind, a: IntValue, b: IntValue) -> RunResult<Value> {
    let (ai, bi) = (a.to_i128(), b.to_i128());
    let result = match op {
        BinOpKind::Add => ai.checked_add(bi),
        BinOpKind::Sub => ai.checked_sub(bi),
        BinOpKind::Mul => ai.checked_mul(bi),
        BinOpKind::Div | BinOpKind::Rem if bi == 0 => {
            return Err(RuntimeError::new("attempt to divide by zero"));
        }
        BinOpKind::Div => ai.checked_div(bi),
        _ => ai.checked_rem(bi),
    };
    let result = result.ok_or_else(overflow)?;
    IntValue::promote(a, b, result)
        .map(Value::Int)
        .ok_or_else(overflow)
}

fn int_bitwise(a: IntValue, b: IntValue, op: impl FnOnce(i128, i128) -> i128) -> Value {
    let result = op(a.to_i128(), b.to_i128());
    let signed = a.is_signed() && b.is_signed();
    let width = a.width().max(b.width());
    // Mask the result to the target width to avoid i128 sign-extension artifacts.
    let result = match width {
        1 if !signed => result & 0xFF,
        2 if !signed => result & 0xFFFF,
        4 if !signed => result & 0xFFFF_FFFF,
        8 if !signed => result & (u64::MAX as i128),
        _ => result,
    };
    IntValue::narrow(result, signed, width)
        .or_else(|| IntValue::narrow(result, signed, 8))
        .map(Value::Int)
        .unwrap_or_else(|| Value::int(result as i64))
}

fn shift_left(a: IntValue, b: IntValue) -> RunResult<Value> {
    let shift = b.to_u32().ok_or_else(|| {
        RuntimeError::new(format!("invalid shift amount: {b}"))
    })?;
    let result = a.to_i128().checked_shl(shift).ok_or_else(overflow)?;
    IntValue::promote(a, b, result)
        .map(Value::Int)
        .ok_or_else(overflow)
}

fn shift_right(a: IntValue, b: IntValue) -> RunResult<Value> {
    let shift = b.to_u32().ok_or_else(|| {
        RuntimeError::new(format!("invalid shift amount: {b}"))
    })?;
    let result = a.to_i128().checked_shr(shift).ok_or_else(overflow)?;
    IntValue::promote(a, b, result)
        .map(Value::Int)
        .ok_or_else(overflow)
}

fn overflow() -> RuntimeError {
    RuntimeError::new("arithmetic overflow")
}

fn compare(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.as_f64().partial_cmp(&b.as_f64()),
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        (Value::Ptr(a), Value::Ptr(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

pub(crate) fn index(base: &Value, index: &Value) -> RunResult<Value> {
    match (base, index) {
        (Value::List(items), Value::Int(i)) => {
            let items = lock(items);
            let idx = i.to_i64().ok_or_else(|| out_of_range(*i, items.len()))?;
            position(idx, items.len()).map(|i| items[i].clone())
        }
        (Value::Bytes(bytes), Value::Int(i)) => {
            let bytes = lock(bytes);
            let idx = i.to_i64().ok_or_else(|| out_of_range(*i, bytes.len()))?;
            position(idx, bytes.len()).map(|i| Value::Int(IntValue::U8(bytes[i])))
        }
        (Value::Tuple(items), Value::Int(i)) => {
            let idx = i.to_i64().ok_or_else(|| out_of_range(*i, items.len()))?;
            position(idx, items.len()).map(|i| items[i].clone())
        }
        (Value::List(items), Value::Range(range)) => {
            let items = lock(items);
            Ok(Value::list(items[slice(range, items.len())?].to_vec()))
        }
        (Value::Bytes(bytes), Value::Range(range)) => {
            let bytes = lock(bytes);
            Ok(Value::bytes(bytes[slice(range, bytes.len())?].to_vec()))
        }
        (Value::Tuple(items), Value::Range(range)) => {
            Ok(Value::tuple(items[slice(range, items.len())?].to_vec()))
        }
        (Value::Str(s), Value::Range(range)) => {
            let chars: Vec<char> = s.chars().collect();
            let text: String = chars[slice(range, chars.len())?].iter().collect();
            Ok(Value::str(&text))
        }
        (Value::Map(map), key) => {
            let k = MapKey(key.clone());
            map.get(&k)
                .map(|v| v.value().clone())
                .ok_or_else(|| RuntimeError::new(format!("key not found: {}", index.kind())))
        }
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
            let idx = i.to_i64().ok_or_else(|| out_of_range(*i, items.len()))?;
            let i = position(idx, items.len())?;
            items[i] = value;
            Ok(())
        }
        (Value::Bytes(bytes), Value::Int(i)) => {
            let byte = match value {
                Value::Int(n) => u8::try_from(n.to_i128())
                    .map_err(|_| RuntimeError::new(format!("{n} is not a byte (0 to 255)")))?,
                other => {
                    return Err(RuntimeError::new(format!(
                        "expected a byte, found {}",
                        other.kind()
                    )));
                }
            };
            let mut bytes = lock(bytes);
            let idx = i.to_i64().ok_or_else(|| out_of_range(*i, bytes.len()))?;
            let i = position(idx, bytes.len())?;
            bytes[i] = byte;
            Ok(())
        }
        (Value::Map(map), key) => {
            map.insert(MapKey(key.clone()), value);
            Ok(())
        }
        _ => Err(RuntimeError::new(format!(
            "cannot assign to an index of {}",
            base.kind()
        ))),
    }
}

/// The positions a range covers in a sequence of length `len`: a missing
/// start is 0 and a missing end is `len`.
fn slice(range: &Range, len: usize) -> RunResult<std::ops::Range<usize>> {
    let bound = |b: i64| {
        usize::try_from(b).map_err(|_| RuntimeError::new(format!("{b} is not a valid index")))
    };
    let start = range.start.map_or(Ok(0), bound)?;
    let end = match range.end {
        Some(end) => bound(end)? + usize::from(range.inclusive),
        None => len,
    };
    if start > end || end > len {
        return Err(RuntimeError::new(format!(
            "range {start}..{end} is out of range for length {len}"
        )));
    }
    Ok(start..end)
}

fn position(index: i64, len: usize) -> RunResult<usize> {
    usize::try_from(index)
        .ok()
        .filter(|&i| i < len)
        .ok_or_else(|| RuntimeError::new(format!("index {index} is out of range for length {len}")))
}

fn out_of_range(index: IntValue, len: usize) -> RuntimeError {
    RuntimeError::new(format!("index {index} is out of range for length {len}"))
}
