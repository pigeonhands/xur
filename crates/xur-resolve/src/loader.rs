//! Loading every module reachable from the root file.
//!
//! A file becomes a module when it is imported with
//! `const name = std::import("path");`. Paths are relative to the importing
//! file, and each file is loaded once however often it is imported, so
//! import cycles are fine.

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use xur_ast::{Expr, ExprKind, ItemKind, Lit, LitKind, Module, NodeId, NodeIdGen};
use xur_errors::Diag;
use xur_parser::parse_file;
use xur_span::{FileLoader, SourceMap, Span, Symbol};

const PRELUDE: &str = include_str!("prelude.xur");

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
    pub fn module(&self, idx: ModuleIdx) -> &LoadedModule {
        &self.modules[idx.0]
    }
}

pub struct LoadedModule {
    /// The file stem, e.g. `math` for `math.xur`.
    pub name: Symbol,
    pub path: PathBuf,
    pub ast: Module,
    /// `const` items initialised with `std::import`, and the module each loads.
    pub imports: HashMap<NodeId, ModuleIdx>,
}

/// If `expr` is a call to `std::import`, returns its arguments.
///
/// Imports are recognised by their syntax, since the loader must find them
/// before names can be resolved.
pub(crate) fn as_std_import(expr: &Expr) -> Option<&[Expr]> {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return None;
    };
    let ExprKind::Path(path) = &callee.kind else {
        return None;
    };
    let names: Vec<_> = path
        .segments
        .iter()
        .map(|segment| {
            segment
                .args
                .is_none()
                .then_some(segment.ident.name.as_str())
        })
        .collect();
    (names == [Some("std"), Some("import")]).then_some(args.as_slice())
}

/// Loads and parses `root` and, transitively, every module it imports.
///
/// Fails only if `root` itself cannot be read; problems with imported files
/// and syntax errors are returned as diagnostics.
pub fn load_crate(root: &Path, loader: &dyn FileLoader) -> io::Result<(Crate, Vec<Diag>)> {
    let root_src = loader.read_file(root)?;
    let mut builder = Builder {
        loader,
        source_map: SourceMap::new(),
        modules: Vec::new(),
        by_path: HashMap::new(),
        ids: NodeIdGen::default(),
        diags: Vec::new(),
    };
    let prelude = builder.add_module(
        Symbol::intern("prelude"),
        PathBuf::from("<prelude>"),
        PRELUDE.to_owned(),
    );
    let root = normalize(root);
    let root = builder.add_module(module_name(&root), root, root_src);

    let mut pending = vec![root];
    while let Some(idx) = pending.pop() {
        for import in builder.find_imports(idx) {
            let target = match builder.by_path.get(&import.path) {
                Some(&loaded) => loaded,
                None => {
                    let Some(loaded) = builder.load(&import) else {
                        continue;
                    };
                    pending.push(loaded);
                    loaded
                }
            };
            builder.modules[idx.0].imports.insert(import.item, target);
        }
    }

    let krate = Crate {
        source_map: builder.source_map,
        modules: builder.modules,
        root,
        prelude,
    };
    Ok((krate, builder.diags))
}

/// A `const name = std::import("path");` item found in a module.
struct Import {
    item: NodeId,
    /// The imported file, relative to the working directory.
    path: PathBuf,
    /// The span of the path literal.
    span: Span,
}

struct Builder<'a> {
    loader: &'a dyn FileLoader,
    source_map: SourceMap,
    modules: Vec<LoadedModule>,
    by_path: HashMap<PathBuf, ModuleIdx>,
    ids: NodeIdGen,
    diags: Vec<Diag>,
}

impl Builder<'_> {
    fn add_module(&mut self, name: Symbol, path: PathBuf, src: String) -> ModuleIdx {
        let file = self.source_map.add_file(path.display().to_string(), src);
        let (ast, diags) = parse_file(&file, &mut self.ids);
        self.diags.extend(diags);

        let idx = ModuleIdx(self.modules.len());
        self.modules.push(LoadedModule {
            name,
            path: path.clone(),
            ast,
            imports: HashMap::new(),
        });
        self.by_path.insert(path, idx);
        idx
    }

    /// Reads and parses the module at `path`, reporting failure at the
    /// import that asked for it.
    fn load(&mut self, import: &Import) -> Option<ModuleIdx> {
        match self.loader.read_file(&import.path) {
            Ok(src) => {
                let name = module_name(&import.path);
                Some(self.add_module(name, import.path.clone(), src))
            }
            Err(error) => {
                let message = format!("couldn't read `{}`: {error}", import.path.display());
                self.diags.push(
                    Diag::error(message, import.span).with_label(import.span, "imported here"),
                );
                None
            }
        }
    }

    /// The top-level `std::import` consts of module `idx`.
    fn find_imports(&mut self, idx: ModuleIdx) -> Vec<Import> {
        let module = &self.modules[idx.0];
        let dir = module.path.parent().unwrap_or(Path::new("")).to_owned();
        let mut imports = Vec::new();
        for item in &module.ast.items {
            let ItemKind::Const(konst) = &item.kind else {
                continue;
            };
            let Some(args) = as_std_import(&konst.expr) else {
                continue;
            };
            if let [arg] = args
                && let ExprKind::Lit(Lit {
                    kind: LitKind::Str(relative),
                    span,
                }) = arg.kind
            {
                imports.push(Import {
                    item: item.id,
                    path: normalize(&dir.join(relative.as_str())),
                    span,
                });
            } else {
                let span = konst.expr.span;
                self.diags.push(
                    Diag::error("`std::import` takes a single string literal", span)
                        .with_label(span, "expected a path such as `\"./math.xur\"`"),
                );
            }
        }
        imports
    }
}

/// A module is named after its file: `math` for `lib/math.xur`.
fn module_name(path: &Path) -> Symbol {
    let stem = path.file_stem().unwrap_or(path.as_os_str());
    Symbol::intern(&stem.to_string_lossy())
}

/// Removes `.` and `..` components without touching the file system, so
/// the same file imported by different relative paths is loaded once.
fn normalize(path: &Path) -> PathBuf {
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
    use crate::tests::MemLoader;

    fn load(files: &[(&str, &str)]) -> (Crate, Vec<String>) {
        let (krate, diags) = load_crate(Path::new(files[0].0), &MemLoader::new(files)).unwrap();
        (krate, diags.into_iter().map(|d| d.message).collect())
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
    fn import_needs_a_string_literal() {
        let (_, diags) = load(&[("main.xur", "const m = std::import(path);")]);
        assert_eq!(diags, ["`std::import` takes a single string literal"]);
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
