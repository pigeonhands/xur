//! Compact, source-like rendering of AST nodes for test assertions.
//!
//! Operators, ranges, closures and jumps are fully parenthesised, so a test
//! pins down exactly how an expression was grouped.

use xur_ast::{
    Arm, Block, Expr, ExprKind, FnDecl, FnRetTy, GenericArgs, Lit, LitKind, Local, Pat, PatKind,
    Path, RangeLimits, Stmt, StmtKind, Ty, TyKind,
};

pub(super) trait Show {
    fn show(&self) -> String;
}

pub(super) fn list<T: Show>(items: &[T]) -> String {
    items.iter().map(Show::show).collect::<Vec<_>>().join(", ")
}

impl<T: Show> Show for Box<T> {
    fn show(&self) -> String {
        (**self).show()
    }
}

impl Show for Lit {
    fn show(&self) -> String {
        match self.kind {
            LitKind::Str(value) => format!("{:?}", value.as_str()),
            LitKind::Int(value) => value.to_string(),
            LitKind::Float(digits) => digits.to_string(),
            LitKind::Bool(value) => value.to_string(),
            LitKind::Err => "<err>".to_owned(),
        }
    }
}

impl Show for Path {
    fn show(&self) -> String {
        let segments: Vec<_> = self
            .segments
            .iter()
            .map(|segment| match &segment.args {
                Some(args) => format!("{}{}", segment.ident, args.show()),
                None => segment.ident.to_string(),
            })
            .collect();
        segments.join("::")
    }
}

impl Show for GenericArgs {
    fn show(&self) -> String {
        format!("<{}>", list(&self.args))
    }
}

impl Show for Ty {
    fn show(&self) -> String {
        match &self.kind {
            TyKind::Path(path) => path.show(),
            TyKind::Tuple(tys) if tys.len() == 1 => format!("({},)", tys[0].show()),
            TyKind::Tuple(tys) => format!("({})", list(tys)),
            TyKind::Paren(ty) => format!("({})", ty.show()),
            TyKind::Infer => "_".to_owned(),
            TyKind::Err => "<err>".to_owned(),
        }
    }
}

impl Show for Pat {
    fn show(&self) -> String {
        match &self.kind {
            PatKind::Wild => "_".to_owned(),
            PatKind::Ident(ident) => ident.to_string(),
            PatKind::Lit(lit) => lit.show(),
            PatKind::Path(path) => path.show(),
            PatKind::TupleStruct(path, fields) => format!("{}({})", path.show(), list(fields)),
            PatKind::Tuple(pats) if pats.len() == 1 => format!("({},)", pats[0].show()),
            PatKind::Tuple(pats) => format!("({})", list(pats)),
            PatKind::Paren(pat) => format!("({})", pat.show()),
            PatKind::Err => "<err>".to_owned(),
        }
    }
}

fn opt<T: Show>(item: &Option<T>) -> String {
    item.as_ref().map(Show::show).unwrap_or_default()
}

impl Show for Expr {
    fn show(&self) -> String {
        match &self.kind {
            ExprKind::Lit(lit) => lit.show(),
            ExprKind::Path(path) => path.show(),
            ExprKind::Paren(expr) => format!("({})", expr.show()),
            ExprKind::Tuple(exprs) if exprs.len() == 1 => format!("({},)", exprs[0].show()),
            ExprKind::Tuple(exprs) => format!("({})", list(exprs)),
            ExprKind::Unary(op, operand) => format!("({}{})", op.as_str(), operand.show()),
            ExprKind::Binary(op, lhs, rhs) => {
                format!("({} {} {})", lhs.show(), op.kind, rhs.show())
            }
            ExprKind::Assign(place, value, _) => format!("({} = {})", place.show(), value.show()),
            ExprKind::Range(start, end, limits) => {
                let op = match limits {
                    RangeLimits::HalfOpen => "..",
                    RangeLimits::Closed => "..=",
                };
                format!("({}{op}{})", opt(start), opt(end))
            }
            ExprKind::Call(callee, args) => format!("{}({})", callee.show(), list(args)),
            ExprKind::MethodCall(call) => {
                let args = call.seg.args.as_ref().map(|a| format!("::{}", a.show()));
                format!(
                    "{}.{}{}({})",
                    call.receiver.show(),
                    call.seg.ident,
                    args.unwrap_or_default(),
                    list(&call.args)
                )
            }
            ExprKind::Field(expr, field) => format!("{}.{field}", expr.show()),
            ExprKind::Try(expr) => format!("{}?", expr.show()),
            ExprKind::Block(block) => block.show(),
            ExprKind::If(cond, then, els) => match els {
                Some(els) => format!("if {} {} else {}", cond.show(), then.show(), els.show()),
                None => format!("if {} {}", cond.show(), then.show()),
            },
            ExprKind::Match(scrutinee, arms) => {
                format!("match {} {{ {} }}", scrutinee.show(), list(arms))
            }
            ExprKind::ForLoop { pat, iter, body } => {
                format!("for {} in {} {}", pat.show(), iter.show(), body.show())
            }
            ExprKind::While(cond, body) => format!("while {} {}", cond.show(), body.show()),
            ExprKind::Loop(body) => format!("loop {}", body.show()),
            ExprKind::Closure(closure) => {
                format!("({} {})", closure.decl.show(), closure.body.show())
            }
            ExprKind::Break(value) => jump("break", value),
            ExprKind::Continue => "continue".to_owned(),
            ExprKind::Ret(value) => jump("return", value),
            ExprKind::Err => "<err>".to_owned(),
        }
    }
}

fn jump(keyword: &str, value: &Option<Box<Expr>>) -> String {
    match value {
        Some(value) => format!("({keyword} {})", value.show()),
        None => keyword.to_owned(),
    }
}

/// Renders a closure's `|params| -> ret`.
impl Show for FnDecl {
    fn show(&self) -> String {
        let params: Vec<_> = self
            .inputs
            .iter()
            .map(|param| match param.ty.kind {
                TyKind::Infer => param.pat.show(),
                _ => format!("{}: {}", param.pat.show(), param.ty.show()),
            })
            .collect();
        match &self.output {
            FnRetTy::Default(_) => format!("|{}|", params.join(", ")),
            FnRetTy::Ty(ty) => format!("|{}| -> {}", params.join(", "), ty.show()),
        }
    }
}

impl Show for Arm {
    fn show(&self) -> String {
        format!("{} => {}", self.pat.show(), self.body.show())
    }
}

impl Show for Block {
    fn show(&self) -> String {
        if self.stmts.is_empty() {
            return "{}".to_owned();
        }
        let stmts: Vec<_> = self.stmts.iter().map(Show::show).collect();
        format!("{{ {} }}", stmts.join(" "))
    }
}

impl Show for Stmt {
    fn show(&self) -> String {
        match &self.kind {
            StmtKind::Let(local) => local.show(),
            StmtKind::Item(_) => "<item>".to_owned(),
            StmtKind::Expr(expr) => expr.show(),
            StmtKind::Semi(expr) => format!("{};", expr.show()),
            StmtKind::Empty => ";".to_owned(),
        }
    }
}

impl Show for Local {
    fn show(&self) -> String {
        let mut out = format!("let {}", self.pat.show());
        if let Some(ty) = &self.ty {
            out += &format!(": {}", ty.show());
        }
        if let Some(init) = &self.init {
            out += &format!(" = {}", init.show());
        }
        out + ";"
    }
}
