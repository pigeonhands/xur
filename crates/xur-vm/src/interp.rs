//! Executing bytecode.

use std::sync::{Arc, Mutex};

use xur_bytecode::{Constant, DefRef, Function, InstKind, RecordFields, Reg, Terminator, TypeArg};

use crate::records::RecordArgs;
use xur_resolve::{DefId, DefKind, ModuleIdx, Res};
use xur_runtime::{Closure, Range, Record, RunResult, RuntimeError, TypeRef, Value, ValueIter, lock};
use xur_span::Span;

use crate::Shared;

/// How deeply calls may nest before the program is stopped, rather than
/// overflowing the interpreter's own stack.
const MAX_DEPTH: usize = 10_000;

/// One thread's interpreter.
pub(crate) struct Interp<'s> {
    pub(crate) shared: &'s Arc<Shared>,
    /// The module of the function running, for module-relative imports.
    pub(crate) module: Option<ModuleIdx>,
    /// The type arguments of the native function being called.
    pub(crate) native_type_args: Vec<TypeRef>,
    depth: usize,
    /// The `?` locations an in-flight error has propagated through, with the
    /// call depth each fired at, innermost first. Only recorded while the
    /// program's tracing is on; read if an error reaches `main`.
    try_trail: Vec<(usize, Span)>,
    /// Registers from the last completed `run`, so `call_closure` can write
    /// back mutated captures.
    last_regs: Option<Vec<Value>>,
}

/// The registers of a running function.
struct Frame<'f> {
    function: &'f Function,
    regs: Vec<Value>,
    type_args: Vec<TypeRef>,
}

impl Frame<'_> {
    fn get(&self, reg: Reg) -> &Value {
        &self.regs[reg.0 as usize]
    }

    fn set(&mut self, reg: Reg, value: Value) {
        self.regs[reg.0 as usize] = value;
    }

    fn values(&self, regs: &[Reg]) -> Vec<Value> {
        regs.iter().map(|&reg| self.get(reg).clone()).collect()
    }
}

impl<'s> Interp<'s> {
    pub(crate) fn new(shared: &'s Arc<Shared>) -> Self {
        Self {
            shared,
            module: None,
            native_type_args: Vec::new(),
            depth: 0,
            try_trail: Vec::new(),
            last_regs: None,
        }
    }

    /// Records a `?` early-return at `span` in the current frame. A new
    /// propagation (one not continuing to unwind to a shallower frame)
    /// clears the trail first.
    fn record_try(&mut self, span: Span) {
        if !self.shared.tracing() {
            return;
        }
        if self
            .try_trail
            .last()
            .is_some_and(|&(depth, _)| self.depth >= depth)
        {
            self.try_trail.clear();
        }
        self.try_trail.push((self.depth, span));
    }

    /// The trail of `?` locations as human-readable notes, outermost last.
    pub(crate) fn try_trace(&self) -> Vec<String> {
        let program = self.shared.program();
        let source_map = &program.krate.source_map;
        self.try_trail
            .iter()
            .map(|&(_, span)| format!("propagated with `?` at {}", source_map.describe_pos(span)))
            .collect()
    }

    /// Calls the function `def` written in Xur.
    pub(crate) fn call_fn(
        &mut self,
        def: DefId,
        args: &[Value],
        type_args: Vec<TypeRef>,
    ) -> RunResult<Value> {
        let function = self.shared.code(def)?;
        self.run(&function, Vec::new(), args, type_args)
    }

    pub(crate) fn call_closure(&mut self, closure: &Closure, args: &[Value]) -> RunResult<Value> {
        let function = Arc::clone(&closure.code)
            .downcast::<Function>()
            .map_err(|_| RuntimeError::new("this closure was created by another engine"))?;
        let captures = lock(&closure.captures).clone();
        let num_captures = captures.len();
        let result = self.run(
            &function,
            captures,
            args,
            closure.type_args.clone(),
        );
        if let Some(frame_regs) = self.last_regs.take() {
            let mut caps = lock(&closure.captures);
            for (i, val) in frame_regs.into_iter().enumerate().take(num_captures) {
                caps[i] = val;
            }
        }
        result
    }

