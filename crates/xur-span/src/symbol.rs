//! Interned strings.
//!
//! As in rustc, every distinct name is stored once in a global interner and
//! referred to by a small [`Symbol`] index, so names are `Copy` and cheap to
//! compare and hash. Interned strings are leaked: they live for the rest of
//! the process, which is exactly how long a compiler needs them.

use std::collections::HashMap;
use std::fmt;
use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::Span;

static INTERNER: LazyLock<Mutex<Interner>> = LazyLock::new(Default::default);

/// An interned string.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(u32);

impl Symbol {
    pub fn intern(string: &str) -> Self {
        interner().intern(string)
    }

    pub fn as_str(self) -> &'static str {
        interner().strings[self.0 as usize]
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name together with where it was written.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: Symbol,
    pub span: Span,
}

impl Ident {
    pub fn new(name: Symbol, span: Span) -> Self {
        Self { name, span }
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}@{:?}", self.name, self.span)
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}

#[derive(Default)]
struct Interner {
    symbols: HashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

impl Interner {
    fn intern(&mut self, string: &str) -> Symbol {
        if let Some(&symbol) = self.symbols.get(string) {
            return symbol;
        }
        let index = u32::try_from(self.strings.len()).expect("too many interned symbols");
        let symbol = Symbol(index);
        let string: &'static str = Box::leak(string.into());
        self.strings.push(string);
        self.symbols.insert(string, symbol);
        symbol
    }
}

fn interner() -> MutexGuard<'static, Interner> {
    // The interner is never left in an inconsistent state, so a panic in
    // another thread while holding the lock does not invalidate it.
    INTERNER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_idempotent() {
        assert_eq!(Symbol::intern("foo"), Symbol::intern("foo"));
        assert_ne!(Symbol::intern("foo"), Symbol::intern("bar"));
    }

    #[test]
    fn round_trips() {
        assert_eq!(Symbol::intern("α_beta").as_str(), "α_beta");
    }

    #[test]
    fn formatting() {
        let ident = Ident::new(Symbol::intern("main"), Span::new(3, 7));
        assert_eq!(ident.to_string(), "main");
        assert_eq!(format!("{ident:?}"), r#""main"@3..7"#);
    }
}
