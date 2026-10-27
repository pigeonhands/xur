//! Lowering resolved syntax to bytecode, one function at a time.
//!
//! Functions are lowered on demand, the first time they are called, so a
//! program only pays for the code it runs and modules imported at runtime
//! need no up-front work.

mod expr;
mod pat;

use std::collections::HashMap;
use std::sync::Arc;

use xur_ast::{Block as AstBlock, Expr, FnRetTy, NodeId, Param, Ty, TyKind};
use xur_resolve::{DefId, DefKind, LangItems, ModuleIdx, PrimTy, Program, Res, Resolutions};
use xur_span::{Span, Symbol};

use crate::ir::{Block, BlockId, Constant, Function, Inst, InstKind, Reg, RtType, Terminator};
use crate::items::{ItemIndex, ItemRef};

/// Lowers the function or constant `def`, or returns `None` if `def` is
/// neither. A constant lowers to a function without parameters that
/// computes its value.
pub fn lower(program: &Program, items: &ItemIndex, def: DefId) -> Option<Function> {
    let resolutions = program.resolutions();
    let name = resolutions.def_path(def);
    match items.get(def)? {
        ItemRef::Fn { item, module } => {
            let mut lowerer = Lowerer::new(program, items, module);
            for param in &item.generics.params {
                lowerer.type_params.push(resolutions.node_defs[&param.id]);
                lowerer.type_param_names.push(param.ident.name);
            }
            if let FnRetTy::Ty(ty) = &item.decl.output {
                lowerer.ret_ty = lowerer.rt_type(ty);
            }
            lowerer.params(&item.decl.inputs);
            let value = lowerer.block(&item.body);
            lowerer.ret(value, item.body.span);
            let params = item.decl.inputs.len() as u32;
            Some(lowerer.finish(name, 0, params, item.body.span))
        }
        ItemRef::Const { item, module } => {
            let mut lowerer = Lowerer::new(program, items, module);
            let value = lowerer.expr(&item.expr);
            lowerer.terminate(Terminator::Return(value), item.expr.span);
            Some(lowerer.finish(name, 0, 0, item.expr.span))
        }
        ItemRef::Ctor(_) => None,
    }
}

/// A block under construction.
struct BlockData {
    insts: Vec<Inst>,
    term: Option<(Terminator, Span)>,
}

/// Where `break` and `continue` go in the innermost loop.
struct LoopScope {
    break_to: BlockId,
    continue_to: BlockId,
    /// Where `break value` puts the loop's value.
    result: Reg,
}

struct Lowerer<'a> {
    program: &'a Program,
    resolutions: &'a Resolutions,
    items: &'a ItemIndex,
    lang: LangItems,
    module: ModuleIdx,
    blocks: Vec<BlockData>,
    current: BlockId,
    registers: u32,
    /// The register of each local variable, by its binding pattern.
    locals: HashMap<NodeId, Reg>,
    loops: Vec<LoopScope>,
    /// The definitions of the function's type parameters, in order.
    type_params: Vec<DefId>,
    type_param_names: Vec<Symbol>,
    closures: Vec<Arc<Function>>,
    /// The declared return type, checked on return.
    ret_ty: Option<RtType>,
}

impl<'a> Lowerer<'a> {
    fn new(program: &'a Program, items: &'a ItemIndex, module: ModuleIdx) -> Self {
        let mut lowerer = Self {
            program,
            resolutions: program.resolutions(),
            items,
            lang: program.lang_items(),
            module,
            blocks: Vec::new(),
            current: BlockId(0),
            registers: 0,
            locals: HashMap::new(),
            loops: Vec::new(),
            type_params: Vec::new(),
            type_param_names: Vec::new(),
            closures: Vec::new(),
            ret_ty: None,
        };
        lowerer.current = lowerer.new_block();
        lowerer
    }

