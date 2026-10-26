use std::path::Path;

use xur_span::MemoryFileLoader;

use super::{natives, resolve_files};
use crate::{Program, Res};

const FILES: &[(&str, &str)] = &[
    (
        "main.xur",
        "const shapes = std::import(\"lib/shapes.xur\");",
    ),
    ("lib/shapes.xur", "pub fn area() {}"),
    (
        "lib/plugin.xur",
        "const shapes = std::import(\"shapes.xur\");\npub fn run() { shapes::area() }",
    ),
    ("lib/broken.xur", "fn f() { nope }"),
];

fn load() -> Program {
    let (program, diags) = Program::load(
        Path::new("main.xur"),
        Box::new(MemoryFileLoader::new(FILES.iter().copied())),
        Box::new(natives()),
    )
    .unwrap();
    assert!(diags.is_empty(), "{diags:#?}");
    program
}

#[test]
fn importing_a_loaded_module_returns_it() {
    let mut program = load();
    let (idx, diags) = program.import(Path::new("lib/./shapes.xur")).unwrap();
    assert!(diags.is_empty());
    assert_eq!(program.krate.module(idx).name.as_str(), "shapes");
    assert_eq!(program.krate.modules.len(), 3);
}

#[test]
fn imports_load_and_resolve_new_modules() {
    let mut program = load();
    let defs_before = program
        .resolutions()
        .defs
        .lineage(program.resolutions().module_defs[1]);
    let (idx, diags) = program.import(Path::new("lib/plugin.xur")).unwrap();
    assert!(diags.is_empty(), "{diags:#?}");
    assert_eq!(program.krate.module(idx).name.as_str(), "plugin");
    // The plugin's own import of `shapes` reuses the loaded module.
    assert_eq!(program.krate.modules.len(), 4);
    // Earlier definitions are untouched.
    let resolutions = program.resolutions();
    assert_eq!(
        resolutions.defs.lineage(resolutions.module_defs[1]),
        defs_before
    );

    let resolved_area = resolutions
        .res
        .values()
        .any(|res| matches!(res, Res::Def(_, def) if resolutions.def_path(*def) == "shapes::area"));
    assert!(resolved_area);
}

#[test]
fn imported_modules_report_their_own_errors() {
    let mut program = load();
    let (_, diags) = program.import(Path::new("lib/broken.xur")).unwrap();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, ["cannot find value `nope` in this scope"]);
}

#[test]
fn importing_a_missing_file_fails() {
    let mut program = load();
    assert!(program.import(Path::new("missing.xur")).is_err());
}

#[test]
fn missing_root_fails() {
    assert!(
        Program::load(
            Path::new("nope.xur"),
            Box::new(MemoryFileLoader::new(FILES.iter().copied())),
            Box::new(natives()),
        )
        .is_err()
    );
    // A program whose root exists loads, whatever its imports.
    resolve_files(FILES).assert_no_errors();
}

/// Provides `lazy` with one function, counting how often it is expanded.
#[derive(Default)]
struct LazyNatives {
    expansions: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::NativeProvider for LazyNatives {
    fn modules(&self) -> Vec<crate::NativeModule> {
        let mut modules = natives();
        modules.push(crate::NativeModule {
            name: "lazy".to_owned(),
            lazy: true,
            ..Default::default()
        });
        modules
    }

    fn expand(&self, path: &[String]) -> crate::NativeModule {
        assert_eq!(path, ["lazy"]);
        self.expansions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::NativeModule {
            name: "lazy".to_owned(),
            functions: vec![crate::NativeFnDecl {
                name: "f".to_owned(),
                id: crate::NativeFnId(0),
            }],
            ..Default::default()
        }
    }
}

fn load_lazy(src: &str) -> (usize, Vec<String>) {
    let natives = LazyNatives::default();
    let expansions = natives.expansions.clone();
    let loader = MemoryFileLoader::new([("main.xur", src)]);
    let (_, diags) =
        Program::load(Path::new("main.xur"), Box::new(loader), Box::new(natives)).unwrap();
    let messages = diags.into_iter().map(|d| d.message).collect();
    (
        expansions.load(std::sync::atomic::Ordering::SeqCst),
        messages,
    )
}

#[test]
fn lazy_native_modules_are_expanded_once_when_used() {
    assert_eq!(load_lazy("fn main() {}"), (0, vec![]));
    assert_eq!(
        load_lazy("fn main() { lazy::f(); lazy::f(); }"),
        (1, vec![])
    );
    assert_eq!(
        load_lazy("fn main() { lazy::g(); }"),
        (1, vec!["cannot find value `g` in module `lazy`".to_owned()])
    );
}