    /// The value of the constant `def`, computed on first use.
    pub(crate) fn constant(&mut self, def: DefId) -> RunResult<Value> {
        if let Some(value) = lock(&self.shared.consts).get(&def) {
            return Ok(value.clone());
        }
        let value = self.call_fn(def, &[], Vec::new())?;
        lock(&self.shared.consts).insert(def, value.clone());
        Ok(value)
    }

    /// Returns the function value for `def`, applying decorators if present.
    fn maybe_decorated(&mut self, def: DefId) -> RunResult<Value> {
        if let Some(value) = lock(&self.shared.decorated).get(&def) {
            return Ok(value.clone());
        }
        let function = self.shared.code(def)?;
        if function.decorators.is_empty() {
            return Ok(Value::Fn(def));
        }
        let mut value = Value::Fn(def);
        for decorator in &function.decorators {
            let decorator_value = match decorator.def {
                DefRef::Fn(d) => self.maybe_decorated(d)?,
                DefRef::Const(d) => self.constant(d)?,
                DefRef::Native(id) => Value::NativeFn(id),
                _ => return Err(RuntimeError::new("decorator must be a function")),
            };
            let args: Vec<Value> = decorator.args.iter().map(constant_to_value).collect();
            if args.is_empty() {
                value = self.call_value(&decorator_value, &[value], Vec::new(), function.span)?;
            } else {
                let factory = self.call_value(&decorator_value, &args, Vec::new(), function.span)?;
                value = self.call_value(&factory, &[value], Vec::new(), function.span)?;
            }
        }
        lock(&self.shared.decorated).insert(def, value.clone());
        Ok(value)
    }

    fn run(
        &mut self,
        function: &Function,
        captures: Vec<Value>,
        args: &[Value],
        mut type_args: Vec<TypeRef>,
    ) -> RunResult<Value> {
        if args.len() != function.params as usize {
            return Err(RuntimeError::new(format!(
                "`{}` takes {} argument(s) but {} were supplied",
                function.name,
                function.params,
                args.len()
            )));
        }
        if !function.type_params.is_empty() {
            if type_args.len() < function.type_params.len() {
                self.infer_type_args(function, args, &mut type_args);
            }
            for (i, required) in function.type_param_required.iter().enumerate() {
                if *required {
                    let is_concrete = type_args
                        .get(i)
                        .is_some_and(|t| t.def.is_some());
                    if !is_concrete {
                        return Err(RuntimeError::new(format!(
                            "`{}` requires an explicit type for `{}`",
                            function.name, function.type_params[i],
                        )));
                    }
                }
            }
            let mut owned = args.to_vec();
            self.check_generic_args(function, &mut owned, &type_args)?;
            if self.depth >= MAX_DEPTH {
                return Err(RuntimeError::new(format!(
                    "calls nested more than {MAX_DEPTH} deep; is there infinite recursion?"
                )));
            }
            self.depth += 1;
            let caller_module = self.module.replace(function.module);

            let mut regs = vec![Value::Unit; function.registers as usize];
            for (reg, value) in regs.iter_mut().zip(captures.into_iter().chain(owned)) {
                *reg = value;
            }
            let mut frame = Frame {
                function,
                regs,
                type_args,
            };
            let result = self.run_frame(&mut frame);

            self.last_regs = Some(frame.regs);
            self.module = caller_module;
            self.depth -= 1;
            return result;
        }
        if self.depth >= MAX_DEPTH {
            return Err(RuntimeError::new(format!(
                "calls nested more than {MAX_DEPTH} deep; is there infinite recursion?"
            )));
        }
        self.depth += 1;
        let caller_module = self.module.replace(function.module);

        let mut regs = vec![Value::Unit; function.registers as usize];
        for (reg, value) in regs.iter_mut().zip(captures.into_iter().chain(args.iter().cloned())) {
            *reg = value;
        }
        let mut frame = Frame {
            function,
            regs,
            type_args,
        };
        let result = self.run_frame(&mut frame);

        self.last_regs = Some(frame.regs);
        self.module = caller_module;
        self.depth -= 1;
        result
    }

