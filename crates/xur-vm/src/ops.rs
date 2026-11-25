//! Operators on values, checked at runtime.
//!
//! Integers and floats don't mix. Integer arithmetic WRAPS to the result's
//! width on overflow (like Rust's release mode); division by zero is still an
//! error. A binary op's result type is the wider of the two operands and signed
//! if either is; a shift keeps the left operand's type and masks the count to
//! its width; floats keep their width (f32 ⊕ f32 stays f32). Strings, lists and
//! bytes can be concatenated with `+`, and indexed with a range to slice them.

use std::cmp::Ordering;

use xur_ast::{BinOpKind, UnOp};
use xur_runtime::{FloatValue, IntValue, MapKey, Range, RunResult, RuntimeError, Value, lock};

use crate::interp::Interp;

pub(crate) fn unary(op: UnOp, value: &Value) -> RunResult<Value> {
    match (op, value) {
        (UnOp::Neg, Value::Int(i)) => Ok(Value::Int(i.wrapping_neg())),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(f.negate())),
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
                Some(Value::Float(float_arith(op, *a, *b)))
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
                    // Two repr enums compare structurally (same variant) — they
                    // must NOT match just because they share a discriminant.
                    (Some(_), Some(_)) => lhs == rhs,
                    // A repr enum vs a plain int coerces the enum to its value.
                    (Some(l), None) => l == rhs,
                    (None, Some(r)) => lhs == r,
                    (None, None) => lhs == rhs,
                }
            }
        }
    }
}

fn int_arith(op: BinOpKind, a: IntValue, b: IntValue) -> RunResult<Value> {
    let (ai, bi) = (a.to_i128(), b.to_i128());
    // Division/remainder by zero is an error; everything else wraps to the
    // result width. i128 is wide enough that wrapping only kicks in at the final
    // narrow for operands up to 64 bits, matching Rust's `wrapping_*`.
    let result = match op {
        BinOpKind::Div | BinOpKind::Rem if bi == 0 => {
            return Err(RuntimeError::new("attempt to divide by zero"));
        }
        BinOpKind::Add => ai.wrapping_add(bi),
        BinOpKind::Sub => ai.wrapping_sub(bi),
        BinOpKind::Mul => ai.wrapping_mul(bi),
        BinOpKind::Div => ai.wrapping_div(bi),
        _ => ai.wrapping_rem(bi),
    };
    let (signed, width) = IntValue::promote_type(a, b);
    Ok(Value::Int(IntValue::wrap(result, signed, width)))
}

fn int_bitwise(a: IntValue, b: IntValue, op: impl FnOnce(i128, i128) -> i128) -> Value {
    let result = op(a.to_i128(), b.to_i128());
    let (signed, width) = IntValue::promote_type(a, b);
    Value::Int(IntValue::wrap(result, signed, width))
}

fn float_arith(op: BinOpKind, a: FloatValue, b: FloatValue) -> FloatValue {
    use BinOpKind::*;
    // Preserve width: f32 ⊕ f32 stays f32 (computed in f32); otherwise f64.
    match (a, b) {
        (FloatValue::F32(x), FloatValue::F32(y)) => FloatValue::F32(match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => x / y,
            _ => x % y,
        }),
        _ => {
            let (x, y) = (a.as_f64(), b.as_f64());
            FloatValue::F64(match op {
                Add => x + y,
                Sub => x - y,
                Mul => x * y,
                Div => x / y,
                _ => x % y,
            })
        }
    }
}

fn shift_amount(a: IntValue, b: IntValue) -> RunResult<u32> {
    let count = b
        .to_u32()
        .ok_or_else(|| RuntimeError::new(format!("invalid shift amount: {b}")))?;
    // Mask the count to the left operand's bit width, as Rust's wrapping shifts do.
    Ok(count % (a.width() as u32 * 8))
}

fn shift_left(a: IntValue, b: IntValue) -> RunResult<Value> {
    let shift = shift_amount(a, b)?;
    // Result keeps the LEFT operand's type; the count never influences it.
    let result = a.to_i128().wrapping_shl(shift);
    Ok(Value::Int(IntValue::wrap(result, a.is_signed(), a.width())))
}

fn shift_right(a: IntValue, b: IntValue) -> RunResult<Value> {
    let shift = shift_amount(a, b)?;
    // to_i128 sign/zero-extends per the left type, so `>>` is arithmetic for a
    // signed operand and logical for an unsigned one; wrap back to the left type.
    let result = a.to_i128() >> shift;
    Ok(Value::Int(IntValue::wrap(result, a.is_signed(), a.width())))
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
