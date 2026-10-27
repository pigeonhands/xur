//! Calls, method calls and field access, dispatched on values at runtime.

use xur_resolve::{DefId, DefKind, Namespace, Res};
use xur_runtime::{Record, RunResult, RuntimeError, TypeRef, Value};
use xur_span::{Span, Symbol};

use crate::interp::Interp;

impl Interp<'_> {
    pub(crate) fn call_value(
        &mut self,
        callee: &Value,
        args: Vec<Value>,
        type_args: Vec<TypeRef>,
        span: Span,
    ) -> RunResult<Value> {
        match callee {
            Value::Fn(def) => self.call_fn(*def, args, type_args),
            Value::Closure(closure) => self.call_closure(closure, args),
            Value::Ctor(ctor) => Ok(Value::adt(*ctor, args)),
            Value::NativeFn(id) => {
                let function = self.shared.registry.function(*id);
                let caller_type_args = std::mem::replace(&mut self.native_type_args, type_args);
                let result = function.call(self, &args).map_err(|error| error.at(span));
                self.native_type_args = caller_type_args;
                result
            }
            other => Err(RuntimeError::new(format!(
                "{} is not callable",
                other.kind()
            ))),
        }
    }

    /// `receiver.method(args)`: an `impl` function of a Xur struct or enum,
    /// a registered method of a native object or built-in value, or a
    /// member of a module.
    pub(crate) fn call_method(
        &mut self,
        receiver: Value,
        method: Symbol,
        args: Vec<Value>,
        span: Span,
    ) -> RunResult<Value> {
        if let Value::Module(module) = receiver {
            let member = self.module_member(module, method)?;
            return self.call_value(&member, args, Vec::new(), span);
        }
        let function = match &receiver {
            Value::Adt(adt) => self.adt_method(adt.ctor, method),
            Value::Native(object) => self
                .shared
                .registry
                .type_function(object.ty, method.as_str())
                .map(Value::NativeFn),
            other => other.builtin_type().and_then(|ty| {
                self.shared
                    .registry
                    .builtin_method(ty, method.as_str())
                    .map(Value::NativeFn)
            }),
        };
        let Some(function) = function else {
            return Err(RuntimeError::new(format!(
                "no method `{method}` on {}",
                self.describe_type(&receiver)
            )));
        };
        let args = std::iter::once(receiver).chain(args).collect();
        self.call_value(&function, args, Vec::new(), span)
    }

    /// The `impl` function `method` of the struct or enum `ctor` belongs to.
    fn adt_method(&self, ctor: DefId, method: Symbol) -> Option<Value> {
        let program = self.shared.program();
        let resolutions = program.resolutions();
        let def = resolutions.defs.get(ctor);
        let ty = match def.kind {
            DefKind::Variant => def.parent?,
            _ => ctor,
        };
        match resolutions.assoc_fn(ty, method)? {
            Res::Def(DefKind::Fn, def) => Some(Value::Fn(def)),
            Res::Def(DefKind::NativeFn(id), _) => Some(Value::NativeFn(id)),
            _ => None,
        }
    }

    /// `base.field`: a field of a native object or record, an element of a
    /// tuple or struct by number, or a module's member.
    pub(crate) fn field(&mut self, base: &Value, field: Symbol) -> RunResult<Value> {
        let missing = || {
            RuntimeError::new(format!(
                "no field `{field}` on {}",
                self.describe_type(base)
            ))
        };
        match base {
            Value::Module(module) => self.module_member(*module, field),
            Value::Native(object) => {
                let registry = &self.shared.registry;
                registry
                    .field(self, object, field.as_str())
                    .ok_or_else(missing)?
            }
            Value::Record(record) => record.field(field).cloned().ok_or_else(missing),
            Value::Tuple(items) => numbered(items, field).ok_or_else(missing),
            Value::Adt(adt) => match self.shared.field_names(adt.ctor) {
                Some(names) => names
                    .iter()
                    .position(|&name| name == field)
                    .map(|index| adt.fields[index].clone())
                    .ok_or_else(missing),
                None => numbered(&adt.fields, field).ok_or_else(missing),
            },
            _ => Err(missing()),
        }
    }

    /// A copy of the struct or record `base` with `field` set to `value`.
    pub(crate) fn set_field(&self, base: &Value, field: Symbol, value: Value) -> RunResult<Value> {
        let missing = || {
            RuntimeError::new(format!(
                "no field `{field}` on {}",
                self.describe_type(base)
            ))
        };
        let adt = match base {
            Value::Adt(adt) => adt,
            Value::Record(record) => {
                let mut fields = record.fields.clone();
                let (_, slot) = fields
                    .iter_mut()
                    .find(|(name, _)| *name == field)
                    .ok_or_else(missing)?;
                *slot = value;
                return Ok(Value::Record(Record { fields }.into()));
            }
            _ => return Err(missing()),
        };
        let index = match self.shared.field_names(adt.ctor) {
            Some(names) => names.iter().position(|&name| name == field),
            None => field
                .as_str()
                .parse()
                .ok()
                .filter(|&i| i < adt.fields.len()),
        }
        .ok_or_else(missing)?;
        let mut fields = adt.fields.clone();
        fields[index] = value;
        Ok(Value::adt(adt.ctor, fields))
    }

    /// The value named `name` in `module`, which modules-as-values expose
    /// as members: `math.add`.
    fn module_member(&mut self, module: DefId, name: Symbol) -> RunResult<Value> {
        let res = {
            let program = self.shared.program();
            let resolutions = program.resolutions();
            resolutions
                .lookup(module, Namespace::Value, name)
                .or_else(|| resolutions.lookup(module, Namespace::Type, name))
                .ok_or_else(|| {
                    RuntimeError::new(format!(
                        "module `{}` has no member `{name}`",
                        resolutions.def_path(module)
                    ))
                })?
        };
        match res {
            Res::Def(DefKind::Fn, def) => Ok(Value::Fn(def)),
            Res::Def(DefKind::NativeFn(id), _) => Ok(Value::NativeFn(id)),
            Res::Def(DefKind::Const, def) => self.constant(def),
            Res::Def(DefKind::Mod, def) => Ok(Value::Module(def)),
            Res::Def(DefKind::Struct | DefKind::Variant, def) => Ok(Value::Ctor(def)),
            other => Err(RuntimeError::new(format!(
                "`{name}` is a {}, not a value",
                other.descr()
            ))),
        }
    }
}

/// Element `field` of `items`, if `field` is a number such as `0`.
fn numbered(items: &[Value], field: Symbol) -> Option<Value> {
    let index: usize = field.as_str().parse().ok()?;
    items.get(index).cloned()
}
