//! Checking values against the types written in the source.
//!
//! `check_type` returns `Ok(None)` when the value already matches, and
//! `Ok(Some(coerced))` when it was converted to match — currently only
//! Record → Adt coercion for JSON-parsed values assigned to typed structs.
//!
//! Missing fields whose type is `Option` are filled with `None` unless the
//! struct is marked `#[coerce(strict)]`.

use xur_ast::MetaItemKind;
use xur_bytecode::RtType;
use xur_resolve::{DefId, DefKind};
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

    /// Tries to coerce `value` into a value matching `ty`.
    ///
    /// Record → Adt: an untyped record whose fields match a struct's
    /// definition is built into that struct. Missing fields whose declared
    /// type is `Option` are filled with `None`; present values that aren't
    /// already wrapped are wrapped in `Some`. Structs with
    /// `#[coerce(strict)]` require every field to be present.
    ///
    /// value → Some(value): a non-Option value assigned to an `Option`
    /// field is wrapped in `Some`.
    fn try_coerce(&self, value: &Value, ty: &RtType) -> RunResult<Option<Value>> {
        let RtType::Adt { def, name } = ty else {
            return Ok(None);
        };
        let option_def = self.shared.option_def();
        let some_ctor = self.shared.lang.some;
        let none_ctor = self.shared.lang.none;

        // value → Some(value): wrap a non-Option value into Option.
        if *def == option_def {
            if let Value::Adt(adt) = value {
                let program = self.shared.program();
                let ctor_def = program.resolutions().defs.get(adt.ctor);
                let parent = match ctor_def.kind {
                    DefKind::Variant => ctor_def.parent,
                    _ => Some(adt.ctor),
                };
                if parent == Some(option_def) {
                    return Ok(None);
                }
            }
            return Ok(Some(Value::adt(some_ctor, vec![value.clone()])));
        }

        // Record → Adt coercion.
        let Value::Record(record) = value else {
            return Ok(None);
        };
        if record.ty.is_some() {
            return Ok(None);
        }
        let Some(field_names) = self.shared.field_names(*def) else {
            return Ok(None);
        };
        let strict = self.has_coerce_strict(*def);
        let mut fields = Vec::with_capacity(field_names.len());
        for (i, expected_name) in field_names.iter().enumerate() {
            match record.field(*expected_name) {
                Some(record_value) => {
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
                None if strict => {
                    return Err(RuntimeError::new(format!(
                        "missing field `{expected_name}` for `{name}`"
                    )));
                }
                None => {
                    let is_option = self
                        .shared
                        .field_type(*def, i)
                        .is_some_and(|ty| matches!(ty, RtType::Adt { def, .. } if def == option_def));
                    if is_option {
                        fields.push(Value::adt(none_ctor, vec![]));
                    } else {
                        return Err(RuntimeError::new(format!(
                            "missing field `{expected_name}` for `{name}`"
                        )));
                    }
                }
            }
        }
        Ok(Some(Value::adt(*def, fields)))
    }

    /// Whether `def` has the `#[coerce(strict)]` attribute.
    fn has_coerce_strict(&self, def: DefId) -> bool {
        let Some(attrs) = self.shared.ctor_attrs(def) else {
            return false;
        };
        attrs.iter().any(|attr| {
            let meta = &attr.meta;
            let name = meta.path.as_ident().map(|i| i.name.as_str());
            matches!(
                (name, &meta.kind),
                (Some("coerce"), MetaItemKind::List(items))
                    if items.iter().any(|item| matches!(item,
                        xur_ast::MetaItemInner::MetaItem(m)
                            if m.path.as_ident().is_some_and(|i| i.name.as_str() == "strict")
                    ))
            )
        })
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
