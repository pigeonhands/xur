use std::collections::HashMap;

use xur_ast::{BinOpKind, Expr, ExprKind, FnRetTy, ItemKind, LitKind, Local, NodeId, PatKind, Ty, TyKind};
use xur_ast::visit::{self, Visitor};
use xur_bytecode::{ItemIndex, ItemRef, type_name};
use xur_resolve::{DefId, DefKind, NativeFnId, Program, Res, Resolutions};
use xur_runtime::{BuiltinType, Registry};
use xur_span::{Ident, Span, Symbol};

pub struct CallCollector(pub Vec<(xur_ast::NodeId, Vec<Span>)>);

impl<'ast> Visitor<'ast> for CallCollector {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && let ExprKind::Path(path) = &callee.kind
        {
            let arg_spans: Vec<_> = args.iter().map(|a| a.span).collect();
            self.0.push((path.id, arg_spans));
        }
        visit::walk_expr(self, expr);
    }
}

pub struct LetBinding<'a> {
    pub pat_end: usize,
    pub binding_id: Option<NodeId>,
    pub init: &'a Expr,
    pub ty: Option<&'a Ty>,
}

pub struct LetCollector<'a>(pub Vec<LetBinding<'a>>);

impl<'ast> Visitor<'ast> for LetCollector<'ast> {
    fn visit_local(&mut self, local: &'ast Local) {
        if let Some(init) = &local.init {
            let binding_id = match &local.pat.kind {
                PatKind::Ident(_) => Some(local.pat.id),
                _ => None,
            };
            self.0.push(LetBinding {
                pat_end: local.pat.span.end,
                binding_id,
                init,
                ty: local.ty.as_ref(),
            });
        }
        visit::walk_local(self, local);
    }
}

pub struct MatchCollector<'a>(pub Vec<(usize, &'a Expr)>);

impl<'ast> Visitor<'ast> for MatchCollector<'ast> {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::Match(scrutinee, _) = &expr.kind {
            self.0.push((scrutinee.span.end, scrutinee));
        }
        visit::walk_expr(self, expr);
    }
}

pub struct ForLoopCollector<'a>(pub Vec<(usize, Option<NodeId>, &'a Expr)>);

impl<'ast> Visitor<'ast> for ForLoopCollector<'ast> {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::ForLoop { pat, iter, .. } = &expr.kind {
            let binding_id = match &pat.kind {
                PatKind::Ident(_) => Some(pat.id),
                _ => None,
            };
            self.0.push((pat.span.end, binding_id, iter));
        }
        visit::walk_expr(self, expr);
    }
}

/// Info about a method call at a cursor position.
pub struct MethodAt {
    pub method_name: String,
    pub method_span: Span,
    pub receiver_ty: Option<String>,
}

/// Finds the method call whose method name contains `offset`.
pub fn method_at_offset(
    program: &Program,
    items: &ItemIndex,
    registry: &Registry,
    offset: usize,
) -> Option<MethodAt> {
    let resolutions = program.resolutions();
    let empty = HashMap::new();
    let mut best: Option<MethodAt> = None;
    for module in &program.krate.modules {
        let mut collector = MethodCallCollector {
            offset,
            resolutions,
            items,
            registry,
            local_types: &empty,
            found: Vec::new(),
        };
        collector.visit_module(module.ast);
        for hit in collector.found {
            if best.as_ref().is_none_or(|b| hit.method_span.len() < b.method_span.len()) {
                best = Some(hit);
            }
        }
    }
    best
}

struct MethodCallCollector<'a> {
    offset: usize,
    resolutions: &'a Resolutions,
    items: &'a ItemIndex,
    registry: &'a Registry,
    local_types: &'a HashMap<NodeId, String>,
    found: Vec<MethodAt>,
}

impl<'ast> Visitor<'ast> for MethodCallCollector<'_> {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::MethodCall(mc) = &expr.kind {
            let ident_span = mc.seg.ident.span;
            if ident_span.start <= self.offset && self.offset < ident_span.end {
                let receiver_ty = infer_expr_type(
                    &mc.receiver, self.resolutions, self.items, self.registry,
                    self.local_types,
                );
                self.found.push(MethodAt {
                    method_name: mc.seg.ident.name.to_string(),
                    method_span: ident_span,
                    receiver_ty,
                });
            }
        }
        visit::walk_expr(self, expr);
    }
}

