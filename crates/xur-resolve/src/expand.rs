//! Compile-time macro expansion, run after parsing and before name resolution.
//!
//! For each module, items carrying an attribute whose name matches a
//! registered [`MacroExpander`] are handed to that expander. The expander
//! returns xur source text, which is parsed as new items and spliced into
//! the module alongside the original.

use std::collections::HashMap;

use xur_ast::{
    Attribute, Impl, Item, ItemKind, LitKind, MetaItemInner, MetaItemKind, Module, TyKind,
    VariantData,
};
use xur_ast::NodeIdGen;
use xur_errors::Diag;

use crate::loader::Crate;
use crate::macros::{MacroAttr, MacroField, MacroFn, MacroParam, MacroTrigger, TriggerKind};
use crate::NativeProvider;

/// Expands macros in every module of `krate`, mutating it in place.
pub(crate) fn expand_macros(
    krate: &mut Crate,
    natives: &dyn NativeProvider,
    ids: &mut NodeIdGen,
    diags: &mut Vec<Diag>,
) {
    for i in 0..krate.modules.len() {
        let generated =
            expand_module(krate.modules[i].ast, natives, &mut krate.source_map, ids, diags);
        if !generated.is_empty() {
            let mut items = krate.modules[i].ast.items.clone();
            items.extend(generated);
            let new_module = Module {
                items,
                span: krate.modules[i].ast.span,
            };
            krate.modules[i].ast = Box::leak(Box::new(new_module));
        }
    }
}

fn expand_module(
    module: &Module,
    natives: &dyn NativeProvider,
    source_map: &mut xur_span::SourceMap,
    ids: &mut NodeIdGen,
    diags: &mut Vec<Diag>,
) -> Vec<Item> {
    let use_map = build_use_map(module);
    let mut generated = Vec::new();
    for item in &module.items {
        for attr in &item.attrs {
            let Some(path) = resolve_attr_path(attr, &use_map) else {
                continue;
            };
            let segments: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
            let Some(expander) = natives.macro_expander(&segments) else {
                continue;
            };
            let trigger = match build_trigger(attr, item) {
                Some(t) => t,
                None => continue,
            };
            match expander.expand(&trigger) {
                Ok(source) => {
                    let (items, parse_diags) =
                        xur_parser::parse_items(&source, source_map, ids);
                    diags.extend(parse_diags);
                    generated.extend(items);
                }
                Err(error) => {
                    let span = error.span.unwrap_or(item.span);
                    diags.push(Diag::error(error.message, span));
                }
            }
        }
    }
    generated
}

fn build_trigger<'a>(attr: &Attribute, item: &'a Item) -> Option<MacroTrigger<'a>> {
    let attr_args = extract_attr_args(attr);
    let kind = match &item.kind {
        ItemKind::Struct(ident, _, data) => {
            let fields = extract_fields(data);
            TriggerKind::Struct {
                name: ident.name.to_string(),
                fields,
            }
        }
        ItemKind::Impl(imp) => {
            let self_ty = ty_to_string(&imp.self_ty);
            let functions = extract_impl_fns(imp);
            TriggerKind::Impl {
                self_ty,
                functions,
            }
        }
        _ => return None,
    };
    Some(MacroTrigger {
        attr_args,
        kind,
        item,
    })
}

