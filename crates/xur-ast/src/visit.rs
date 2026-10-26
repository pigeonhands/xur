//! Read-only traversal of the AST, after `rustc_ast::visit`.
//!
//! Each `visit_*` method defaults to the matching `walk_*` function, which
//! visits the node's children. A pass overrides the methods for the nodes it
//! cares about and calls `walk_*` itself to keep descending.

use crate::{
    Arm, Block, Expr, ExprKind, FieldDef, FnDecl, FnRetTy, GenericArgs, GenericParam, Generics,
    Item, ItemKind, Local, Module, Param, Pat, PatKind, Path, Stmt, StmtKind, Ty, TyKind, Variant,
    VariantData,
};

pub trait Visitor: Sized {
    fn visit_module(&mut self, module: &Module) {
        walk_module(self, module);
    }
    fn visit_item(&mut self, item: &Item) {
        walk_item(self, item);
    }
    fn visit_generics(&mut self, generics: &Generics) {
        walk_generics(self, generics);
    }
    fn visit_generic_param(&mut self, _param: &GenericParam) {}
    fn visit_fn_decl(&mut self, decl: &FnDecl) {
        walk_fn_decl(self, decl);
    }
    fn visit_param(&mut self, param: &Param) {
        walk_param(self, param);
    }
    fn visit_variant_data(&mut self, data: &VariantData) {
        walk_variant_data(self, data);
    }
    fn visit_variant(&mut self, variant: &Variant) {
        walk_variant(self, variant);
    }
    fn visit_field_def(&mut self, field: &FieldDef) {
        walk_field_def(self, field);
    }
    fn visit_block(&mut self, block: &Block) {
        walk_block(self, block);
    }
    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }
    fn visit_local(&mut self, local: &Local) {
        walk_local(self, local);
    }
    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }
    fn visit_arm(&mut self, arm: &Arm) {
        walk_arm(self, arm);
    }
    fn visit_pat(&mut self, pat: &Pat) {
        walk_pat(self, pat);
    }
    fn visit_ty(&mut self, ty: &Ty) {
        walk_ty(self, ty);
    }
    fn visit_path(&mut self, path: &Path) {
        walk_path(self, path);
    }
    fn visit_generic_args(&mut self, args: &GenericArgs) {
        walk_generic_args(self, args);
    }
}

pub fn walk_module<V: Visitor>(visitor: &mut V, module: &Module) {
    for item in &module.items {
        visitor.visit_item(item);
    }
}

/// Attributes are not walked: their paths name attributes, not items.
pub fn walk_item<V: Visitor>(visitor: &mut V, item: &Item) {
    match &item.kind {
        ItemKind::Use(tree) => visitor.visit_path(&tree.path),
        ItemKind::Const(item) => {
            if let Some(ty) = &item.ty {
                visitor.visit_ty(ty);
            }
            visitor.visit_expr(&item.expr);
        }
        ItemKind::Fn(item) => {
            visitor.visit_generics(&item.generics);
            visitor.visit_fn_decl(&item.decl);
            visitor.visit_block(&item.body);
        }
        ItemKind::Struct(_, generics, data) => {
            visitor.visit_generics(generics);
            visitor.visit_variant_data(data);
        }
        ItemKind::Enum(_, generics, def) => {
            visitor.visit_generics(generics);
            for variant in &def.variants {
                visitor.visit_variant(variant);
            }
        }
        ItemKind::Impl(imp) => {
            visitor.visit_generics(&imp.generics);
            visitor.visit_ty(&imp.self_ty);
            for item in &imp.items {
                visitor.visit_item(item);
            }
        }
    }
}

pub fn walk_generics<V: Visitor>(visitor: &mut V, generics: &Generics) {
    for param in &generics.params {
        visitor.visit_generic_param(param);
    }
}

pub fn walk_fn_decl<V: Visitor>(visitor: &mut V, decl: &FnDecl) {
    for param in &decl.inputs {
        visitor.visit_param(param);
    }
    if let FnRetTy::Ty(ty) = &decl.output {
        visitor.visit_ty(ty);
    }
}

pub fn walk_param<V: Visitor>(visitor: &mut V, param: &Param) {
    visitor.visit_pat(&param.pat);
    visitor.visit_ty(&param.ty);
}

pub fn walk_variant_data<V: Visitor>(visitor: &mut V, data: &VariantData) {
    match data {
        VariantData::Struct(fields) | VariantData::Tuple(fields) => {
            for field in fields {
                visitor.visit_field_def(field);
            }
        }
        VariantData::Unit => {}
    }
}

pub fn walk_variant<V: Visitor>(visitor: &mut V, variant: &Variant) {
    visitor.visit_variant_data(&variant.data);
}

pub fn walk_field_def<V: Visitor>(visitor: &mut V, field: &FieldDef) {
    visitor.visit_ty(&field.ty);
}

pub fn walk_block<V: Visitor>(visitor: &mut V, block: &Block) {
    for stmt in &block.stmts {
        visitor.visit_stmt(stmt);
    }
}