    fn finish(self, name: String, captures: u32, params: u32, span: Span) -> Function {
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| {
                let (term, term_span) = block.term.unwrap_or((Terminator::Unreachable, span));
                Block {
                    insts: block.insts,
                    term,
                    term_span,
                }
            })
            .collect();
        Function {
            name,
            module: self.module,
            captures,
            params,
            registers: self.registers,
            type_params: self.type_param_names,
            blocks,
            closures: self.closures,
            span,
        }
    }

    /// Binds the parameters, which arrive in the registers after any
    /// captures, checking those whose type is written.
    fn params(&mut self, params: &[Param]) {
        let registers: Vec<_> = params.iter().map(|_| self.new_reg()).collect();
        for (param, reg) in params.iter().zip(registers) {
            self.check_type(reg, &param.ty, param.span);
            self.bind_irrefutable(&param.pat, reg);
        }
    }

    /// Returns `value`, checking it against the declared return type.
    fn ret(&mut self, value: Reg, span: Span) {
        if let Some(ty) = self.ret_ty.clone() {
            self.emit(InstKind::CheckType { value, ty }, span);
        }
        self.terminate(Terminator::Return(value), span);
    }

    /// Checks at runtime that `value` has the type `ty`, if `ty` is written
    /// and concrete.
    fn check_type(&mut self, value: Reg, ty: &Ty, span: Span) {
        if let Some(ty) = self.rt_type(ty) {
            self.emit(InstKind::CheckType { value, ty }, span);
        }
    }

    /// The runtime check for a written type, or `None` if nothing about
    /// it can be checked.
    fn rt_type(&self, ty: &Ty) -> Option<RtType> {
        match self.rt_type_or_any(ty) {
            RtType::Any => None,
            ty => Some(ty),
        }
    }

    fn rt_type_or_any(&self, ty: &Ty) -> RtType {
        match &ty.kind {
            TyKind::Path(path) => match self.resolutions.res.get(&path.id) {
                Some(Res::PrimTy(prim)) => prim_rt_type(*prim),
                Some(&Res::Def(DefKind::Struct | DefKind::Enum, def)) => RtType::Adt {
                    def,
                    name: self.resolutions.def_path(def),
                },
                Some(&Res::Def(DefKind::NativeType(id), def)) => RtType::Native {
                    id,
                    name: self.resolutions.def_path(def),
                },
                _ => RtType::Any,
            },
            TyKind::Tuple(tys) if tys.is_empty() => RtType::Unit,
            TyKind::Tuple(tys) => {
                RtType::Tuple(tys.iter().map(|ty| self.rt_type_or_any(ty)).collect())
            }
            TyKind::Paren(ty) => self.rt_type_or_any(ty),
            TyKind::List(_) => RtType::List,
            TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => RtType::Any,
        }
    }

    fn new_reg(&mut self) -> Reg {
        let reg = Reg(self.registers);
        self.registers += 1;
        reg
    }

    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BlockData {
            insts: Vec::new(),
            term: None,
        });
        id
    }

    fn switch_to(&mut self, block: BlockId) {
        self.current = block;
    }

    fn emit(&mut self, kind: InstKind, span: Span) {
        self.blocks[self.current.0 as usize]
            .insts
            .push(Inst { kind, span });
    }

    /// Ends the current block. Code lowered after a `return`, `break` or
    /// `continue` goes into a fresh block that nothing jumps to.
    fn terminate(&mut self, term: Terminator, span: Span) {
        let block = &mut self.blocks[self.current.0 as usize];
        if block.term.is_none() {
            block.term = Some((term, span));
        }
    }

    /// Ends the current block and continues in a new, unreachable one.
    fn diverge(&mut self, term: Terminator, span: Span) {
        self.terminate(term, span);
        let dead = self.new_block();
        self.switch_to(dead);
    }

    fn constant(&mut self, value: Constant, span: Span) -> Reg {
        let dst = self.new_reg();
        self.emit(InstKind::Const { dst, value }, span);
        dst
    }

    fn unit(&mut self, span: Span) -> Reg {
        self.constant(Constant::Unit, span)
    }

    fn error(&mut self, message: impl Into<String>, span: Span) -> Reg {
        self.emit(
            InstKind::Error {
                message: message.into(),
            },
            span,
        );
        self.unit(span)
    }

    /// Lowers a block, returning the register holding its value.
    fn block(&mut self, block: &AstBlock) -> Reg {
        self.block_value(block)
    }

    fn expr_or_unit(&mut self, expr: Option<&Expr>, span: Span) -> Reg {
        match expr {
            Some(expr) => self.expr(expr),
            None => self.unit(span),
        }
    }
}

fn prim_rt_type(prim: PrimTy) -> RtType {
    let int = |name, min, max| RtType::Int { name, min, max };
    match prim {
        PrimTy::Bool => RtType::Bool,
        PrimTy::Str => RtType::Str,
        PrimTy::Bytes => RtType::Bytes,
        PrimTy::F32 | PrimTy::F64 => RtType::Float,
        PrimTy::U8 => int("u8", 0, u8::MAX.into()),
        PrimTy::U16 => int("u16", 0, u16::MAX.into()),
        PrimTy::U32 => int("u32", 0, u32::MAX.into()),
        PrimTy::U64 => int("u64", 0, i64::MAX),
        PrimTy::Usize => int("usize", 0, i64::MAX),
        PrimTy::I8 => int("i8", i8::MIN.into(), i8::MAX.into()),
        PrimTy::I16 => int("i16", i16::MIN.into(), i16::MAX.into()),
        PrimTy::I32 => int("i32", i32::MIN.into(), i32::MAX.into()),
        PrimTy::I64 => int("i64", i64::MIN, i64::MAX),
        PrimTy::Isize => int("isize", i64::MIN, i64::MAX),
    }
}
