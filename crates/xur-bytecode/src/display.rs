//! A readable listing of bytecode, for `xur debug bytecode` and tests.

use std::fmt::Write;

use xur_resolve::Resolutions;

use crate::ir::{Constant, DefRef, Function, InstKind, RecordFields, Reg, Terminator, TypeArg};

/// Lists `function` and the closures it creates.
pub fn display_function(function: &Function, resolutions: &Resolutions) -> String {
    let mut out = String::new();
    write_function(&mut out, function, resolutions, &function.name);
    out
}

fn write_function(out: &mut String, function: &Function, resolutions: &Resolutions, name: &str) {
    let _ = writeln!(
        out,
        "fn {name} (captures: {}, params: {}, registers: {})",
        function.captures, function.params, function.registers
    );
    for (i, block) in function.blocks.iter().enumerate() {
        let _ = writeln!(out, "  bb{i}:");
        for inst in &block.insts {
            let _ = writeln!(out, "    {}", inst_text(&inst.kind, resolutions));
        }
        let _ = writeln!(out, "    {}", term_text(&block.term));
    }
    for (i, closure) in function.closures.iter().enumerate() {
        let _ = writeln!(out);
        write_function(out, closure, resolutions, &format!("{name}::closure#{i}"));
    }
}

fn regs(regs: &[Reg]) -> String {
    let regs: Vec<_> = regs.iter().map(|reg| format!("{reg:?}")).collect();
    regs.join(", ")
}

fn inst_text(kind: &InstKind, resolutions: &Resolutions) -> String {
    let path = |def| resolutions.def_path(def);
    match kind {
        InstKind::Const { dst, value } => format!("{dst:?} = {}", constant_text(value)),
        InstKind::Copy { dst, src } => format!("{dst:?} = {src:?}"),
        InstKind::Def { dst, def } => {
            let def = match *def {
                DefRef::Fn(def) => format!("fn {}", path(def)),
                DefRef::Ctor(def) => format!("ctor {}", path(def)),
                DefRef::Const(def) => format!("const {}", path(def)),
                DefRef::Module(def) => format!("module {}", path(def)),
                DefRef::Native(id) => format!("native #{}", id.0),
            };
            format!("{dst:?} = {def}")
        }
        InstKind::Unary { dst, op, src } => format!("{dst:?} = {}{src:?}", op.as_str()),
        InstKind::Binary { dst, op, lhs, rhs } => format!("{dst:?} = {lhs:?} {op} {rhs:?}"),
        InstKind::Call {
            dst,
            callee,
            args,
            type_args,
        } => {
            let type_args = if type_args.is_empty() {
                String::new()
            } else {
                let args: Vec<_> = type_args
                    .iter()
                    .map(|arg| match arg {
                        TypeArg::Named { name, .. } => name.clone(),
                        TypeArg::Param(i) => format!("T{i}"),
                        TypeArg::Record => "@".to_owned(),
                    })
                    .collect();
                format!("::<{}>", args.join(", "))
            };
            format!("{dst:?} = call {callee:?}{type_args}({})", regs(args))
        }
        InstKind::MethodCall {
            dst,
            receiver,
            method,
            args,
        } => {
            format!("{dst:?} = {receiver:?}.{method}({})", regs(args))
        }
        InstKind::Field { dst, base, field } => format!("{dst:?} = {base:?}.{field}"),
        InstKind::SetField {
            dst,
            base,
            field,
            value,
        } => format!("{dst:?} = {base:?} with .{field} = {value:?}"),
        InstKind::Index { dst, base, index } => format!("{dst:?} = {base:?}[{index:?}]"),
        InstKind::SetIndex { base, index, value } => format!("{base:?}[{index:?}] = {value:?}"),
        InstKind::Tuple { dst, items } => format!("{dst:?} = ({})", regs(items)),
        InstKind::List { dst, items } => format!("{dst:?} = [{}]", regs(items)),
        InstKind::Adt { dst, ctor, fields } => {
            format!("{dst:?} = {}({})", path(*ctor), regs(fields))
        }
        InstKind::Range {
            dst,
            start,
            end,
            inclusive,
        } => {
            let bound = |reg: &Option<Reg>| reg.map(|reg| format!("{reg:?}")).unwrap_or_default();
            let op = if *inclusive { "..=" } else { ".." };
            format!("{dst:?} = {}{op}{}", bound(start), bound(end))
        }
        InstKind::Closure {
            dst,
            function,
            captures,
        } => {
            format!("{dst:?} = closure#{function}({})", regs(captures))
        }
        InstKind::IsCtor { dst, value, ctor } => format!("{dst:?} = {value:?} is {}", path(*ctor)),
        InstKind::Record {
            dst,
            ty,
            variant,
            fields,
        } => {
            let fields = match fields {
                RecordFields::Positional(regs) => format!("({})", self::regs(regs)),
                RecordFields::Named(fields) => {
                    let fields: Vec<_> = fields
                        .iter()
                        .map(|(name, reg)| format!("{name}: {reg:?}"))
                        .collect();
                    format!(" {{ {} }}", fields.join(", "))
                }
            };
            format!("{dst:?} = record {}{fields}", record_name(*ty, *variant))
        }
        InstKind::IsRecord {
            dst,
            value,
            ty,
            variant,
        } => format!("{dst:?} = {value:?} is {}", record_name(*ty, *variant)),
        InstKind::RecordArity { dst, value, len } => {
            format!("{dst:?} = {value:?} is record of {len}")
        }
        InstKind::IsTuple { dst, value, len } => format!("{dst:?} = {value:?} is tuple of {len}"),
        InstKind::Project { dst, value, index } => format!("{dst:?} = {value:?}.{index}"),
        InstKind::IterStart { dst, iterable } => format!("{dst:?} = iter {iterable:?}"),
        InstKind::CheckType { value, ty, coerce } => {
            let op = if *coerce { "coerce" } else { "check" };
            format!("{op} {value:?}: {ty}")
        }
        InstKind::TyParamCall { dst, type_param, method, args } => {
            let args: Vec<_> = args.iter().map(|r| format!("{r:?}")).collect();
            format!("{dst:?} = T{type_param}::{method}({})", args.join(", "))
        }
        InstKind::Error { message } => format!("error {message:?}"),
    }
}

