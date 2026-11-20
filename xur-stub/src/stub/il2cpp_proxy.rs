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

struct DataEntry {
    var_name: String,
    param_name: String,
    param_ty: String,
}

/// Generates `setup_hooks` for an impl block with `#[il2cpp_hook]` functions.
///
/// Hook data (extra params beyond `ctx`) is deduplicated by param name within
/// the impl block — hooks sharing the same param name+type get the same
/// instance. Each data entry becomes an `Option<Type>` parameter on
/// `setup_hooks`, so the caller can pass `Some(value)` for pre-configured
/// state or `None` to use `Type::default()`.
fn expand_impl(
    self_ty: &str,
    functions: &[xur_resolve::macros::MacroFn],
    _attr_args: &[(String, String)],
) -> Result<String, MacroError> {
    struct HookInfo {
        fn_name: String,
        method_field: String,
        data_params: Vec<(String, String)>, // (name, type)
    }

    let mut hooks = Vec::new();

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

        let data_params: Vec<(String, String)> = func
            .params
            .iter()
            .skip(1)
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();

        hooks.push(HookInfo {
            fn_name: func.name.clone(),
            method_field: method_field.to_owned(),
            data_params,
        });
    }

    if hooks.is_empty() {
        return Ok(String::new());
    }

    // Deduplicate data params by name. Same name + same type = shared.
    // Same name + different type = error.
    let mut data_entries: Vec<DataEntry> = Vec::new();
    let mut var_map: Vec<(String, String, usize)> = Vec::new();

    for hook in &hooks {
        for (pname, pty) in &hook.data_params {
            if let Some(idx) = data_entries.iter().position(|e| e.param_name == *pname) {
                if data_entries[idx].param_ty != *pty {
                    return Err(MacroError::new(format!(
                        "hook data `{pname}` has conflicting types: `{}` vs `{pty}`",
                        data_entries[idx].param_ty
                    )));
                }
                var_map.push((hook.fn_name.clone(), pname.clone(), idx));
            } else {
                let idx = data_entries.len();
                data_entries.push(DataEntry {
                    var_name: format!("_{pname}"),
                    param_name: pname.clone(),
                    param_ty: pty.clone(),
                });
                var_map.push((hook.fn_name.clone(), pname.clone(), idx));
            }
        }
    }

    // Generate setup_hooks params.
    let sig_params: Vec<String> = data_entries
        .iter()
        .map(|e| format!("{}: Option<{}>", e.param_name, e.param_ty))
        .collect();

    // Generate let bindings.
    let data_lets: Vec<String> = data_entries
        .iter()
        .map(|e| {
            format!(
                "        let {} = {}.unwrap_or({}::default());",
                e.var_name, e.param_name, e.param_ty
            )
        })
        .collect();

    // Generate hook wiring.
    let mut hook_lines = Vec::new();
    for hook in &hooks {
        if hook.data_params.is_empty() {
            hook_lines.push(format!(
                "        self.{}.hook({self_ty}::{});",
                hook.method_field, hook.fn_name
            ));
        } else {
            let call_args: Vec<String> = std::iter::once("ctx".to_owned())
                .chain(hook.data_params.iter().map(|(pname, _)| {
                    let idx = var_map
                        .iter()
                        .find(|(fn_name, param_name, _)| {
                            *fn_name == hook.fn_name && *param_name == *pname
                        })
                        .unwrap()
                        .2;
                    data_entries[idx].var_name.clone()
                }))
                .collect();

            hook_lines.push(format!(
                "        self.{}.hook(|ctx| {{\n            \
                 {self_ty}::{}({})\n        }});",
                hook.method_field,
                hook.fn_name,
                call_args.join(", ")
            ));
        }
    }

    // Generate record fields.
    let record_fields: Vec<String> = data_entries
        .iter()
        .map(|e| format!("{}: {}", e.param_name, e.var_name))
        .collect();

    let data_lets_str = if data_lets.is_empty() {
        String::new()
    } else {
        format!("{}\n", data_lets.join("\n"))
    };

    let sig_extra = if sig_params.is_empty() {
        String::new()
    } else {
        format!(", {}", sig_params.join(", "))
    };

    let body = if record_fields.is_empty() {
        format!(
            "impl {self_ty} {{\n    fn setup_hooks(self{sig_extra}) {{\n\
             {data_lets_str}{}\n    }}\n}}",
            hook_lines.join("\n")
        )
    } else {
        format!(
            "impl {self_ty} {{\n    fn setup_hooks(self{sig_extra}) -> @ {{\n\
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
