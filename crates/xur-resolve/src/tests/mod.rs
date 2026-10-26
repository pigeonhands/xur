mod bodies;
mod imports;
mod items;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use xur_ast::visit::{self, Visitor};
use xur_span::{FileLoader, Span};

use crate::{Crate, Res, Resolutions, load_crate, resolve};

/// Serves files from memory.
#[derive(Default)]
pub(crate) struct MemLoader(HashMap<PathBuf, String>);

impl MemLoader {
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        Self(
            files
                .iter()
                .map(|(path, src)| (PathBuf::from(path), (*src).to_owned()))
                .collect(),
        )
    }
}

impl FileLoader for MemLoader {
    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "file not found"))
    }
}

/// A program loaded from memory and resolved. The first file is the root.
pub(crate) struct Resolved {
    krate: Crate,
    resolutions: Resolutions,
    pub errors: Vec<String>,
}

pub(crate) fn resolve_files(files: &[(&str, &str)]) -> Resolved {
    let loader = MemLoader::new(files);
    let (krate, load_diags) = load_crate(Path::new(files[0].0), &loader).expect("root exists");
    let (resolutions, diags) = resolve(&krate);
    let errors = load_diags
        .into_iter()
        .chain(diags)
        .map(|d| d.message)
        .collect();
    Resolved {
        krate,
        resolutions,
        errors,
    }
}

/// Resolves a program consisting of a single file.
pub(crate) fn resolve_src(src: &str) -> Resolved {
    resolve_files(&[("main.xur", src)])
}

impl Resolved {
    pub(crate) fn assert_no_errors(&self) {
        assert_eq!(self.errors, Vec::<String>::new());
    }

    /// Describes what the first path or identifier pattern written exactly
    /// as `text` (in any module but the prelude) resolved to, e.g.
    /// "builtin function std::io::println".
    pub(crate) fn res_of(&self, text: &str) -> String {
        self.res_of_nth(text, 0)
    }

    /// Like [`Self::res_of`], for the `n`th (from 0) occurrence of `text`.
    pub(crate) fn res_of_nth(&self, text: &str, n: usize) -> String {
        let mut names = Names(Vec::new());
        let prelude = self.krate.module(self.krate.prelude);
        for module in &self.krate.modules {
            if !std::ptr::eq(module, prelude) {
                names.visit_module(&module.ast);
            }
        }
        let (_, id) = names
            .0
            .into_iter()
            .filter(|&(span, _)| self.krate.source_map.slice(span) == text)
            .nth(n)
            .unwrap_or_else(|| panic!("no occurrence {n} of `{text}`"));
        match self.resolutions.res.get(&id) {
            None => "unresolved".to_owned(),
            Some(&res) => self.describe(res),
        }
    }

    pub(crate) fn describe(&self, res: Res) -> String {
        match res {
            Res::Def(kind, def) => format!("{} {}", kind.descr(), self.resolutions.def_path(def)),
            Res::Local(id) => {
                let ident = self.resolutions.locals[&id];
                let pos = self.krate.source_map.describe_pos(ident.span);
                format!("local {ident} at {pos}")
            }
            Res::PrimTy(ty) => format!("builtin type {}", ty.name()),
            Res::Err => "error".to_owned(),
        }
    }
}

/// Collects every path and identifier pattern in a module, with node ids.
struct Names(Vec<(Span, xur_ast::NodeId)>);

impl Visitor for Names {
    fn visit_path(&mut self, path: &xur_ast::Path) {
        self.0.push((path.span, path.id));
        visit::walk_path(self, path);
    }

    fn visit_pat(&mut self, pat: &xur_ast::Pat) {
        if let xur_ast::PatKind::Ident(_) = pat.kind {
            self.0.push((pat.span, pat.id));
        }
        visit::walk_pat(self, pat);
    }
}
