//! `std::stub::unreal` — Unreal Engine introspection for game mods.
//!
//! Wraps unrealkit to provide UObject iteration, name resolution, and
//! class introspection from xur scripts.
//!
//! ```text
//! use std::stub::unreal;
//!
//! let sdk = unreal::init(unreal::UE5, text_base, text_size).unwrap();
//! for obj in unreal::objects() {
//!     io::println(unreal::obj_full_name(obj));
//! }
//! ```

use std::sync::OnceLock;

use facet::Facet;
use unrealkit::UeVersion;
use xur_runtime::convert::into_value;
use xur_runtime::registry::Context;
use xur_runtime::{Registry, RuntimeError, Value};

static SDK: OnceLock<unrealkit::Sdk> = OnceLock::new();
static PROCESS_EVENT: OnceLock<unrealkit::ProcessEventFn> = OnceLock::new();

fn err(msg: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(msg.to_string())
}

fn get_sdk() -> Result<&'static unrealkit::Sdk, RuntimeError> {
    SDK.get().ok_or_else(|| err("unreal SDK not initialized — call unreal::init() first"))
}

fn xur_option(cx: &dyn Context, value: Option<Value>) -> Value {
    let lang = cx.lang();
    match value {
        Some(v) => Value::adt(lang.some, vec![v]),
        None => Value::adt(lang.none, Vec::new()),
    }
}

fn xur_result(cx: &dyn Context, value: Result<Value, String>) -> Value {
    let lang = cx.lang();
    match value {
        Ok(v) => Value::adt(lang.ok, vec![v]),
        Err(msg) => Value::adt(lang.err, vec![Value::str(&msg)]),
    }
}

// ---------------------------------------------------------------------------
// Opaque wrappers
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurSdk;

impl std::fmt::Display for XurSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnrealSdk")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurUObject(*const u8);

unsafe impl Send for XurUObject {}
unsafe impl Sync for XurUObject {}

impl std::fmt::Display for XurUObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(sdk) = get_sdk() {
            let obj = unrealkit::UObject::new(self.0, &sdk.names);
            write!(f, "UObject({})", obj.name())
        } else {
            write!(f, "UObject({:#x})", self.0 as usize)
        }
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurUClass(*const u8);

unsafe impl Send for XurUClass {}
unsafe impl Sync for XurUClass {}

impl std::fmt::Display for XurUClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(sdk) = get_sdk() {
            let cls = unrealkit::UClass::new(self.0, &sdk.names);
            write!(f, "UClass({})", cls.name())
        } else {
            write!(f, "UClass({:#x})", self.0 as usize)
        }
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurUProperty(*const u8);

unsafe impl Send for XurUProperty {}
unsafe impl Sync for XurUProperty {}

impl std::fmt::Display for XurUProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(sdk) = get_sdk() {
            let prop = unrealkit::FProperty::new(self.0, &sdk.names);
            write!(f, "UProperty({}: {})", prop.name(), prop.class_name())
        } else {
            write!(f, "UProperty({:#x})", self.0 as usize)
        }
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurUFunction(*const u8);

unsafe impl Send for XurUFunction {}
unsafe impl Sync for XurUFunction {}

