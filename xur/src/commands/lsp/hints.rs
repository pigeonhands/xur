use std::collections::HashMap;
use std::fmt;

use xur_ast::{BinOpKind, Expr, ExprKind, FnRetTy, ItemKind, LitKind, Local, NodeId, PatKind, Ty, TyKind};
use xur_ast::visit::{self, Visitor};
use xur_bytecode::{ItemIndex, ItemRef};
use xur_resolve::{DefId, DefKind, NativeFnId, Program, Res, Resolutions};
use xur_runtime::{BuiltinType, Registry};
use xur_span::{Ident, Span, Symbol};

#[derive(Clone, Debug, PartialEq)]
pub enum InferredTy {
    Named(String),
    Unit,
    Bool,
    Int,
    Float,
    Str,
    Bytes,
    Ptr(Box<InferredTy>),
    List,
    Map,
    Tuple(Vec<InferredTy>),
    Fn { params: Vec<InferredTy>, ret: Box<InferredTy> },
    Record,
    Module,
}

impl InferredTy {
    fn builtin(&self) -> Option<BuiltinType> {
        Some(match self {
            Self::Bool => BuiltinType::Bool,
            Self::Int => BuiltinType::Int,
            Self::Float => BuiltinType::Float,
            Self::Str => BuiltinType::Str,
            Self::List => BuiltinType::List,
            Self::Map => BuiltinType::Map,
            Self::Bytes => BuiltinType::Bytes,
            Self::Record => BuiltinType::Record,
            Self::Ptr(_) => BuiltinType::Ptr,
            _ => return None,
        })
    }

    fn call_return(&self) -> Option<InferredTy> {
        match self {
            Self::Fn { ret, .. } => Some((**ret).clone()),
            Self::Ptr(inner) => {
                if let Self::Fn { ret, .. } = inner.as_ref() {
                    Some((**ret).clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn deref_target(&self) -> Option<InferredTy> {
        match self {
            Self::Ptr(inner) => Some((**inner).clone()),
            _ => None,
        }
    }

    pub fn from_ast(ty: &Ty, resolutions: &Resolutions) -> Self {
        match &ty.kind {
            TyKind::Path(path) => {
                let name = xur_bytecode::type_name(resolutions, ty);
                match name.as_str() {
                    "bool" => Self::Bool,
                    "i64" | "u8" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "u16" => Self::Int,
                    "f64" | "f32" => Self::Float,
                    "str" => Self::Str,
                    "bytes" => Self::Bytes,
                    "()" => Self::Unit,
                    _ => {
                        if let Some(res) = resolutions.res.get(&path.id) {
                            if let Res::Def(DefKind::Mod, _) = res {
                                return Self::Module;
                            }
                        }
                        Self::Named(name)
                    }
                }
            }
            TyKind::Tuple(elems) => {
                if elems.is_empty() {
                    Self::Unit
                } else {
                    Self::Tuple(elems.iter().map(|t| Self::from_ast(t, resolutions)).collect())
                }
            }
            TyKind::Paren(inner) => Self::from_ast(inner, resolutions),
            TyKind::List(_) => Self::List,
            TyKind::Record => Self::Record,
            TyKind::Ptr(inner) => Self::Ptr(Box::new(Self::from_ast(inner, resolutions))),
            TyKind::Fn { params, ret } => Self::Fn {
                params: params.iter().map(|t| Self::from_ast(t, resolutions)).collect(),
                ret: Box::new(ret.as_ref()
                    .map(|t| Self::from_ast(t, resolutions))
                    .unwrap_or(Self::Unit)),
            },
            TyKind::Infer => Self::Named("_".into()),
            _ => Self::Named("_".into()),
        }
    }

    fn from_registry(id: NativeFnId, registry: &Registry) -> Option<Self> {
        let name = registry.native_return_name(id)?;
        Some(Self::from_name(&name))
    }

    fn from_shape(shape: &'static facet::Shape, registry: &Registry) -> Self {
        Self::from_name(&registry.shape_xur_name(shape))
    }

    fn from_name(name: &str) -> Self {
        match name {
            "bool" => Self::Bool,
            "i64" | "u8" | "u32" | "u64" | "usize" => Self::Int,
            "f64" => Self::Float,
            "str" => Self::Str,
            "bytes" => Self::Bytes,
            "list" => Self::List,
            "map" => Self::Map,
            "record" => Self::Record,
            "()" => Self::Unit,
            "module" => Self::Module,
            _ if name.starts_with('*') => {
                Self::Ptr(Box::new(Self::from_name(&name[1..])))
            }
            _ => Self::Named(name.to_string()),
        }
    }

    fn is_uninferrable(&self) -> bool {
        matches!(self, Self::Named(n) if n == "_")
    }
}

impl fmt::Display for InferredTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(n) => f.write_str(n),
            Self::Unit => f.write_str("()"),
            Self::Bool => f.write_str("bool"),
            Self::Int => f.write_str("i64"),
            Self::Float => f.write_str("f64"),
            Self::Str => f.write_str("str"),
            Self::Bytes => f.write_str("bytes"),
            Self::Ptr(inner) => write!(f, "*{inner}"),
            Self::List => f.write_str("list"),
            Self::Map => f.write_str("map"),
            Self::Tuple(elems) => {
                write!(f, "(")?;
                for (i, t) in elems.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{t}")?;
                }
                write!(f, ")")
            }
            Self::Fn { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{p}")?;
                }
                if matches!(**ret, Self::Unit) {
                    write!(f, ")")
                } else {
                    write!(f, ") -> {ret}")
                }
            }
            Self::Record => f.write_str("record"),
            Self::Module => f.write_str("module"),
        }
    }
}

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

