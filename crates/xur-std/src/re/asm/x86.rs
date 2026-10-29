//! `std::re::asm::x86`: x86/x64 disassembly and assembly, backed by
//! [iced-x86](https://docs.rs/iced-x86).
//!
//! ```text
//! let insns = std::re::asm::x86::decode(64, bytes, 0x1000);
//! for insn in insns { io::println("{}", insn); }
//! let bytes = std::re::asm::x86::encode(64, insns, 0x1000).unwrap();
//! ```

use std::collections::HashMap;
use std::sync::LazyLock;

use iced_x86::{
    Code, Decoder, DecoderOptions, Encoder, Formatter as IcedFormatter, GasFormatter, Instruction,
    IntelFormatter, MasmFormatter, MemoryOperand, NasmFormatter, Register as IcedRegister,
};
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::error_text;

static CODE_MAP: LazyLock<HashMap<String, Code>> =
    LazyLock::new(|| Code::values().map(|c| (format!("{c:?}"), c)).collect());

static REG_MAP: LazyLock<HashMap<String, IcedRegister>> = LazyLock::new(|| {
    IcedRegister::values()
        .map(|r| {
            let name = format!("{r:?}");
            (name.to_uppercase(), r)
        })
        .collect()
});

/// The wrapped instruction — `Clone` + `Facet` so it can live as a native
/// value. `Display` delegates to iced-x86's MASM formatter so it shows
/// as readable assembly when printed.
#[derive(Clone, facet::Facet)]
#[facet(opaque)]
struct Insn(Instruction);

impl std::fmt::Display for Insn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = String::new();
        MasmFormatter::new().format(&self.0, &mut output);
        f.write_str(&output)
    }
}

pub(super) fn register(_registry: &Registry, asm: &mut ModuleBuilder<'_>) {
    let mut x86 = asm.submodule("x86");

    x86.raw_function("decode", decode)
        .raw_function("encode", encode).returns::<Vec<u8>>()
        .raw_function("format", format_insn)
        .raw_function("insn", create_insn).returns::<Insn>()
        .raw_function("mem", create_mem).returns::<MemOp>();

    register_insn(&mut x86);
    register_enums(&mut x86);
}

fn register_enums(x86: &mut ModuleBuilder<'_>) {
    let codes: Vec<(&str, i64)> = CODE_MAP.iter().map(|(name, code)| {
        (name.as_str(), *code as i64)
    }).collect();
    x86.repr_enum("Code", &codes);

    let regs: Vec<(&str, i64)> = REG_MAP.iter().map(|(name, reg)| {
        (name.as_str(), *reg as i64)
    }).collect();
    x86.repr_enum("Register", &regs);
}

// ── decode ──────────────────────────────────────────────────────────

/// `x86::decode(bitness, bytes, ip?)` → list of `Instruction`.
///
/// `bitness` is 16, 32 or 64. `ip` defaults to 0.
fn decode(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (bitness, data, ip) = match args {
        [Value::Int(b), Value::Bytes(data)] => (*b, data, 0u64),
        [Value::Int(b), Value::Bytes(data), Value::Int(ip)] => (*b, data, *ip as u64),
        _ => {
            return Err(RuntimeError::new(
                "`decode` takes (bitness, bytes) or (bitness, bytes, ip)",
            ));
        }
    };
    let bitness = validate_bitness(bitness)?;
    let bytes = xur_runtime::lock(data);
    let decoder = Decoder::with_ip(bitness, &bytes, ip, DecoderOptions::NONE);
    let mut list = Vec::new();
    for insn in decoder {
        list.push(into_value(cx, Insn(insn))?);
    }
    Ok(Value::list(list))
}

// ── encode ──────────────────────────────────────────────────────────

/// `x86::encode(bitness, instructions, ip?)` → `Result<bytes, str>`.
///
/// Re-encodes the instructions at the given `ip` (default 0).
fn encode(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (bitness, list, ip) = match args {
        [Value::Int(b), Value::List(list)] => (*b, list, 0u64),
        [Value::Int(b), Value::List(list), Value::Int(ip)] => (*b, list, *ip as u64),
        _ => {
            return Err(RuntimeError::new(
                "`encode` takes (bitness, instructions) or (bitness, instructions, ip)",
            ));
        }
    };
    let bitness = validate_bitness(bitness)?;
    let items = xur_runtime::lock(list);
    let mut encoder = Encoder::new(bitness);
    let mut rip = ip;
    for value in items.iter() {
        let insn = extract_insn(value)?;
        match encoder.encode(&insn.0, rip) {
            Ok(len) => rip += len as u64,
            Err(e) => return Ok(crate::result(cx, Err(error_text(e)))),
        }
    }
    Ok(crate::result(cx, Ok(Value::bytes(encoder.take_buffer()))))
}

