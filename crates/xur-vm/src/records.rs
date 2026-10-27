//! Building records of Rust types registered by shape, as in
//! `ImageFileMachine::Amd64` or `ExportLocation::Address(4096)`.

use xur_resolve::{NativeTypeId, NativeVariantKind};
use xur_runtime::{Record, RunResult, RuntimeError, Value};
use xur_span::Symbol;

use crate::interp::Interp;

/// The fields a record is built with, as written.
pub(crate) enum RecordArgs {
    Positional(Vec<Value>),
    Named(Vec<(Symbol, Value)>),
}

impl Interp<'_> {
    /// A record of `ty` (of its variant `variant`), checking the fields
    /// against the Rust type's shape and putting them in its order.
    pub(crate) fn build_record(
        &self,
        ty: NativeTypeId,
        variant: Option<Symbol>,
        args: RecordArgs,
    ) -> RunResult<Value> {
        let registry = &self.shared.registry;
        let name = match variant {
            Some(variant) => format!("{}::{variant}", registry.type_name(ty)),
            None => registry.type_name(ty),
        };
        let Some(ctor) = registry.record_ctor(ty, variant) else {
            return Err(RuntimeError::new(format!(
                "`{name}` cannot be built from Xur"
            )));
        };
        let error = |message: String| Err(RuntimeError::new(message));
        let values = match (ctor.kind, args) {
            (
                NativeVariantKind::Unit | NativeVariantKind::Tuple,
                RecordArgs::Positional(values),
            ) => {
                if values.len() != ctor.fields.len() {
                    return error(format!(
                        "`{name}` takes {} field(s), but {} were given",
                        ctor.fields.len(),
                        values.len()
                    ));
                }
                values
            }
            (NativeVariantKind::Named, RecordArgs::Named(mut given)) => {
                if let Some((unknown, _)) = given.iter().find(|(n, _)| !ctor.fields.contains(n)) {
                    return error(format!("`{name}` has no field `{unknown}`"));
                }
                let mut values = Vec::with_capacity(ctor.fields.len());
                for field in &ctor.fields {
                    let Some(index) = given.iter().position(|(n, _)| n == field) else {
                        return error(format!("missing field `{field}` for `{name}`"));
                    };
                    values.push(given.remove(index).1);
                }
                if let Some((repeated, _)) = given.first() {
                    return error(format!("field `{repeated}` specified more than once"));
                }
                values
            }
            (NativeVariantKind::Named, RecordArgs::Positional(_)) => {
                return error(format!(
                    "`{name}` has named fields; use `{name} {{ field: value }}`"
                ));
            }
            (_, RecordArgs::Named(_)) => {
                return error(format!("`{name}` has no named fields; use `{name}(...)`"));
            }
        };
        Ok(Value::Record(
            Record {
                ty: Some(ty),
                variant,
                fields: ctor.fields.into_iter().zip(values).collect(),
            }
            .into(),
        ))
    }
}