pub struct MethodAt {
    pub method_name: String,
    pub method_span: Span,
    pub receiver_ty: Option<String>,
}

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
    local_types: &'a HashMap<NodeId, InferredTy>,
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
                    receiver_ty: receiver_ty.map(|t| t.to_string()),
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
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<InferredTy> {
    match &expr.kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Str(_) => Some(InferredTy::Str),
            LitKind::Int(_) => Some(InferredTy::Int),
            LitKind::Float(_) => Some(InferredTy::Float),
            LitKind::Bool(_) => Some(InferredTy::Bool),
            LitKind::ByteStr(_) => Some(InferredTy::Bytes),
            LitKind::Err => None,
        },
        ExprKind::List(_) => Some(InferredTy::List),
        ExprKind::Map(_) => Some(InferredTy::Map),
        ExprKind::Record(_) => Some(InferredTy::Record),
        ExprKind::Tuple(elems) => {
            if elems.is_empty() {
                return Some(InferredTy::Unit);
            }
            let types: Vec<_> = elems.iter()
                .map(|e| infer_expr_type(e, resolutions, items, registry, local_types)
                    .unwrap_or(InferredTy::Named("_".into())))
                .collect();
            Some(InferredTy::Tuple(types))
        }
        ExprKind::Struct(s) => {
            let res = resolutions.res.get(&s.path.id)?;
            match res {
                Res::Def(_, def) => {
                    Some(InferredTy::Named(resolutions.defs.get(*def).name.to_string()))
                }
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
                                FnRetTy::Ty(ty) => Some(InferredTy::from_ast(ty, resolutions)),
                                FnRetTy::Default(_) => Some(InferredTy::Unit),
                            }
                        } else {
                            None
                        }
                    }
                    Res::Def(DefKind::NativeFn(id), _) => {
                        if let Some(ty) = InferredTy::from_registry(*id, registry) {
                            return Some(ty);
                        }
                        path.segments.last()
                            .and_then(|seg| seg.args.as_ref())
                            .and_then(|args| args.args.first())
                            .map(|ty| InferredTy::from_ast(ty, resolutions))
                    }
                    Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
                        Some(InferredTy::Named(resolutions.defs.get(*def).name.to_string()))
                    }
                    Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
                        let parent = resolutions.defs.get(*def).parent?;
                        Some(InferredTy::Named(resolutions.defs.get(parent).name.to_string()))
                    }
                    Res::Local(node_id) => {
                        local_types.get(node_id)?.call_return()
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        ExprKind::MethodCall(mc) => {
            let receiver_ty = infer_expr_type(&mc.receiver, resolutions, items, registry, local_types)?;
            let method = mc.seg.ident.name.as_str();
            if let Some(ty) = method_return_type(&receiver_ty, method, registry) {
                return Some(ty);
            }
            if let InferredTy::Named(ref name) = receiver_ty {
                if let Some(ty) = user_method_return_type(
                    name, method, &mc.receiver, resolutions, items, registry,
                ) {
                    return Some(ty);
                }
                if resolutions.find_type(name).is_some() {
                    if let Some(id) = registry.builtin_method(BuiltinType::Struct, method) {
                        return InferredTy::from_registry(id, registry);
                    }
                }
            }
            None
        }
        ExprKind::Deref(inner) => {
            infer_expr_type(inner, resolutions, items, registry, local_types)
                .and_then(|t| t.deref_target())
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
                    return Some(InferredTy::from_shape(r.t(), registry));
                }
                if let Ok(o) = shape.def.into_option() {
                    return Some(InferredTy::from_shape(o.t(), registry));
                }
                return Some(InferredTy::from_shape(shape, registry));
            }
            infer_expr_type(inner, resolutions, items, registry, local_types)
                .and_then(|t| match t {
                    InferredTy::Named(ref n) if n == "Result" || n == "Option" => None,
                    other => Some(other),
                })
        }
        ExprKind::Cast(_, ty) => Some(InferredTy::from_ast(ty, resolutions)),
        ExprKind::Closure(closure) => {
            let params: Vec<_> = closure.decl.inputs.iter()
                .map(|p| InferredTy::from_ast(&p.ty, resolutions))
                .collect();
            let ret = match &closure.decl.output {
                FnRetTy::Ty(ty) => InferredTy::from_ast(ty, resolutions),
                FnRetTy::Default(_) => {
                    infer_expr_type(&closure.body, resolutions, items, registry, local_types)
                        .unwrap_or(InferredTy::Named("_".into()))
                }
            };
            if params.iter().all(|p| p.is_uninferrable()) && ret.is_uninferrable() {
                return None;
            }
            Some(InferredTy::Fn { params, ret: Box::new(ret) })
        }
        ExprKind::Binary(op, lhs, rhs) => match op.kind {
            BinOpKind::Eq | BinOpKind::Ne | BinOpKind::Lt | BinOpKind::Le
            | BinOpKind::Gt | BinOpKind::Ge | BinOpKind::And | BinOpKind::Or => {
                Some(InferredTy::Bool)
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
                Res::Def(DefKind::Mod, _) => Some(InferredTy::Module),
                _ => None,
            }
        }
        _ => None,
    }
}

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
            let id = resolve_method_by_name(&receiver_name, method, registry)?;
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
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<InferredTy> {
    match &iter.kind {
        ExprKind::Range(..) => Some(InferredTy::Int),
        ExprKind::List(elems) => {
            let mut seen = Vec::new();
            for e in elems {
                if let Some(ty) = infer_expr_type(e, resolutions, items, registry, local_types) {
                    if !seen.contains(&ty) {
                        seen.push(ty);
                    }
                }
            }
            if seen.len() == 1 {
                Some(seen.remove(0))
            } else {
                None
            }
        }
        _ => {
            if let Some(shape) = native_call_return_shape(iter, resolutions, registry) {
                if let Ok(list) = shape.def.into_list() {
                    return Some(InferredTy::from_shape(list.t(), registry));
                }
            }
            None
        }
    }
}