// ── format ──────────────────────────────────────────────────────────

/// `x86::format(instruction, syntax?)` → string.
///
/// `syntax` is `"intel"`, `"nasm"`, `"masm"` (default), or `"gas"`.
fn format_insn(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (value, syntax) = match args {
        [value] => (value, None),
        [value, Value::Str(s)] => (value, Some(s.clone())),
        _ => {
            return Err(RuntimeError::new(
                "`format` takes (instruction) or (instruction, syntax)",
            ));
        }
    };
    let insn = extract_insn(value)?;
    let _ = cx;
    let text = format_with(syntax.as_deref().unwrap_or("masm"), &insn.0)?;
    Ok(Value::str(&text))
}

fn format_with(syntax: &str, insn: &Instruction) -> RunResult<String> {
    let mut output = String::new();
    match syntax {
        "intel" => IntelFormatter::new().format(insn, &mut output),
        "nasm" => NasmFormatter::new().format(insn, &mut output),
        "masm" => MasmFormatter::new().format(insn, &mut output),
        "gas" | "att" => GasFormatter::new().format(insn, &mut output),
        _ => {
            return Err(RuntimeError::new(format!(
                "unknown syntax `{syntax}`; use \"intel\", \"nasm\", \"masm\" or \"gas\""
            )));
        }
    }
    Ok(output)
}

// ── Instruction type ────────────────────────────────────────────────

fn register_insn(x86: &mut ModuleBuilder<'_>) {
    x86.ty::<Insn>("Instruction")
        // ── address & size ──
        .method("ip", |insn: &Insn| insn.0.ip())
        .method("next_ip", |insn: &Insn| insn.0.next_ip())
        .method("len", |insn: &Insn| insn.0.len() as u64)
        .method("is_invalid", |insn: &Insn| insn.0.is_invalid())
        // ── opcode ──
        .method("mnemonic", |insn: &Insn| format!("{:?}", insn.0.mnemonic()))
        .method("code", |insn: &Insn| format!("{:?}", insn.0.code()))
        // ── operands ──
        .method("op_count", |insn: &Insn| insn.0.op_count())
        .method("op_kind", |insn: &Insn, n: u32| {
            format!("{:?}", insn.0.op_kind(n))
        })
        .method("op_register", |insn: &Insn, n: u32| {
            format!("{:?}", insn.0.op_register(n))
        })
        .method("immediate", |insn: &Insn, n: u32| insn.0.immediate(n))
        .method("near_branch_target", |insn: &Insn| {
            insn.0.near_branch_target()
        })
        // ── memory operand ──
        .method("memory_base", |insn: &Insn| {
            format!("{:?}", insn.0.memory_base())
        })
        .method("memory_index", |insn: &Insn| {
            format!("{:?}", insn.0.memory_index())
        })
        .method("memory_index_scale", |insn: &Insn| {
            insn.0.memory_index_scale()
        })
        .method("memory_displacement", |insn: &Insn| {
            insn.0.memory_displacement64()
        })
        .method("memory_segment", |insn: &Insn| {
            format!("{:?}", insn.0.memory_segment())
        })
        .method("memory_size", |insn: &Insn| {
            format!("{:?}", insn.0.memory_size())
        })
        .method("is_ip_rel_memory_operand", |insn: &Insn| {
            insn.0.is_ip_rel_memory_operand()
        })
        .method("ip_rel_memory_address", |insn: &Insn| {
            insn.0.ip_rel_memory_address()
        })
        // ── control flow ──
        .method("flow_control", |insn: &Insn| {
            format!("{:?}", insn.0.flow_control())
        })
        .method("is_privileged", |insn: &Insn| insn.0.is_privileged())
        .method("is_stack_instruction", |insn: &Insn| {
            insn.0.is_stack_instruction()
        })
        .method("stack_pointer_increment", |insn: &Insn| {
            insn.0.stack_pointer_increment()
        })
        // ── prefixes ──
        .method("has_lock_prefix", |insn: &Insn| insn.0.has_lock_prefix())
        .method("has_rep_prefix", |insn: &Insn| insn.0.has_rep_prefix())
        .method("has_repe_prefix", |insn: &Insn| insn.0.has_repe_prefix())
        .method("has_repne_prefix", |insn: &Insn| insn.0.has_repne_prefix())
        .method("segment_prefix", |insn: &Insn| {
            format!("{:?}", insn.0.segment_prefix())
        })
        // ── formatting ──
        .raw_function("format", |cx, args| {
            let (this, syntax) = match args {
                [this] => (this, None),
                [this, Value::Str(s)] => (this, Some(s.clone())),
                _ => {
                    return Err(RuntimeError::new(
                        "`format` takes an optional syntax string",
                    ));
                }
            };
            let insn = extract_insn(this)?;
            let _ = cx;
            let text = format_with(syntax.as_deref().unwrap_or("masm"), &insn.0)?;
            Ok(Value::str(&text))
        });
}

