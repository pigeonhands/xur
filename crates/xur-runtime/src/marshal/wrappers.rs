//! Shared mutable collection types for the typed function API.

use std::sync::{Arc, Mutex};

use crate::convert::ConvertCx;
use crate::{RunResult, RuntimeError, Value};

use super::{FromValue, IntoXurValue};

/// A shared mutable list, extracted from `Value::List` without copying.
pub struct XurList(pub Arc<Mutex<Vec<Value>>>);

impl FromValue for XurList {
    fn from_value(_cx: &dyn ConvertCx, value: &Value) -> RunResult<Self> {
        match value {
            Value::List(arc) => Ok(XurList(Arc::clone(arc))),
            _ => Err(RuntimeError::new("expected a list")),
        }
    }
}

impl IntoXurValue for XurList {
    fn into_xur_value(self, _cx: &dyn ConvertCx) -> RunResult<Value> {
        Ok(Value::List(self.0))
    }
}

/// A shared mutable byte buffer, extracted from `Value::Bytes` without copying.
pub struct XurBytes(pub Arc<Mutex<Vec<u8>>>);

impl FromValue for XurBytes {
    fn from_value(_cx: &dyn ConvertCx, value: &Value) -> RunResult<Self> {
        match value {
            Value::Bytes(arc) => Ok(XurBytes(Arc::clone(arc))),
            _ => Err(RuntimeError::new("expected bytes")),
        }
    }
}

impl IntoXurValue for XurBytes {
    fn into_xur_value(self, _cx: &dyn ConvertCx) -> RunResult<Value> {
        Ok(Value::Bytes(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock;

    #[test]
    fn xur_list_shares_the_arc() {
        let list = Value::list(vec![Value::int(1), Value::int(2)]);
        let Value::List(arc) = &list else { panic!() };
        let orig = Arc::clone(arc);

        let XurList(inner) = XurList(Arc::clone(arc));
        lock(&inner).push(Value::int(3));

        assert_eq!(lock(&orig).len(), 3);
    }

    #[test]
    fn xur_bytes_shares_the_arc() {
        let bytes = Value::Bytes(Arc::new(Mutex::new(vec![1, 2])));
        let Value::Bytes(arc) = &bytes else { panic!() };
        let orig = Arc::clone(arc);

        let XurBytes(inner) = XurBytes(Arc::clone(arc));
        lock(&inner).push(3);

        assert_eq!(lock(&orig).len(), 3);
    }
}
