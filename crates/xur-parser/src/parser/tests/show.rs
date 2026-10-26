//! Compact, source-like rendering of AST nodes for test assertions.

use xur_ast::{GenericArgs, Lit, LitKind, Pat, PatKind, Path, Ty, TyKind};

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
