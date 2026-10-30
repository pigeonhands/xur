//! `std::stub::il2cpp` — runtime Il2Cpp access for game mods.
//!
//! Exposes class lookup, field read/write (instance and static), and
//! method invocation through the live Il2Cpp runtime.
//!
//! ```text
//! let il2cpp = std::stub::il2cpp::loaded()?;
//! let cls = il2cpp.class("Assembly-CSharp", "Assets.Scripts.Actors.Player", "MyPlayer")?;
//! let player = cls.static_field("Instance");
//! cls.field("isInvincible").write(player, true);
//! ```

use std::ffi::c_void;
use std::sync::OnceLock;

use facet::Facet;
use hao_il2cpp_runtime::{self as rt, Il2Cpp};
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

// ---------------------------------------------------------------------------
// Static-field API — not in hao's Api struct, loaded from the game binary.
// GameAssembly.so/dll exports all il2cpp_* symbols; hao's Api struct omits
// these two, so we resolve them at runtime via libloading.
// ---------------------------------------------------------------------------

type StaticGetFn = unsafe extern "C" fn(field: *mut c_void, value: *mut c_void);
type StaticSetFn = unsafe extern "C" fn(field: *mut c_void, value: *const c_void);

struct StaticApi {
    get: StaticGetFn,
    set: StaticSetFn,
}

static STATIC_API: OnceLock<Option<StaticApi>> = OnceLock::new();

fn static_api() -> Option<&'static StaticApi> {
    STATIC_API
        .get_or_init(|| {
            let path = rt::loaded_game_assembly()?;
            let lib = unsafe { libloading::Library::new(&path) }.ok()?;
            let get: libloading::Symbol<StaticGetFn> =
                unsafe { lib.get(b"il2cpp_field_static_get_value") }.ok()?;
            let set: libloading::Symbol<StaticSetFn> =
                unsafe { lib.get(b"il2cpp_field_static_set_value") }.ok()?;
            let api = StaticApi { get: *get, set: *set };
            // The game assembly stays loaded for the process lifetime.
            std::mem::forget(lib);
            Some(api)
        })
        .as_ref()
}

// ---------------------------------------------------------------------------
// Opaque wrappers
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurIl2Cpp;

impl std::fmt::Display for XurIl2Cpp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Il2Cpp")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurClass(usize); // rt::Class doesn't impl Clone, store the raw pointer

unsafe impl Send for XurClass {}
unsafe impl Sync for XurClass {}

impl XurClass {
    fn new(class: &rt::Class) -> Self {
        Self(class.as_ptr() as usize)
    }

    fn class(&self) -> rt::Class {
        // Reconstruct the Class from its pointer.
        // SAFETY: Il2Cpp class pointers are valid for the process lifetime.
        let il2cpp = Il2Cpp::get().unwrap();
        // We need to reconstruct a Class from the raw pointer.
        // Since Class { il2cpp, ptr } is the layout, and the fields are
        // an &'static Il2Cpp and a Ptr, we can reconstruct it.
        unsafe { std::mem::transmute((il2cpp as *const Il2Cpp, self.0 as *mut c_void)) }
    }
}

impl std::fmt::Display for XurClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let class = self.class();
        write!(f, "Class({}.{})", class.namespace(), class.name())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurField(usize); // rt::Field raw pointer

unsafe impl Send for XurField {}
unsafe impl Sync for XurField {}

impl XurField {
    fn new(field: &rt::Field) -> Self {
        Self(field.as_ptr() as usize)
    }

    fn field(&self) -> rt::Field {
        let il2cpp = Il2Cpp::get().unwrap();
        unsafe { std::mem::transmute((il2cpp as *const Il2Cpp, self.0 as *mut c_void)) }
    }
}

impl std::fmt::Display for XurField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field = self.field();
        write!(f, "Field({})", field.name())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurMethod(usize); // rt::Method raw pointer

unsafe impl Send for XurMethod {}
unsafe impl Sync for XurMethod {}