// ── instruction construction ────────────────────────────────────────

/// `x86::insn(code, ops...)` → `Result<Instruction, str>`.
///
/// Builds an instruction from a `Code` name and 0–4 operands.
/// Operands can be register names (strings like `"RAX"`), integers, or
/// memory operands built with `x86::mem(...)`.
fn create_insn(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (code, ops) = match args.split_first() {
        Some((Value::Str(name), rest)) => (parse_code(name)?, rest),
        Some((Value::Int(n), rest)) => (int_to_code(*n)?, rest),
        Some((Value::Adt(adt), rest)) => {
            let v = cx.repr_value(adt.ctor)
                .ok_or_else(|| RuntimeError::new("expected a Code enum value"))?;
            (int_to_code(v)?, rest)
        }
        _ => {
            return Err(RuntimeError::new(
                "`insn` takes a Code (name or enum) and 0–4 operands",
            ));
        }
    };
    let insn = match ops.len() {
        0 => Ok(Instruction::with(code)),
        1 => build1(cx, code, &ops[0]),
        2 => build2(cx, code, &ops[0], &ops[1]),
        3 => build3(cx, code, &ops[0], &ops[1], &ops[2]),
        _ => {
            return Err(RuntimeError::new(
                "`insn` supports at most 3 operands for now",
            ));
        }
    };
    match insn {
        Ok(i) => {
            let value = into_value(cx, Insn(i))?;
            Ok(crate::result(cx, Ok(value)))
        }
        Err(e) => Ok(crate::result(cx, Err(e.message))),
    }
}

/// A memory operand wrapper for passing through Xur.
#[derive(Clone, facet::Facet)]
#[facet(opaque)]
struct MemOp(MemoryOperand);

impl std::fmt::Display for MemOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let m = &self.0;
        write!(f, "[{:?}", m.base)?;
        if m.index != IcedRegister::None {
            write!(f, "+{:?}*{}", m.index, m.scale)?;
        }
        if m.displacement != 0 {
            write!(f, "+{:#x}", m.displacement)?;
        }
        write!(f, "]")
    }
}

/// `x86::mem(base)` or `x86::mem(base, index, scale, displacement)`.
fn create_mem(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let mem = match args {
        [Value::Str(base)] => MemoryOperand::with_base(parse_reg(base)?),
        [Value::Str(base), Value::Int(disp)] => {
            MemoryOperand::with_base_displ(parse_reg(base)?, *disp)
        }
        [
            Value::Str(base),
            Value::Str(index),
            Value::Int(scale),
            Value::Int(disp),
        ] => MemoryOperand::with_base_index_scale_displ_size(
            parse_reg(base)?,
            parse_reg(index)?,
            *scale as u32,
            *disp,
            1,
        ),
        _ => {
            return Err(RuntimeError::new(
                "`mem` takes (base), (base, disp), or (base, index, scale, disp)",
            ));
        }
    };
    into_value(cx, MemOp(mem))
}

/// An operand as seen by the instruction builder.
enum Operand {
    Reg(IcedRegister),
    Imm(i64),
    Mem(MemoryOperand),
}

fn parse_operand(cx: &dyn Context, value: &Value) -> RunResult<Operand> {
    match value {
        Value::Str(s) => Ok(Operand::Reg(parse_reg(s)?)),
        Value::Int(n) => Ok(Operand::Imm(*n)),
        Value::Adt(adt) => {
            let v = cx.repr_value(adt.ctor)
                .ok_or_else(|| RuntimeError::new("enum operand has no repr value"))?;
            Ok(Operand::Reg(int_to_reg(v)?))
        }
        Value::Native(obj) => {
            if let Some(m) = obj.with(|m: &MemOp| m.0) {
                return Ok(Operand::Mem(m));
            }
            Err(RuntimeError::new(
                "operand must be a register, integer, or memory operand",
            ))
        }
        _ => Err(RuntimeError::new(
            "operand must be a register, integer, or memory operand",
        )),
    }
}