impl std::fmt::Display for XurUFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(sdk) = get_sdk() {
            let func = unrealkit::UFunction::new(self.0, &sdk.names);
            write!(f, "UFunction({})", func.name())
        } else {
            write!(f, "UFunction({:#x})", self.0 as usize)
        }
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::unreal", |m| {

    // init(version: int, text_base: ptr, text_size: int) -> Result<UnrealSdk>
    m.raw_function_returns::<XurSdk>("init", |cx, args| {
        let (version_int, text_base, text_size) = match args {
            [Value::Int(v), Value::Ptr(base), Value::Int(size)] => (*v, *base, *size),
            _ => return Err(err("`init` takes (version: int, text_base: ptr, text_size: int)")),
        };

        let version = match version_int {
            4 => UeVersion::Ue4,
            5 => UeVersion::Ue5,
            _ => return Err(err(format!("unknown UE version: {version_int}, use 4 or 5"))),
        };

        match unsafe {
            unrealkit::discovery::discover(text_base as *const u8, text_size as usize, version)
        } {
            Ok(sdk) => {
                SDK.set(sdk).map_err(|_| err("unreal SDK already initialized"))?;
                let v = into_value(cx, XurSdk)?;
                Ok(xur_result(cx, Ok(v)))
            }
            Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
        }
    });

    // init_from_pointers(version: int, gnames: ptr, gobjects: ptr) -> Result<UnrealSdk>
    m.raw_function_returns::<XurSdk>("init_from_pointers", |cx, args| {
        let (version_int, gnames_ptr, gobjects_ptr) = match args {
            [Value::Int(v), Value::Ptr(gn), Value::Ptr(go)] => (*v, *gn, *go),
            _ => return Err(err("`init_from_pointers` takes (version, gnames_ptr, gobjects_ptr)")),
        };

        let version = match version_int {
            4 => UeVersion::Ue4,
            5 => UeVersion::Ue5,
            _ => return Err(err(format!("unknown UE version: {version_int}"))),
        };

        match unsafe {
            unrealkit::Sdk::from_pointers(
                gnames_ptr as *const u8,
                gobjects_ptr as *const u8,
                version,
            )
        } {
            Ok(sdk) => {
                SDK.set(sdk).map_err(|_| err("unreal SDK already initialized"))?;
                let v = into_value(cx, XurSdk)?;
                Ok(xur_result(cx, Ok(v)))
            }
            Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
        }
    });

    // objects() -> [UObject]
    m.raw_function_returns::<XurUObject>("objects", |cx, args| {
        if !args.is_empty() {
            return Err(err("`objects` takes no arguments"));
        }

        let sdk = get_sdk()?;
        let objects: Result<Vec<Value>, _> = sdk
            .objects()
            .map(|obj| into_value(cx, XurUObject(obj.ptr())))
            .collect();

        Ok(Value::list(objects?))
    });

    // find_object(name: str) -> Option<UObject>
    m.raw_function_returns::<XurUObject>("find_object", |cx, args| {
        let name = match args {
            [Value::Str(s)] => s.clone(),
            _ => return Err(err("`find_object` takes (name: str)")),
        };

        let sdk = get_sdk()?;
        let found = sdk
            .find_object_by_name(&name)
            .map(|obj| into_value(cx, XurUObject(obj.ptr())))
            .transpose()?;
        Ok(xur_option(cx, found))
    });

    // find_objects_by_class(class_name: str) -> [UObject]
    m.raw_function_returns::<XurUObject>("find_objects_by_class", |cx, args| {
        let class_name = match args {
            [Value::Str(s)] => s.clone(),
            _ => return Err(err("`find_objects_by_class` takes (class_name: str)")),
        };

        let sdk = get_sdk()?;
        let objects: Result<Vec<Value>, _> = sdk
            .objects()
            .filter(|obj| obj.class().name() == &*class_name)
            .map(|obj| into_value(cx, XurUObject(obj.ptr())))
            .collect();

        Ok(Value::list(objects?))
    });

    // set_process_event(addr: int) -> ()
    m.raw_function("set_process_event", |_cx, args| {
        let [Value::Int(addr)] = args else {
            return Err(err("`set_process_event` takes (addr: int)"));
        };
        let fn_ptr: unrealkit::ProcessEventFn = unsafe { std::mem::transmute(*addr as usize) };
        PROCESS_EVENT
            .set(fn_ptr)
            .map_err(|_| err("ProcessEvent already set"))?;
        Ok(Value::Unit)
    });

    // set_process_event_vtable(index: int) -> ()
    m.raw_function("set_process_event_vtable", |_cx, args| {
        let [Value::Int(index)] = args else {
            return Err(err("`set_process_event_vtable` takes (index: int)"));
        };
        let sdk = get_sdk()?;
        let obj = sdk
            .objects()
            .next()
            .ok_or_else(|| err("no UObjects found for vtable read"))?;
        let vtable_ptr = unsafe { *(obj.ptr() as *const *const *const u8) };
        let fn_ptr = unsafe { *vtable_ptr.add(*index as usize) };
        let pe: unrealkit::ProcessEventFn = unsafe { std::mem::transmute(fn_ptr) };
        PROCESS_EVENT
            .set(pe)
            .map_err(|_| err("ProcessEvent already set"))?;
        Ok(Value::Unit)
    });

    // --- UObject type ---

    m.ty::<XurUObject>("UObject")
        .function("name", |obj: &XurUObject| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UObject::new(obj.0, &sdk.names).name()
        })
        .function("full_name", |obj: &XurUObject| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UObject::new(obj.0, &sdk.names).full_name()
        })
        .raw_function_returns::<XurUClass>("class", |cx, args| {
            let this = extract_uobject(&args[0])?;
            let sdk = get_sdk()?;
            let uobj = unrealkit::UObject::new(this.0, &sdk.names);
            into_value(cx, XurUClass(uobj.class().ptr()))
        })
        .raw_function_returns::<XurUObject>("outer", |cx, args| {
            let this = extract_uobject(&args[0])?;
            let sdk = get_sdk()?;
            let uobj = unrealkit::UObject::new(this.0, &sdk.names);
            let outer = uobj
                .outer()
                .map(|o| into_value(cx, XurUObject(o.ptr())))
                .transpose()?;
            Ok(xur_option(cx, outer))
        })
        .raw_function("is_a", |_cx, args| {
            if args.len() != 2 {
                return Err(err("`is_a` takes (class_name: str)"));
            }
            let this = extract_uobject(&args[0])?;
            let Value::Str(class_name) = &args[1] else {
                return Err(err("`is_a` takes (class_name: str)"));
            };
            let sdk = get_sdk()?;
            let uobj = unrealkit::UObject::new(this.0, &sdk.names);
            Ok(Value::Bool(uobj.is_a(class_name)))
        })
        .function("ptr", |obj: &XurUObject| -> u64 {
            obj.0 as u64
        })
        .raw_function("get", |cx, args| {
            if args.len() != 2 {
                return Err(err("`get` takes (name: str)"));
            }
            let this = extract_uobject(&args[0])?;
            let Value::Str(name) = &args[1] else {
                return Err(err("`get` takes (name: str)"));
            };
            let sdk = get_sdk()?;
            let uobj = unrealkit::UObject::new(this.0, &sdk.names);
            let ucls = uobj.class();
            let prop = ucls.find_property(name)
                .ok_or_else(|| err(&format!("no property '{name}'")))?;
            let pv = unsafe { prop.read(this.0) };
            property_value_to_xur(cx, pv)
        })
        .raw_function("set", |_cx, args| {
            if args.len() != 3 {
                return Err(err("`set` takes (name: str, value)"));
            }
            let this = extract_uobject(&args[0])?;
            let Value::Str(name) = &args[1] else {
                return Err(err("`set` takes (name: str, value)"));
            };
            let sdk = get_sdk()?;
            let uobj = unrealkit::UObject::new(this.0, &sdk.names);
            let ucls = uobj.class();
            let prop = ucls.find_property(name)
                .ok_or_else(|| err(&format!("no property '{name}'")))?;
            let pv = xur_to_property_value(&args[2])?;
            unsafe { prop.write(this.0 as *mut u8, &pv) };
            Ok(Value::Unit)
        });

    // --- UClass type ---

    m.ty::<XurUClass>("UClass")
        .function("name", |cls: &XurUClass| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UClass::new(cls.0, &sdk.names).name()
        })
        .raw_function_returns::<XurUClass>("super_class", |cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let sup = ucls
                .super_class()
                .map(|s| into_value(cx, XurUClass(s.ptr())))
                .transpose()?;
            Ok(xur_option(cx, sup))
        })
        .raw_function("children", |_cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let children: Vec<Value> = ucls
                .children()
                .map(|f| {
                    let desc = format!("{} ({})", f.name(), f.class_name());
                    Value::str(&desc)
                })
                .collect();
            Ok(Value::list(children))
        })
        .function("instance_size", |cls: &XurUClass| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UClass::new(cls.0, &sdk.names).instance_size() as i64
        })
        .raw_function("hierarchy", |_cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let chain: Vec<Value> = ucls
                .class_hierarchy()
                .map(|c| Value::str(&c.name()))
                .collect();
            Ok(Value::list(chain))
        })
        .raw_function_returns::<XurUProperty>("property", |cx, args| {
            if args.len() != 2 {
                return Err(err("`property` takes (name: str)"));
            }
            let this = extract_uclass(&args[0])?;
            let Value::Str(name) = &args[1] else {
                return Err(err("`property` takes (name: str)"));
            };
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            match ucls.find_property(name) {
                Some(prop) => {
                    let v = into_value(cx, XurUProperty(prop.ptr()))?;
                    Ok(xur_option(cx, Some(v)))
                }
                None => Ok(xur_option(cx, None)),
            }
        })
        .raw_function("properties", |cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let props: Result<Vec<Value>, _> = ucls
                .properties()
                .map(|p| into_value(cx, XurUProperty(p.ptr())))
                .collect();
            Ok(Value::list(props?))
        })
        .raw_function("all_properties", |cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let props: Result<Vec<Value>, _> = ucls
                .class_hierarchy()
                .flat_map(|c| c.properties())
                .map(|p| into_value(cx, XurUProperty(p.ptr())))
                .collect();
            Ok(Value::list(props?))
        })
        .raw_function_returns::<XurUFunction>("method", |cx, args| {
            if args.len() != 2 {
                return Err(err("`method` takes (name: str)"));
            }
            let this = extract_uclass(&args[0])?;
            let Value::Str(name) = &args[1] else {
                return Err(err("`method` takes (name: str)"));
            };
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            match ucls.find_function(name) {
                Some(func) => {
                    let v = into_value(cx, XurUFunction(func.ptr()))?;
                    Ok(xur_option(cx, Some(v)))
                }
                None => Ok(xur_option(cx, None)),
            }
        })
        .raw_function("methods", |cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let funcs: Result<Vec<Value>, _> = ucls
                .functions()
                .map(|f| into_value(cx, XurUFunction(f.ptr())))
                .collect();
            Ok(Value::list(funcs?))
        })
        .raw_function("all_methods", |cx, args| {
            let this = extract_uclass(&args[0])?;
            let sdk = get_sdk()?;
            let ucls = unrealkit::UClass::new(this.0, &sdk.names);
            let funcs: Result<Vec<Value>, _> = ucls
                .class_hierarchy()
                .flat_map(|c| c.functions())
                .map(|f| into_value(cx, XurUFunction(f.ptr())))
                .collect();
            Ok(Value::list(funcs?))
        });

    // --- UProperty type ---

    m.ty::<XurUProperty>("UProperty")
        .function("name", |prop: &XurUProperty| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::FProperty::new(prop.0, &sdk.names).name()
        })
        .function("type_name", |prop: &XurUProperty| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::FProperty::new(prop.0, &sdk.names).class_name()
        })
        .function("offset", |prop: &XurUProperty| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::FProperty::new(prop.0, &sdk.names).offset() as i64
        })
        .function("element_size", |prop: &XurUProperty| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::FProperty::new(prop.0, &sdk.names).element_size() as i64
        })
        .function("ptr", |prop: &XurUProperty| -> i64 {
            prop.0 as usize as i64
        })
        .raw_function("read", |cx, args| {
            if args.len() != 2 {
                return Err(err("`read` takes (obj: UObject)"));
            }
            let this = extract_uproperty(&args[0])?;
            let obj = extract_uobject(&args[1])?;
            let sdk = get_sdk()?;
            let prop = unrealkit::FProperty::new(this.0, &sdk.names);
            let pv = unsafe { prop.read(obj.0) };
            property_value_to_xur(cx, pv)
        })
        .raw_function("write", |_cx, args| {
            if args.len() != 3 {
                return Err(err("`write` takes (obj: UObject, value)"));
            }
            let this = extract_uproperty(&args[0])?;
            let obj = extract_uobject(&args[1])?;
            let sdk = get_sdk()?;
            let prop = unrealkit::FProperty::new(this.0, &sdk.names);
            let pv = xur_to_property_value(&args[2])?;
            unsafe { prop.write(obj.0 as *mut u8, &pv) };
            Ok(Value::Unit)
        });

    // --- UFunction type ---

    m.ty::<XurUFunction>("UFunction")
        .function("name", |f: &XurUFunction| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).name()
        })
        .function("full_name", |f: &XurUFunction| -> String {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).full_name()
        })
        .function("num_params", |f: &XurUFunction| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).num_params() as i64
        })
        .function("params_size", |f: &XurUFunction| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).params_size() as i64
        })
        .function("flags", |f: &XurUFunction| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).function_flags() as i64
        })
        .function("native_ptr", |f: &XurUFunction| -> i64 {
            let sdk = get_sdk().expect("SDK not initialized");
            unrealkit::UFunction::new(f.0, &sdk.names).native_func_ptr() as usize as i64
        })
        .function("ptr", |f: &XurUFunction| -> i64 {
            f.0 as usize as i64
        })
        .raw_function("signature", |_cx, args| {
            let this = extract_ufunction(&args[0])?;
            let sdk = get_sdk()?;
            let func = unrealkit::UFunction::new(this.0, &sdk.names);
            Ok(Value::str(&func.signature()))
        })
        .raw_function("params", |cx, args| {
            let this = extract_ufunction(&args[0])?;
            let sdk = get_sdk()?;
            let func = unrealkit::UFunction::new(this.0, &sdk.names);
            let params: Result<Vec<Value>, _> = func
                .params()
                .map(|p| into_value(cx, XurUProperty(p.ptr())))
                .collect();
            Ok(Value::list(params?))
        })
        .raw_function("hook", |_cx, args| {
            if args.len() != 2 {
                return Err(err("`hook` takes (detour_addr: int)"));
            }
            let this = extract_ufunction(&args[0])?;
            let Value::Int(detour_addr) = &args[1] else {
                return Err(err("`hook` takes (detour_addr: int)"));
            };
            let sdk = get_sdk()?;
            let func = unrealkit::UFunction::new(this.0, &sdk.names);
            let native = func.native_func_ptr();
            if native.is_null() {
                return Err(err("UFunction has no native pointer"));
            }
            let mut hook = unsafe {
                hooking::Hook::create(native as *mut u8, *detour_addr as *mut u8)
            }
            .map_err(|e| err(format!("{e}")))?;
            unsafe { hook.apply_hook() }
                .map_err(|e| err(format!("{e}")))?;
            let original = hook.original_function().as_ptr() as i64;
            std::mem::forget(hook);
            Ok(Value::Int(original))
        })
        .raw_function("invoke", |cx, args| {
            if args.len() < 2 {
                return Err(err("`invoke` takes (obj: UObject, ...params)"));
            }
            let this = extract_ufunction(&args[0])?;
            let obj = extract_uobject(&args[1])?;
            let pe = PROCESS_EVENT
                .get()
                .ok_or_else(|| err("ProcessEvent not set — call set_process_event() first"))?;
            let sdk = get_sdk()?;
            let func = unrealkit::UFunction::new(this.0, &sdk.names);

            let mut params_buf = func.alloc_params();

            let input_params: Vec<_> = func.input_params().collect();
            for (i, prop) in input_params.iter().enumerate() {
                if let Some(arg_val) = args.get(i + 2) {
                    let pv = xur_to_property_value(arg_val)?;
                    unsafe { prop.write(params_buf.as_mut_ptr(), &pv) };
                }
            }

            unsafe { pe(obj.0, func.ptr(), params_buf.as_mut_ptr()) };

            if let Some(ret_prop) = func.return_param() {
                let pv = unsafe { ret_prop.read(params_buf.as_ptr()) };
                property_value_to_xur(cx, pv)
            } else {
                Ok(Value::Unit)
            }
        });

    }); // end lazy_module
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_uobject(val: &Value) -> Result<XurUObject, RuntimeError> {
    let Value::Native(n) = val else {
        return Err(err("expected a UObject"));
    };
    n.with(|o: &XurUObject| o.clone())
        .ok_or_else(|| err("expected a UObject"))
}

