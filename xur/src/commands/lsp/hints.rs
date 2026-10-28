use std::collections::HashMap;

use xur_ast::{Expr, ExprKind, FnRetTy, ItemKind, LitKind, Local, PatKind};
use xur_ast::visit::{self, Visitor};
use xur_bytecode::{ItemIndex, ItemRef, type_name};
use xur_resolve::{DefId, DefKind, NativeFnId, Program, Res, Resolutions};
use xur_runtime::{BuiltinType, Registry};
use xur_span::{Ident, Span};

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

pub struct LetCollector<'a>(pub Vec<(usize, &'a Expr)>);

impl<'ast> Visitor<'ast> for LetCollector<'ast> {
    fn visit_local(&mut self, local: &'ast Local) {
        if local.ty.is_none()
            && let Some(init) = &local.init
        {
            self.0.push((local.pat.span.end, init));
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
    let mut best: Option<MethodAt> = None;
    for module in &program.krate.modules {
        let mut collector = MethodCallCollector {
            offset,
            resolutions,
            items,
            registry,
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
    found: Vec<MethodAt>,
}

impl<'ast> Visitor<'ast> for MethodCallCollector<'_> {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::MethodCall(mc) = &expr.kind {
            let ident_span = mc.seg.ident.span;
            if ident_span.start <= self.offset && self.offset < ident_span.end {
                let receiver_ty = infer_expr_type(
                    &mc.receiver, self.resolutions, self.items, self.registry,
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
                .map(|e| infer_expr_type(e, resolutions, items, registry)
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
                        native_return_type(*id, registry)
                    }
                    Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
                        Some(resolutions.defs.get(*def).name.to_string())
                    }
                    Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
                        let parent = resolutions.defs.get(*def).parent?;
                        Some(resolutions.defs.get(parent).name.to_string())
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        ExprKind::MethodCall(mc) => {
            let receiver_ty = infer_expr_type(&mc.receiver, resolutions, items, registry);
            let method = mc.seg.ident.name.as_str();
            method_return_type(receiver_ty.as_deref(), method, registry)
        }
        ExprKind::Paren(inner) => infer_expr_type(inner, resolutions, items, registry),
        ExprKind::Block(block) => {
            block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry)
                } else {
                    None
                }
            })
        }
        ExprKind::If(_, then_block, _) => {
            then_block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry)
                } else {
                    None
                }
            })
        }
        ExprKind::Match(_, arms) => {
            arms.first().and_then(|arm| infer_expr_type(&arm.body, resolutions, items, registry))
        }
        ExprKind::Try(inner) => {
            infer_expr_type(inner, resolutions, items, registry)
                .map(|t| match t.as_str() {
                    "Result" | "Option" => "_".into(),
                    _ => t,
                })
        }
        ExprKind::Path(_) => None,
        _ => None,
    }
}

fn native_return_type(id: NativeFnId, registry: &Registry) -> Option<String> {
    let native_fn = registry.function(id);
    let sig = native_fn.signature.as_ref()?;
    Some(registry.shape_xur_name(sig.ret))
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
