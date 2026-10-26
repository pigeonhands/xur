use std::path::Path;

use xur_span::MemoryFileLoader;

use super::resolve_files;
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
            Box::new(MemoryFileLoader::new(FILES.iter().copied()))
        )
        .is_err()
    );
    // A program whose root exists loads, whatever its imports.
    resolve_files(FILES).assert_no_errors();
}
