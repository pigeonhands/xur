//! Loading every module reachable from the root file.
//!
//! `std::import(path)` is an ordinary function returning a module value, so
//! it can be called anywhere with any string. Calls whose path is a string
//! literal are found here and their files loaded ahead of time: missing
//! files are reported before the program runs, and the module's names can
//! be resolved statically. Other calls load their module at runtime.
//!
//! Paths are relative to the importing file, and each file is loaded once
//! however often it is imported, so import cycles are fine.

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use xur_ast::visit::{self, Visitor};
use xur_ast::{Expr, ExprKind, Lit, LitKind, Module, NodeId, NodeIdGen};
use xur_errors::Diag;
use xur_parser::parse_file;
use xur_span::{FileLoader, SourceMap, Span, Symbol};

/// Index of a module in [`Crate::modules`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleIdx(pub usize);

/// Every module of a program, parsed.
pub struct Crate {
    pub source_map: SourceMap,
    pub modules: Vec<LoadedModule>,
    /// The file the program was started from.
    pub root: ModuleIdx,
    /// The built-in prelude, whose public items are in scope everywhere.
    pub prelude: ModuleIdx,
}

impl Crate {
    /// A crate containing only the prelude, ready for its root module to be
    /// added next. The prelude source is provided by the caller (typically
    /// from the standard library's generated `PRELUDE` constant).
    pub(crate) fn with_prelude(
        loader: &mut Loader,
        diags: &mut Vec<Diag>,
        prelude: &str,
    ) -> Self {
        let mut krate = Self {
            source_map: SourceMap::new(),
            modules: Vec::new(),
            prelude: ModuleIdx(0),
            root: ModuleIdx(1),
        };
        let path = PathBuf::from("<prelude>");
        let idx = loader.add_tree(&mut krate, path, prelude.to_owned(), diags);
        debug_assert_eq!(idx, krate.prelude);
        krate.modules[idx.0].name = Symbol::intern("prelude");
        krate
    }

    /// A crate with no prelude — the root module will be module 0.
    pub(crate) fn without_prelude() -> Self {
        Self {
            source_map: SourceMap::new(),
            modules: Vec::new(),
            prelude: ModuleIdx(0),
            root: ModuleIdx(0),
        }
    }

    pub fn module(&self, idx: ModuleIdx) -> &LoadedModule {
        &self.modules[idx.0]
    }
}

pub struct LoadedModule {
    /// The file stem, e.g. `math` for `math.xur`.
    pub name: Symbol,
    pub path: PathBuf,
    pub ast: &'static Module,
    /// The module loaded by each `std::import` call with a literal path,
    /// keyed by the node id of the call's `std::import` path.
    pub imports: HashMap<NodeId, ModuleIdx>,
}

/// A `std::import("path")` call with a literal path.
struct Import {
    /// The node id of the call's `std::import` path.
    callee: NodeId,
    path: PathBuf,
    /// The span of the path literal.
    span: Span,
}

/// If `expr` is `std::import("path")` with a literal path, returns the node
/// id of its `std::import` path, the path and the literal's span.
///
/// Imports are recognised by their syntax, since files must be loaded
/// before names can be resolved.
pub(crate) fn static_import(expr: &Expr) -> Option<(NodeId, Symbol, Span)> {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return None;
    };
    let ExprKind::Path(path) = &callee.kind else {
        return None;
    };
    let is_std_import = matches!(
        path.segments.as_slice(),
        [std, import]
            if std.ident.name.as_str() == "std"
                && import.ident.name.as_str() == "import"
                && std.args.is_none()
                && import.args.is_none()
    );
    match args.as_slice() {
        [
            Expr {
                kind:
                    ExprKind::Lit(Lit {
                        kind: LitKind::Str(relative),
                        span,
                    }),
                ..
            },
        ] if is_std_import => Some((path.id, *relative, *span)),
        _ => None,
    }
}