fn constant_text(value: &Constant) -> String {
    match value {
        Constant::Unit => "()".to_owned(),
        Constant::Bool(b) => b.to_string(),
        Constant::Int(i) => i.to_string(),
        Constant::Float(f) => format!("{f:?}"),
        Constant::Str(s) => format!("{s:?}"),
        Constant::Bytes(bytes) => format!("b\"{}\"", bytes.escape_ascii()),
    }
}

fn term_text(term: &Terminator) -> String {
    match term {
        Terminator::Goto(target) => format!("goto {target:?}"),
        Terminator::Branch {
            cond,
            then,
            otherwise,
        } => {
            format!("branch {cond:?} ? {then:?} : {otherwise:?}")
        }
        Terminator::Return(value) => format!("return {value:?}"),
        Terminator::TryReturn(value) => format!("return? {value:?}"),
        Terminator::Next {
            iter,
            item,
            body,
            done,
        } => {
            format!("{item:?} = next {iter:?} ? {body:?} : {done:?}")
        }
        Terminator::NoMatch => "no match".to_owned(),
        Terminator::Unreachable => "unreachable".to_owned(),
    }
}

/// A native record type, as `native#3` or `native#3::Variant`.
fn record_name(ty: xur_resolve::NativeTypeId, variant: Option<xur_span::Symbol>) -> String {
    match variant {
        Some(variant) => format!("{ty:?}::{variant}"),
        None => format!("{ty:?}"),
    }
}