    fn infer_type_args(
        &self,
        function: &Function,
        args: &[Value],
        type_args: &mut Vec<TypeRef>,
    ) {
        type_args.resize(function.type_params.len(), TypeRef::named("_"));
        for (i, arg) in args.iter().enumerate() {
            if let Some(tp_index) = function.param_type_params.get(i).copied().flatten() {
                let idx = tp_index as usize;
                if idx < type_args.len()
                    && type_args[idx].def.is_none()
                    && !type_args[idx].is_record()
                    && let Some(tr) = self.type_ref_of(arg)
                {
                    type_args[idx] = tr;
                }
            }
        }
    }

    fn check_generic_args(
        &self,
        function: &Function,
        args: &mut [Value],
        type_args: &[TypeRef],
    ) -> RunResult<()> {
        for (i, arg) in args.iter_mut().enumerate() {
            let Some(tp_index) = function.param_type_params.get(i).copied().flatten() else {
                continue;
            };
            let idx = tp_index as usize;
            let Some(expected) = type_args.get(idx) else { continue };
            if expected.is_record() {
                *arg = self.value_to_record(arg)?;
                continue;
            }
            let Some(expected_def) = expected.def else { continue };
            let program = self.shared.program();
            let name = program.resolutions().def_path(expected_def);
            let ty = xur_bytecode::RtType::Adt { def: expected_def, name };
            match self.check_type(arg, &ty) {
                Ok(Some(coerced)) => *arg = coerced,
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn value_to_record(&self, value: &Value) -> RunResult<Value> {
        match value {
            Value::Record(_) => Ok(value.clone()),
            Value::Adt(adt) => {
                let names = self.shared.field_names(adt.ctor).unwrap_or_default();
                let fields: Vec<_> = names
                    .into_iter()
                    .zip(adt.fields.iter().cloned())
                    .collect();
                Ok(Value::Record(Arc::new(Record::new(fields))))
            }
            _ => Err(RuntimeError::new(format!(
                "cannot coerce {} to a record",
                self.describe_type(value),
            ))),
        }
    }

    fn type_ref_of(&self, value: &Value) -> Option<TypeRef> {
        match value {
            Value::Adt(adt) => {
                let program = self.shared.program();
                let defs = &program.resolutions().defs;
                let def_data = defs.get(adt.ctor);
                let (name, def) = match def_data.kind {
                    DefKind::Variant => {
                        let parent = def_data.parent?;
                        (defs.get(parent).name, parent)
                    }
                    _ => (def_data.name, adt.ctor),
                };
                Some(TypeRef {
                    name: name.to_string(),
                    def: Some(def),
                })
            }
            Value::Native(object) => {
                let name = self.shared.registry.type_name(object.ty);
                Some(TypeRef {
                    name: name.to_owned(),
                    def: None,
                })
            }
            _ => None,
        }
    }


    fn run_frame(&mut self, frame: &mut Frame<'_>) -> RunResult<Value> {
        let function = frame.function;
        let mut block = &function.blocks[0];
        loop {
            for inst in &block.insts {
                self.exec(frame, &inst.kind, inst.span)
                    .map_err(|error| error.at(inst.span))?;
            }
            let span = block.term_span;
            let next = match &block.term {
                Terminator::Goto(target) => *target,
                Terminator::Branch {
                    cond,
                    then,
                    otherwise,
                } => match frame.get(*cond) {
                    Value::Bool(true) => *then,
                    Value::Bool(false) => *otherwise,
                    other => {
                        let message = format!("expected a bool condition, found {}", other.kind());
                        return Err(RuntimeError::new(message).at(span));
                    }
                },
                Terminator::Return(value) => return Ok(frame.get(*value).clone()),
                Terminator::TryReturn(value) => {
                    self.record_try(span);
                    return Ok(frame.get(*value).clone());
                }
                Terminator::Next {
                    iter,
                    item,
                    body,
                    done,
                } => {
                    let Value::Iterator(iter) = frame.get(*iter) else {
                        unreachable!("`next` is only used on iterators")
                    };
                    match iter.next() {
                        Some(value) => {
                            frame.set(*item, value);
                            *body
                        }
                        None => *done,
                    }
                }
                Terminator::NoMatch => {
                    return Err(RuntimeError::new("no pattern matched the value").at(span));
                }
                Terminator::Unreachable => {
                    return Err(
                        RuntimeError::new("reached code that should be unreachable").at(span)
                    );
                }
            };
            block = &function.blocks[next.0 as usize];
        }
    }

    fn exec(&mut self, frame: &mut Frame<'_>, inst: &InstKind, span: Span) -> RunResult<()> {
        match inst {
            InstKind::Const { dst, value } => {
                let value = match value {
                    Constant::Unit => Value::Unit,
                    Constant::Bool(b) => Value::Bool(*b),
                    Constant::Int(i) => Value::Int(*i),
                    Constant::Float(f) => Value::Float(*f),
                    Constant::Str(s) => Value::Str(Arc::clone(s)),
                    // Each evaluation makes a new buffer, so changing one
                    // doesn't change the literal.
                    Constant::Bytes(bytes) => Value::bytes(bytes.to_vec()),
                };
                frame.set(*dst, value);
            }
            InstKind::Copy { dst, src } => {
                let value = frame.get(*src).clone();
                frame.set(*dst, value);
            }
            InstKind::Def { dst, def } => {
                let value = match *def {
                    DefRef::Fn(def) => self.maybe_decorated(def)?,
                    DefRef::Ctor(def) => Value::Ctor(def),
                    DefRef::Const(def) => self.constant(def)?,
                    DefRef::Module(def) => Value::Module(def),
                    DefRef::Native(id) => Value::NativeFn(id),
                };
                frame.set(*dst, value);
            }
            InstKind::Unary { dst, op, src } => {
                let value = crate::ops::unary(*op, frame.get(*src))?;
                frame.set(*dst, value);
            }
            InstKind::Binary { dst, op, lhs, rhs } => {
                let value = self.binary(*op, frame.get(*lhs), frame.get(*rhs))?;
                frame.set(*dst, value);
            }
            InstKind::Call {
                dst,
                callee,
                args,
                type_args,
            } => {
                let callee = frame.get(*callee).clone();
                let args = frame.values(args);
                let type_args = type_args
                    .iter()
                    .map(|arg| match arg {
                        TypeArg::Named { name, def } => TypeRef {
                            name: name.clone(),
                            def: *def,
                        },
                        TypeArg::Param(index) => frame
                            .type_args
                            .get(*index as usize)
                            .cloned()
                            .unwrap_or_else(|| TypeRef::named("_")),
                        TypeArg::Record => TypeRef::record(),
                    })
                    .collect();
                let value = self.call_value(&callee, &args, type_args, span)?;
                frame.set(*dst, value);
            }
            InstKind::TyParamCall {
                dst,
                type_param,
                method,
                args,
            } => {
                let type_arg = frame.type_args.get(*type_param as usize);
                let args = frame.values(args);
                let value = if type_arg.is_some_and(|t| t.is_record()) {
                    if method.as_str() == "new" {
                        Value::Record(Arc::new(Record::new(Vec::new())))
                    } else {
                        return Err(RuntimeError::new(format!(
                            "cannot call `{method}` on `@` (record mode only supports `new`)",
                        )));
                    }
                } else {
                    match type_arg.and_then(|t| t.def) {
                        Some(ty_def) => {
                            let program = self.shared.program();
                            let resolutions = program.resolutions();
                            let callee = match resolutions.assoc_fn(ty_def, *method) {
                                Some(Res::Def(DefKind::Fn, def)) => Value::Fn(def),
                                Some(Res::Def(DefKind::NativeFn(id), _)) => Value::NativeFn(id),
                                _ => {
                                    let ty_name = resolutions.def_path(ty_def);
                                    return Err(RuntimeError::new(format!(
                                        "no function `{method}` on `{ty_name}`",
                                    )));
                                }
                            };
                            self.call_value(&callee, &args, Vec::new(), span)?
                        }
                        None => {
                            return Err(RuntimeError::new(format!(
                                "cannot call `{method}` on an unresolved type parameter",
                            )));
                        }
                    }
                };
                frame.set(*dst, value);
            }
            InstKind::MethodCall {
                dst,
                receiver,
                method,
                args,
            } => {
                let receiver = frame.get(*receiver).clone();
                let args = frame.values(args);
                let value = self.call_method(receiver, *method, &args, span)?;
                frame.set(*dst, value);
            }
            InstKind::Field { dst, base, field } => {
                let value = self.field(frame.get(*base), *field)?;
                frame.set(*dst, value);
            }
            InstKind::SetField {
                dst,
                base,
                field,
                value,
            } => {
                let value = self.set_field(frame.get(*base), *field, frame.get(*value).clone())?;
                frame.set(*dst, value);
            }
            InstKind::Index { dst, base, index } => {
                let value = crate::ops::index(frame.get(*base), frame.get(*index))?;
                frame.set(*dst, value);
            }
            InstKind::SetIndex { base, index, value } => {
                crate::ops::set_index(
                    frame.get(*base),
                    frame.get(*index),
                    frame.get(*value).clone(),
                )?;
            }
            InstKind::Tuple { dst, items } => {
                let value = Value::tuple(frame.values(items));
                frame.set(*dst, value);
            }
            InstKind::List { dst, items } => {
                let value = Value::list(frame.values(items));
                frame.set(*dst, value);
            }
            InstKind::Map { dst, entries } => {
                let regs = frame.values(entries);
                let pairs: Vec<(Value, Value)> = regs
                    .chunks(2)
                    .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
                    .collect();
                frame.set(*dst, Value::map(pairs));
            }
            InstKind::Adt { dst, ctor, fields } => {
                let value = Value::adt(*ctor, frame.values(fields));
                frame.set(*dst, value);
            }
            InstKind::Range {
                dst,
                start,
                end,
                inclusive,
            } => {
                let bound = |reg: &Option<Reg>| -> RunResult<Option<i64>> {
                    match reg.map(|reg| frame.get(reg)) {
                        None => Ok(None),
                        Some(Value::Int(i)) => Ok(Some(*i)),
                        Some(other) => Err(RuntimeError::new(format!(
                            "range bounds must be integers, found {}",
                            other.kind()
                        ))),
                    }
                };
                let range = Range {
                    start: bound(start)?,
                    end: bound(end)?,
                    inclusive: *inclusive,
                };
                frame.set(*dst, Value::Range(range));
            }
            InstKind::Closure {
                dst,
                function,
                captures,
            } => {
                let code = Arc::clone(&frame.function.closures[*function as usize]);
                let closure = Closure {
                    code,
                    captures: Mutex::new(frame.values(captures)),
                    type_args: frame.type_args.clone(),
                };
                frame.set(*dst, Value::Closure(Arc::new(closure)));
            }
            InstKind::IsCtor { dst, value, ctor } => {
                let is = matches!(frame.get(*value), Value::Adt(adt) if adt.ctor == *ctor);
                frame.set(*dst, Value::Bool(is));
            }
            InstKind::RecordLit { dst, fields } => {
                let fields: Vec<_> = fields
                    .iter()
                    .map(|(name, reg)| (*name, frame.get(*reg).clone()))
                    .collect();
                frame.set(*dst, Value::Record(Arc::new(Record::new(fields))));
            }
            InstKind::Record {
                dst,
                ty,
                variant,
                fields,
            } => {
                let fields = match fields {
                    RecordFields::Positional(regs) => RecordArgs::Positional(frame.values(regs)),
                    RecordFields::Named(fields) => RecordArgs::Named(
                        fields
                            .iter()
                            .map(|(name, reg)| (*name, frame.get(*reg).clone()))
                            .collect(),
                    ),
                };
                let record = self.build_record(*ty, *variant, fields)?;
                frame.set(*dst, record);
            }
            InstKind::IsRecord {
                dst,
                value,
                ty,
                variant,
            } => {
                let is = matches!(
                    frame.get(*value),
                    Value::Record(record)
                        if record.ty == Some(*ty) && (variant.is_none() || record.variant == *variant)
                );
                frame.set(*dst, Value::Bool(is));
            }
            InstKind::RecordArity { dst, value, len } => {
                let is = matches!(
                    frame.get(*value),
                    Value::Record(record) if record.fields.len() == *len as usize
                );
                frame.set(*dst, Value::Bool(is));
            }
            InstKind::IsTuple { dst, value, len } => {
                let is = match (frame.get(*value), *len) {
                    (Value::Unit, 0) => true,
                    (Value::Tuple(items), n) => items.len() == n as usize,
                    _ => false,
                };
                frame.set(*dst, Value::Bool(is));
            }
            InstKind::IsList {
                dst,
                value,
                len,
                at_least,
            } => {
                let is = match frame.get(*value) {
                    Value::List(list) => {
                        let list = list.lock().unwrap();
                        if *at_least {
                            list.len() >= *len as usize
                        } else {
                            list.len() == *len as usize
                        }
                    }
                    _ => false,
                };
                frame.set(*dst, Value::Bool(is));
            }
            InstKind::Project { dst, value, index } => {
                let index = *index as usize;
                let element = match frame.get(*value) {
                    Value::Tuple(items) => items.get(index).cloned(),
                    Value::Adt(adt) => adt.fields.get(index).cloned(),
                    Value::List(list) => list.lock().unwrap().get(index).cloned(),
                    _ => None,
                };
                let element =
                    element.ok_or_else(|| RuntimeError::new(format!("no field {index}")))?;
                frame.set(*dst, element);
            }
            InstKind::CheckType { value, ty, coerce } => {
                let result = if *coerce {
                    self.check_type(frame.get(*value), ty)?
                } else {
                    self.strict_check_type(frame.get(*value), ty)?
                };
                if let Some(coerced) = result {
                    frame.set(*value, coerced);
                }
            }
            InstKind::IterStart { dst, iterable } => {
                let iter = iterate(frame.get(*iterable))?;
                frame.set(*dst, Value::Iterator(iter));
            }
            InstKind::TypeLiteral { dst, ty } => {
                frame.set(*dst, Value::Type(Box::new(ty.clone())));
            }
            InstKind::Error { message } => return Err(RuntimeError::new(message.clone())),
        }
        Ok(())
    }
}

/// An iterator over a list (a snapshot of it), a range or a string's
/// characters.
fn iterate(value: &Value) -> RunResult<Arc<ValueIter>> {
    let iter = match value {
        Value::Iterator(iter) => return Ok(Arc::clone(iter)),
        Value::List(items) => ValueIter::new(lock(items).clone().into_iter()),
        Value::Bytes(bytes) => {
            let bytes = lock(bytes).clone();
            ValueIter::new(bytes.into_iter().map(|byte| Value::Int(byte.into())))
        }
        Value::Tuple(items) => {
            ValueIter::new(items.iter().cloned().collect::<Vec<_>>().into_iter())
        }
        Value::Str(s) => {
            let chars: Vec<_> = s
                .chars()
                .map(|c| Value::str(c.encode_utf8(&mut [0; 4])))
                .collect();
            ValueIter::new(chars.into_iter())
        }
        Value::Range(Range {
            start,
            end,
            inclusive,
        }) => {
            let start =
                start.ok_or_else(|| RuntimeError::new("cannot iterate a range without a start"))?;
            match (end, inclusive) {
                (Some(end), false) => ValueIter::new((start..*end).map(Value::Int)),
                (Some(end), true) => ValueIter::new((start..=*end).map(Value::Int)),
                (None, _) => ValueIter::new((start..).map(Value::Int)),
            }
        }
        other => {
            return Err(RuntimeError::new(format!(
                "{} is not iterable",
                other.kind()
            )));
        }
    };
    Ok(Arc::new(iter))
}

fn constant_to_value(c: &Constant) -> Value {
    match c {
        Constant::Unit => Value::Unit,
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Int(i) => Value::Int(*i),
        Constant::Float(f) => Value::Float(*f),
        Constant::Str(s) => Value::Str(Arc::clone(s)),
        Constant::Bytes(b) => Value::bytes(b.to_vec()),
    }
}