pub fn infer_expr_type(
    expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, String>,
) -> Option<String> {
    match &expr.kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Str(_) => Some("str".into()),
            LitKind::Int(_) => Some("i64".into()),
            LitKind::Float(_) => Some("f64".into()),
            LitKind::Bool(_) => Some("bool".into()),
            LitKind::ByteStr(_) => Some("bytes".into()),
            LitKind::Err => None,
        },
        ExprKind::List(_) => Some("list".into()),
        ExprKind::Map(_) => Some("map".into()),
        ExprKind::Record(_) => Some("record".into()),
        ExprKind::Tuple(elems) => {
            if elems.is_empty() {
                return Some("()".into());
            }
            let types: Vec<_> = elems.iter()
                .map(|e| infer_expr_type(e, resolutions, items, registry, local_types)
                    .unwrap_or_else(|| "_".into()))
                .collect();
            Some(format!("({})", types.join(", ")))
        }
        ExprKind::Struct(s) => {
            let res = resolutions.res.get(&s.path.id)?;
            match res {
                Res::Def(_, def) => Some(resolutions.defs.get(*def).name.to_string()),
                _ => None,
            }
        }
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(path) = &callee.kind {
                let res = resolutions.res.get(&path.id)?;
                match res {
                    Res::Def(DefKind::Fn, def) => {
                        if let Some(ItemRef::Fn { item, .. }) = items.get(*def) {
                            match &item.decl.output {
                                FnRetTy::Ty(ty) => Some(type_name(resolutions, ty)),
                                FnRetTy::Default(_) => Some("()".into()),
                            }
                        } else {
                            None
                        }
                    }
                    Res::Def(DefKind::NativeFn(id), _) => {
                        if let Some(ty) = native_return_type(*id, registry) {
                            return Some(ty);
                        }
                        path.segments.last()
                            .and_then(|seg| seg.args.as_ref())
                            .and_then(|args| args.args.first())
                            .map(|ty| type_name(resolutions, ty))
                    }
                    Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
                        Some(resolutions.defs.get(*def).name.to_string())
                    }
                    Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
                        let parent = resolutions.defs.get(*def).parent?;
                        Some(resolutions.defs.get(parent).name.to_string())
                    }
                    Res::Local(node_id) => {
                        let ty = local_types.get(node_id)?;
                        fn_return_type(ty)
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        ExprKind::MethodCall(mc) => {
            let receiver_ty = infer_expr_type(&mc.receiver, resolutions, items, registry, local_types);
            let method = mc.seg.ident.name.as_str();
            if let Some(ty) = method_return_type(receiver_ty.as_deref(), method, registry) {
                return Some(ty);
            }
            if let Some(receiver_name) = receiver_ty.as_deref() {
                if let Some(ty) = user_method_return_type(
                    receiver_name, method, &mc.receiver, resolutions, items, registry,
                ) {
                    return Some(ty);
                }
                if resolutions.find_type(receiver_name).is_some() {
                    if let Some(id) = registry.builtin_method(BuiltinType::Struct, method) {
                        return native_return_type(id, registry);
                    }
                }
            }
            None
        }
        ExprKind::Deref(inner) => {
            infer_expr_type(inner, resolutions, items, registry, local_types)
                .and_then(|t| t.strip_prefix('*').map(|s| s.to_string()))
        }
        ExprKind::Paren(inner) => infer_expr_type(inner, resolutions, items, registry, local_types),
        ExprKind::Block(block) => {
            block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry, local_types)
                } else {
                    None
                }
            })
        }
        ExprKind::If(_, then_block, _) => {
            then_block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry, local_types)
                } else {
                    None
                }
            })
        }
        ExprKind::Match(_, arms) => {
            arms.first().and_then(|arm| infer_expr_type(&arm.body, resolutions, items, registry, local_types))
        }
        ExprKind::Try(inner) => {
            if let Some(shape) = native_call_return_shape(inner, resolutions, registry) {
                if let Ok(r) = shape.def.into_result() {
                    return Some(registry.shape_xur_name(r.t()));
                }
                if let Ok(o) = shape.def.into_option() {
                    return Some(registry.shape_xur_name(o.t()));
                }
                return Some(registry.shape_xur_name(shape));
            }
            infer_expr_type(inner, resolutions, items, registry, local_types)
                .map(|t| match t.as_str() {
                    "Result" | "Option" => "_".into(),
                    _ => t,
                })
        }
        ExprKind::Cast(_, ty) => Some(type_name(resolutions, ty)),
        ExprKind::Closure(closure) => {
            let params: Vec<_> = closure.decl.inputs.iter()
                .map(|p| type_name(resolutions, &p.ty))
                .collect();
            let ret = match &closure.decl.output {
                FnRetTy::Ty(ty) => type_name(resolutions, ty),
                FnRetTy::Default(_) => {
                    infer_expr_type(&closure.body, resolutions, items, registry, local_types)
                        .unwrap_or_else(|| "_".into())
                }
            };
            if params.iter().all(|p| p == "_") && ret == "_" {
                return None;
            }
            if ret == "()" {
                Some(format!("fn({})", params.join(", ")))
            } else {
                Some(format!("fn({}) -> {}", params.join(", "), ret))
            }
        }
        ExprKind::Binary(op, lhs, rhs) => match op.kind {
            BinOpKind::Eq | BinOpKind::Ne | BinOpKind::Lt | BinOpKind::Le
            | BinOpKind::Gt | BinOpKind::Ge | BinOpKind::And | BinOpKind::Or => {
                Some("bool".into())
            }
            _ => {
                infer_expr_type(lhs, resolutions, items, registry, local_types)
                    .or_else(|| infer_expr_type(rhs, resolutions, items, registry, local_types))
            }
        },
        ExprKind::Unary(_, inner) => infer_expr_type(inner, resolutions, items, registry, local_types),
        ExprKind::Path(path) => {
            let res = resolutions.res.get(&path.id)?;
            match res {
                Res::Local(node_id) => local_types.get(node_id).cloned(),
                Res::Def(DefKind::Mod, _) => Some("module".into()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn fn_return_type(ty: &str) -> Option<String> {
    let s = ty.strip_prefix('*').unwrap_or(ty);
    if !s.starts_with("fn(") {
        return None;
    }
    match s.rsplit_once(" -> ") {
        Some((_, ret)) => Some(ret.to_string()),
        None => Some("()".into()),
    }
}

fn native_return_type(id: NativeFnId, registry: &Registry) -> Option<String> {
    registry.native_return_name(id)
}

/// Walk the receiver expression to find a native function call's return Shape.
/// Handles `native_fn(args)`, chained `.method()` on native results, `(expr)`,
/// and `expr?` (peels Result/Option).
fn native_call_return_shape(
    expr: &Expr,
    resolutions: &Resolutions,
    registry: &Registry,
) -> Option<&'static facet::Shape> {
    match &expr.kind {
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(path) = &callee.kind
                && let Some(Res::Def(DefKind::NativeFn(id), _)) = resolutions.res.get(&path.id)
            {
                return registry.native_return_shape(*id);
            }
            None
        }
        ExprKind::MethodCall(mc) => {
            let receiver_shape = native_call_return_shape(&mc.receiver, resolutions, registry)?;
            let receiver_name = registry.shape_xur_name(receiver_shape);
            let method = mc.seg.ident.name.as_str();
            let id = resolve_method(&receiver_name, method, registry)?;
            registry.native_return_shape(id)
        }
        ExprKind::Paren(inner) => native_call_return_shape(inner, resolutions, registry),
        ExprKind::Try(inner) => {
            let shape = native_call_return_shape(inner, resolutions, registry)?;
            if let Ok(r) = shape.def.into_result() {
                return Some(r.t());
            }
            if let Ok(o) = shape.def.into_option() {
                return Some(o.t());
            }
            None
        }
        _ => None,
    }
}

pub fn infer_iter_element_type(
    iter: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, String>,
) -> Option<String> {
    match &iter.kind {
        ExprKind::Range(..) => Some("i64".into()),
        ExprKind::List(elems) => {
            let mut seen = Vec::new();
            for e in elems {
                if let Some(ty) = infer_expr_type(e, resolutions, items, registry, local_types) {
                    if !seen.contains(&ty) {
                        seen.push(ty);
                    }
                }
            }
            if seen.is_empty() {
                return None;
            }
            if seen.len() <= 2 {
                Some(seen.join(" | "))
            } else {
                Some(format!("{} | {} | ...", seen[0], seen[1]))
            }
        }
        _ => {
            if let Some(shape) = native_call_return_shape(iter, resolutions, registry) {
                if let Ok(list) = shape.def.into_list() {
                    return Some(registry.shape_xur_name(list.t()));
                }
            }
            None
        }
    }
}

/// Resolve a user-defined `impl` method's return type through the resolution
/// system. If the return type is a type parameter (e.g. `T` in `unwrap() -> T`),
/// tries to resolve it via the receiver's native Shape.
fn user_method_return_type(
    receiver_name: &str,
    method: &str,
    receiver_expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
) -> Option<String> {
    let type_def = resolutions.find_type(receiver_name)?;
    let res = resolutions.assoc_fn(type_def, Symbol::intern(method))?;
    match res {
        Res::Def(DefKind::Fn, def) => {
            let ItemRef::Fn { item, .. } = items.get(def)? else { return None };
            match &item.decl.output {
                FnRetTy::Ty(ty) => {
                    if let TyKind::Path(path) = &ty.kind
                        && let Some(&Res::Def(DefKind::TyParam, param_def)) =
                            resolutions.res.get(&path.id)
                    {
                        let index = type_param_position(resolutions, param_def)?;
                        let shape =
                            native_call_return_shape(receiver_expr, resolutions, registry)?;
                        return registry.shape_type_param(shape, index);
                    }
                    Some(type_name(resolutions, ty))
                }
                FnRetTy::Default(_) => Some("()".into()),
            }
        }
        Res::Def(DefKind::NativeFn(id), _) => native_return_type(id, registry),
        _ => None,
    }
}

/// Position of a type parameter within its owning generic param list.
/// Impl block params are created consecutively by `declare_generics`,
/// so we find the contiguous run of TyParams sharing the same parent.
fn type_param_position(resolutions: &Resolutions, param_def: DefId) -> Option<usize> {
    let parent = resolutions.defs.get(param_def).parent;
    let all: Vec<_> = resolutions.defs.iter().collect();
    let idx = all.iter().position(|(id, _)| *id == param_def)?;
    // Walk backwards to find the start of this contiguous TyParam group.
    let mut start = idx;
    while start > 0 {
        let (_, prev) = &all[start - 1];
        if prev.kind == DefKind::TyParam && prev.parent == parent {
            start -= 1;
        } else {
            break;
        }
    }
    Some(idx - start)
}

fn xur_name_to_builtin(name: &str) -> Option<BuiltinType> {
    Some(match name {
        "bool" => BuiltinType::Bool,
        "i64" | "u8" | "u32" | "u64" | "usize" => BuiltinType::Int,
        "f64" => BuiltinType::Float,
        "str" => BuiltinType::Str,
        "list" => BuiltinType::List,
        "map" => BuiltinType::Map,
        "bytes" => BuiltinType::Bytes,
        "record" => BuiltinType::Record,
        _ if name.starts_with('*') => BuiltinType::Ptr,
        _ => return None,
    })
}

fn resolve_method(receiver: &str, method: &str, registry: &Registry) -> Option<NativeFnId> {
    if let Some(builtin) = xur_name_to_builtin(receiver) {
        return registry.builtin_method(builtin, method);
    }
    let type_id = registry.type_id_by_name(receiver)?;
    registry.type_function(type_id, method)
}

/// Look up a method's return type through the registry.
pub fn method_return_type(
    receiver_ty: Option<&str>,
    method: &str,
    registry: &Registry,
) -> Option<String> {
    let id = resolve_method(receiver_ty?, method, registry)?;
    native_return_type(id, registry)
}

/// Format a native method's signature for hover display.
pub fn method_signature(
    receiver_ty: Option<&str>,
    method: &str,
    registry: &Registry,
) -> Option<String> {
    let receiver = receiver_ty?;
    let id = resolve_method(receiver, method, registry)?;
    let native_fn = registry.function(id);
    let sig = native_fn.signature.as_ref()?;
    // Skip the first param (self/receiver).
    let params: Vec<String> = sig.params.iter().skip(1).enumerate().map(|(i, shape)| {
        let ty = registry.shape_xur_name(shape);
        if let Some(pname) = native_fn.param_names.get(i + 1)
            && !pname.is_empty()
        {
            format!("{pname}: {ty}")
        } else {
            ty
        }
    }).collect();
    let ret = registry.shape_xur_name(sig.ret);
    let ret_str = if ret == "()" { String::new() } else { format!(" -> {ret}") };
    Some(format!("fn {receiver}.{method}({}){ret_str}", params.join(", ")))
}

pub fn build_param_name_map(program: &Program) -> HashMap<DefId, Vec<Ident>> {
    let resolutions = program.resolutions();
    let mut map = HashMap::new();
    for module in &program.krate.modules {
        for item in &module.ast.items {
            if let ItemKind::Fn(f) = &item.kind
                && let Some(&def_id) = resolutions.node_defs.get(&item.id)
            {
                let names = param_idents(&f.decl.inputs);
                map.insert(def_id, names);
            }
            if let ItemKind::Impl(imp) = &item.kind {
                for impl_item in &imp.items {
                    if let ItemKind::Fn(f) = &impl_item.kind
                        && let Some(&def_id) = resolutions.node_defs.get(&impl_item.id)
                    {
                        let names = param_idents(&f.decl.inputs);
                        map.insert(def_id, names);
                    }
                }
            }
        }
    }
    map
}

fn param_idents(inputs: &[xur_ast::Param]) -> Vec<Ident> {
    inputs.iter().filter_map(|p| {
        if let PatKind::Ident(ident) = &p.pat.kind {
            Some(*ident)
        } else {
            None
        }
    }).collect()
}
