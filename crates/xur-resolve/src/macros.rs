//! Compile-time macro expansion: Rust code registered with the `Registry`
//! inspects annotated items and emits xur source that gets parsed and spliced
//! into the module before name resolution.

use xur_ast::Item;
use xur_span::Span;

/// A registered macro expander that transforms annotated items at compile time.
///
/// Expanders receive a [`MacroTrigger`] describing the annotated item and
/// return xur source text. The source is parsed as module items and added
/// alongside the original item before name resolution runs.
pub trait MacroExpander: Send + Sync {
    fn expand(&self, trigger: &MacroTrigger) -> Result<String, MacroError>;
}

/// The annotated item a macro is being asked to expand.
pub struct MacroTrigger<'a> {
    /// Key-value pairs from the top-level attribute, e.g.
    /// `#[il2cpp_proxy(asm="X", class="Y")]` → `[("asm","X"), ("class","Y")]`.
    pub attr_args: Vec<(String, String)>,
    /// Whether this is a struct or impl block, with their metadata.
    pub kind: TriggerKind,
    /// The raw AST item, for expanders that need more than the extracted
    /// metadata.
    pub item: &'a Item,
}

/// What kind of item triggered the expansion.
pub enum TriggerKind {
    Struct {
        name: String,
        fields: Vec<MacroField>,
    },
    Impl {
        self_ty: String,
        functions: Vec<MacroFn>,
    },
}

/// A field of the annotated struct.
pub struct MacroField {
    pub name: String,
    pub ty: String,
    pub attrs: Vec<MacroAttr>,
}

/// A function inside an annotated impl block.
pub struct MacroFn {
    pub name: String,
    pub attrs: Vec<MacroAttr>,
}

/// A parsed attribute on a field or function.
pub struct MacroAttr {
    pub name: String,
    /// Key-value pairs for `#[name(k1="v1", k2="v2")]`.
    pub args: Vec<(String, String)>,
    /// The value for `#[name="value"]` (NameValue form).
    pub value: Option<String>,
}

impl MacroAttr {
    /// The value: either the NameValue `#[name="val"]` form or the `name`
    /// key from `#[name(name="val")]`.
    pub fn value_or_name_arg(&self) -> Option<&str> {
        self.value.as_deref().or_else(|| self.arg("name"))
    }

    pub fn arg(&self, key: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn arg_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.arg(key).unwrap_or(default)
    }

    /// Whether a bare word is present in the args, e.g. `static` in
    /// `#[il2cpp_field(name="x", static)]`.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.args.iter().any(|(k, v)| k == flag && v.is_empty())
    }
}

/// An error produced by a macro expander.
pub struct MacroError {
    pub message: String,
    pub span: Option<Span>,
}

impl MacroError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }

    pub fn with_span(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
        }
    }
}
