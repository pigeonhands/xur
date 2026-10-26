mod bodies;
mod imports;
mod items;
mod program;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use xur_span::FileLoader;

use crate::{Program, names};

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
    pub program: Program,
    pub errors: Vec<String>,
}

pub(crate) fn resolve_files(files: &[(&str, &str)]) -> Resolved {
    let loader = Box::new(MemLoader::new(files));
    let (program, diags) = Program::load(Path::new(files[0].0), loader).expect("root exists");
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
    /// "builtin function std::io::println".
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