impl XurMethod {
    fn new(method: &rt::Method) -> Self {
        Self(method.as_ptr() as usize)
    }

    fn method(&self) -> rt::Method {
        let il2cpp = Il2Cpp::get().unwrap();
        unsafe { std::mem::transmute((il2cpp as *const Il2Cpp, self.0 as *mut c_void)) }
    }
}

impl std::fmt::Display for XurMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let method = self.method();
        write!(f, "Method({})", method.name())
    }
}

#[derive(Facet, Clone, Copy)]
#[facet(opaque)]
struct XurObject(usize); // Object raw pointer

unsafe impl Send for XurObject {}
unsafe impl Sync for XurObject {}

impl XurObject {
    fn from_obj(obj: rt::Object) -> Self {
        Self(obj.as_ptr() as usize)
    }

    fn obj(&self) -> rt::Object {
        rt::Object(self.0 as *mut c_void)
    }
}

impl std::fmt::Display for XurObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            write!(f, "Object(null)")
        } else {
            write!(f, "Object(0x{:x})", self.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Field read/write helpers — dispatch on the Il2Cpp element type
// ---------------------------------------------------------------------------

const BOOLEAN: i32 = 0x02;
const I1: i32 = 0x04;
const U1: i32 = 0x05;
const I2: i32 = 0x06;
const U2: i32 = 0x07;
const I4: i32 = 0x08;
const U4: i32 = 0x09;
const I8: i32 = 0x0A;
const U8: i32 = 0x0B;
const R4: i32 = 0x0C;
const R8: i32 = 0x0D;
const STRING: i32 = 0x0E;
const CLASS: i32 = 0x12;
const VALUETYPE: i32 = 0x11;
const GENERICINST: i32 = 0x15;
const OBJECT: i32 = 0x1C;
const SZARRAY: i32 = 0x1D;

fn field_element_type(field: &rt::Field) -> i32 {
    let il2cpp = Il2Cpp::get().unwrap();
    let api = il2cpp.api();
    let ty = unsafe { (api.il2cpp_field_get_type)(field.as_ptr()) };
    unsafe { (api.il2cpp_type_get_type)(ty) }
}

fn read_instance_field(
    cx: &mut dyn xur_runtime::registry::Context,
    field: &rt::Field,
    object: rt::Object,
) -> Result<Value, RuntimeError> {
    let elem = field_element_type(field);
    match elem {
        BOOLEAN => {
            let v: bool = unsafe { field.read(object) };
            Ok(Value::Bool(v))
        }
        I1 => {
            let v: i8 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        U1 => {
            let v: u8 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        I2 => {
            let v: i16 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        U2 => {
            let v: u16 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        I4 => {
            let v: i32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        U4 => {
            let v: u32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        I8 => {
            let v: i64 = unsafe { field.read(object) };
            Ok(Value::Int(v))
        }
        U8 => {
            let v: u64 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        R4 => {
            let v: f32 = unsafe { field.read(object) };
            Ok(Value::Float(v as f64))
        }
        R8 => {
            let v: f64 = unsafe { field.read(object) };
            Ok(Value::Float(v))
        }
        STRING | CLASS | OBJECT | GENERICINST | SZARRAY => {
            let v: rt::Object = unsafe { field.read(object) };
            Ok(into_value(cx, XurObject::from_obj(v))?)
        }
        // Enums backed by an integer and value types (structs) — read as i32 by default.
        VALUETYPE => {
            let il2cpp_r = Il2Cpp::get().unwrap();
            let api = il2cpp_r.api();
            let ty = unsafe { (api.il2cpp_field_get_type)(field.as_ptr()) };
            let class = unsafe { (api.il2cpp_type_get_class_or_element_class)(ty) };
            if !class.is_null() && unsafe { (api.il2cpp_class_is_enum)(class) } {
                let base_ty = unsafe { (api.il2cpp_class_enum_basetype)(class) };
                let base_elem = unsafe { (api.il2cpp_type_get_type)(base_ty) };
                return read_instance_field_by_elem(field, object, base_elem);
            }
            let v: i32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        _ => Err(RuntimeError::new(format!(
            "unsupported field type (element {})",
            elem
        ))),
    }
}

fn read_instance_field_by_elem(
    field: &rt::Field,
    object: rt::Object,
    elem: i32,
) -> Result<Value, RuntimeError> {
    match elem {
        I1 => Ok(Value::Int(unsafe { field.read::<i8>(object) } as i64)),
        U1 => Ok(Value::Int(unsafe { field.read::<u8>(object) } as i64)),
        I2 => Ok(Value::Int(unsafe { field.read::<i16>(object) } as i64)),
        U2 => Ok(Value::Int(unsafe { field.read::<u16>(object) } as i64)),
        I4 => Ok(Value::Int(unsafe { field.read::<i32>(object) } as i64)),
        U4 => Ok(Value::Int(unsafe { field.read::<u32>(object) } as i64)),
        I8 => Ok(Value::Int(unsafe { field.read::<i64>(object) })),
        U8 => Ok(Value::Int(unsafe { field.read::<u64>(object) } as i64)),
        _ => Ok(Value::Int(unsafe { field.read::<i32>(object) } as i64)),
    }
}

fn write_instance_field(
    field: &rt::Field,
    object: rt::Object,
    value: &Value,
) -> Result<(), RuntimeError> {
    let elem = field_element_type(field);
    match (elem, value) {
        (BOOLEAN, Value::Bool(v)) => unsafe { field.write(object, *v) },
        (I1, Value::Int(v)) => unsafe { field.write(object, *v as i8) },
        (U1, Value::Int(v)) => unsafe { field.write(object, *v as u8) },
        (I2, Value::Int(v)) => unsafe { field.write(object, *v as i16) },
        (U2, Value::Int(v)) => unsafe { field.write(object, *v as u16) },
        (I4, Value::Int(v)) => unsafe { field.write(object, *v as i32) },
        (U4, Value::Int(v)) => unsafe { field.write(object, *v as u32) },
        (I8, Value::Int(v)) => unsafe { field.write(object, *v) },
        (U8, Value::Int(v)) => unsafe { field.write(object, *v as u64) },
        (R4, Value::Float(v)) => unsafe { field.write(object, *v as f32) },
        (R4, Value::Int(v)) => unsafe { field.write(object, *v as f32) },
        (R8, Value::Float(v)) => unsafe { field.write(object, *v) },
        (R8, Value::Int(v)) => unsafe { field.write(object, *v as f64) },
        (BOOLEAN, Value::Int(v)) => unsafe { field.write(object, *v != 0) },
        (STRING | CLASS | OBJECT | GENERICINST | SZARRAY, Value::Native(n)) => {
            if let Some(obj) = n.with(|o: &XurObject| o.obj()) {
                unsafe { field.write(object, obj) };
            } else {
                return Err(RuntimeError::new("expected an Object"));
            }
        }
        (VALUETYPE, Value::Int(v)) => unsafe { field.write(object, *v as i32) },
        _ => {
            return Err(RuntimeError::new(format!(
                "cannot write value to field with element type {}",
                elem
            )));
        }
    }
    Ok(())
}

fn read_static_field(
    cx: &mut dyn xur_runtime::registry::Context,
    field: &rt::Field,
) -> Result<Value, RuntimeError> {
    let api = static_api().ok_or_else(|| {
        RuntimeError::new("il2cpp_field_static_get_value not available")
    })?;

    let elem = field_element_type(field);
    match elem {
        BOOLEAN => {
            let mut v: bool = false;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut bool as *mut c_void) };
            Ok(Value::Bool(v))
        }
        I4 | VALUETYPE => {
            let mut v: i32 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut i32 as *mut c_void) };
            Ok(Value::Int(v as i64))
        }
        U4 => {
            let mut v: u32 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut u32 as *mut c_void) };
            Ok(Value::Int(v as i64))
        }
        I8 => {
            let mut v: i64 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut i64 as *mut c_void) };
            Ok(Value::Int(v))
        }
        R4 => {
            let mut v: f32 = 0.0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut f32 as *mut c_void) };
            Ok(Value::Float(v as f64))
        }
        R8 => {
            let mut v: f64 = 0.0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut f64 as *mut c_void) };
            Ok(Value::Float(v))
        }
        STRING | CLASS | OBJECT | GENERICINST | SZARRAY => {
            let mut v: *mut c_void = std::ptr::null_mut();
            unsafe {
                (api.get)(field.as_ptr(), &mut v as *mut *mut c_void as *mut c_void);
            }
            Ok(into_value(cx, XurObject::from_obj(rt::Object(v)))?)
        }
        _ => Err(RuntimeError::new(format!(
            "unsupported static field type (element {})",
            elem
        ))),
    }
}

