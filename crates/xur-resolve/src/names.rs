//! Listing what every name in a module resolved to, for tooling such as
//! `xur debug resolve`.

use xur_ast::visit::{self, Visitor};
use xur_ast::{Module, NodeId, Pat, PatKind, Path};
use xur_span::{SourceMap, Span};

use crate::{Res, Resolutions};

/// A place where a name is written: a path, or an identifier pattern.
#[derive(Debug, Clone, Copy)]
pub struct NameRef {
    pub id: NodeId,
    pub span: Span,
}

/// Every path and identifier pattern in `module`, in source order.
pub fn names(module: &Module) -> Vec<NameRef> {
    let mut collector = Collector(Vec::new());
    collector.visit_module(module);
    collector.0.sort_by_key(|name| name.span.start);
    collector.0
}

struct Collector(Vec<NameRef>);

impl<'ast> Visitor<'ast> for Collector {
    fn visit_path(&mut self, path: &'ast Path) {
        self.0.push(NameRef {
            id: path.id,
            span: path.span,
        });
        visit::walk_path(self, path);
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        if let PatKind::Ident(_) = pat.kind {
            self.0.push(NameRef {
                id: pat.id,
                span: pat.span,
            });
        }
        visit::walk_pat(self, pat);
    }
}

impl Resolutions {
    /// Describes `res`, e.g. "builtin function std::io::println" or
    /// "local x at main.xur:2:9".
    pub fn describe(&self, res: Res, source_map: &SourceMap) -> String {
        match res {
            Res::Def(kind, def) => format!("{} {}", kind.descr(), self.def_path(def)),
            Res::Local(id) => {
                let ident = self.locals[&id];
                format!("local {ident} at {}", source_map.describe_pos(ident.span))
            }
            Res::PrimTy(ty) => format!("builtin type {}", ty.name()),
            Res::TyParamAssoc(param, name) => {
                format!("{}::{name}", self.def_path(param))
            }
            Res::Err => "error".to_owned(),
        }
    }
}
