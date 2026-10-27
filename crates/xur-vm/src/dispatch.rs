//! Calls, method calls and field access, dispatched on values at runtime.

use xur_resolve::{DefId, DefKind, Namespace, Res, RtType};
use xur_runtime::{BuiltinType, Record, RunResult, RuntimeError, TypeRef, Value};
use xur_span::{Span, Symbol};

use crate::interp::Interp;

impl Interp<'_> {
    pub(crate) fn call_value(
        &mut self,
        callee: &Value,
        args: &[Value],
        type_args: Vec<TypeRef>,
        span: Span,
    ) -> RunResult<Value> {
        match callee {
            Value::Fn(def) => self.call_fn(*def, args, type_args),
            Value::Closure(closure) => self.call_closure(closure, args),
            Value::Ctor(ctor) => Ok(Value::adt(*ctor, args.to_vec())),
            Value::NativeFn(id) => {
                let function = self.shared.registry.function(*id);
                let caller_type_args = std::mem::replace(&mut self.native_type_args, type_args);
                let result = function.call(self, args).map_err(|error| error.at(span));
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
        args: &[Value],
        span: Span,
    ) -> RunResult<Value> {
        if let Value::Module(module) = receiver {
            match method.as_str() {
                "get_method" => return self.module_get_method(module, args),
                "get_attr" => return self.module_get_attr(module, args),
                _ => {}
            }
            let member = self.module_member(module, method)?;
            return self.call_value(&member, args, Vec::new(), span);
        }
        let function = match &receiver {
            Value::Adt(adt) => self.adt_method(adt.ctor, method).or_else(|| {
                let registry = &self.shared.registry;
                registry
                    .builtin_method(BuiltinType::Struct, method.as_str())
                    .map(Value::NativeFn)
            }),
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
        let mut full = Vec::with_capacity(1 + args.len());
        full.push(receiver);
        full.extend_from_slice(args);
        self.call_value(&function, &full, Vec::new(), span)
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

    /// A copy of the struct, record or tuple `base` with `field` set to
    /// `value`.
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
                let record = Record {
                    fields,
                    ..Record::clone(record)
                };
                return Ok(Value::Record(record.into()));
            }
            Value::Tuple(items) => {
                let index: usize = field.as_str().parse().map_err(|_| missing())?;
                let mut items = items.to_vec();
                *items.get_mut(index).ok_or_else(missing)? = value;
                return Ok(Value::tuple(items));
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
        let value = if let Some(ty) = self.shared.field_type(adt.ctor, index) {
            match self.strict_check_type(&value, &ty)? {
                Some(coerced) => coerced,
                None => value,
            }
        } else {
            value
        };
        let mut fields = adt.fields.clone();
        fields[index] = value;
        Ok(Value::adt(adt.ctor, fields))
    }

    /// `module.get_method("name")`, `module.get_method("name", 2)`, or
    /// `module.get_method("name", fn(u32, u32) -> u32)`.
    /// Returns `Ok(fn)` or `Err(message)`.
    fn module_get_method(&mut self, module: DefId, args: &[Value]) -> RunResult<Value> {
        let (name_str, expected_params, expected_sig) = match args {
            [Value::Str(name)] => (name.as_ref(), None, None),
            [Value::Str(name), Value::Int(n)] => (name.as_ref(), Some(*n as usize), None),
            [Value::Str(name), Value::Type(ty)] => (name.as_ref(), None, Some(ty.as_ref())),
            _ => return Err(RuntimeError::new(
                "`get_method` takes a name and an optional param count or type: `mod.get_method(\"name\")`, `mod.get_method(\"name\", 2)`, or `mod.get_method(\"name\", fn(u32) -> str)`",
            )),
        };
        let name = Symbol::intern(name_str);
        let res = {
            let program = self.shared.program();
            let resolutions = program.resolutions();
            resolutions.lookup(module, Namespace::Value, name)
        };
        let (value, actual_params, fn_param_types, fn_ret_type) = match res {
            Some(Res::Def(DefKind::Fn, def)) => {
                let function = self.shared.code(def)?;
                let params = function.params as usize;
                let param_types = function.param_types.clone();
                let ret_type = function.ret_type.clone();
                (Value::Fn(def), Some(params), Some(param_types), ret_type)
            }
            Some(Res::Def(DefKind::NativeFn(id), _)) => {
                let function = self.shared.registry.function(id);
                let params = function.signature.as_ref().map(|s| s.params.len());
                (Value::NativeFn(id), params, None, None)
            }
            Some(_) => {
                return Ok(self.result_err(format!("`{name_str}` is not a function")));
            }
            None => {
                let program = self.shared.program();
                let mod_name = program.resolutions().def_path(module);
                return Ok(self.result_err(format!(
                    "module `{mod_name}` has no member `{name_str}`"
                )));
            }
        };
        if let Some(expected) = expected_params {
            if let Some(actual) = actual_params {
                if actual != expected {
                    return Ok(self.result_err(format!(
                        "`{name_str}` takes {actual} parameter(s), expected {expected}"
                    )));
                }
            }
        }
        if let Some(RtType::Fn { params: sig_params, ret: sig_ret }) = expected_sig {
            if let Some(actual) = actual_params {
                if actual != sig_params.len() {
                    return Ok(self.result_err(format!(
                        "`{name_str}` takes {actual} parameter(s), expected {}",
                        sig_params.len()
                    )));
                }
            }
            if let Some(param_types) = &fn_param_types {
                for (i, (actual, expected)) in param_types.iter().zip(sig_params.iter()).enumerate() {
                    if let Some(actual) = actual {
                        if *expected != RtType::Any && actual != expected {
                            return Ok(self.result_err(format!(
                                "`{name_str}` param {} has type {actual}, expected {expected}",
                                i + 1
                            )));
                        }
                    }
                }
            }
            if **sig_ret != RtType::Any {
                if let Some(actual_ret) = &fn_ret_type {
                    if actual_ret != sig_ret.as_ref() {
                        return Ok(self.result_err(format!(
                            "`{name_str}` returns {actual_ret}, expected {sig_ret}"
                        )));
                    }
                }
            }
        }
        Ok(self.result_ok(value))
    }

    /// `module.get_attr("name")`. Returns `Ok(value)` or `Err(message)`.
    fn module_get_attr(&mut self, module: DefId, args: &[Value]) -> RunResult<Value> {
        let name_str = match args {
            [Value::Str(name)] => name.as_ref(),
            _ => return Err(RuntimeError::new(
                "`get_attr` takes a name: `mod.get_attr(\"name\")`",
            )),
        };
        let name = Symbol::intern(name_str);
        let res = {
            let program = self.shared.program();
            let resolutions = program.resolutions();
            resolutions
                .lookup(module, Namespace::Value, name)
                .or_else(|| resolutions.lookup(module, Namespace::Type, name))
        };
        match res {
            Some(Res::Def(DefKind::Const, def)) => {
                let value = self.constant(def)?;
                Ok(self.result_ok(value))
            }
            Some(Res::Def(DefKind::Fn | DefKind::NativeFn(_), _)) => {
                Ok(self.result_err(format!("`{name_str}` is a function, not an attribute")))
            }
            Some(Res::Def(DefKind::Mod, def)) => {
                Ok(self.result_ok(Value::Module(def)))
            }
            Some(Res::Def(DefKind::Struct | DefKind::Variant, def)) => {
                Ok(self.result_ok(Value::Ctor(def)))
            }
            Some(_) | None => {
                let program = self.shared.program();
                let mod_name = program.resolutions().def_path(module);
                Ok(self.result_err(format!(
                    "module `{mod_name}` has no attribute `{name_str}`"
                )))
            }
        }
    }

    fn result_ok(&self, value: Value) -> Value {
        Value::adt(self.shared.lang.ok, vec![value])
    }

    fn result_err(&self, message: String) -> Value {
        Value::adt(self.shared.lang.err, vec![Value::str(&message)])
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
