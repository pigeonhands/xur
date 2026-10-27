//! Formatting values, as `{}` does.

use xur_resolve::DefKind;
use xur_runtime::{Range, Value, lock};

use crate::interp::Interp;

impl Interp<'_> {
    /// Formats `value` for display: strings as they are, and everything
    /// else as it would be written, with strings inside quoted.
    pub(crate) fn display_value(&self, value: &Value) -> String {
        match value {
            Value::Str(s) => s.to_string(),
            other => self.repr(other),
        }
    }

    fn repr(&self, value: &Value) -> String {
        let list = |items: &[Value]| {
            let items: Vec<_> = items.iter().map(|item| self.repr(item)).collect();
            items.join(", ")
        };
        match value {
            Value::Unit => "()".to_owned(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Str(s) => format!("{s:?}"),
            Value::Tuple(items) if items.len() == 1 => format!("({},)", self.repr(&items[0])),
            Value::Tuple(items) => format!("({})", list(items)),
            Value::List(items) => format!("[{}]", list(&lock(items))),
            Value::Bytes(bytes) => format!("b\"{}\"", lock(bytes).escape_ascii()),
            Value::Adt(adt) => {
                let name = self.def_name(adt.ctor);
                if adt.fields.is_empty() {
                    name
                } else if let Some(names) = self.shared.field_names(adt.ctor) {
                    let fields: Vec<_> = names
                        .iter()
                        .zip(&adt.fields)
                        .map(|(name, value)| format!("{name}: {}", self.repr(value)))
                        .collect();
                    format!("{name} {{ {} }}", fields.join(", "))
                } else {
                    format!("{name}({})", list(&adt.fields))
                }
            }
            Value::Record(record) => {
                let registry = &self.shared.registry;
                let name = match (record.ty, record.variant) {
                    (Some(ty), Some(variant)) => format!("{}::{variant}", registry.type_name(ty)),
                    (Some(ty), None) => registry.type_name(ty),
                    (None, Some(variant)) => variant.to_string(),
                    (None, None) => String::new(),
                };
                let is_tuple = record
                    .fields
                    .iter()
                    .enumerate()
                    .all(|(i, (field, _))| field.as_str() == i.to_string());
                let fields: Vec<_> = record
                    .fields
                    .iter()
                    .map(|(field, value)| match is_tuple {
                        true => self.repr(value),
                        false => format!("{field}: {}", self.repr(value)),
                    })
                    .collect();
                match (name.is_empty(), fields.is_empty(), is_tuple) {
                    (false, true, _) => name,
                    (false, false, true) => format!("{name}({})", fields.join(", ")),
                    (false, false, false) => format!("{name} {{ {} }}", fields.join(", ")),
                    (true, _, _) => format!("{{ {} }}", fields.join(", ")),
                }
            }
            Value::Native(object) => self.shared.registry.display(object),
            Value::Fn(def) | Value::Ctor(def) => format!("<fn {}>", self.def_path(*def)),
            Value::NativeFn(id) => format!("<fn {}>", self.shared.registry.function(*id).name),
            Value::Closure(_) => "<closure>".to_owned(),
            Value::Module(def) => format!("<module {}>", self.def_path(*def)),
            Value::Iterator(_) => "<iterator>".to_owned(),
            Value::Range(Range {
                start,
                end,
                inclusive,
            }) => {
                let bound = |b: &Option<i64>| b.map(|b| b.to_string()).unwrap_or_default();
                let op = if *inclusive { "..=" } else { ".." };
                format!("{}{op}{}", bound(start), bound(end))
            }
        }
    }

    /// The name of `value`'s type, for messages: `Shape`, `list`, ...
    pub(crate) fn describe_type(&self, value: &Value) -> String {
        match value {
            Value::Adt(adt) => {
                let program = self.shared.program();
                let defs = &program.resolutions().defs;
                let def = defs.get(adt.ctor);
                let ty = match def.kind {
                    DefKind::Variant => def.parent.map_or(def.name, |parent| defs.get(parent).name),
                    _ => def.name,
                };
                format!("`{ty}`")
            }
            Value::Native(object) => format!("`{}`", self.shared.registry.type_name(object.ty)),
            Value::Record(record) if record.ty.is_some() => {
                let ty = record.ty.map(|ty| self.shared.registry.type_name(ty));
                format!("`{}`", ty.unwrap_or_default())
            }
            other => other.kind().to_owned(),
        }
    }

    fn def_name(&self, def: xur_resolve::DefId) -> String {
        self.shared
            .program()
            .resolutions()
            .defs
            .get(def)
            .name
            .to_string()
    }

    fn def_path(&self, def: xur_resolve::DefId) -> String {
        self.shared.program().resolutions().def_path(def)
    }
}
