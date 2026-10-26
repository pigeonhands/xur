//! Definitions the runtime itself needs to know, after rustc's lang items.

use xur_resolve::{DefId, DefKind, Namespace, Program, Res};
use xur_span::Symbol;

/// The prelude's `Option` and `Result` variants, which Rust's `Option` and
/// `Result` convert to and from.
#[derive(Debug, Clone, Copy)]
pub struct LangItems {
    pub some: DefId,
    pub none: DefId,
    pub ok: DefId,
    pub err: DefId,
}

impl LangItems {
    pub fn new(program: &Program) -> Self {
        let resolutions = program.resolutions();
        let prelude = resolutions.module_defs[program.krate.prelude.0];
        let variant = |name: &str| match resolutions.lookup(
            prelude,
            Namespace::Value,
            Symbol::intern(name),
        ) {
            Some(Res::Def(DefKind::Variant, def)) => def,
            other => panic!("the prelude must define the variant `{name}`, found {other:?}"),
        };
        Self {
            some: variant("Some"),
            none: variant("None"),
            ok: variant("Ok"),
            err: variant("Err"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use xur_span::MemoryFileLoader;

    use super::*;

    #[test]
    fn finds_the_prelude_variants() {
        let loader = MemoryFileLoader::new([("main.xur", "")]);
        let (program, _) = Program::load(Path::new("main.xur"), Box::new(loader)).unwrap();
        let lang = LangItems::new(&program);
        let path = |def| program.resolutions().def_path(def);
        assert_eq!(path(lang.some), "Option::Some");
        assert_eq!(path(lang.err), "Result::Err");
    }
}
