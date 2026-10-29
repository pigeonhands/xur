use std::collections::HashMap;
use std::fmt;

use xur_ast::{BinOpKind, Expr, ExprKind, FnRetTy, ItemKind, LitKind, Local, NodeId, PatKind, Ty, TyKind};
use xur_ast::visit::{self, Visitor};
use xur_bytecode::{ItemIndex, ItemRef};
use xur_resolve::{DefId, DefKind, NativeFnId, NativeTypeId, PrimTy, Program, Res, Resolutions};
use xur_runtime::{BuiltinType, Registry};
use xur_span::{Ident, Span, Symbol};

#[derive(Clone, PartialEq)]
pub enum InferredTy {
    Def(DefId),
    Prim(PrimTy),
    /// Registry bridge: a native type name with no DefId (from facet shapes).
    Named(String),
    Infer,
    Unit,
    Ptr(Box<InferredTy>),
    List(Option<Box<InferredTy>>),
    Map,
    Tuple(Vec<InferredTy>),
    Fn { params: Vec<InferredTy>, ret: Box<InferredTy> },
    Record,
    Module,
}

impl InferredTy {
    fn builtin(&self) -> Option<BuiltinType> {
        Some(match self {
            Self::Prim(p) => match p {
                PrimTy::Bool => BuiltinType::Bool,
                PrimTy::Str => BuiltinType::Str,
                PrimTy::Bytes => BuiltinType::Bytes,
                PrimTy::I8 | PrimTy::I16 | PrimTy::I32 | PrimTy::I64 | PrimTy::Isize
                | PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64 | PrimTy::Usize => {
                    BuiltinType::Int
                }
                PrimTy::F32 | PrimTy::F64 => BuiltinType::Float,
                PrimTy::Type => return None,
            },
            Self::List(_) => BuiltinType::List,
            Self::Map => BuiltinType::Map,
            Self::Record => BuiltinType::Record,
            Self::Ptr(_) => BuiltinType::Ptr,
            _ => return None,
        })
    }