/// Collects the literal-path imports in a module, with paths as written.
struct ImportFinder(Vec<Import>);

impl<'ast> Visitor<'ast> for ImportFinder {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Some((callee, path, span)) = static_import(expr) {
            self.0.push(Import {
                callee,
                path: PathBuf::from(path.as_str()),
                span,
            });
        }
        visit::walk_expr(self, expr);
    }
}

/// Loads files into a [`Crate`], remembering which are already loaded.
pub(crate) struct Loader {
    file_loader: Box<dyn FileLoader>,
    by_path: HashMap<PathBuf, ModuleIdx>,
    ids: NodeIdGen,
}

impl Loader {
    pub(crate) fn new(file_loader: Box<dyn FileLoader>) -> Self {
        Self {
            file_loader,
            by_path: HashMap::new(),
            ids: NodeIdGen::default(),
        }
    }

    pub(crate) fn read(&self, path: &Path) -> io::Result<String> {
        self.file_loader.read_file(path)
    }

    /// The module already loaded from `path`, which must be normalised.
    pub(crate) fn loaded(&self, path: &Path) -> Option<ModuleIdx> {
        self.by_path.get(path).copied()
    }

    /// Adds the module at `path` with source `src` and, transitively, every
    /// not yet loaded module it imports with a literal path.
    pub(crate) fn add_tree(
        &mut self,
        krate: &mut Crate,
        path: PathBuf,
        src: String,
        diags: &mut Vec<Diag>,
    ) -> ModuleIdx {
        let first = self.add_module(krate, path, src, diags);
        let mut pending = vec![first];
        while let Some(idx) = pending.pop() {
            for mut import in find_imports(krate.module(idx)) {
                if import.path.is_dir() {
                    import.path = import.path.join("lib.xur");
                }
                let target = match self.loaded(&import.path) {
                    Some(loaded) => loaded,
                    None => match self.read(&import.path) {
                        Ok(src) => {
                            let loaded = self.add_module(krate, import.path, src, diags);
                            pending.push(loaded);
                            loaded
                        }
                        Err(error) => {
                            let message =
                                format!("couldn't read `{}`: {error}", import.path.display());
                            diags.push(
                                Diag::error(message, import.span)
                                    .with_label(import.span, "imported here"),
                            );
                            continue;
                        }
                    },
                };
                krate.modules[idx.0].imports.insert(import.callee, target);
            }
        }
        first
    }

    /// Parses `src` as the module at `path`. The AST is allocated for the
    /// rest of the process, as rustc arena-allocates its ASTs, so that later
    /// stages can hold references into it while more modules are loaded.
    fn add_module(
        &mut self,
        krate: &mut Crate,
        path: PathBuf,
        src: String,
        diags: &mut Vec<Diag>,
    ) -> ModuleIdx {
        let file = krate.source_map.add_file(path.display().to_string(), src);
        let (ast, parse_diags) = parse_file(&file, &mut self.ids);
        diags.extend(parse_diags);

        let idx = ModuleIdx(krate.modules.len());
        krate.modules.push(LoadedModule {
            name: module_name(&path),
            path: path.clone(),
            ast: Box::leak(Box::new(ast)),
            imports: HashMap::new(),
        });
        self.by_path.insert(path, idx);
        idx
    }
}

/// The literal-path `std::import` calls anywhere in `module`, with paths
/// relative to the working directory.
fn find_imports(module: &LoadedModule) -> Vec<Import> {
    let dir = module.path.parent().unwrap_or(Path::new(""));
    let mut finder = ImportFinder(Vec::new());
    finder.visit_module(module.ast);
    finder
        .0
        .into_iter()
        .map(|import| Import {
            path: normalize(&dir.join(&import.path)),
            ..import
        })
        .collect()
}

