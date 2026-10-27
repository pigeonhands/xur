//! Checking values against the types written in the source.

use xur_bytecode::RtType;
use xur_resolve::DefKind;
use xur_runtime::{RunResult, RuntimeError, Value};

use crate::interp::Interp;

impl Interp<'_> {
    pub(crate) fn check_type(&self, value: &Value, ty: &RtType) -> RunResult<()> {
        if self.has_type(value, ty) {
            return Ok(());
        }
        let found = match value {
            Value::Int(i) if matches!(ty, RtType::Int { .. }) => i.to_string(),
            other => self.describe_type(other),
        };
        Err(RuntimeError::new(format!("expected `{ty}`, found {found}")))
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