fn extract_uclass(val: &Value) -> Result<XurUClass, RuntimeError> {
    let Value::Native(n) = val else {
        return Err(err("expected a UClass"));
    };
    n.with(|c: &XurUClass| c.clone())
        .ok_or_else(|| err("expected a UClass"))
}

fn extract_ufunction(val: &Value) -> Result<XurUFunction, RuntimeError> {
    let Value::Native(n) = val else {
        return Err(err("expected a UFunction"));
    };
    n.with(|f: &XurUFunction| f.clone())
        .ok_or_else(|| err("expected a UFunction"))
}

fn extract_uproperty(val: &Value) -> Result<XurUProperty, RuntimeError> {
    let Value::Native(n) = val else {
        return Err(err("expected a UProperty"));
    };
    n.with(|p: &XurUProperty| p.clone())
        .ok_or_else(|| err("expected a UProperty"))
}

fn property_value_to_xur(cx: &mut dyn Context, pv: unrealkit::PropertyValue) -> Result<Value, RuntimeError> {
    match pv {
        unrealkit::PropertyValue::Bool(v) => Ok(Value::Bool(v)),
        unrealkit::PropertyValue::Int(v) => Ok(Value::Int(v)),
        unrealkit::PropertyValue::Float(v) => Ok(Value::Float(v)),
        unrealkit::PropertyValue::Name(s) | unrealkit::PropertyValue::Str(s) => Ok(Value::str(&s)),
        unrealkit::PropertyValue::Object(ptr) => {
            if ptr.is_null() {
                let none = cx.lang().none;
                Ok(Value::adt(none, Vec::new()))
            } else {
                into_value(cx, XurUObject(ptr))
            }
        }
        unrealkit::PropertyValue::Bytes(b) => Ok(Value::bytes(b)),
    }
}

fn xur_to_property_value(val: &Value) -> Result<unrealkit::PropertyValue, RuntimeError> {
    match val {
        Value::Bool(v) => Ok(unrealkit::PropertyValue::Bool(*v)),
        Value::Int(v) => Ok(unrealkit::PropertyValue::Int(*v)),
        Value::Float(v) => Ok(unrealkit::PropertyValue::Float(*v)),
        Value::Str(s) => Ok(unrealkit::PropertyValue::Str(s.to_string())),
        Value::Bytes(b) => {
            let bytes = b.lock().map_err(|e| err(e.to_string()))?.clone();
            Ok(unrealkit::PropertyValue::Bytes(bytes))
        }
        Value::Native(_) => {
            let obj = extract_uobject(val)?;
            Ok(unrealkit::PropertyValue::Object(obj.0))
        }
        _ => Err(err("cannot convert value to property type")),
    }
}