fn user_method_return_type(
    receiver_name: &str,
    method: &str,
    receiver_expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
) -> Option<InferredTy> {
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
                        let name = registry.shape_type_param(shape, index)?;
                        return Some(InferredTy::from_name(&name));
                    }
                    Some(InferredTy::from_ast(ty, resolutions))
                }
                FnRetTy::Default(_) => Some(InferredTy::Unit),
            }
        }
        Res::Def(DefKind::NativeFn(id), _) => InferredTy::from_registry(id, registry),
        _ => None,
    }
}

fn type_param_position(resolutions: &Resolutions, param_def: DefId) -> Option<usize> {
    let parent = resolutions.defs.get(param_def).parent;
    let all: Vec<_> = resolutions.defs.iter().collect();
    let idx = all.iter().position(|(id, _)| *id == param_def)?;
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

fn resolve_method(ty: &InferredTy, method: &str, registry: &Registry) -> Option<NativeFnId> {
    if let Some(builtin) = ty.builtin() {
        return registry.builtin_method(builtin, method);
    }
    if let InferredTy::Named(name) = ty {
        let type_id = registry.type_id_by_name(name)?;
        return registry.type_function(type_id, method);
    }
    None
}

fn resolve_method_by_name(name: &str, method: &str, registry: &Registry) -> Option<NativeFnId> {
    resolve_method(&InferredTy::from_name(name), method, registry)
}

fn method_return_type(
    receiver_ty: &InferredTy,
    method: &str,
    registry: &Registry,
) -> Option<InferredTy> {
    let id = resolve_method(receiver_ty, method, registry)?;
    InferredTy::from_registry(id, registry)
}

pub fn method_signature(
    receiver_ty: Option<&str>,
    method: &str,
    registry: &Registry,
) -> Option<String> {
    let receiver = receiver_ty?;
    let id = resolve_method_by_name(receiver, method, registry)?;
    let native_fn = registry.function(id);
    let sig = native_fn.signature.as_ref()?;
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
