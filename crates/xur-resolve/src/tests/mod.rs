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

    /// Describes what the first path written exactly as `text` (in any
    /// module) resolved to, e.g. "builtin function std::io::println".
    pub(crate) fn res_of(&self, text: &str) -> String {
        let mut paths = Paths(Vec::new());
        for module in &self.krate.modules {
            paths.visit_module(&module.ast);
        }
        let (_, id) = paths
            .0
            .into_iter()
            .find(|&(span, _)| self.krate.source_map.slice(span) == text)
            .unwrap_or_else(|| panic!("no path `{text}`"));
        match self.resolutions.res.get(&id) {
            None => "unresolved".to_owned(),
            Some(&res) => self.describe(res),
        }
    }

    pub(crate) fn describe(&self, res: Res) -> String {
        match res {
            Res::Def(kind, def) => format!("{} {}", kind.descr(), self.resolutions.def_path(def)),
            Res::Local(_) => "local".to_owned(),
            Res::PrimTy(ty) => format!("builtin type {}", ty.name()),
            Res::Err => "error".to_owned(),
        }
    }
}

/// Collects every path in a module with its node id.
struct Paths(Vec<(Span, xur_ast::NodeId)>);

impl Visitor for Paths {
    fn visit_path(&mut self, path: &xur_ast::Path) {
        self.0.push((path.span, path.id));
        visit::walk_path(self, path);
    }
}
