//! `#[il2cpp_proxy]` macro expander — generates constructors and hook wiring
//! for structs that map to IL2CPP classes.

use xur_resolve::macros::{MacroExpander, MacroError, MacroField, MacroTrigger, TriggerKind};

pub(crate) struct Il2CppProxyExpander;

impl MacroExpander for Il2CppProxyExpander {
    fn expand(&self, trigger: &MacroTrigger) -> Result<String, MacroError> {
        match &trigger.kind {
            TriggerKind::Struct { name, fields } => {
                expand_struct(name, fields, &trigger.attr_args)
            }
            TriggerKind::Impl { self_ty, functions } => {
                expand_impl(self_ty, functions, &trigger.attr_args)
            }
        }
    }
}

/// Generates `from_runtime` and `from_class` constructors for a proxy struct.
fn expand_struct(
    name: &str,
    fields: &[MacroField],
    attr_args: &[(String, String)],
) -> Result<String, MacroError> {
    let asm = find_arg(attr_args, "asm")
        .ok_or_else(|| MacroError::new("#[il2cpp_proxy] requires `asm` argument"))?;
    let ns = find_arg(attr_args, "ns").unwrap_or("");
    let class = find_arg(attr_args, "class")
        .ok_or_else(|| MacroError::new("#[il2cpp_proxy] requires `class` argument"))?;

    let mut field_inits = Vec::new();
    let mut method_temps = Vec::new();

    for field in fields {
        match classify_field(field) {
            FieldKind::Il2CppField { il2cpp_name, _is_static: _ } => {
                field_inits.push(format!(
                    "            {}: _cls.field(\"{il2cpp_name}\")?,",
                    field.name
                ));
            }
            FieldKind::Il2CppMethod { il2cpp_name, params } => {
                let temp = format!("_m_{}", field.name);
                method_temps.push(format!(
                    "        let {temp} = _cls.method(\"{il2cpp_name}\", {params})?;"
                ));
                field_inits.push(format!("            {}: {temp},", field.name));
            }
            FieldKind::Il2CppHook { il2cpp_name, params } => {
                let temp = format!("_m_{}", field.name);
                method_temps.push(format!(
                    "        let {temp} = _cls.method(\"{il2cpp_name}\", {params})?;"
                ));
                field_inits.push(format!(
                    "            {}: {temp}.hook_void_block(),",
                    field.name
                ));
            }
            FieldKind::Plain => continue,
        }
    }

    let method_temps_str = if method_temps.is_empty() {
        String::new()
    } else {
        format!("{}\n", method_temps.join("\n"))
    };

    let field_inits_str = field_inits.join("\n");

    Ok(format!(
        r#"impl {name} {{
    fn from_runtime(rt: il2cpp::Runtime) -> Result<{name}> {{
        let _cls = rt.class("{asm}", "{ns}", "{class}")?;
        {name}::from_class(_cls)
    }}

    fn from_class(_cls: il2cpp::Class) -> Result<{name}> {{
{method_temps_str}        Ok({name} {{
{field_inits_str}
        }})
    }}
}}"#
    ))
}

/// Generates `setup_hooks` for an impl block with `#[il2cpp_hook]` functions.
///
/// When hook functions have extra parameters beyond `ctx`, those are
/// treated as hook data: the expander generates `Type::default()` calls,
/// captures the data in closures, and returns a record with handles.
fn expand_impl(
    self_ty: &str,
    functions: &[xur_resolve::macros::MacroFn],
    _attr_args: &[(String, String)],
) -> Result<String, MacroError> {
    let mut data_lets = Vec::new();
    let mut hook_lines = Vec::new();
    let mut record_fields = Vec::new();

    for func in functions {
        let Some(hook_attr) = func.attrs.iter().find(|a| a.name == "il2cpp_hook") else {
            continue;
        };
        let method_field = hook_attr
            .arg("method")
            .or_else(|| hook_attr.value_or_name_arg())
            .ok_or_else(|| {
                MacroError::new(format!(
                    "#[il2cpp_hook] on `{}` requires `method` argument",
                    func.name
                ))
            })?;

        // Extra params beyond the first (ctx) are hook data.
        let data_params: Vec<_> = func.params.iter().skip(1).collect();

        if data_params.is_empty() {
            hook_lines.push(format!(
                "        self.{method_field}.hook({self_ty}::{});",
                func.name
            ));
        } else {
            for param in &data_params {
                let var = format!("_{fn}_{name}", fn = func.name, name = param.name);
                data_lets.push(format!(
                    "        let {var} = {ty}::default();",
                    ty = param.ty
                ));
            }

            let call_args: Vec<String> = std::iter::once("ctx".to_owned())
                .chain(data_params.iter().map(|p| {
                    format!("_{fn}_{name}", fn = func.name, name = p.name)
                }))
                .collect();

            hook_lines.push(format!(
                "        self.{method_field}.hook(|ctx| {{\n            \
                 {self_ty}::{}({})\n        }});",
                func.name,
                call_args.join(", ")
            ));

            for param in &data_params {
                record_fields.push(format!(
                    "{}: _{fn}_{name}",
                    param.name,
                    fn = func.name,
                    name = param.name
                ));
            }
        }
    }

    if hook_lines.is_empty() {
        return Ok(String::new());
    }

    let data_lets_str = if data_lets.is_empty() {
        String::new()
    } else {
        format!("{}\n", data_lets.join("\n"))
    };

    let body = if record_fields.is_empty() {
        format!(
            "impl {self_ty} {{\n    fn setup_hooks(self) {{\n{data_lets_str}{}\n    }}\n}}",
            hook_lines.join("\n")
        )
    } else {
        format!(
            "impl {self_ty} {{\n    fn setup_hooks(self) -> @ {{\n\
             {data_lets_str}{}\n        @{{ {} }}\n    }}\n}}",
            hook_lines.join("\n"),
            record_fields.join(", ")
        )
    };

    Ok(body)
}

enum FieldKind<'a> {
    Il2CppField { il2cpp_name: &'a str, _is_static: bool },
    Il2CppMethod { il2cpp_name: &'a str, params: &'a str },
    Il2CppHook { il2cpp_name: &'a str, params: &'a str },
    Plain,
}

fn classify_field<'a>(field: &'a MacroField) -> FieldKind<'a> {
    for attr in &field.attrs {
        match attr.name.as_str() {
            "il2cpp_field" => {
                let il2cpp_name = attr
                    .value_or_name_arg()
                    .unwrap_or(field.name.as_str());
                return FieldKind::Il2CppField {
                    il2cpp_name,
                    _is_static: attr.has_flag("static"),
                };
            }
            "il2cpp_method" => {
                let il2cpp_name = attr
                    .value_or_name_arg()
                    .unwrap_or(field.name.as_str());
                let params = attr.arg_or("params", "0");
                return FieldKind::Il2CppMethod { il2cpp_name, params };
            }
            "il2cpp_hook" => {
                let il2cpp_name = attr
                    .value_or_name_arg()
                    .unwrap_or(field.name.as_str());
                let params = attr.arg_or("params", "0");
                return FieldKind::Il2CppHook { il2cpp_name, params };
            }
            _ => {}
        }
    }
    FieldKind::Plain
}

fn find_arg<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
