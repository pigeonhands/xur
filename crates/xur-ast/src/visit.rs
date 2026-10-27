//! Read-only traversal of the AST, after `rustc_ast::visit`.
//!
//! Each `visit_*` method defaults to the matching `walk_*` function, which
//! visits the node's children. As in rustc, the visitor is parameterised by
//! the AST's lifetime `'ast`, so it can keep references to what it visits. A pass overrides the methods for the nodes it
//! cares about and calls `walk_*` itself to keep descending.

use crate::{
    Arm, Block, Expr, ExprKind, FieldDef, FnDecl, FnRetTy, GenericArgs, GenericParam, Generics,
    Item, ItemKind, Local, Module, Param, Pat, PatKind, Path, Stmt, StmtKind, Ty, TyKind, Variant,
    VariantData,
};

pub trait Visitor<'ast>: Sized {
    fn visit_module(&mut self, module: &'ast Module) {
        walk_module(self, module);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        walk_item(self, item);
    }
    fn visit_generics(&mut self, generics: &'ast Generics) {
        walk_generics(self, generics);
    }
    fn visit_generic_param(&mut self, _param: &'ast GenericParam) {}
    fn visit_fn_decl(&mut self, decl: &'ast FnDecl) {
        walk_fn_decl(self, decl);
    }
    fn visit_param(&mut self, param: &'ast Param) {
        walk_param(self, param);
    }
    fn visit_variant_data(&mut self, data: &'ast VariantData) {
        walk_variant_data(self, data);
    }
    fn visit_variant(&mut self, variant: &'ast Variant) {
        walk_variant(self, variant);
    }
    fn visit_field_def(&mut self, field: &'ast FieldDef) {
        walk_field_def(self, field);
    }
    fn visit_block(&mut self, block: &'ast Block) {
        walk_block(self, block);
    }
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        walk_stmt(self, stmt);
    }
    fn visit_local(&mut self, local: &'ast Local) {
        walk_local(self, local);
    }
    fn visit_expr(&mut self, expr: &'ast Expr) {
        walk_expr(self, expr);
    }
    fn visit_arm(&mut self, arm: &'ast Arm) {
        walk_arm(self, arm);
    }
    fn visit_pat(&mut self, pat: &'ast Pat) {
        walk_pat(self, pat);
    }
    fn visit_ty(&mut self, ty: &'ast Ty) {
        walk_ty(self, ty);
    }
    fn visit_path(&mut self, path: &'ast Path) {
        walk_path(self, path);
    }
    fn visit_generic_args(&mut self, args: &'ast GenericArgs) {
        walk_generic_args(self, args);
    }
}

pub fn walk_module<'a, V: Visitor<'a>>(visitor: &mut V, module: &'a Module) {
    for item in &module.items {
        visitor.visit_item(item);
    }
}

