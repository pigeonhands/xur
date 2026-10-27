//! Lowering expressions, statements and control flow.

use std::collections::HashSet;
use std::sync::Arc;

use xur_ast::visit::{self, Visitor};
use xur_ast::{
    Arm, BinOpKind, Block as AstBlock, Closure, Expr, ExprKind, GenericArgs, LitKind, Local,
    NodeId, Pat, PatKind, Path, RangeLimits, StmtKind, Ty, TyKind, VariantData,
};
use xur_resolve::{DefId, DefKind, Res};
use xur_span::Span;

use super::{LoopScope, Lowerer};
use crate::ir::{BlockId, Constant, DefRef, InstKind, Reg, Terminator, TypeArg};
use crate::items::ItemRef;

impl Lowerer<'_> {
    /// Lowers `expr`, returning the register that holds its value.
    pub(super) fn expr(&mut self, expr: &Expr) -> Reg {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Lit(lit) => match lit.kind {
                LitKind::Str(s) => self.constant(Constant::Str(s.as_str().into()), span),
                LitKind::Int(i) => match i64::try_from(i) {
                    Ok(i) => self.constant(Constant::Int(i), span),
                    Err(_) => self.error(format!("{i} does not fit in a Xur integer"), span),
                },
                LitKind::Float(digits) => match digits.as_str().parse() {
                    Ok(f) => self.constant(Constant::Float(f), span),
                    Err(_) => self.error("invalid float literal", span),
                },
                LitKind::Bool(b) => self.constant(Constant::Bool(b), span),
                LitKind::Err => self.error("invalid literal", span),
            },
            ExprKind::Path(path) => self.path(path),
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Tuple(items) => {
                let items = self.exprs(items);
                let dst = self.new_reg();
                self.emit(InstKind::Tuple { dst, items }, span);
                dst
            }
            ExprKind::List(items) => {
                let items = self.exprs(items);
                let dst = self.new_reg();
                self.emit(InstKind::List { dst, items }, span);
                dst
            }
            ExprKind::Unary(op, operand) => {
                let src = self.expr(operand);
                let dst = self.new_reg();
                self.emit(InstKind::Unary { dst, op: *op, src }, span);
                dst
            }
            ExprKind::Binary(op, lhs, rhs) => match op.kind {
                BinOpKind::And | BinOpKind::Or => self.short_circuit(op.kind, lhs, rhs, span),
                kind => {
                    let lhs = self.expr(lhs);
                    let rhs = self.expr(rhs);
                    let dst = self.new_reg();
                    self.emit(
                        InstKind::Binary {
                            dst,
                            op: kind,
                            lhs,
                            rhs,
                        },
                        span,
                    );
                    dst
                }
            },
            ExprKind::Assign(place, value, _) => self.assign(place, value, span),
            ExprKind::Range(start, end, limits) => {
                let start = start.as_deref().map(|start| self.expr(start));
                let end = end.as_deref().map(|end| self.expr(end));
                let dst = self.new_reg();
                let inclusive = *limits == RangeLimits::Closed;
                self.emit(
                    InstKind::Range {
                        dst,
                        start,
                        end,
                        inclusive,
                    },
                    span,
                );
                dst
            }
            ExprKind::Call(callee, args) => self.call(callee, args, span),
            ExprKind::MethodCall(call) => {
                let receiver = self.expr(&call.receiver);
                let args = self.exprs(&call.args);
                let dst = self.new_reg();
                let method = call.seg.ident.name;
                self.emit(
                    InstKind::MethodCall {
                        dst,
                        receiver,
                        method,
                        args,
                    },
                    span,
                );
                dst
            }
            ExprKind::Field(base, field) => {
                let base = self.expr(base);
                let dst = self.new_reg();
                self.emit(
                    InstKind::Field {
                        dst,
                        base,
                        field: field.name,
                    },
                    span,
                );
                dst
            }
            ExprKind::Index(base, index) => {
                let base = self.expr(base);
                let index = self.expr(index);
                let dst = self.new_reg();
                self.emit(InstKind::Index { dst, base, index }, span);
                dst
            }
            ExprKind::Try(inner) => self.try_(inner, span),
            ExprKind::Block(block) => self.block_value(block),
            ExprKind::If(cond, then, otherwise) => self.if_(cond, then, otherwise.as_deref(), span),
            ExprKind::Match(scrutinee, arms) => self.match_(scrutinee, arms, span),
            ExprKind::ForLoop { pat, iter, body } => self.for_(pat, iter, body, span),
            ExprKind::While(cond, body) => self.while_(cond, body, span),
            ExprKind::Loop(body) => self.loop_(body, span),
            ExprKind::Closure(closure) => self.closure(closure, span),
            ExprKind::Break(value) => {
                let value = self.expr_or_unit(value.as_deref(), span);
                let Some(scope) = self.loops.last() else {
                    return self.error("`break` outside of a loop", span);
                };
                let (result, target) = (scope.result, scope.break_to);
                self.emit(
                    InstKind::Copy {
                        dst: result,
                        src: value,
                    },
                    span,
                );
                self.diverge(Terminator::Goto(target), span);
                self.unit(span)
            }
            ExprKind::Continue => {
                let Some(scope) = self.loops.last() else {
                    return self.error("`continue` outside of a loop", span);
                };
                let target = scope.continue_to;
                self.diverge(Terminator::Goto(target), span);
                self.unit(span)
            }
            ExprKind::Ret(value) => {
                let value = self.expr_or_unit(value.as_deref(), span);
                self.ret(value, span);
                let dead = self.new_block();
                self.switch_to(dead);
                self.unit(span)
            }
            ExprKind::Err => self.error("invalid expression", span),
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> Vec<Reg> {
        exprs.iter().map(|expr| self.expr(expr)).collect()
    }

    /// A path in value position.
    fn path(&mut self, path: &Path) -> Reg {
        let span = path.span;
        let res = self
            .resolutions
            .res
            .get(&path.id)
            .copied()
            .unwrap_or(Res::Err);
        let def = match res {
            Res::Local(binding) => match self.locals.get(&binding) {
                Some(&reg) => return reg,
                None => return self.error("use of a variable from outside this function", span),
            },
            Res::Def(DefKind::Fn, def) => DefRef::Fn(def),
            Res::Def(DefKind::NativeFn(id), _) => DefRef::Native(id),
            Res::Def(DefKind::Const, def) => DefRef::Const(def),
            Res::Def(DefKind::Mod, def) => DefRef::Module(def),
            Res::Def(DefKind::Struct | DefKind::Variant, def) => match self.ctor_shape(def) {
                Some(VariantData::Unit) => {
                    let dst = self.new_reg();
                    self.emit(
                        InstKind::Adt {
                            dst,
                            ctor: def,
                            fields: Vec::new(),
                        },
                        span,
                    );
                    return dst;
                }
                Some(VariantData::Tuple(_)) => DefRef::Ctor(def),
                _ => {
                    return self.error("structs with named fields cannot be constructed yet", span);
                }
            },
            Res::Err => return self.error("unresolved name", span),
            other => {
                return self.error(format!("expected a value, found a {}", other.descr()), span);
            }
        };
        let dst = self.new_reg();
        self.emit(InstKind::Def { dst, def }, span);
        dst
    }

    fn ctor_shape(&self, def: DefId) -> Option<&'static VariantData> {
        match self.items.get(def)? {
            ItemRef::Ctor(data) => Some(data),
            _ => None,
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Reg {
        // Calling a tuple constructor directly builds the value.
        if let ExprKind::Path(path) = &callee.kind
            && let Some(&Res::Def(DefKind::Struct | DefKind::Variant, ctor)) =
                self.resolutions.res.get(&path.id)
            && let Some(VariantData::Tuple(_)) = self.ctor_shape(ctor)
        {
            let fields = self.exprs(args);
            let dst = self.new_reg();
            self.emit(InstKind::Adt { dst, ctor, fields }, span);
            return dst;
        }
        let type_args = match &callee.kind {
            ExprKind::Path(path) => path
                .segments
                .last()
                .and_then(|segment| segment.args.as_ref())
                .map(|args| self.type_args(args))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let callee = self.expr(callee);
        let args = self.exprs(args);
        let dst = self.new_reg();
        self.emit(
            InstKind::Call {
                dst,
                callee,
                args,
                type_args,
            },
            span,
        );
        dst
    }

    fn type_args(&self, args: &GenericArgs) -> Vec<TypeArg> {
        args.args.iter().map(|ty| self.type_arg(ty)).collect()
    }

    fn type_arg(&self, ty: &Ty) -> TypeArg {
        if let TyKind::Path(path) = &ty.kind
            && let Some(&Res::Def(DefKind::TyParam, param)) = self.resolutions.res.get(&path.id)
            && let Some(index) = self.type_params.iter().position(|&p| p == param)
        {
            return TypeArg::Param(index as u32);
        }
        TypeArg::Named(self.type_name(ty))
    }

    /// How a type is named at runtime, e.g. by `type_name`.
    fn type_name(&self, ty: &Ty) -> String {
        match &ty.kind {
            TyKind::Path(path) => match self.resolutions.res.get(&path.id) {
                Some(Res::Def(_, def)) => self.resolutions.defs.get(*def).name.to_string(),
                Some(Res::PrimTy(prim)) => prim.name().to_owned(),
                _ => "_".to_owned(),
            },
            TyKind::Tuple(tys) => {
                let names: Vec<_> = tys.iter().map(|ty| self.type_name(ty)).collect();
                format!("({})", names.join(", "))
            }
            TyKind::Paren(ty) => self.type_name(ty),
            TyKind::List(ty) => format!("[{}]", self.type_name(ty)),
            TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => "_".to_owned(),
        }
    }

    fn short_circuit(&mut self, op: BinOpKind, lhs: &Expr, rhs: &Expr, span: Span) -> Reg {
        let result = self.new_reg();
        let lhs = self.expr(lhs);
        self.emit(
            InstKind::Copy {
                dst: result,
                src: lhs,
            },
            span,
        );
        let (eval_rhs, join) = (self.new_block(), self.new_block());
        let (then, otherwise) = match op {
            BinOpKind::And => (eval_rhs, join),
            _ => (join, eval_rhs),
        };
        self.terminate(
            Terminator::Branch {
                cond: lhs,
                then,
                otherwise,
            },
            span,
        );
        self.switch_to(eval_rhs);
        let rhs = self.expr(rhs);
        self.emit(
            InstKind::Copy {
                dst: result,
                src: rhs,
            },
            span,
        );
        self.terminate(Terminator::Goto(join), span);
        self.switch_to(join);
        result
    }

    fn assign(&mut self, place: &Expr, value: &Expr, span: Span) -> Reg {
        match &place.kind {
            ExprKind::Path(path) => {
                let value = self.expr(value);
                match self.resolutions.res.get(&path.id) {
                    Some(Res::Local(binding)) if self.locals.contains_key(binding) => {
                        let dst = self.locals[binding];
                        self.emit(InstKind::Copy { dst, src: value }, span);
                    }
                    _ => return self.error("only local variables can be assigned to", place.span),
                }
            }
            ExprKind::Index(base, index) => {
                let base = self.expr(base);
                let index = self.expr(index);
                let value = self.expr(value);
                self.emit(InstKind::SetIndex { base, index, value }, span);
            }
            _ => return self.error("this cannot be assigned to", place.span),
        }
        self.unit(span)
    }

    /// `value?`: the `Ok` or `Some` value, or else return the `Err` or
    /// `None` itself.
    fn try_(&mut self, inner: &Expr, span: Span) -> Reg {
        let value = self.expr(inner);
        let result = self.new_reg();
        let (unwrap, check_some, early_return, join) = (
            self.new_block(),
            self.new_block(),
            self.new_block(),
            self.new_block(),
        );
        for (ctor, otherwise) in [(self.lang.ok, check_some), (self.lang.some, early_return)] {
            let is = self.new_reg();
            self.emit(
                InstKind::IsCtor {
                    dst: is,
                    value,
                    ctor,
                },
                span,
            );
            self.terminate(
                Terminator::Branch {
                    cond: is,
                    then: unwrap,
                    otherwise,
                },
                span,
            );
            self.switch_to(otherwise);
        }
        self.terminate(Terminator::Return(value), span);
        self.switch_to(unwrap);
        self.emit(
            InstKind::Project {
                dst: result,
                value,
                index: 0,
            },
            span,
        );
        self.terminate(Terminator::Goto(join), span);
        self.switch_to(join);
        result
    }

    pub(super) fn block_value(&mut self, block: &AstBlock) -> Reg {
        let saved_locals = self.locals.clone();
        let mut value = None;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_tail = i + 1 == block.stmts.len();
            match &stmt.kind {
                StmtKind::Let(local) => self.local(local),
                StmtKind::Expr(expr) => {
                    let reg = self.expr(expr);
                    if is_tail {
                        value = Some(reg);
                    }
                }
                StmtKind::Semi(expr) => {
                    self.expr(expr);
                }
                // Items are lowered on their own, when used.
                StmtKind::Item(_) | StmtKind::Empty => {}
            }
        }
        self.locals = saved_locals;
        value.unwrap_or_else(|| self.unit(block.span))
    }

    fn local(&mut self, local: &Local) {
        let init = self.expr_or_unit(local.init.as_deref(), local.span);
        if let (Some(ty), Some(_)) = (&local.ty, &local.init) {
            self.check_type(init, ty, local.span);
        }
        // The initializer may be another variable's register, as in
        // `let x = y;`; the new variable needs its own.
        let value = self.new_reg();
        self.emit(
            InstKind::Copy {
                dst: value,
                src: init,
            },
            local.span,
        );
        self.bind_irrefutable(&local.pat, value);
    }

    fn if_(&mut self, cond: &Expr, then: &AstBlock, otherwise: Option<&Expr>, span: Span) -> Reg {
        let cond = self.expr(cond);
        let result = self.new_reg();
        let (then_block, else_block, join) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(
            Terminator::Branch {
                cond,
                then: then_block,
                otherwise: else_block,
            },
            span,
        );
        self.switch_to(then_block);
        let value = self.block_value(then);
        self.emit(
            InstKind::Copy {
                dst: result,
                src: value,
            },
            span,
        );
        self.terminate(Terminator::Goto(join), span);
        self.switch_to(else_block);
        let value = self.expr_or_unit(otherwise, span);
        self.emit(
            InstKind::Copy {
                dst: result,
                src: value,
            },
            span,
        );
        self.terminate(Terminator::Goto(join), span);
        self.switch_to(join);
        result
    }

    fn match_(&mut self, scrutinee: &Expr, arms: &[Arm], span: Span) -> Reg {
        let value = self.expr(scrutinee);
        let result = self.new_reg();
        let join = self.new_block();
        for arm in arms {
            let next_arm = self.new_block();
            let saved_locals = self.locals.clone();
            self.pat(&arm.pat, value, next_arm);
            let body = self.expr(&arm.body);
            self.emit(
                InstKind::Copy {
                    dst: result,
                    src: body,
                },
                arm.span,
            );
            self.terminate(Terminator::Goto(join), arm.span);
            self.locals = saved_locals;
            self.switch_to(next_arm);
        }
        self.terminate(Terminator::NoMatch, span);
        self.switch_to(join);
        result
    }

    fn for_(&mut self, pat: &Pat, iterable: &Expr, body: &AstBlock, span: Span) -> Reg {
        let iterable = self.expr(iterable);
        let iter = self.new_reg();
        self.emit(
            InstKind::IterStart {
                dst: iter,
                iterable,
            },
            span,
        );
        let (head, body_block, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(Terminator::Goto(head), span);
        self.switch_to(head);
        let item = self.new_reg();
        self.terminate(
            Terminator::Next {
                iter,
                item,
                body: body_block,
                done: exit,
            },
            span,
        );
        self.switch_to(body_block);
        let saved_locals = self.locals.clone();
        self.bind_irrefutable(pat, item);
        self.loop_body(body, head, exit, span);
        self.locals = saved_locals;
        self.switch_to(exit);
        self.unit(span)
    }

    fn while_(&mut self, cond: &Expr, body: &AstBlock, span: Span) -> Reg {
        let (head, body_block, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(Terminator::Goto(head), span);
        self.switch_to(head);
        let cond = self.expr(cond);
        self.terminate(
            Terminator::Branch {
                cond,
                then: body_block,
                otherwise: exit,
            },
            span,
        );
        self.switch_to(body_block);
        self.loop_body(body, head, exit, span);
        self.switch_to(exit);
        self.unit(span)
    }

    fn loop_(&mut self, body: &AstBlock, span: Span) -> Reg {
        let (head, exit) = (self.new_block(), self.new_block());
        self.terminate(Terminator::Goto(head), span);
        self.switch_to(head);
        let result = self.loop_body(body, head, exit, span);
        self.switch_to(exit);
        result
    }

    /// Lowers a loop body that continues at `head` and breaks to `exit`,
    /// returning the register `break value` writes to.
    fn loop_body(&mut self, body: &AstBlock, head: BlockId, exit: BlockId, span: Span) -> Reg {
        let result = self.unit(span);
        self.loops.push(LoopScope {
            break_to: exit,
            continue_to: head,
            result,
        });
        self.block_value(body);
        self.loops.pop();
        self.terminate(Terminator::Goto(head), span);
        result
    }

    /// A closure becomes a separate function whose first registers hold the
    /// values of the enclosing locals it uses, captured when it is created.
    fn closure(&mut self, closure: &Closure, span: Span) -> Reg {
        let captured: Vec<NodeId> = captures(closure, self.resolutions)
            .into_iter()
            .filter(|binding| self.locals.contains_key(binding))
            .collect();

        let mut inner = Lowerer::new(self.program, self.items, self.module);
        inner.type_params = self.type_params.clone();
        inner.type_param_names = self.type_param_names.clone();
        for &binding in &captured {
            let reg = inner.new_reg();
            inner.locals.insert(binding, reg);
        }
        inner.params(&closure.decl.inputs);
        let value = inner.expr(&closure.body);
        inner.terminate(Terminator::Return(value), closure.body.span);
        let params = closure.decl.inputs.len() as u32;
        let function = inner.finish("<closure>".to_owned(), captured.len() as u32, params, span);

        let index = self.closures.len() as u32;
        self.closures.push(Arc::new(function));
        let captures = captured
            .iter()
            .map(|binding| self.locals[binding])
            .collect();
        let dst = self.new_reg();
        self.emit(
            InstKind::Closure {
                dst,
                function: index,
                captures,
            },
            span,
        );
        dst
    }
}

/// The local variables a closure uses but doesn't bind itself, in order of
/// first use.
fn captures(closure: &Closure, resolutions: &xur_resolve::Resolutions) -> Vec<NodeId> {
    struct Uses<'r> {
        resolutions: &'r xur_resolve::Resolutions,
        bound: HashSet<NodeId>,
        used: Vec<NodeId>,
    }
    impl<'ast> Visitor<'ast> for Uses<'_> {
        fn visit_path(&mut self, path: &'ast Path) {
            if let Some(&Res::Local(binding)) = self.resolutions.res.get(&path.id)
                && !self.used.contains(&binding)
            {
                self.used.push(binding);
            }
            visit::walk_path(self, path);
        }

        fn visit_pat(&mut self, pat: &'ast Pat) {
            if let PatKind::Ident(_) = pat.kind
                && self.resolutions.res.get(&pat.id) == Some(&Res::Local(pat.id))
            {
                self.bound.insert(pat.id);
            }
            visit::walk_pat(self, pat);
        }
    }
    let mut uses = Uses {
        resolutions,
        bound: HashSet::new(),
        used: Vec::new(),
    };
    uses.visit_fn_decl(&closure.decl);
    uses.visit_expr(&closure.body);
    uses.used.retain(|binding| !uses.bound.contains(binding));
    uses.used
}