pub fn walk_stmt<V: Visitor>(visitor: &mut V, stmt: &Stmt) {
    match &stmt.kind {
        StmtKind::Let(local) => visitor.visit_local(local),
        StmtKind::Item(item) => visitor.visit_item(item),
        StmtKind::Expr(expr) | StmtKind::Semi(expr) => visitor.visit_expr(expr),
        StmtKind::Empty => {}
    }
}

/// Visits the initializer before the pattern, since the pattern's bindings
/// are not in scope in it.
pub fn walk_local<V: Visitor>(visitor: &mut V, local: &Local) {
    if let Some(ty) = &local.ty {
        visitor.visit_ty(ty);
    }
    if let Some(init) = &local.init {
        visitor.visit_expr(init);
    }
    visitor.visit_pat(&local.pat);
}

pub fn walk_expr<V: Visitor>(visitor: &mut V, expr: &Expr) {
    match &expr.kind {
        ExprKind::Lit(_) | ExprKind::Continue | ExprKind::Err => {}
        ExprKind::Path(path) => visitor.visit_path(path),
        ExprKind::Paren(inner) | ExprKind::Unary(_, inner) | ExprKind::Try(inner) => {
            visitor.visit_expr(inner);
        }
        ExprKind::Field(receiver, _) => visitor.visit_expr(receiver),
        ExprKind::Tuple(exprs) | ExprKind::List(exprs) => {
            for expr in exprs {
                visitor.visit_expr(expr);
            }
        }
        ExprKind::Binary(_, lhs, rhs)
        | ExprKind::Assign(lhs, rhs, _)
        | ExprKind::Index(lhs, rhs) => {
            visitor.visit_expr(lhs);
            visitor.visit_expr(rhs);
        }
        ExprKind::Range(start, end, _) => {
            for expr in [start, end].into_iter().flatten() {
                visitor.visit_expr(expr);
            }
        }
        ExprKind::Call(callee, args) => {
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        ExprKind::MethodCall(call) => {
            visitor.visit_expr(&call.receiver);
            if let Some(args) = &call.seg.args {
                visitor.visit_generic_args(args);
            }
            for arg in &call.args {
                visitor.visit_expr(arg);
            }
        }
        ExprKind::Block(block) | ExprKind::Loop(block) => visitor.visit_block(block),
        ExprKind::If(cond, then, els) => {
            visitor.visit_expr(cond);
            visitor.visit_block(then);
            if let Some(els) = els {
                visitor.visit_expr(els);
            }
        }
        ExprKind::Match(scrutinee, arms) => {
            visitor.visit_expr(scrutinee);
            for arm in arms {
                visitor.visit_arm(arm);
            }
        }
        ExprKind::ForLoop { pat, iter, body } => {
            visitor.visit_expr(iter);
            visitor.visit_pat(pat);
            visitor.visit_block(body);
        }
        ExprKind::While(cond, body) => {
            visitor.visit_expr(cond);
            visitor.visit_block(body);
        }
        ExprKind::Closure(closure) => {
            visitor.visit_fn_decl(&closure.decl);
            visitor.visit_expr(&closure.body);
        }
        ExprKind::Break(value) | ExprKind::Ret(value) => {
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
    }
}

pub fn walk_arm<V: Visitor>(visitor: &mut V, arm: &Arm) {
    visitor.visit_pat(&arm.pat);
    visitor.visit_expr(&arm.body);
}

pub fn walk_pat<V: Visitor>(visitor: &mut V, pat: &Pat) {
    match &pat.kind {
        PatKind::Wild | PatKind::Ident(_) | PatKind::Lit(_) | PatKind::Err => {}
        PatKind::Path(path) => visitor.visit_path(path),
        PatKind::TupleStruct(path, fields) => {
            visitor.visit_path(path);
            for field in fields {
                visitor.visit_pat(field);
            }
        }
        PatKind::Tuple(pats) => {
            for pat in pats {
                visitor.visit_pat(pat);
            }
        }
        PatKind::Paren(pat) => visitor.visit_pat(pat),
    }
}

pub fn walk_ty<V: Visitor>(visitor: &mut V, ty: &Ty) {
    match &ty.kind {
        TyKind::Path(path) => visitor.visit_path(path),
        TyKind::Tuple(tys) => {
            for ty in tys {
                visitor.visit_ty(ty);
            }
        }
        TyKind::Paren(ty) | TyKind::List(ty) => visitor.visit_ty(ty),
        TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => {}
    }
}

pub fn walk_path<V: Visitor>(visitor: &mut V, path: &Path) {
    for segment in &path.segments {
        if let Some(args) = &segment.args {
            visitor.visit_generic_args(args);
        }
    }
}

pub fn walk_generic_args<V: Visitor>(visitor: &mut V, args: &GenericArgs) {
    for ty in &args.args {
        visitor.visit_ty(ty);
    }
}
