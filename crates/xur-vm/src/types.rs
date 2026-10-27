//! Checking values against the types written in the source.
//!
//! `check_type` returns `Ok(None)` when the value already matches, and
//! `Ok(Some(coerced))` when it was converted to match — currently only
//! Record → Adt coercion for JSON-parsed values assigned to typed structs.

use xur_bytecode::RtType;
use xur_resolve::DefKind;
use xur_runtime::{RunResult, RuntimeError, Value};
use crate::interp::Interp;

impl Interp<'_> {
    /// Checks that `value` matches `ty`, returning a coerced value when a
    /// conversion was needed (Record → Adt).
    pub(crate) fn check_type(&self, value: &Value, ty: &RtType) -> RunResult<Option<Value>> {
        if self.has_type(value, ty) {
            return Ok(None);
        }
        if let Some(coerced) = self.try_coerce(value, ty)? {
            return Ok(Some(coerced));
        }
        let found = match value {
            Value::Int(i) if matches!(ty, RtType::Int { .. }) => i.to_string(),
            other => self.describe_type(other),
        };
        Err(RuntimeError::new(format!("expected `{ty}`, found {found}")))
    }

    /// Tries to coerce `value` into a value matching `ty`. Currently handles
    /// Record → Adt: an untyped record (from JSON parsing, for instance)
    /// whose fields match a struct's definition is built into that struct.
    fn try_coerce(&self, value: &Value, ty: &RtType) -> RunResult<Option<Value>> {
        let (RtType::Adt { def, name }, Value::Record(record)) = (ty, value) else {
            return Ok(None);
        };
        if record.ty.is_some() {
            return Ok(None);
        }
        let Some(field_names) = self.shared.field_names(*def) else {
            return Ok(None);
        };
        if field_names.len() != record.fields.len() {
            return Err(RuntimeError::new(format!(
                "expected `{name}` with {} fields, record has {}",
                field_names.len(),
                record.fields.len()
            )));
        }
        let mut fields = Vec::with_capacity(field_names.len());
        for (i, expected_name) in field_names.iter().enumerate() {
            let record_value = record.field(*expected_name).ok_or_else(|| {
                RuntimeError::new(format!(
                    "missing field `{expected_name}` for `{name}`"
                ))
            })?;
            // Recursively coerce nested records.
            let field_value = if let Some(field_ty) = self.shared.field_type(*def, i) {
                match self.check_type(record_value, &field_ty)? {
                    Some(coerced) => coerced,
                    None => record_value.clone(),
                }
            } else {
                record_value.clone()
            };
            fields.push(field_value);
        }
        Ok(Some(Value::adt(*def, fields)))
    }

    fn has_type(&self, value: &Value, ty: &RtType) -> bool {
        match (ty, value) {
            (RtType::Any, _)
            | (RtType::Unit, Value::Unit)
            | (RtType::Bool, Value::Bool(_))
            | (RtType::Str, Value::Str(_))
            | (RtType::Bytes, Value::Bytes(_))
            | (RtType::Float, Value::Float(_))
            | (RtType::List, Value::List(_)) => true,
            (RtType::Int { min, max, .. }, Value::Int(i)) => (min..=max).contains(&i),
            (RtType::Tuple(tys), Value::Tuple(items)) => {
                tys.len() == items.len()
                    && tys
                        .iter()
                        .zip(items.iter())
                        .all(|(ty, item)| self.has_type(item, ty))
            }
            (RtType::Adt { def, .. }, Value::Adt(adt)) => {
                let program = self.shared.program();
                let ctor = program.resolutions().defs.get(adt.ctor);
                let ty = match ctor.kind {
                    DefKind::Variant => ctor.parent,
                    _ => Some(adt.ctor),
                };
                ty == Some(*def)
            }
            (RtType::Native { id, .. }, Value::Native(object)) => object.ty == *id,
            (RtType::Native { id, .. }, Value::Record(record)) => record.ty == Some(*id),
            _ => false,
        }
    }
}
