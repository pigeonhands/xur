//! `std::re::dotnet`: .NET assembly reading and writing, with
//! [hao](https://docs.rs/hao).
//!
//! `dotnet::open(path)` loads a module and its dependencies;
//! `dotnet::parse(bytes)` parses raw bytes without resolving.
//! The returned `Module` gives access to every type, method, field,
//! property and event in the assembly.
//!
//! ```text
//! let module = std::re::dotnet::open("Game.dll")?;
//! for ty in module.all_types() {
//!     io::println("{}.{}", ty.namespace, ty.name);
//! }
//! ```

mod emitter;
pub(super) mod il2cpp;
mod runtime;
mod sig;

use std::sync::{Arc, Mutex};

use facet::{Facet, Peek};
use hao::dotnet::entries::{
    Assembly, AssemblyRef, Entry, Event, ExportedType, Field, File, GenericParam, ManifestResource,
    MemberRef, Method, MethodSpec, ModuleDef, ModuleRef, Param, Property, TypeDef, TypeRef,
    TypeSpec, WeakEntry,
};
use hao::dotnet::il::MemberToken;
use hao::dotnet::md::streams::tables_stream::{
    AssemblyFlags, EventFlags, FieldFlags, FileFlags, GenericParamFlags, ManifestResourceFlags,
    MethodFlags, MethodImplFlags, PInvokeMapFlags, ParamFlags, PropertyFlags, TypeAttributes,
};
use hao::dotnet::module::{ModuleKind, TargetRuntime};
use hao::dotnet::signature::{MethodSig, SignatureFlags, TypeSig};
use xur_runtime::convert::{into_value, reflect_error, to_value};
use xur_runtime::registry::{ModuleBuilder, XurType};
use xur_runtime::{Registry, RuntimeError, Value};

use crate::{error_text, result};

// ---------------------------------------------------------------------------
// Opaque wrapper types
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetModule(Arc<Mutex<hao::Module>>);

impl std::fmt::Display for DotnetModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Module({})", self.0.lock().unwrap().name())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetEmitter(Arc<Mutex<Option<hao::dotnet::il::Emitter>>>);

impl std::fmt::Display for DotnetEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Emitter")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetToken(MemberToken);

impl std::fmt::Display for DotnetToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetTypeSig(TypeSig);

impl std::fmt::Display for DotnetTypeSig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetMethodSig(MethodSig);

impl std::fmt::Display for DotnetMethodSig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MethodSig")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetTypeDef(Entry<TypeDef>);

impl std::fmt::Display for DotnetTypeDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TypeDef({})", self.0.get().full_name())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetMethodEntry(Entry<Method>);

impl std::fmt::Display for DotnetMethodEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Method({})", self.0.get().name)
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetMethodBody(hao::dotnet::il::MethodBody);

impl std::fmt::Display for DotnetMethodBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MethodBody({} instructions)", self.0.instructions.len())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetTypeRef(hao::dotnet::entries::TypeDefOrRef);

impl std::fmt::Display for DotnetTypeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TypeRef({})", self.0.full_name())
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(super) fn register(registry: &Registry, re: &mut ModuleBuilder<'_>) {
    let mut dotnet = re.submodule("dotnet");
    register_in(registry, &mut dotnet);

    let skip = [hao::Module::SHAPE];
    super::walk::register(
        registry,
        &mut dotnet,
        hao::reflect::roots(),
        submodule_of,
        &skip,
    );

    il2cpp::register(registry, &mut dotnet);
    runtime::register(registry, &mut dotnet);
}

fn submodule_of(shape: &facet::Shape) -> Option<&'static str> {
    let path = shape.module_path?;
    if !path.starts_with("hao") {
        return None;
    }
    let mut segments = path.split("::");
    let _ = segments.next(); // "hao"
    match segments.next() {
        Some("dotnet") => match segments.next() {
            Some("il") => Some("il"),
            Some("signature") => Some("sig"),
            _ => Some(""),
        },
        _ => Some(""),
    }
}

// ---------------------------------------------------------------------------
// Core registration: module I/O + methods
// ---------------------------------------------------------------------------