/// Attributes are not walked: their paths name attributes, not items.
pub fn walk_item<'a, V: Visitor<'a>>(visitor: &mut V, item: &'a Item) {
    match &item.kind {
        ItemKind::Use(trees) => {
            for tree in trees {
                visitor.visit_path(&tree.path);
            }
        }
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

pub fn walk_generics<'a, V: Visitor<'a>>(visitor: &mut V, generics: &'a Generics) {
    for param in &generics.params {
        visitor.visit_generic_param(param);
        if let Some(default) = &param.default {
            visitor.visit_ty(default);
        }
    }
}

pub fn walk_fn_decl<'a, V: Visitor<'a>>(visitor: &mut V, decl: &'a FnDecl) {
    for param in &decl.inputs {
        visitor.visit_param(param);
    }
    if let FnRetTy::Ty(ty) = &decl.output {
        visitor.visit_ty(ty);
    }
}

pub fn walk_param<'a, V: Visitor<'a>>(visitor: &mut V, param: &'a Param) {
    visitor.visit_pat(&param.pat);
    visitor.visit_ty(&param.ty);
}

pub fn walk_variant_data<'a, V: Visitor<'a>>(visitor: &mut V, data: &'a VariantData) {
    match data {
        VariantData::Struct(fields) | VariantData::Tuple(fields) => {
            for field in fields {
                visitor.visit_field_def(field);
            }
        }
        VariantData::Unit => {}
    }
}

pub fn walk_variant<'a, V: Visitor<'a>>(visitor: &mut V, variant: &'a Variant) {
    visitor.visit_variant_data(&variant.data);
}

pub fn walk_field_def<'a, V: Visitor<'a>>(visitor: &mut V, field: &'a FieldDef) {
    visitor.visit_ty(&field.ty);
}

pub fn walk_block<'a, V: Visitor<'a>>(visitor: &mut V, block: &'a Block) {
    for stmt in &block.stmts {
        visitor.visit_stmt(stmt);
    }
}

pub fn walk_stmt<'a, V: Visitor<'a>>(visitor: &mut V, stmt: &'a Stmt) {
    match &stmt.kind {
        StmtKind::Let(local) => visitor.visit_local(local),
        StmtKind::Item(item) => visitor.visit_item(item),
        StmtKind::Expr(expr) | StmtKind::Semi(expr) => visitor.visit_expr(expr),
        StmtKind::Empty => {}
    }
}

/// Visits the initializer before the pattern, since the pattern's bindings
/// are not in scope in it.
pub fn walk_local<'a, V: Visitor<'a>>(visitor: &mut V, local: &'a Local) {
    if let Some(ty) = &local.ty {
        visitor.visit_ty(ty);
    }
    if let Some(init) = &local.init {
        visitor.visit_expr(init);
    }
    visitor.visit_pat(&local.pat);
}

pub fn walk_expr<'a, V: Visitor<'a>>(visitor: &mut V, expr: &'a Expr) {
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
        ExprKind::Struct(literal) => {
            visitor.visit_path(&literal.path);
            for field in &literal.fields {
                visitor.visit_expr(&field.expr);
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
        ExprKind::Cast(expr, ty) => {
            visitor.visit_expr(expr);
            visitor.visit_ty(ty);
        }
        ExprKind::Is(expr, pat) => {
            visitor.visit_expr(expr);
            visitor.visit_pat(pat);
        }
        ExprKind::Break(value) | ExprKind::Ret(value) => {
            if let Some(value) = value {
                visitor.visit_expr(value);
            }
        }
    }
}

pub fn walk_arm<'a, V: Visitor<'a>>(visitor: &mut V, arm: &'a Arm) {
    visitor.visit_pat(&arm.pat);
    visitor.visit_expr(&arm.body);
}

pub fn walk_pat<'a, V: Visitor<'a>>(visitor: &mut V, pat: &'a Pat) {
    match &pat.kind {
        PatKind::Wild | PatKind::Ident(_) | PatKind::Lit(_) | PatKind::Err => {}
        PatKind::Path(path) => visitor.visit_path(path),
        PatKind::TupleStruct(path, fields) => {
            visitor.visit_path(path);
            for field in fields {
                visitor.visit_pat(field);
            }
        }
        PatKind::Struct { path, fields, .. } => {
            visitor.visit_path(path);
            for field in fields {
                visitor.visit_pat(&field.pat);
            }
        }
        PatKind::Tuple(pats) | PatKind::List(pats, _) => {
            for pat in pats {
                visitor.visit_pat(pat);
            }
        }
        PatKind::Paren(pat) => visitor.visit_pat(pat),
    }
}

pub fn walk_ty<'a, V: Visitor<'a>>(visitor: &mut V, ty: &'a Ty) {
    match &ty.kind {
        TyKind::Path(path) => visitor.visit_path(path),
        TyKind::Tuple(tys) => {
            for ty in tys {
                visitor.visit_ty(ty);
            }
        }
        TyKind::Paren(ty) | TyKind::List(ty) => visitor.visit_ty(ty),
        TyKind::Record | TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => {}
    }
}

pub fn walk_path<'a, V: Visitor<'a>>(visitor: &mut V, path: &'a Path) {
    for segment in &path.segments {
        if let Some(args) = &segment.args {
            visitor.visit_generic_args(args);
        }
    }
}

pub fn walk_generic_args<'a, V: Visitor<'a>>(visitor: &mut V, args: &'a GenericArgs) {
    for ty in &args.args {
        visitor.visit_ty(ty);
    }
}