fn write_static_field(field: &rt::Field, value: &Value) -> Result<(), RuntimeError> {
    let api = static_api().ok_or_else(|| {
        RuntimeError::new("il2cpp_field_static_set_value not available")
    })?;

    let elem = field_element_type(field);
    match (elem, value) {
        (BOOLEAN, Value::Bool(v)) => unsafe {
            (api.set)(field.as_ptr(), v as *const bool as *const c_void);
        },
        (I4 | VALUETYPE, Value::Int(v)) => {
            let v = *v as i32;
            unsafe { (api.set)(field.as_ptr(), &v as *const i32 as *const c_void) };
        }
        (R4, Value::Float(v)) => {
            let v = *v as f32;
            unsafe { (api.set)(field.as_ptr(), &v as *const f32 as *const c_void) };
        }
        (R8, Value::Float(v)) => unsafe {
            (api.set)(field.as_ptr(), v as *const f64 as *const c_void);
        },
        (STRING | CLASS | OBJECT | GENERICINST | SZARRAY, Value::Native(n)) => {
            if let Some(obj) = n.with(|o: &XurObject| o.obj()) {
                let ptr = obj.as_ptr();
                unsafe { (api.set)(field.as_ptr(), &ptr as *const *mut c_void as *const c_void) };
            } else {
                return Err(RuntimeError::new("expected an Object"));
            }
        }
        _ => {
            return Err(RuntimeError::new(format!(
                "cannot write value to static field with element type {}",
                elem
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn rt_err(e: rt::Error) -> RuntimeError {
    RuntimeError::new(e.to_string())
}

fn xur_result(cx: &dyn xur_runtime::registry::Context, value: Result<Value, String>) -> Value {
    let lang = cx.lang();
    match value {
        Ok(v) => Value::adt(lang.ok, vec![v]),
        Err(msg) => Value::adt(lang.err, vec![Value::str(&msg)]),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry, m: &mut ModuleBuilder<'_>) {
    // --- loaded() ---
    m.raw_function_returns::<XurIl2Cpp>("loaded", |cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`loaded` takes no arguments"));
        }
        match Il2Cpp::loaded() {
            Ok(_) => {
                let v = into_value(cx, XurIl2Cpp)?;
                Ok(xur_result(cx, Ok(v)))
            }
            Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
        }
    });

    // --- null() ---
    m.raw_function_returns::<XurObject>("null", |cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`null` takes no arguments"));
        }
        into_value(cx, XurObject(0))
    });

    // --- Il2Cpp methods ---
    m.ty::<XurIl2Cpp>("Runtime")
        .raw_function_returns::<XurClass>("class", |cx, args| {
            let [_this, Value::Str(assembly), Value::Str(namespace), Value::Str(name)] = args
            else {
                return Err(RuntimeError::new(
                    "`class` takes (assembly, namespace, name)",
                ));
            };
            let il2cpp = Il2Cpp::loaded().map_err(rt_err)?;
            match il2cpp.class(assembly, namespace, name) {
                Ok(cls) => {
                    let v = into_value(cx, XurClass::new(&cls))?;
                    Ok(xur_result(cx, Ok(v)))
                }
                Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
            }
        })
        .raw_function_returns::<XurObject>("string", |cx, args| {
            let [_this, Value::Str(value)] = args else {
                return Err(RuntimeError::new("`string` takes a string"));
            };
            let il2cpp = Il2Cpp::loaded().map_err(rt_err)?;
            match il2cpp.string(value) {
                Ok(s) => into_value(cx, XurObject::from_obj(s.0)),
                Err(e) => Err(rt_err(e)),
            }
        });

    // --- Class methods ---
    m.ty::<XurClass>("Class")
        .raw_function_returns::<XurField>("field", |cx, args| {
            let [this, Value::Str(name)] = args else {
                return Err(RuntimeError::new("`field` takes a field name"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Class`"));
            };
            let result = obj
                .with(|c: &XurClass| -> Result<Value, RuntimeError> {
                    let cls = c.class();
                    match cls.field(name) {
                        Ok(f) => {
                            let v = into_value(cx, XurField::new(&f))?;
                            Ok(xur_result(cx, Ok(v)))
                        }
                        Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
                    }
                })
                .ok_or_else(|| RuntimeError::new("expected a `Class`"))??;
            Ok(result)
        })
        .raw_function_returns::<XurMethod>("method", |cx, args| {
            let [this, Value::Str(name), Value::Int(param_count)] = args else {
                return Err(RuntimeError::new(
                    "`method` takes (name, parameter_count)",
                ));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Class`"));
            };
            let result = obj
                .with(|c: &XurClass| -> Result<Value, RuntimeError> {
                    let cls = c.class();
                    match cls.method(name, *param_count as i32) {
                        Ok(m) => {
                            let v = into_value(cx, XurMethod::new(&m))?;
                            Ok(xur_result(cx, Ok(v)))
                        }
                        Err(e) => Ok(xur_result(cx, Err(e.to_string()))),
                    }
                })
                .ok_or_else(|| RuntimeError::new("expected a `Class`"))??;
            Ok(result)
        })
        .raw_function_returns::<XurObject>("new_object", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`new_object` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Class`"));
            };
            let result = obj
                .with(|c: &XurClass| {
                    let cls = c.class();
                    into_value(cx, XurObject::from_obj(cls.new_object()))
                })
                .ok_or_else(|| RuntimeError::new("expected a `Class`"))??;
            Ok(result)
        })
        .method("name", |c: &XurClass| -> String {
            c.class().name().to_owned()
        })
        .method("namespace", |c: &XurClass| -> String {
            c.class().namespace().to_owned()
        });

    // --- Field methods ---
    m.ty::<XurField>("Field")
        .raw_function("read", |cx, args| {
            let [this, obj_val] = args else {
                return Err(RuntimeError::new("`read` takes an object"));
            };
            let field = extract_field(this)?;
            let object = extract_object(obj_val)?;
            read_instance_field(cx, &field, object)
        })
        .raw_function("write", |_cx, args| {
            let [this, obj_val, value] = args else {
                return Err(RuntimeError::new("`write` takes (object, value)"));
            };
            let field = extract_field(this)?;
            let object = extract_object(obj_val)?;
            write_instance_field(&field, object, value)?;
            Ok(Value::Unit)
        })
        .raw_function("read_static", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`read_static` takes no arguments"));
            };
            let field = extract_field(this)?;
            read_static_field(cx, &field)
        })
        .raw_function("write_static", |_cx, args| {
            let [this, value] = args else {
                return Err(RuntimeError::new("`write_static` takes a value"));
            };
            let field = extract_field(this)?;
            write_static_field(&field, value)?;
            Ok(Value::Unit)
        })
        .method("name", |f: &XurField| -> String {
            f.field().name().to_owned()
        })
        .method("type_name", |f: &XurField| -> String {
            f.field().type_name()
        })
        .method("offset", |f: &XurField| -> i64 {
            f.field().offset() as i64
        })
        .method("is_static", |f: &XurField| -> bool {
            f.field().is_static()
        });

    // --- Method methods ---
    m.ty::<XurMethod>("Method")
        .raw_function("invoke", |cx, args| {
            if args.is_empty() {
                return Err(RuntimeError::new("`invoke` takes (object, ...args)"));
            }
            let method = extract_method(&args[0])?;

            // Build the args array for il2cpp_runtime_invoke.
            let il2cpp = Il2Cpp::loaded().map_err(rt_err)?;
            let api = il2cpp.api();
            let this_obj = if args.len() > 1 {
                extract_object(&args[1])?
            } else {
                rt::Object::NULL
            };

            let mut invoke_args: Vec<*mut c_void> = Vec::new();
            // For il2cpp_runtime_invoke, args after the object are boxed values.
            // Integer/float args need to be passed as pointers to their stack values.
            let mut arg_storage: Vec<i64> = Vec::new();
            for arg in &args[2..] {
                match arg {
                    Value::Int(v) => {
                        arg_storage.push(*v);
                        let ptr = arg_storage.last().unwrap() as *const i64 as *mut c_void;
                        invoke_args.push(ptr);
                    }
                    Value::Float(v) => {
                        arg_storage.push(v.to_bits() as i64);
                        let ptr = arg_storage.last().unwrap() as *const i64 as *mut c_void;
                        invoke_args.push(ptr);
                    }
                    Value::Bool(v) => {
                        arg_storage.push(*v as i64);
                        let ptr = arg_storage.last().unwrap() as *const i64 as *mut c_void;
                        invoke_args.push(ptr);
                    }
                    Value::Native(n) => {
                        if let Some(obj) = n.with(|o: &XurObject| o.obj()) {
                            invoke_args.push(obj.as_ptr());
                        } else {
                            invoke_args.push(std::ptr::null_mut());
                        }
                    }
                    _ => invoke_args.push(std::ptr::null_mut()),
                }
            }

            let args_ptr = if invoke_args.is_empty() {
                std::ptr::null_mut()
            } else {
                invoke_args.as_mut_ptr()
            };

            let mut exception: *mut c_void = std::ptr::null_mut();
            let result = unsafe {
                (api.il2cpp_runtime_invoke)(
                    method.as_ptr(),
                    this_obj.as_ptr(),
                    args_ptr,
                    &mut exception,
                )
            };

            if !exception.is_null() {
                return Err(RuntimeError::new("Il2Cpp method threw an exception"));
            }

            // Return the result as an Object (caller can read fields if needed).
            // For void methods, result is null.
            into_value(cx, XurObject::from_obj(rt::Object(result)))
        })
        .method("name", |m: &XurMethod| -> String {
            m.method().name().to_owned()
        })
        .method("signature", |m: &XurMethod| -> String {
            m.method().signature()
        })
        .raw_function("code_address", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`code_address` takes no arguments"));
            };
            let method = extract_method(this)?;
            Ok(Value::Int(method.code() as usize as i64))
        });

    // --- Object methods ---
    m.ty::<XurObject>("Object")
        .method("address", |o: &XurObject| -> i64 { o.0 as i64 })
        .method("is_null", |o: &XurObject| -> bool { o.0 == 0 });

    let _ = registry;
}

// ---------------------------------------------------------------------------
// Extractors
// ---------------------------------------------------------------------------

fn extract_field(val: &Value) -> Result<rt::Field, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Field`"));
    };
    obj.with(|f: &XurField| f.field())
        .ok_or_else(|| RuntimeError::new("expected a `Field`"))
}

fn extract_method(val: &Value) -> Result<rt::Method, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Method`"));
    };
    obj.with(|m: &XurMethod| m.method())
        .ok_or_else(|| RuntimeError::new("expected a `Method`"))
}

fn extract_object(val: &Value) -> Result<rt::Object, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected an `Object`"));
    };
    obj.with(|o: &XurObject| o.obj())
        .ok_or_else(|| RuntimeError::new("expected an `Object`"))
}