fn register_in(registry: &Registry, dotnet: &mut ModuleBuilder<'_>) {
    register_flags(registry);
    register_entries(registry);

    dotnet.ty::<DotnetToken>("MemberToken");
    dotnet.ty::<DotnetTypeSig>("TypeSigDef");
    dotnet.ty::<DotnetMethodSig>("MethodSigDef");
    dotnet.ty::<DotnetTypeDef>("TypeDefEntry");
    dotnet.ty::<DotnetMethodEntry>("MethodEntry");
    dotnet.ty::<DotnetMethodBody>("MethodBody");
    dotnet.ty::<DotnetTypeRef>("BaseTypeRef");

    // --- module creation / loading ---

    dotnet
        .raw_function_returns::<DotnetModule>("new", |cx, args| {
            let (name, kind) = match args {
                [Value::Str(name)] => (name.as_ref(), ModuleKind::Dll),
                [Value::Str(name), Value::Str(kind)] => {
                    let k = match kind.as_ref() {
                        "dll" => ModuleKind::Dll,
                        "console" | "exe" => ModuleKind::Console,
                        "windows" => ModuleKind::Windows,
                        other => {
                            return Err(RuntimeError::new(format!(
                                "unknown module kind `{other}`; use \"dll\", \"console\" or \"windows\""
                            )))
                        }
                    };
                    (name.as_ref(), k)
                }
                _ => return Err(RuntimeError::new("`new` takes a name and optional kind")),
            };
            let module = hao::Module::new(name, kind, TargetRuntime::NetCore(9));
            let v = into_value(cx, DotnetModule(Arc::new(Mutex::new(module))))?;
            Ok(v)
        })
        .raw_function_returns::<DotnetModule>("open", |cx, args| {
            let [Value::Str(path)] = args else {
                return Err(RuntimeError::new("`open` takes a path string"));
            };
            match hao::Module::from_path(path.as_ref()) {
                Ok(module) => {
                    let v = into_value(cx, DotnetModule(Arc::new(Mutex::new(module))))?;
                    Ok(result(cx, Ok(v)))
                }
                Err(e) => Ok(result(cx, Err(error_text(e)))),
            }
        })
        .raw_function_returns::<DotnetModule>("parse", |cx, args| {
            let [Value::Bytes(bytes)] = args else {
                return Err(RuntimeError::new("`parse` takes bytes"));
            };
            let bytes = bytes.lock().unwrap();
            match hao::Module::from_bytes(&bytes) {
                Ok(module) => {
                    let v = into_value(cx, DotnetModule(Arc::new(Mutex::new(module))))?;
                    Ok(result(cx, Ok(v)))
                }
                Err(e) => Ok(result(cx, Err(error_text(e)))),
            }
        });

    // --- read-only module methods ---

    dotnet
        .ty::<DotnetModule>("Module")
        .method("name", |m: &DotnetModule| m.0.lock().unwrap().name())
        .method("runtime_version", |m: &DotnetModule| {
            m.0.lock().unwrap().runtime_version.clone()
        })
        .method("all_types", |m: &DotnetModule| {
            m.0.lock().unwrap().all_types()
        })
        .method("all_methods", |m: &DotnetModule| {
            m.0.lock().unwrap().all_methods()
        })
        .method("all_fields", |m: &DotnetModule| {
            m.0.lock().unwrap().all_fields()
        })
        .method(
            "find_type",
            |m: &DotnetModule, namespace: String, name: String| {
                m.0.lock().unwrap().find_type(&namespace, &name)
            },
        )
        .method(
            "find_type_by_full_name",
            |m: &DotnetModule, full_name: String| {
                m.0.lock().unwrap().find_type_by_full_name(&full_name)
            },
        )
        .method("global_type", |m: &DotnetModule| {
            m.0.lock().unwrap().global_type()
        })
        .method("original_image", |m: &DotnetModule| {
            m.0.lock().unwrap().original_image().to_vec()
        })
        .method("to_csharp", |m: &DotnetModule| {
            m.0.lock().unwrap().to_csharp()
        })
        .raw_function("write", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`write` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let bytes = obj
                .with(|m: &DotnetModule| m.0.lock().unwrap().write().map_err(error_text))
                .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            match bytes {
                Ok(b) => Ok(result(cx, Ok(Value::bytes(b)))),
                Err(e) => Ok(result(cx, Err(e))),
            }
        })
        .raw_function("write_to_path", |cx, args| {
            let [this, Value::Str(path)] = args else {
                return Err(RuntimeError::new("`write_to_path` takes a path string"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let res = obj
                .with(|m: &DotnetModule| {
                    m.0.lock()
                        .unwrap()
                        .write_to_path(path.as_ref())
                        .map_err(error_text)
                })
                .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            match res {
                Ok(()) => Ok(result(cx, Ok(Value::Unit))),
                Err(e) => Ok(result(cx, Err(e))),
            }
        })
        .raw_function_returns_xur("add_assembly_ref", XurType::UNIT, |_cx, args| {
            let [this, Value::Str(name)] = args else {
                return Err(RuntimeError::new(
                    "`add_assembly_ref` takes an assembly name",
                ));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            obj.with(|m: &DotnetModule| {
                m.0.lock()
                    .unwrap()
                    .add_assembly_ref(name.as_ref(), Default::default(), &[]);
            })
            .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            Ok(Value::Unit)
        })
        .raw_function_returns::<DotnetToken>("import_method", |cx, args| {
            let [
                this,
                Value::Str(assembly),
                Value::Str(type_name),
                Value::Str(method_name),
                Value::Native(sig),
            ] = args
            else {
                return Err(RuntimeError::new(
                    "`import_method` takes (assembly, type, method, method_sig)",
                ));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let method_sig = sig
                .with(|s: &DotnetMethodSig| s.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a MethodSig"))?;
            let res = obj
                .with(|m: &DotnetModule| {
                    let mut module = m.0.lock().unwrap();
                    let ty = module
                        .import_type_from(assembly.as_ref(), type_name.as_ref())
                        .map_err(error_text)?;
                    module
                        .import_method_on(&ty, method_name.as_ref(), method_sig)
                        .map_err(error_text)
                })
                .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            match res {
                Ok(token) => {
                    let v = into_value(cx, DotnetToken(token))?;
                    Ok(result(cx, Ok(v)))
                }
                Err(e) => Ok(result(cx, Err(e))),
            }
        })
        .raw_function_returns::<DotnetTypeRef>("corlib_type", |cx, args| {
            let [this, Value::Str(full_name)] = args else {
                return Err(RuntimeError::new("`corlib_type` takes a full type name"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let res = obj
                .with(|m: &DotnetModule| {
                    m.0.lock()
                        .unwrap()
                        .corlib_type(full_name.as_ref())
                        .map_err(error_text)
                })
                .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            match res {
                Ok(tyref) => {
                    let v = into_value(cx, DotnetTypeRef(tyref))?;
                    Ok(result(cx, Ok(v)))
                }
                Err(e) => Ok(result(cx, Err(e))),
            }
        })
        .raw_function_returns_xur("add_type", XurType::UNIT, |_cx, args| {
            let [this, Value::Native(typedef)] = args else {
                return Err(RuntimeError::new("`add_type` takes a TypeDef"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let entry = typedef
                .with(|t: &DotnetTypeDef| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a TypeDef"))?;
            obj.with(|m: &DotnetModule| {
                m.0.lock().unwrap().types.push(entry);
            })
            .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            Ok(Value::Unit)
        })
        .raw_function_returns_xur("set_entry_point", XurType::UNIT, |_cx, args| {
            let [this, Value::Native(method)] = args else {
                return Err(RuntimeError::new("`set_entry_point` takes a Method"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Module`"));
            };
            let entry = method
                .with(|m: &DotnetMethodEntry| m.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Method"))?;
            obj.with(|m: &DotnetModule| {
                m.0.lock().unwrap().entry_point = Some(entry);
            })
            .ok_or_else(|| RuntimeError::new("expected a `Module`"))?;
            Ok(Value::Unit)
        });

    // --- type/method construction ---

    dotnet.raw_function_returns::<DotnetTypeDef>("static_class", |cx, args| {
        let [
            Value::Native(module),
            Value::Str(namespace),
            Value::Str(name),
        ] = args
        else {
            return Err(RuntimeError::new(
                "`static_class` takes (module, namespace, name)",
            ));
        };
        let base = module
            .with(|m: &DotnetModule| {
                m.0.lock()
                    .unwrap()
                    .corlib_type("System.Object")
                    .map_err(error_text)
            })
            .ok_or_else(|| RuntimeError::new("expected a Module"))?
            .map_err(RuntimeError::new)?;
        let td = TypeDef::new_static_class(namespace.as_ref(), name.as_ref(), base);
        let entry = Entry::new(td);
        into_value(cx, DotnetTypeDef(entry))
    });

    dotnet.raw_function_returns::<DotnetMethodEntry>("public_static_method", |cx, args| {
        let [
            Value::Str(name),
            Value::Native(sig),
            Value::Native(body_val),
        ] = args
        else {
            return Err(RuntimeError::new(
                "`public_static_method` takes (name, method_sig, body)",
            ));
        };
        let method_sig = sig
            .with(|s: &DotnetMethodSig| s.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a MethodSig"))?;
        let body = body_val
            .with(|b: &DotnetMethodBody| b.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a MethodBody"))?;
        let method = Method::public_static(name.as_ref(), method_sig).with_body(body);
        into_value(cx, DotnetMethodEntry(Entry::new(method)))
    });

    dotnet.raw_function_returns::<DotnetMethodEntry>("add_method", |cx, args| {
        let [Value::Native(typedef), Value::Native(method)] = args else {
            return Err(RuntimeError::new("`add_method` takes (typedef, method)"));
        };
        let td_entry = typedef
            .with(|t: &DotnetTypeDef| t.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a TypeDef"))?;
        let m_entry = method
            .with(|m: &DotnetMethodEntry| m.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a Method"))?;
        let m_inner = m_entry.get().clone();
        let result_entry = TypeDef::add_method(&td_entry, m_inner);
        into_value(cx, DotnetMethodEntry(result_entry))
    });

    emitter::register(dotnet);
    sig::register(dotnet);

    // --- flags ---

    super::flags::register_flags!(dotnet, "type_attributes", TypeAttributes, u32);
    super::flags::register_flags!(dotnet, "method_flags", MethodFlags, u16);
    super::flags::register_flags!(dotnet, "field_flags", FieldFlags, u16);
    super::flags::register_flags!(dotnet, "param_flags", ParamFlags, u16);
    super::flags::register_flags!(dotnet, "event_flags", EventFlags, u16);
    super::flags::register_flags!(dotnet, "property_flags", PropertyFlags, u16);
    super::flags::register_flags!(dotnet, "assembly_flags", AssemblyFlags, u32);
    super::flags::register_flags!(dotnet, "generic_param_flags", GenericParamFlags, u16);
}

// ---------------------------------------------------------------------------
// Converters: flags, entries, weak entries
// ---------------------------------------------------------------------------

macro_rules! flags_converter {
    ($registry:expr, $($ty:ty),* $(,)?) => {
        $(
            $registry.converter(<$ty as Facet>::SHAPE, |_, peek: Peek<'_, 'static>| {
                let flags: &$ty = peek.get().map_err(reflect_error)?;
                Ok(Value::Int(flags.bits() as i64))
            });
        )*
    }
}

macro_rules! entry_converter {
    ($registry:expr, $($ty:ty),* $(,)?) => {
        $(
            $registry.converter(<Entry<$ty> as Facet>::SHAPE, |cx, peek: Peek<'_, 'static>| {
                let entry: &Entry<$ty> = peek.get().map_err(reflect_error)?;
                let guard = entry.get();
                to_value(cx, &*guard)
            });
        )*
    }
}

macro_rules! weak_entry_converter {
    ($registry:expr, $($ty:ty),* $(,)?) => {
        $(
            $registry.converter(<WeakEntry<$ty> as Facet>::SHAPE, |_, _peek: Peek<'_, 'static>| {
                Ok(Value::Unit)
            });
        )*
    }
}

fn register_entries(registry: &Registry) {
    entry_converter!(
        registry,
        TypeDef,
        Method,
        Field,
        Property,
        Event,
        GenericParam,
        Param,
        Assembly,
        AssemblyRef,
        TypeRef,
        TypeSpec,
        MemberRef,
        MethodSpec,
        ModuleRef,
        ModuleDef,
        File,
        ExportedType,
        ManifestResource,
    );
    weak_entry_converter!(registry, TypeDef, Method, Field,);
}

fn register_flags(registry: &Registry) {
    flags_converter!(
        registry,
        TypeAttributes,
        FieldFlags,
        MethodFlags,
        MethodImplFlags,
        ParamFlags,
        EventFlags,
        PropertyFlags,
        PInvokeMapFlags,
        AssemblyFlags,
        FileFlags,
        ManifestResourceFlags,
        GenericParamFlags,
        SignatureFlags,
    );
}