    fn def_id(&self) -> Option<DefId> {
        match self {
            Self::Def(id) => Some(*id),
            _ => None,
        }
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

    fn from_prim(prim: PrimTy) -> Self {
        if prim == PrimTy::Type {
            Self::Infer
        } else {
            Self::Prim(prim)
        }
    }

    pub fn from_ast(ty: &Ty, resolutions: &Resolutions) -> Self {
        match &ty.kind {
            TyKind::Path(path) => {
                let res = resolutions.res.get(&path.id);
                match res {
                    Some(Res::PrimTy(prim)) => Self::from_prim(*prim),
                    Some(Res::Def(DefKind::Mod, _)) => Self::Module,
                    Some(Res::Def(_, def)) => Self::Def(*def),
                    _ => Self::Infer,
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
            TyKind::List(inner) => Self::List(Some(Box::new(Self::from_ast(inner, resolutions)))),
            TyKind::Record => Self::Record,
            TyKind::Ptr(inner) => Self::Ptr(Box::new(Self::from_ast(inner, resolutions))),
            TyKind::Fn { params, ret } => Self::Fn {
                params: params.iter().map(|t| Self::from_ast(t, resolutions)).collect(),
                ret: Box::new(ret.as_ref()
                    .map(|t| Self::from_ast(t, resolutions))
                    .unwrap_or(Self::Unit)),
            },
            TyKind::Infer => Self::Infer,
            _ => Self::Infer,
        }
    }

    fn from_registry(
        id: NativeFnId,
        registry: &Registry,
        resolutions: &Resolutions,
    ) -> Option<Self> {
        let f = registry.function(id);
        if let Some(sig) = &f.signature {
            return Some(Self::from_shape(sig.ret, registry, resolutions));
        }
        match f.return_hint {
            Some(xur_runtime::registry::ReturnHint::Shape(s)) => {
                Some(Self::from_shape(s, registry, resolutions))
            }
            Some(xur_runtime::registry::ReturnHint::Xur(xur)) => {
                Some(Self::from_xur_type(xur, registry, resolutions))
            }
            None => None,
        }
    }

    fn from_xur_type(
        xur: xur_runtime::registry::XurType,
        registry: &Registry,
        resolutions: &Resolutions,
    ) -> Self {
        use xur_runtime::registry::{ElemHint, XurType};
        match xur {
            XurType::Unit => Self::Unit,
            XurType::Ptr(Some(inner)) => {
                Self::Ptr(Box::new(Self::from_shape(inner, registry, resolutions)))
            }
            XurType::Ptr(None) => Self::Ptr(Box::new(Self::Infer)),
            XurType::List(Some(ElemHint::Shape(s))) => {
                Self::List(Some(Box::new(Self::from_shape(s, registry, resolutions))))
            }
            XurType::List(Some(ElemHint::Name(n))) => {
                Self::List(Some(Box::new(Self::from_name(n))))
            }
            XurType::List(Some(ElemHint::Param(_))) | XurType::List(None) => Self::List(None),
            XurType::Map(_, _) => Self::Map,
            XurType::Named(_, _) => Self::Infer,
            XurType::Record => Self::Record,
            XurType::Module => Self::Module,
        }
    }

    pub fn from_shape(
        shape: &'static facet::Shape,
        registry: &Registry,
        resolutions: &Resolutions,
    ) -> Self {
        if let Some(ty_id) = registry.type_of_shape(shape) {
            if let Some(def) = def_of_native_type(ty_id, resolutions) {
                return Self::Def(def);
            }
        }
        if let Ok(list) = shape.def.into_list() {
            let elem = Self::from_shape(list.t(), registry, resolutions);
            if matches!(elem, Self::Prim(PrimTy::U8)) {
                return Self::Prim(PrimTy::Bytes);
            }
            return Self::List(Some(Box::new(elem)));
        }
        Self::from_rust_shape(shape)
    }

    fn from_rust_shape(shape: &'static facet::Shape) -> Self {
        let name = shape.type_name().to_string();
        match name.as_str() {
            "String" | "&str" | "alloc::string::String" => Self::Prim(PrimTy::Str),
            "()" => Self::Unit,
            "Vec<u8>" | "alloc::vec::Vec<u8>" => Self::Prim(PrimTy::Bytes),
            _ if name.starts_with("Vec<") || name == "Vec" => {
                if let Ok(list) = shape.def.into_list() {
                    Self::List(Some(Box::new(Self::from_rust_shape(list.t()))))
                } else {
                    Self::List(None)
                }
            }
            _ => {
                if let Some(prim) = PrimTy::from_name(Symbol::intern(&name)) {
                    Self::from_prim(prim)
                } else {
                    Self::Named(name)
                }
            }
        }
    }

    /// Bridge for registry APIs that only return a string name (e.g. shape_type_param).
    fn from_name(name: &str) -> Self {
        if let Some(prim) = PrimTy::from_name(Symbol::intern(name)) {
            return Self::from_prim(prim);
        }
        match name {
            "list" => Self::List(None),
            "map" => Self::Map,
            "record" => Self::Record,
            "()" => Self::Unit,
            "module" => Self::Module,
            "_" => Self::Infer,
            _ if name.starts_with('*') => {
                Self::Ptr(Box::new(Self::from_name(&name[1..])))
            }
            _ => Self::Named(name.to_string()),
        }
    }

    pub fn display<'a>(&'a self, resolutions: &'a Resolutions) -> DisplayTy<'a> {
        DisplayTy(self, resolutions)
    }
}

/// Wrapper for displaying InferredTy with access to resolutions.
pub struct DisplayTy<'a>(&'a InferredTy, &'a Resolutions);

impl fmt::Display for DisplayTy<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            InferredTy::Def(id) => {
                f.write_str(self.1.defs.get(*id).name.as_str())
            }
            InferredTy::Named(n) => f.write_str(n),
            InferredTy::Ptr(inner) => write!(f, "*{}", inner.display(self.1)),
            InferredTy::List(Some(inner)) => write!(f, "list[{}]", inner.display(self.1)),
            InferredTy::Tuple(elems) => {
                write!(f, "(")?;
                for (i, t) in elems.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", t.display(self.1))?;
                }
                write!(f, ")")
            }
            InferredTy::Fn { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", p.display(self.1))?;
                }
                if matches!(**ret, InferredTy::Unit) {
                    write!(f, ")")
                } else {
                    write!(f, ") -> {}", ret.display(self.1))
                }
            }
            other => write!(f, "{other}"),
        }
    }
}

