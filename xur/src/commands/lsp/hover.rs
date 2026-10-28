use xur_ast::{FnRetTy, PatKind, VariantData};
use xur_bytecode::{ItemIndex, ItemRef, type_name};
use xur_resolve::{DefKind, Res, Resolutions};
use xur_runtime::Registry;
use xur_span::SourceMap;

pub fn describe(
    res: Res,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
    sm: &SourceMap,
) -> String {
    match res {
        Res::Def(DefKind::Fn, def) => {
            if let Some(ItemRef::Fn { item, .. }) = items.get(def) {
                let name = resolutions.def_path(def);
                let params: Vec<String> = item.decl.inputs.iter().map(|p| {
                    let ty = type_name(resolutions, &p.ty);
                    if let PatKind::Ident(ident) = &p.pat.kind {
                        format!("{ident}: {ty}")
                    } else {
                        ty
                    }
                }).collect();
                let ret = match &item.decl.output {
                    FnRetTy::Ty(ty) => format!(" -> {}", type_name(resolutions, ty)),
                    FnRetTy::Default(_) => String::new(),
                };
                format!("fn {name}({}){ret}", params.join(", "))
            } else {
                resolutions.describe(res, sm)
            }
        }
        Res::Def(DefKind::NativeFn(id), def) => {
            let name = resolutions.def_path(def);
            let native_fn = registry.function(id);
            if let Some(sig) = native_fn.signature.as_ref() {
                let params: Vec<String> = sig.params.iter().enumerate().map(|(i, shape)| {
                    let ty = registry.shape_xur_name(shape);
                    if let Some(pname) = native_fn.param_names.get(i)
                        && !pname.is_empty()
                    {
                        return format!("{pname}: {ty}");
                    }
                    ty
                }).collect();
                let ret = registry.shape_xur_name(sig.ret);
                if ret == "()" {
                    format!("fn {name}({})", params.join(", "))
                } else {
                    format!("fn {name}({}) -> {ret}", params.join(", "))
                }
            } else {
                format!("fn {name}(..)")
            }
        }
        Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
            let name = resolutions.defs.get(def).name;
            if let Some(fields) = items.named_fields(def) {
                if fields.is_empty() {
                    format!("struct {name}")
                } else {
                    let fields_str: Vec<String> = fields.iter().map(|f| {
                        let ty = type_name(resolutions, &f.ty);
                        if let Some(ident) = f.ident {
                            format!("    {ident}: {ty}")
                        } else {
                            format!("    {ty}")
                        }
                    }).collect();
                    format!("struct {name} {{\n{}\n}}", fields_str.join("\n"))
                }
            } else if let Some(fields) = items.fields(def) {
                let types: Vec<String> = fields.iter()
                    .map(|f| type_name(resolutions, &f.ty))
                    .collect();
                format!("struct {name}({})", types.join(", "))
            } else {
                format!("struct {name}")
            }
        }
        Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
            let data = resolutions.defs.get(def);
            let variant_name = data.name;
            let enum_name = data.parent
                .map(|p| resolutions.defs.get(p).name.to_string())
                .unwrap_or_default();
            if let Some(ItemRef::Ctor { data: vdata, .. }) = items.get(def) {
                match vdata {
                    VariantData::Unit => format!("{enum_name}::{variant_name}"),
                    VariantData::Struct(fields) => {
                        let fields_str: Vec<String> = fields.iter().map(|f| {
                            let ty = type_name(resolutions, &f.ty);
                            if let Some(ident) = f.ident {
                                format!("{ident}: {ty}")
                            } else {
                                ty
                            }
                        }).collect();
                        format!("{enum_name}::{variant_name} {{ {} }}", fields_str.join(", "))
                    }
                    VariantData::Tuple(fields) => {
                        let types: Vec<String> = fields.iter()
                            .map(|f| type_name(resolutions, &f.ty))
                            .collect();
                        format!("{enum_name}::{variant_name}({})", types.join(", "))
                    }
                }
            } else {
                format!("{enum_name}::{variant_name}")
            }
        }
        Res::Def(DefKind::Enum, def) => {
            let name = resolutions.defs.get(def).name;
            let variants = resolutions.scope_entries(def, xur_resolve::Namespace::Value);
            if variants.is_empty() {
                format!("enum {name}")
            } else {
                let variant_names: Vec<String> = variants.iter().map(|(sym, _)| {
                    format!("    {sym}")
                }).collect();
                format!("enum {name} {{\n{}\n}}", variant_names.join("\n"))
            }
        }
        _ => resolutions.describe(res, sm),
    }
}