fn extract_attr_args(attr: &Attribute) -> Vec<(String, String)> {
    match &attr.meta.kind {
        MetaItemKind::List(items) => items
            .iter()
            .filter_map(|inner| match inner {
                MetaItemInner::MetaItem(meta) => {
                    let key = meta
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::");
                    match &meta.kind {
                        MetaItemKind::NameValue(lit) => {
                            Some((key, lit_to_string(lit)))
                        }
                        MetaItemKind::Word => Some((key, String::new())),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect(),
        MetaItemKind::NameValue(lit) => {
            vec![("value".to_owned(), lit_to_string(lit))]
        }
        MetaItemKind::Word => Vec::new(),
    }
}

fn extract_fields(data: &VariantData) -> Vec<MacroField> {
    let fields = match data {
        VariantData::Struct(fields) | VariantData::Tuple(fields) => fields,
        VariantData::Unit => return Vec::new(),
    };
    fields
        .iter()
        .map(|field| MacroField {
            name: field
                .ident
                .map(|id| id.name.to_string())
                .unwrap_or_default(),
            ty: ty_to_string(&field.ty),
            attrs: extract_field_attrs(&field.attrs),
        })
        .collect()
}

fn extract_impl_fns(imp: &Impl) -> Vec<MacroFn> {
    imp.items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Fn(func) => {
                let params = func
                    .decl
                    .inputs
                    .iter()
                    .map(|param| {
                        let name = match &param.pat.kind {
                            xur_ast::PatKind::Ident(ident) => ident.name.to_string(),
                            _ => "_".to_string(),
                        };
                        MacroParam {
                            name,
                            ty: ty_to_string(&param.ty),
                        }
                    })
                    .collect();
                Some(MacroFn {
                    name: func.ident.name.to_string(),
                    attrs: extract_field_attrs(&item.attrs),
                    params,
                })
            }
            _ => None,
        })
        .collect()
}

fn extract_field_attrs(attrs: &[Attribute]) -> Vec<MacroAttr> {
    attrs
        .iter()
        .map(|attr| {
            let name = attr_name(attr).unwrap_or_default().to_owned();
            let (args, value) = match &attr.meta.kind {
                MetaItemKind::NameValue(lit) => {
                    (Vec::new(), Some(lit_to_string(lit)))
                }
                MetaItemKind::List(items) => {
                    let args = items
                        .iter()
                        .filter_map(|inner| match inner {
                            MetaItemInner::MetaItem(meta) => {
                                let key = meta
                                    .path
                                    .segments
                                    .iter()
                                    .map(|s| s.ident.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join("::");
                                match &meta.kind {
                                    MetaItemKind::NameValue(lit) => {
                                        Some((key, lit_to_string(lit)))
                                    }
                                    MetaItemKind::Word => {
                                        Some((key, String::new()))
                                    }
                                    _ => None,
                                }
                            }
                            _ => None,
                        })
                        .collect();
                    (args, None)
                }
                MetaItemKind::Word => (Vec::new(), None),
            };
            MacroAttr { name, args, value }
        })
        .collect()
}

/// Resolves an attribute's path to a full module path suitable for
/// `NativeProvider::macro_expander`. Multi-segment attributes like
/// `#[std::stub::il2cpp::il2cpp_proxy]` are taken directly. Single-segment
/// attributes like `#[il2cpp_proxy]` are looked up in `use_map` to find
/// their full imported path.
fn resolve_attr_path(
    attr: &Attribute,
    use_map: &HashMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let segments: Vec<&str> = attr
        .meta
        .path
        .segments
        .iter()
        .map(|s| s.ident.name.as_str())
        .collect();
    if segments.len() > 1 {
        Some(segments.iter().map(|s| s.to_string()).collect())
    } else if segments.len() == 1 {
        use_map.get(segments[0]).cloned()
    } else {
        None
    }
}

/// Scans a module's `use` items to build a map from the last segment of
/// each imported path to the full path segments. For example,
/// `use std::stub::il2cpp::il2cpp_proxy;` maps `"il2cpp_proxy"` →
/// `["std", "stub", "il2cpp", "il2cpp_proxy"]`.
fn build_use_map(module: &Module) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for item in &module.items {
        if let ItemKind::Use(trees) = &item.kind {
            for tree in trees {
                let segments: Vec<String> = tree
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.name.to_string())
                    .collect();
                if let Some(last) = segments.last() {
                    map.insert(last.clone(), segments);
                }
            }
        }
    }
    map
}

fn attr_name(attr: &Attribute) -> Option<&str> {
    let segments = &attr.meta.path.segments;
    if segments.len() == 1 {
        Some(segments[0].ident.name.as_str())
    } else {
        None
    }
}

fn lit_to_string(lit: &xur_ast::Lit) -> String {
    match &lit.kind {
        LitKind::Str(s) => s.to_string(),
        LitKind::Int(n) => n.to_string(),
        LitKind::Float(s) => s.to_string(),
        LitKind::Bool(b) => b.to_string(),
        LitKind::ByteStr(_) | LitKind::Err => String::new(),
    }
}

fn ty_to_string(ty: &xur_ast::Ty) -> String {
    match &ty.kind {
        TyKind::Path(path) => path
            .segments
            .iter()
            .map(|seg| {
                let name = seg.ident.name.to_string();
                match &seg.args {
                    Some(args) => {
                        let params: Vec<_> = args.args.iter().map(ty_to_string).collect();
                        format!("{name}<{}>", params.join(", "))
                    }
                    None => name,
                }
            })
            .collect::<Vec<_>>()
            .join("::"),
        TyKind::Tuple(tys) => {
            let inner: Vec<_> = tys.iter().map(ty_to_string).collect();
            format!("({})", inner.join(", "))
        }
        TyKind::List(ty) => format!("[{}]", ty_to_string(ty)),
        TyKind::Ptr(ty) => format!("*{}", ty_to_string(ty)),
        TyKind::Paren(ty) => format!("({})", ty_to_string(ty)),
        TyKind::Fn { params, ret } => {
            let ps: Vec<_> = params.iter().map(ty_to_string).collect();
            match ret {
                Some(r) => format!("fn({}) -> {}", ps.join(", "), ty_to_string(r)),
                None => format!("fn({})", ps.join(", ")),
            }
        }
        TyKind::Infer => "_".to_owned(),
        TyKind::ImplicitSelf => "self".to_owned(),
        TyKind::Record => "@".to_owned(),
        TyKind::Err => "_".to_owned(),
    }
}

