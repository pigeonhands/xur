//! What `std::reflect` can see of a running program.

use std::sync::PoisonError;

use xur_ast::{PatKind, VariantData};
use xur_bytecode::ItemRef;
use xur_resolve::{DefId, DefKind};
use xur_runtime::{AdtInfo, FieldInfo, TypeRef, Value};

use crate::interp::Interp;

impl Interp<'_> {
    /// The struct or enum variant `def`, with its fields' written types.
    pub(crate) fn adt_info(&self, def: DefId) -> Option<AdtInfo> {
        let items = self
            .shared
            .items
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let ItemRef::Ctor { data, attrs } = items.get(def)? else {
            return None;
        };
        let program = self.shared.program();
        let resolutions = program.resolutions();
        let fields = match data {
            VariantData::Struct(fields) | VariantData::Tuple(fields) => fields.as_slice(),
            VariantData::Unit => &[],
        };
        let fields = fields
            .iter()
            .map(|field| FieldInfo {
                name: field.ident.map(|ident| ident.name),
                ty: TypeRef {
                    name: xur_bytecode::type_name(resolutions, &field.ty),
                    def: xur_bytecode::type_def(resolutions, &field.ty),
                },
                attrs: &field.attrs,
            })
            .collect();
        Some(AdtInfo {
            name: resolutions.def_path(def),
            ctor: def,
            is_variant: resolutions.defs.get(def).kind == DefKind::Variant,
            attrs,
            fields,
        })
    }

    /// The methods `value.name()` can call: `impl` functions taking `self`,
    /// and the registered methods of native and built-in values.
    pub(crate) fn methods_of(&self, value: &Value) -> Vec<String> {
        let registry = &self.shared.registry;
        match value {
            Value::Adt(adt) => self.adt_methods(adt.ctor),
            Value::Native(object) => registry.type_methods(object.ty),
            other => other
                .builtin_type()
                .map(|ty| registry.builtin_methods(ty))
                .unwrap_or_default(),
        }
    }

    fn adt_methods(&self, ctor: DefId) -> Vec<String> {
        let program = self.shared.program();
        let resolutions = program.resolutions();
        let def = resolutions.defs.get(ctor);
        let ty = match def.kind {
            DefKind::Variant => def.parent.unwrap_or(ctor),
            _ => ctor,
        };
        let items = self
            .shared
            .items
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        resolutions
            .assoc_fns(ty)
            .into_iter()
            .filter(|&(_, def)| match items.get(def) {
                Some(ItemRef::Fn { item, .. }) => item.decl.inputs.first().is_some_and(
                    |param| matches!(param.pat.kind, PatKind::Ident(ident) if ident.name.as_str() == "self"),
                ),
                // Functions registered for a native type.
                _ => true,
            })
            .map(|(name, _)| name.to_string())
            .collect()
    }
}