/// A module is named after its file: `math` for `lib/math.xur`.
/// A `lib.xur` is named after its parent folder: `my_plugin` for
/// `my_plugin/lib.xur`.
pub(crate) fn module_name(path: &Path) -> Symbol {
    let stem = path.file_stem().unwrap_or(path.as_os_str());
    if stem == "lib"
        && let Some(parent) = path.parent().and_then(|p| p.file_name())
    {
        return Symbol::intern(&parent.to_string_lossy());
    }
    Symbol::intern(&stem.to_string_lossy())
}

/// Removes `.` and `..` components without touching the file system, so
/// the same file imported by different relative paths is loaded once.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) =>
            {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Program;
    use xur_span::MemoryFileLoader;

    fn load(files: &[(&str, &str)]) -> (Crate, Vec<String>) {
        let loader = Box::new(MemoryFileLoader::new(files.iter().copied()));
        let (program, diags) = Program::load(
            Path::new(files[0].0),
            loader,
            Box::new(crate::tests::natives()),
            Some(crate::tests::TEST_PRELUDE),
        )
        .unwrap();
        (
            program.krate,
            diags.into_iter().map(|d| d.message).collect(),
        )
    }

    fn names(krate: &Crate) -> Vec<&str> {
        krate.modules.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn loads_imports_transitively() {
        let (krate, diags) = load(&[
            (
                "src/main.xur",
                r#"const math = std::import("./lib/math.xur");"#,
            ),
            (
                "src/lib/math.xur",
                r#"const util = std::import("../util.xur");"#,
            ),
            ("src/util.xur", "fn helper() {}"),
        ]);
        assert_eq!(diags, Vec::<String>::new());
        assert_eq!(names(&krate), ["prelude", "main", "math", "util"]);
        assert_eq!(krate.module(krate.root).path, Path::new("src/main.xur"));
        assert_eq!(krate.modules[2].path, Path::new("src/lib/math.xur"));
        assert_eq!(krate.modules[3].path, Path::new("src/util.xur"));
    }

    #[test]
    fn each_file_is_loaded_once() {
        let (krate, diags) = load(&[
            (
                "a.xur",
                r#"const b = std::import("b.xur"); const b2 = std::import("./b.xur");"#,
            ),
            ("b.xur", r#"const a = std::import("a.xur");"#),
        ]);
        assert_eq!(diags, Vec::<String>::new());
        assert_eq!(names(&krate), ["prelude", "a", "b"]);
        let a = krate.module(krate.root);
        assert!(a.imports.values().all(|&target| target == ModuleIdx(2)));
        assert_eq!(krate.modules[2].imports.values().next(), Some(&krate.root));
    }

    #[test]
    fn missing_import_is_reported() {
        let (krate, diags) = load(&[("main.xur", r#"const m = std::import("missing.xur");"#)]);
        assert_eq!(diags, ["couldn't read `missing.xur`: file not found"]);
        assert!(krate.module(krate.root).imports.is_empty());
    }

    #[test]
    fn literal_imports_anywhere_are_loaded() {
        let (krate, diags) = load(&[
            (
                "main.xur",
                r#"fn f() { let mods = [std::import("m.xur")]; }"#,
            ),
            ("m.xur", ""),
        ]);
        assert_eq!(diags, Vec::<String>::new());
        assert_eq!(names(&krate), ["prelude", "main", "m"]);
    }

    #[test]
    fn dynamic_imports_are_left_for_runtime() {
        let (krate, diags) = load(&[("main.xur", "fn f(path: str) { std::import(path); }")]);
        assert_eq!(diags, Vec::<String>::new());
        assert!(krate.module(krate.root).imports.is_empty());
    }

    #[test]
    fn syntax_errors_in_imported_files_are_reported() {
        let (_, diags) = load(&[
            ("main.xur", r#"const m = std::import("m.xur");"#),
            ("m.xur", "fn f( {}"),
        ]);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize(Path::new("a/./b/../c.xur")), Path::new("a/c.xur"));
        assert_eq!(normalize(Path::new("../x.xur")), Path::new("../x.xur"));
    }
}
