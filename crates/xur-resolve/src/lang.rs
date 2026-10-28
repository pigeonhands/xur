//! Definitions the runtime itself needs to know, after rustc's lang items.

use crate::{DefId, DefKind, Namespace, Program, Res};
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
    pub(crate) fn new(program: &Program) -> Self {
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

    const TEST_PRELUDE: &str = "\
#[try(unwrap = \"Ok\")]
pub enum Result<T, E = str> { Ok(T) Err(E) }
pub use Result::Ok;
pub use Result::Err;
#[try(unwrap = \"Some\")]
pub enum Option<T> { Some(T) None }
pub use Option::Some;
pub use Option::None;
";

    #[test]
    fn finds_the_prelude_variants() {
        let loader = MemoryFileLoader::new([("main.xur", "")]);
        let (program, _) = Program::load(
            Path::new("main.xur"),
            Box::new(loader),
            Box::new(Vec::new()),
            Some(TEST_PRELUDE),
        )
        .unwrap();
        let lang = LangItems::new(&program);
        let path = |def| program.resolutions().def_path(def);
        assert_eq!(path(lang.some), "Option::Some");
        assert_eq!(path(lang.err), "Result::Err");
    }
}
