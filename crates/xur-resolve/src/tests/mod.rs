mod bodies;
mod impls;
mod imports;
mod items;
mod program;

use std::path::Path;

use xur_span::MemoryFileLoader;

use crate::{
    NativeFnDecl, NativeFnId, NativeModule, NativeTypeDecl, NativeTypeId, NativeVariantDecl,
    NativeVariantKind, Program, names,
};

/// A program loaded from memory and resolved. The first file is the root.
pub(crate) struct Resolved {
    pub program: Program,
    pub errors: Vec<String>,
}

pub(crate) fn resolve_files(files: &[(&str, &str)]) -> Resolved {
    let loader = Box::new(MemoryFileLoader::new(files.iter().copied()));
    let (program, diags) =
        Program::load(Path::new(files[0].0), loader, Box::new(natives()), Some(xur_std::PRELUDE)).expect("root exists");
    let errors = diags.into_iter().map(|d| d.message).collect();
    Resolved { program, errors }
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
    /// "native function std::io::println".
    pub(crate) fn res_of(&self, text: &str) -> String {
        self.res_of_nth(text, 0)
    }

    /// Like [`Self::res_of`], for the `n`th (from 0) occurrence of `text`.
    pub(crate) fn res_of_nth(&self, text: &str, n: usize) -> String {
        let krate = &self.program.krate;
        let source_map = &krate.source_map;
        let prelude = krate.module(krate.prelude);
        let name = krate
            .modules
            .iter()
            .filter(|module| !std::ptr::eq(*module, prelude))
            .flat_map(|module| names(module.ast))
            .filter(|name| source_map.slice(name.span) == text)
            .nth(n)
            .unwrap_or_else(|| panic!("no occurrence {n} of `{text}`"));
        let resolutions = self.program.resolutions();
        match resolutions.res.get(&name.id) {
            None => "unresolved".to_owned(),
            Some(&res) => resolutions.describe(res, source_map),
        }
    }
}

/// Native modules like the runtime's `std`, plus `geo` with a native type.
pub(crate) fn natives() -> Vec<NativeModule> {
    let mut next = 0;
    let mut f = |name: &str| {
        next += 1;
        NativeFnDecl {
            name: name.to_owned(),
            id: NativeFnId(next),
            param_names: Vec::new(),
        }
    };
    let module = |name: &str, functions, modules| NativeModule {
        name: name.to_owned(),
        functions,
        modules,
        ..NativeModule::default()
    };
    let std = module(
        "std",
        vec![f("import"), f("panic")],
        vec![
            module("io", vec![f("print"), f("println")], Vec::new()),
            module("any", vec![f("type_name")], Vec::new()),
            module("mem", vec![f("size_of"), f("from_bytes")], Vec::new()),
        ],
    );
    let geo = NativeModule {
        types: vec![
            NativeTypeDecl {
                name: "Point".to_owned(),
                id: NativeTypeId(0),
                functions: vec![f("new"), f("norm")],
                variants: Vec::new(),
            },
            NativeTypeDecl {
                name: "Machine".to_owned(),
                id: NativeTypeId(1),
                functions: Vec::new(),
                variants: [
                    ("Amd64", NativeVariantKind::Unit),
                    ("Other", NativeVariantKind::Tuple),
                    ("Named", NativeVariantKind::Named),
                ]
                .into_iter()
                .map(|(name, kind)| NativeVariantDecl {
                    name: name.to_owned(),
                    kind,
                    repr_value: None,
                })
                .collect(),
            },
        ],
        ..module("geo", Vec::new(), Vec::new())
    };
    vec![std, geo]
}