fn build1(cx: &dyn Context, code: Code, op0: &Value) -> RunResult<Instruction> {
    match parse_operand(cx, op0)? {
        Operand::Reg(r) => Instruction::with1(code, r).map_err(iced_err),
        Operand::Imm(n) => Instruction::with1::<i32>(code, n as i32).map_err(iced_err),
        Operand::Mem(m) => Instruction::with1(code, m).map_err(iced_err),
    }
}

fn build2(cx: &dyn Context, code: Code, op0: &Value, op1: &Value) -> RunResult<Instruction> {
    let a = parse_operand(cx, op0)?;
    let b = parse_operand(cx, op1)?;
    match (a, b) {
        (Operand::Reg(r0), Operand::Reg(r1)) => Instruction::with2(code, r0, r1),
        (Operand::Reg(r), Operand::Imm(n)) => Instruction::with2::<_, i32>(code, r, n as i32),
        (Operand::Reg(r), Operand::Mem(m)) => Instruction::with2(code, r, m),
        (Operand::Mem(m), Operand::Reg(r)) => Instruction::with2(code, m, r),
        (Operand::Mem(m), Operand::Imm(n)) => Instruction::with2::<_, i32>(code, m, n as i32),
        (Operand::Imm(a), Operand::Imm(b)) => {
            Instruction::with2::<i32, i32>(code, a as i32, b as i32)
        }
        (Operand::Imm(n), Operand::Reg(r)) => Instruction::with2::<i32, _>(code, n as i32, r),
        _ => return Err(RuntimeError::new("unsupported operand combination")),
    }
    .map_err(iced_err)
}

fn build3(cx: &dyn Context, code: Code, op0: &Value, op1: &Value, op2: &Value) -> RunResult<Instruction> {
    let a = parse_operand(cx, op0)?;
    let b = parse_operand(cx, op1)?;
    let c = parse_operand(cx, op2)?;
    match (a, b, c) {
        (Operand::Reg(r0), Operand::Reg(r1), Operand::Reg(r2)) => {
            Instruction::with3(code, r0, r1, r2)
        }
        (Operand::Reg(r0), Operand::Reg(r1), Operand::Imm(n)) => {
            Instruction::with3::<_, _, i32>(code, r0, r1, n as i32)
        }
        (Operand::Reg(r0), Operand::Mem(m), Operand::Reg(r2)) => {
            Instruction::with3(code, r0, m, r2)
        }
        (Operand::Reg(r0), Operand::Mem(m), Operand::Imm(n)) => {
            Instruction::with3::<_, _, i32>(code, r0, m, n as i32)
        }
        (Operand::Mem(m), Operand::Reg(r1), Operand::Reg(r2)) => {
            Instruction::with3(code, m, r1, r2)
        }
        (Operand::Mem(m), Operand::Reg(r1), Operand::Imm(n)) => {
            Instruction::with3::<_, _, i32>(code, m, r1, n as i32)
        }
        _ => return Err(RuntimeError::new("unsupported 3-operand combination")),
    }
    .map_err(iced_err)
}

fn iced_err(e: iced_x86::IcedError) -> RuntimeError {
    RuntimeError::new(error_text(e))
}

fn parse_code(name: &str) -> RunResult<Code> {
    CODE_MAP
        .get(name)
        .copied()
        .ok_or_else(|| RuntimeError::new(format!("unknown Code `{name}`")))
}

fn int_to_code(n: i64) -> RunResult<Code> {
    Code::try_from(n as usize)
        .map_err(|_| RuntimeError::new(format!("invalid Code value `{n}`")))
}

fn int_to_reg(n: i64) -> RunResult<IcedRegister> {
    IcedRegister::try_from(n as usize)
        .map_err(|_| RuntimeError::new(format!("invalid Register value `{n}`")))
}

fn parse_reg(name: &str) -> RunResult<IcedRegister> {
    REG_MAP
        .get(&name.to_uppercase())
        .copied()
        .ok_or_else(|| RuntimeError::new(format!("unknown register `{name}`")))
}

// ── helpers ─────────────────────────────────────────────────────────

fn validate_bitness(b: i64) -> RunResult<u32> {
    match b {
        16 | 32 | 64 => Ok(b as u32),
        _ => Err(RuntimeError::new(format!(
            "bitness must be 16, 32 or 64, got {b}"
        ))),
    }
}

fn extract_insn(value: &Value) -> RunResult<Insn> {
    match value {
        Value::Native(obj) => obj
            .with(|insn: &Insn| insn.clone())
            .ok_or_else(|| RuntimeError::new("expected an x86 Instruction")),
        _ => Err(RuntimeError::new("expected an x86 Instruction")),
    }
}