/// Display for self-describing variants (no resolutions needed).
impl fmt::Display for InferredTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("()"),
            Self::Prim(p) => f.write_str(p.name()),
            Self::List(Some(inner)) => write!(f, "list[{inner}]"),
            Self::List(None) => f.write_str("list"),
            Self::Map => f.write_str("map"),
            Self::Record => f.write_str("record"),
            Self::Module => f.write_str("module"),
            Self::Infer => f.write_str("_"),
            Self::Named(n) => f.write_str(n),
            Self::Def(_) => f.write_str("_"),
            Self::Ptr(inner) => write!(f, "*{inner}"),
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
        }
    }
}

fn def_of_native_type(ty_id: NativeTypeId, resolutions: &Resolutions) -> Option<DefId> {
    resolutions.defs.iter().find_map(|(id, data)| {
        matches!(data.kind, DefKind::NativeType(nid) if nid == ty_id).then_some(id)
    })
}

pub struct LetBinding<'a> {
    pub pat_end: usize,
    pub binding_id: Option<NodeId>,
    pub init: &'a Expr,
    pub ty: Option<&'a Ty>,
}

pub struct HintCollector<'a> {
    pub calls: Vec<(NodeId, Vec<Span>)>,
    pub lets: Vec<LetBinding<'a>>,
    pub chains: Vec<(usize, usize, &'a Expr)>,
    pub matches: Vec<(usize, &'a Expr)>,
    pub match_arms: Vec<(usize, NodeId, &'a Expr, NodeId)>,
    pub for_loops: Vec<(usize, Option<NodeId>, &'a Expr)>,
}

impl<'a> HintCollector<'a> {
    pub fn new() -> Self {
        Self {
            calls: Vec::new(),
            lets: Vec::new(),
            chains: Vec::new(),
            matches: Vec::new(),
            match_arms: Vec::new(),
            for_loops: Vec::new(),
        }
    }
}

impl<'ast> Visitor<'ast> for HintCollector<'ast> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match &expr.kind {
            ExprKind::Call(callee, args) if matches!(callee.kind, ExprKind::Path(_)) => {
                if let ExprKind::Path(path) = &callee.kind {
                    self.calls.push((path.id, args.iter().map(|a| a.span).collect()));
                }
            }
            ExprKind::MethodCall(mc)
                if matches!(mc.receiver.kind, ExprKind::MethodCall(_) | ExprKind::Call(_, _)) =>
            {
                self.chains.push((mc.receiver.span.end, mc.seg.ident.span.start, &mc.receiver));
            }
            ExprKind::Match(scrutinee, arms) => {
                self.matches.push((scrutinee.span.end, scrutinee));
                for arm in arms {
                    if let PatKind::TupleStruct(path, pats) = &arm.pat.kind
                        && pats.len() == 1
                        && let PatKind::Ident(_) = &pats[0].kind
                    {
                        self.match_arms.push((pats[0].span.end, pats[0].id, scrutinee, path.id));
                    }
                }
            }
            ExprKind::ForLoop { pat, iter, .. } => {
                let binding_id = match &pat.kind {
                    PatKind::Ident(_) => Some(pat.id),
                    _ => None,
                };
                self.for_loops.push((pat.span.end, binding_id, iter));
            }
            _ => {}
        }
        visit::walk_expr(self, expr);
    }

    fn visit_local(&mut self, local: &'ast Local) {
        if let Some(init) = &local.init {
            let binding_id = match &local.pat.kind {
                PatKind::Ident(_) => Some(local.pat.id),
                _ => None,
            };
            self.lets.push(LetBinding {
                pat_end: local.pat.span.end,
                binding_id,
                init,
                ty: local.ty.as_ref(),
            });
        }
        visit::walk_local(self, local);
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
                    receiver_ty: receiver_ty.map(|t| t.display(self.resolutions).to_string()),
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
            LitKind::Str(_) => Some(InferredTy::Prim(PrimTy::Str)),
            LitKind::Int(_) => Some(InferredTy::Prim(PrimTy::I64)),
            LitKind::Float(_) => Some(InferredTy::Prim(PrimTy::F64)),
            LitKind::Bool(_) => Some(InferredTy::Prim(PrimTy::Bool)),
            LitKind::ByteStr(_) => Some(InferredTy::Prim(PrimTy::Bytes)),
            LitKind::Err => None,
        },
        ExprKind::List(_) => Some(InferredTy::List(None)),
        ExprKind::Map(_) => Some(InferredTy::Map),
        ExprKind::Record(_) => Some(InferredTy::Record),
        ExprKind::Tuple(elems) => {
            if elems.is_empty() {
                return Some(InferredTy::Unit);
            }
            let types: Vec<_> = elems.iter()
                .map(|e| infer_expr_type(e, resolutions, items, registry, local_types)
                    .unwrap_or(InferredTy::Infer))
                .collect();
            Some(InferredTy::Tuple(types))
        }
        ExprKind::Struct(s) => {
            let res = resolutions.res.get(&s.path.id)?;
            match res {
                Res::Def(_, def) => Some(InferredTy::Def(*def)),
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
                        if let Some(ty) = InferredTy::from_registry(*id, registry, resolutions) {
                            return Some(ty);
                        }
                        path.segments.last()
                            .and_then(|seg| seg.args.as_ref())
                            .and_then(|args| args.args.first())
                            .map(|ty| InferredTy::from_ast(ty, resolutions))
                    }
                    Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
                        Some(InferredTy::Def(*def))
                    }
                    Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
                        let parent = resolutions.defs.get(*def).parent?;
                        Some(InferredTy::Def(parent))
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
            if let Some(id) = resolve_method(&receiver_ty, method, resolutions, registry) {
                if let Some(ty) = InferredTy::from_registry(id, registry, resolutions) {
                    return Some(resolve_params(ty, &receiver_ty));
                }
            }
            if let Some(def) = receiver_ty.def_id() {
                if let Some(ty) = user_method_return_type(
                    def, method, &mc.receiver, resolutions, items, registry,
                ) {
                    return Some(ty);
                }
                if let Some(id) = registry.builtin_method(BuiltinType::Struct, method) {
                    return InferredTy::from_registry(id, registry, resolutions);
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
            let shape = return_shape(inner, resolutions, items, registry, local_types);
            if let Some(shape) = shape {
                if let Ok(r) = shape.def.into_result() {
                    return Some(InferredTy::from_shape(r.t(), registry, resolutions));
                }
                if let Ok(o) = shape.def.into_option() {
                    return Some(InferredTy::from_shape(o.t(), registry, resolutions));
                }
                return Some(InferredTy::from_shape(shape, registry, resolutions));
            }
            None
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
                        .unwrap_or(InferredTy::Infer)
                }
            };
            if params.iter().all(|p| matches!(p, InferredTy::Infer)) && matches!(ret, InferredTy::Infer) {
                return None;
            }
            Some(InferredTy::Fn { params, ret: Box::new(ret) })
        }
        ExprKind::Binary(op, lhs, rhs) => match op.kind {
            BinOpKind::Eq | BinOpKind::Ne | BinOpKind::Lt | BinOpKind::Le
            | BinOpKind::Gt | BinOpKind::Ge | BinOpKind::And | BinOpKind::Or => {
                Some(InferredTy::Prim(PrimTy::Bool))
            }
            _ => {
                infer_expr_type(lhs, resolutions, items, registry, local_types)
                    .or_else(|| infer_expr_type(rhs, resolutions, items, registry, local_types))
            }
        },
        ExprKind::Unary(_, inner) => infer_expr_type(inner, resolutions, items, registry, local_types),
        ExprKind::Index(collection, index) => {
            let col_ty = infer_expr_type(collection, resolutions, items, registry, local_types)?;
            if matches!(index.kind, ExprKind::Range(..)) {
                return Some(col_ty);
            }
            match col_ty {
                InferredTy::List(Some(inner)) => Some(*inner),
                InferredTy::Prim(PrimTy::Str) => Some(InferredTy::Prim(PrimTy::Str)),
                InferredTy::Prim(PrimTy::Bytes) => Some(InferredTy::Prim(PrimTy::U8)),
                _ => None,
            }
        }
        ExprKind::Path(path) => {
            let res = resolutions.res.get(&path.id)?;
            match res {
                Res::Local(node_id) => local_types.get(node_id).cloned(),
                Res::Def(DefKind::Mod, _) => Some(InferredTy::Module),
                Res::Def(DefKind::Fn, def) => {
                    if let Some(ItemRef::Fn { item, .. }) = items.get(*def) {
                        let params: Vec<_> = item.decl.inputs.iter()
                            .map(|p| InferredTy::from_ast(&p.ty, resolutions))
                            .collect();
                        let ret = match &item.decl.output {
                            FnRetTy::Ty(ty) => InferredTy::from_ast(ty, resolutions),
                            FnRetTy::Default(_) => InferredTy::Unit,
                        };
                        Some(InferredTy::Fn { params, ret: Box::new(ret) })
                    } else {
                        None
                    }
                }
                Res::Def(DefKind::NativeFn(id), _) => {
                    let f = registry.function(*id);
                    if let Some(sig) = &f.signature {
                        let params: Vec<_> = sig.params.iter()
                            .map(|s| InferredTy::from_shape(s, registry, resolutions))
                            .collect();
                        let ret = InferredTy::from_shape(sig.ret, registry, resolutions);
                        Some(InferredTy::Fn { params, ret: Box::new(ret) })
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn return_shape(
    expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<&'static facet::Shape> {
    native_call_return_shape(expr, resolutions, registry)
        .or_else(|| inferred_return_shape(expr, resolutions, items, registry, local_types))
}

/// Infer the type of a match arm binding (e.g. `m` in `Ok(m)`) by decomposing
/// the scrutinee's return type.
pub fn infer_match_arm_type(
    scrutinee: &Expr,
    variant_def: DefId,
    lang: &xur_resolve::LangItems,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<InferredTy> {
    // Shape-based: works for typed Rust functions returning Result<T, E> / Option<T>.
    if let Some(shape) = return_shape(scrutinee, resolutions, items, registry, local_types) {
        if let Ok(r) = shape.def.into_result() {
            if variant_def == lang.ok {
                return Some(InferredTy::from_shape(r.t(), registry, resolutions));
            }
            if variant_def == lang.err {
                return Some(InferredTy::from_shape(r.e(), registry, resolutions));
            }
        }
        if let Ok(o) = shape.def.into_option() {
            if variant_def == lang.some {
                return Some(InferredTy::from_shape(o.t(), registry, resolutions));
            }
        }
    }
    // XurType-based: for raw functions that declare Result/Option return hints.
    let id = resolve_callee_id(scrutinee, resolutions, items, registry, local_types)?;
    let hint = registry.function(id).return_hint?;
    use xur_runtime::registry::{ElemHint, ReturnHint, XurType};
    if let ReturnHint::Xur(XurType::Named(_, params)) = hint {
        let index = if variant_def == lang.ok || variant_def == lang.some {
            Some(0)
        } else if variant_def == lang.err {
            Some(1)
        } else {
            None
        };
        if let Some(i) = index {
            if let Some(&hint) = params.get(i) {
                return Some(match hint {
                    ElemHint::Shape(s) => InferredTy::from_shape(s, registry, resolutions),
                    ElemHint::Name(n) => InferredTy::from_name(n),
                    ElemHint::Param(_) => InferredTy::Infer,
                });
            }
        }
    }
    None
}

fn resolve_callee_id(
    expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<NativeFnId> {
    match &expr.kind {
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(path) = &callee.kind
                && let Some(Res::Def(DefKind::NativeFn(id), _)) = resolutions.res.get(&path.id)
            {
                return Some(*id);
            }
            None
        }
        ExprKind::MethodCall(mc) => {
            let receiver_ty = infer_expr_type(&mc.receiver, resolutions, items, registry, local_types)?;
            resolve_method(&receiver_ty, mc.seg.ident.name.as_str(), resolutions, registry)
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
            let method = mc.seg.ident.name.as_str();
            let ty = InferredTy::from_shape(receiver_shape, registry, resolutions);
            let id = resolve_method(&ty, method, resolutions, registry)?;
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

/// Fallback for [`native_call_return_shape`]: resolves the callee's return shape
/// via type inference when the receiver is a local variable.
fn inferred_return_shape(
    expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    local_types: &HashMap<NodeId, InferredTy>,
) -> Option<&'static facet::Shape> {
    match &expr.kind {
        ExprKind::MethodCall(mc) => {
            let receiver_ty = infer_expr_type(&mc.receiver, resolutions, items, registry, local_types)?;
            let method = mc.seg.ident.name.as_str();
            let id = resolve_method(&receiver_ty, method, resolutions, registry)?;
            registry.native_return_shape(id)
        }
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(path) = &callee.kind
                && let Some(Res::Def(DefKind::NativeFn(id), _)) = resolutions.res.get(&path.id)
            {
                return registry.native_return_shape(*id);
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
        ExprKind::Range(..) => Some(InferredTy::Prim(PrimTy::I64)),
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
                    return Some(InferredTy::from_shape(list.t(), registry, resolutions));
                }
            }
            match infer_expr_type(iter, resolutions, items, registry, local_types) {
                Some(InferredTy::List(Some(inner))) => return Some(*inner),
                Some(InferredTy::Prim(PrimTy::Bytes)) => return Some(InferredTy::Prim(PrimTy::U8)),
                Some(InferredTy::Prim(PrimTy::Str)) => return Some(InferredTy::Prim(PrimTy::Str)),
                _ => {}
            }
            None
        }
    }
}

fn user_method_return_type(
    type_def: DefId,
    method: &str,
    receiver_expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
) -> Option<InferredTy> {
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
        Res::Def(DefKind::NativeFn(id), _) => InferredTy::from_registry(id, registry, resolutions),
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

/// Substitute `ElemHint::Param(n)` in a return type with the nth type parameter
/// of the receiver. For a `List(Some(elem))` receiver, param 0 is `elem`.
fn resolve_params(ret: InferredTy, receiver: &InferredTy) -> InferredTy {
    match ret {
        InferredTy::List(None) => {
            if let InferredTy::List(Some(inner)) = receiver {
                InferredTy::List(Some(inner.clone()))
            } else {
                ret
            }
        }
        _ => ret,
    }
}

fn resolve_method(
    ty: &InferredTy,
    method: &str,
    resolutions: &Resolutions,
    registry: &Registry,
) -> Option<NativeFnId> {
    if let Some(builtin) = ty.builtin() {
        return registry.builtin_method(builtin, method);
    }
    if let Some(def) = ty.def_id() {
        return resolutions.assoc_fn(def, Symbol::intern(method))
            .and_then(|res| match res {
                Res::Def(DefKind::NativeFn(id), _) => Some(id),
                _ => None,
            });
    }
    if let InferredTy::Named(name) = ty {
        let type_id = registry.type_id_by_name(name)?;
        return registry.type_function(type_id, method);
    }
    None
}


pub fn method_signature(
    receiver_ty: Option<&str>,
    method: &str,
    registry: &Registry,
) -> Option<String> {
    let receiver = receiver_ty?;
    let ty = InferredTy::from_name(receiver);
    let empty = Resolutions::empty();
    let id = resolve_method(&ty, method, &empty, registry)?;
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
