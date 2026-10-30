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
use std::sync::{Mutex, OnceLock};

use facet::Facet;
use hao_il2cpp_runtime::{self as rt, Il2Cpp};
use xur_runtime::convert::{ConvertCx, into_value};
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

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurImage(usize); // rt::Image raw pointer (the Il2CppImage*)

unsafe impl Send for XurImage {}
unsafe impl Sync for XurImage {}

impl XurImage {
    fn new(image: &rt::Image) -> Self {
        // Image { il2cpp, ptr } — same layout as Class.
        let (_, ptr): (&Il2Cpp, *mut c_void) = unsafe { std::mem::transmute_copy(image) };
        Self(ptr as usize)
    }

    fn image(&self) -> rt::Image {
        let il2cpp = Il2Cpp::get().unwrap();
        unsafe { std::mem::transmute((il2cpp as *const Il2Cpp, self.0 as *mut c_void)) }
    }
}

impl std::fmt::Display for XurImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let image = self.image();
        write!(f, "Image({})", image.name())
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurVoidBlockHook(&'static std::sync::atomic::AtomicBool);

unsafe impl Send for XurVoidBlockHook {}
unsafe impl Sync for XurVoidBlockHook {}

impl std::fmt::Display for XurVoidBlockHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VoidBlockHook(blocked={})",
            self.0.load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

/// The context object passed to a `method.hook()` callback in script.
/// Created per-call. Arg values are cloned; the register pointers are
/// only valid for the duration of the callback.
#[derive(Facet, Clone)]
#[facet(opaque)]
struct XurHookContext {
    args: Vec<(String, Value)>,
    modified: std::sync::Arc<Mutex<Vec<(usize, Value)>>>,
    signature: String,
    // For ctx.call() — raw pointers to the trampoline's saved registers.
    // Valid only during the dispatch callback.
    original: usize,
    int_regs: *mut u64,
    sse_regs: *mut [u8; 16],
    params: Vec<rt::GenericParam>,
    return_element: i32,
    handle: xur_vm::SharedHandle,
    called: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

unsafe impl Send for XurHookContext {}
unsafe impl Sync for XurHookContext {}

impl std::fmt::Display for XurHookContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HookContext({})", self.signature)
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

use rt::ElementType;

fn read_instance_field(
    cx: &mut dyn xur_runtime::registry::Context,
    field: &rt::Field,
    object: rt::Object,
) -> Result<Value, RuntimeError> {
    let elem = field.element_type();
    match elem {
        ElementType::Boolean => {
            let v: bool = unsafe { field.read(object) };
            Ok(Value::Bool(v))
        }
        ElementType::I1 => {
            let v: i8 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::U1 => {
            let v: u8 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::I2 => {
            let v: i16 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::U2 => {
            let v: u16 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::I4 => {
            let v: i32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::U4 => {
            let v: u32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::I8 => {
            let v: i64 = unsafe { field.read(object) };
            Ok(Value::Int(v))
        }
        ElementType::U8 => {
            let v: u64 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        ElementType::R4 => {
            let v: f32 = unsafe { field.read(object) };
            Ok(Value::Float(v as f64))
        }
        ElementType::R8 => {
            let v: f64 = unsafe { field.read(object) };
            Ok(Value::Float(v))
        }
        ElementType::String | ElementType::Class | ElementType::Object
        | ElementType::GenericInst | ElementType::SzArray => {
            let v: rt::Object = unsafe { field.read(object) };
            Ok(into_value(cx, XurObject::from_obj(v))?)
        }
        ElementType::ValueType => {
            let il2cpp_r = Il2Cpp::get().unwrap();
            let api = il2cpp_r.api();
            let ty = unsafe { (api.il2cpp_field_get_type)(field.as_ptr()) };
            let class = unsafe { (api.il2cpp_type_get_class_or_element_class)(ty) };
            if !class.is_null() && unsafe { (api.il2cpp_class_is_enum)(class) } {
                let base_ty = unsafe { (api.il2cpp_class_enum_basetype)(class) };
                let base_elem = unsafe { (api.il2cpp_type_get_type)(base_ty) };
                return read_instance_field_by_elem(field, object, ElementType::from(base_elem));
            }
            let v: i32 = unsafe { field.read(object) };
            Ok(Value::Int(v as i64))
        }
        _ => Err(RuntimeError::new(format!(
            "unsupported field type (element {elem})",
        ))),
    }
}

fn read_instance_field_by_elem(
    field: &rt::Field,
    object: rt::Object,
    elem: ElementType,
) -> Result<Value, RuntimeError> {
    match elem {
        ElementType::I1 => Ok(Value::Int(unsafe { field.read::<i8>(object) } as i64)),
        ElementType::U1 => Ok(Value::Int(unsafe { field.read::<u8>(object) } as i64)),
        ElementType::I2 => Ok(Value::Int(unsafe { field.read::<i16>(object) } as i64)),
        ElementType::U2 => Ok(Value::Int(unsafe { field.read::<u16>(object) } as i64)),
        ElementType::I4 => Ok(Value::Int(unsafe { field.read::<i32>(object) } as i64)),
        ElementType::U4 => Ok(Value::Int(unsafe { field.read::<u32>(object) } as i64)),
        ElementType::I8 => Ok(Value::Int(unsafe { field.read::<i64>(object) })),
        ElementType::U8 => Ok(Value::Int(unsafe { field.read::<u64>(object) } as i64)),
        _ => Ok(Value::Int(unsafe { field.read::<i32>(object) } as i64)),
    }
}

fn write_instance_field(
    field: &rt::Field,
    object: rt::Object,
    value: &Value,
) -> Result<(), RuntimeError> {
    let elem = field.element_type();
    match (elem, value) {
        (ElementType::Boolean, Value::Bool(v)) => unsafe { field.write(object, *v) },
        (ElementType::I1, Value::Int(v)) => unsafe { field.write(object, *v as i8) },
        (ElementType::U1, Value::Int(v)) => unsafe { field.write(object, *v as u8) },
        (ElementType::I2, Value::Int(v)) => unsafe { field.write(object, *v as i16) },
        (ElementType::U2, Value::Int(v)) => unsafe { field.write(object, *v as u16) },
        (ElementType::I4, Value::Int(v)) => unsafe { field.write(object, *v as i32) },
        (ElementType::U4, Value::Int(v)) => unsafe { field.write(object, *v as u32) },
        (ElementType::I8, Value::Int(v)) => unsafe { field.write(object, *v) },
        (ElementType::U8, Value::Int(v)) => unsafe { field.write(object, *v as u64) },
        (ElementType::R4, Value::Float(v)) => unsafe { field.write(object, *v as f32) },
        (ElementType::R4, Value::Int(v)) => unsafe { field.write(object, *v as f32) },
        (ElementType::R8, Value::Float(v)) => unsafe { field.write(object, *v) },
        (ElementType::R8, Value::Int(v)) => unsafe { field.write(object, *v as f64) },
        (ElementType::Boolean, Value::Int(v)) => unsafe { field.write(object, *v != 0) },
        (ElementType::String | ElementType::Class | ElementType::Object
        | ElementType::GenericInst | ElementType::SzArray, Value::Native(n)) => {
            if let Some(obj) = n.with(|o: &XurObject| o.obj()) {
                unsafe { field.write(object, obj) };
            } else {
                return Err(RuntimeError::new("expected an Object"));
            }
        }
        (ElementType::ValueType, Value::Int(v)) => unsafe { field.write(object, *v as i32) },
        _ => {
            return Err(RuntimeError::new(format!(
                "cannot write value to field with element type {elem}",
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

    let elem = field.element_type();
    match elem {
        ElementType::Boolean => {
            let mut v: bool = false;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut bool as *mut c_void) };
            Ok(Value::Bool(v))
        }
        ElementType::I4 | ElementType::ValueType => {
            let mut v: i32 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut i32 as *mut c_void) };
            Ok(Value::Int(v as i64))
        }
        ElementType::U4 => {
            let mut v: u32 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut u32 as *mut c_void) };
            Ok(Value::Int(v as i64))
        }
        ElementType::I8 => {
            let mut v: i64 = 0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut i64 as *mut c_void) };
            Ok(Value::Int(v))
        }
        ElementType::R4 => {
            let mut v: f32 = 0.0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut f32 as *mut c_void) };
            Ok(Value::Float(v as f64))
        }
        ElementType::R8 => {
            let mut v: f64 = 0.0;
            unsafe { (api.get)(field.as_ptr(), &mut v as *mut f64 as *mut c_void) };
            Ok(Value::Float(v))
        }
        ElementType::String | ElementType::Class | ElementType::Object
        | ElementType::GenericInst | ElementType::SzArray => {
            let mut v: *mut c_void = std::ptr::null_mut();
            unsafe {
                (api.get)(field.as_ptr(), &mut v as *mut *mut c_void as *mut c_void);
            }
            Ok(into_value(cx, XurObject::from_obj(rt::Object(v)))?)
        }
        _ => Err(RuntimeError::new(format!(
            "unsupported static field type (element {elem})",
        ))),
    }
}

fn write_static_field(field: &rt::Field, value: &Value) -> Result<(), RuntimeError> {
    let api = static_api().ok_or_else(|| {
        RuntimeError::new("il2cpp_field_static_set_value not available")
    })?;

    let elem = field.element_type();
    match (elem, value) {
        (ElementType::Boolean, Value::Bool(v)) => unsafe {
            (api.set)(field.as_ptr(), v as *const bool as *const c_void);
        },
        (ElementType::I4 | ElementType::ValueType, Value::Int(v)) => {
            let v = *v as i32;
            unsafe { (api.set)(field.as_ptr(), &v as *const i32 as *const c_void) };
        }
        (ElementType::R4, Value::Float(v)) => {
            let v = *v as f32;
            unsafe { (api.set)(field.as_ptr(), &v as *const f32 as *const c_void) };
        }
        (ElementType::R8, Value::Float(v)) => unsafe {
            (api.set)(field.as_ptr(), v as *const f64 as *const c_void);
        },
        (ElementType::String | ElementType::Class | ElementType::Object
        | ElementType::GenericInst | ElementType::SzArray, Value::Native(n)) => {
            if let Some(obj) = n.with(|o: &XurObject| o.obj()) {
                let ptr = obj.as_ptr();
                unsafe { (api.set)(field.as_ptr(), &ptr as *const *mut c_void as *const c_void) };
            } else {
                return Err(RuntimeError::new("expected an Object"));
            }
        }
        _ => {
            return Err(RuntimeError::new(format!(
                "cannot write value to static field with element type {elem}",
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Invoke helpers
// ---------------------------------------------------------------------------

fn ensure_thread_attached() {
    thread_local! {
        static ATTACHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    ATTACHED.with(|a| {
        if !a.get() {
            if let Ok(il2cpp) = Il2Cpp::loaded() {
                il2cpp.attach_thread();
                a.set(true);
            }
        }
    });
}

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

/// Core invoke logic shared by `invoke` and `invoke_on_main`.
fn do_invoke(
    cx: &dyn ConvertCx,
    method_ptr: usize,
    this_ptr: usize,
    call_args: &[Value],
) -> Result<Value, RuntimeError> {
    ensure_thread_attached();
    let il2cpp = Il2Cpp::loaded().map_err(rt_err)?;
    let api = il2cpp.api();

    let method_raw = method_ptr as *mut c_void;
    let this_raw = this_ptr as *mut c_void;

    let mut arg_storage: Vec<i64> = Vec::new();
    let mut invoke_args: Vec<*mut c_void> = Vec::new();
    for arg in call_args {
        match arg {
            Value::Int(v) => {
                arg_storage.push(*v);
                invoke_args
                    .push(arg_storage.last().unwrap() as *const i64 as *mut c_void);
            }
            Value::Float(v) => {
                arg_storage.push(v.to_bits() as i64);
                invoke_args
                    .push(arg_storage.last().unwrap() as *const i64 as *mut c_void);
            }
            Value::Bool(v) => {
                arg_storage.push(*v as i64);
                invoke_args
                    .push(arg_storage.last().unwrap() as *const i64 as *mut c_void);
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
        (api.il2cpp_runtime_invoke)(method_raw, this_raw, args_ptr, &mut exception)
    };

    if !exception.is_null() {
        let msg = unsafe { il2cpp.exception_message(exception) };
        return Err(RuntimeError::new(msg));
    }

    if result.is_null() {
        return into_value(cx, XurObject(0));
    }

    let ret_type = unsafe { (api.il2cpp_method_get_return_type)(method_raw) };
    let ret_elem = ElementType::from(unsafe { (api.il2cpp_type_get_type)(ret_type) });
    unmarshal_invoke_result(cx, api, result, ret_elem)
}

fn unmarshal_invoke_result(
    cx: &dyn ConvertCx,
    api: &rt::Api,
    result: *mut c_void,
    ret_elem: ElementType,
) -> Result<Value, RuntimeError> {
    match ret_elem {
        ElementType::Void => Ok(Value::Unit),
        ElementType::Boolean => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Bool(unsafe { *(ptr as *const bool) }))
        }
        ElementType::I1 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const i8) } as i64))
        }
        ElementType::U1 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const u8) } as i64))
        }
        ElementType::I2 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const i16) } as i64))
        }
        ElementType::U2 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const u16) } as i64))
        }
        ElementType::I4 | ElementType::ValueType => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const i32) } as i64))
        }
        ElementType::U4 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const u32) } as i64))
        }
        ElementType::I8 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const i64) }))
        }
        ElementType::U8 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Int(unsafe { *(ptr as *const u64) } as i64))
        }
        ElementType::R4 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Float(unsafe { *(ptr as *const f32) } as f64))
        }
        ElementType::R8 => {
            let ptr = unsafe { (api.il2cpp_object_unbox)(result) };
            Ok(Value::Float(unsafe { *(ptr as *const f64) }))
        }
        _ => into_value(cx, XurObject::from_obj(rt::Object(result))),
    }
}

// ---------------------------------------------------------------------------
// Generic hook marshalling
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
fn marshal_arg(
    cx: &dyn ConvertCx,
    regs: &hooking::SavedRegs,
    param: &rt::GenericParam,
) -> Result<Value, RuntimeError> {
    use rt::element;
    match param.slot {
        rt::ParamSlot::IntReg(i) => {
            let raw = regs.read_int(i);
            match param.type_info.element {
                element::VOID => Ok(Value::Unit),
                element::BOOLEAN => Ok(Value::Bool(raw != 0)),
                element::I1 => Ok(Value::Int(raw as i8 as i64)),
                element::U1 => Ok(Value::Int(raw as u8 as i64)),
                element::I2 => Ok(Value::Int(raw as i16 as i64)),
                element::U2 => Ok(Value::Int(raw as u16 as i64)),
                element::I4 => Ok(Value::Int(raw as i32 as i64)),
                element::U4 => Ok(Value::Int(raw as u32 as i64)),
                element::I8 => Ok(Value::Int(raw as i64)),
                element::U8 => Ok(Value::Int(raw as u64 as i64)),
                element::R4 => Ok(Value::Float(f32::from_bits(raw as u32) as f64)),
                element::R8 => Ok(Value::Float(f64::from_bits(raw))),
                element::STRING | element::CLASS | element::OBJECT
                | element::GENERICINST | element::SZARRAY | element::ARRAY => {
                    into_value(cx, XurObject(raw as usize))
                }
                _ => Ok(Value::Int(raw as i64)),
            }
        }
        rt::ParamSlot::FloatReg(i) => {
            match param.type_info.element {
                element::R4 => Ok(Value::Float(regs.read_f32(i) as f64)),
                _ => Ok(Value::Float(regs.read_f64(i))),
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn write_arg_to_regs(
    regs: &mut hooking::SavedRegs,
    param: &rt::GenericParam,
    val: &Value,
) {
    match param.slot {
        rt::ParamSlot::IntReg(i) => {
            let raw: u64 = match val {
                Value::Bool(b) => *b as u64,
                Value::Int(n) => *n as u64,
                Value::Float(f) => *f as u64,
                Value::Native(n) => {
                    n.with(|o: &XurObject| o.0 as u64).unwrap_or(0)
                }
                _ => return,
            };
            regs.write_int(i, raw);
        }
        rt::ParamSlot::FloatReg(i) => {
            match val {
                Value::Float(f) => {
                    if param.type_info.element == rt::element::R4 {
                        regs.write_f32(i, *f as f32);
                    } else {
                        regs.write_f64(i, *f);
                    }
                }
                Value::Int(n) => {
                    if param.type_info.element == rt::element::R4 {
                        regs.write_f32(i, *n as f32);
                    } else {
                        regs.write_f64(i, *n as f64);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn value_to_override(val: Value, return_element: i32) -> hooking::HookAction {
    use rt::element;
    match val {
        Value::Unit => hooking::HookAction::OverrideInt(0),
        Value::Bool(b) => hooking::HookAction::OverrideInt(b as u64),
        Value::Int(i) => {
            match return_element {
                element::R4 => hooking::HookAction::OverrideFloat((i as f32) as f64),
                element::R8 => hooking::HookAction::OverrideFloat(i as f64),
                _ => hooking::HookAction::OverrideInt(i as u64),
            }
        }
        Value::Float(f) => hooking::HookAction::OverrideFloat(f),
        Value::Native(ref n) => {
            if let Some(ptr) = n.with(|o: &XurObject| o.0 as u64) {
                hooking::HookAction::OverrideInt(ptr)
            } else {
                hooking::HookAction::OverrideInt(0)
            }
        }
        _ => hooking::HookAction::OverrideInt(0),
    }
}

/// Interprets the hook callback's return value as a HookAction.
/// None → Passthrough, Some(value) → override.
#[cfg(target_arch = "x86_64")]
fn interpret_hook_result(
    result: Value,
    return_element: i32,
    lang: &xur_resolve::LangItems,
) -> hooking::HookAction {
    if let Value::Adt(ref adt) = result {
        if adt.ctor == lang.none {
            return hooking::HookAction::Passthrough;
        }
        if adt.ctor == lang.some {
            let inner = adt.fields.first().cloned().unwrap_or(Value::Unit);
            return value_to_override(inner, return_element);
        }
    }
    // Backwards compat: bare Unit still means passthrough
    if matches!(result, Value::Unit) {
        return hooking::HookAction::Passthrough;
    }
    value_to_override(result, return_element)
}

/// Marshals the return value from calling the original function.
#[cfg(target_arch = "x86_64")]
fn marshal_call_result(
    cx: &dyn ConvertCx,
    call_result: &hooking::CallResult,
    return_element: i32,
) -> Value {
    use rt::element;
    match return_element {
        0x01 => Value::Unit, // void
        element::BOOLEAN => Value::Bool(call_result.int_val != 0),
        element::I1 => Value::Int(call_result.int_val as i8 as i64),
        element::U1 => Value::Int(call_result.int_val as u8 as i64),
        element::I2 => Value::Int(call_result.int_val as i16 as i64),
        element::U2 => Value::Int(call_result.int_val as u16 as i64),
        element::I4 | element::VALUETYPE => Value::Int(call_result.int_val as i32 as i64),
        element::U4 => Value::Int(call_result.int_val as u32 as i64),
        element::I8 => Value::Int(call_result.int_val as i64),
        element::U8 => Value::Int(call_result.int_val as u64 as i64),
        element::R4 => Value::Float(call_result.float_val),
        element::R8 => Value::Float(call_result.float_val),
        element::STRING | element::CLASS | element::OBJECT
        | element::GENERICINST | element::SZARRAY | element::ARRAY => {
            into_value(cx, XurObject(call_result.int_val as usize))
                .unwrap_or_else(|_| Value::Int(call_result.int_val as i64))
        }
        _ => Value::Int(call_result.int_val as i64),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::il2cpp", |m| {
    // --- loaded() ---
    m.raw_function_returns::<XurIl2Cpp>("loaded", |cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`loaded` takes no arguments"));
        }
        match Il2Cpp::loaded() {
            Ok(_il2cpp) => {
                ensure_thread_attached();
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
        })
        .raw_function("assemblies", |cx, args| {
            let [_this] = args else {
                return Err(RuntimeError::new("`assemblies` takes no arguments"));
            };
            let il2cpp = Il2Cpp::loaded().map_err(rt_err)?;
            let images = il2cpp.assemblies();
            let list: Result<Vec<Value>, _> = images
                .iter()
                .map(|img| into_value(cx, XurImage::new(img)))
                .collect();
            Ok(Value::list(list?))
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
        .raw_function("fields", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`fields` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Class`"));
            };
            let list = obj
                .with(|c: &XurClass| -> Result<Vec<Value>, RuntimeError> {
                    c.class()
                        .fields()
                        .map(|f| into_value(cx, XurField::new(&f)))
                        .collect()
                })
                .ok_or_else(|| RuntimeError::new("expected a `Class`"))??;
            Ok(Value::list(list))
        })
        .raw_function("methods", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`methods` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Class`"));
            };
            let list = obj
                .with(|c: &XurClass| -> Result<Vec<Value>, RuntimeError> {
                    c.class()
                        .methods()
                        .map(|m| into_value(cx, XurMethod::new(&m)))
                        .collect()
                })
                .ok_or_else(|| RuntimeError::new("expected a `Class`"))??;
            Ok(Value::list(list))
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
            let this_obj = if args.len() > 1 {
                extract_object(&args[1])?
            } else {
                rt::Object::NULL
            };
            do_invoke(cx, method.as_ptr() as usize, this_obj.as_ptr() as usize, &args[2..])
        })
        .raw_function("invoke_on_main", |cx, args| {
            #[cfg(not(feature = "il2cpp-unity"))]
            return Err(RuntimeError::new(
                "invoke_on_main requires the il2cpp-unity feature",
            ));

            #[cfg(feature = "il2cpp-unity")]
            {
                if args.is_empty() {
                    return Err(RuntimeError::new(
                        "`invoke_on_main` takes (object, ...args)",
                    ));
                }
                let method = extract_method(&args[0])?;
                let this_obj = if args.len() > 1 {
                    extract_object(&args[1])?
                } else {
                    rt::Object::NULL
                };

                let handle: Box<xur_vm::SharedHandle> = cx
                    .shared_handle()
                    .and_then(|h| h.downcast().ok())
                    .ok_or_else(|| {
                        RuntimeError::new("invoke_on_main requires VM support")
                    })?;

                let method_ptr = method.as_ptr() as usize;
                let this_ptr = this_obj.as_ptr() as usize;
                let call_args: Vec<Value> = args[2..].to_vec();

                hao_il2cpp_unity::dispatch::call(move || {
                    do_invoke(&*handle, method_ptr, this_ptr, &call_args)
                })
                .map_err(rt_err)?
            }
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
        })
        .raw_function("hook_void_block", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`hook_void_block` takes no arguments"));
            };
            let method = extract_method(this)?;
            let hook = method.hook_void_block().map_err(rt_err)?;
            let state = hook.state;
            std::mem::forget(hook);
            into_value(cx, XurVoidBlockHook(state))
        })
        .raw_function("hook", |cx, args| {
            let [this, callback] = args else {
                return Err(RuntimeError::new("`hook` takes a callback closure"));
            };
            let method = extract_method(this)?;
            let callback = callback.clone();

            let handle: Box<xur_vm::SharedHandle> = cx
                .shared_handle()
                .and_then(|h| h.downcast().ok())
                .ok_or_else(|| RuntimeError::new("hook requires VM support"))?;
            let handle = *handle;

            let signature = method.signature();
            let return_element = {
                let il2cpp = Il2Cpp::get().ok_or_else(|| RuntimeError::new("il2cpp not loaded"))?;
                let ret_ty = unsafe {
                    (il2cpp.api().il2cpp_method_get_return_type)(method.as_ptr())
                };
                unsafe { (il2cpp.api().il2cpp_type_get_type)(ret_ty) }
            };

            let hook = method.hook_generic(move |original, meta, regs| {
                let mut arg_pairs = Vec::with_capacity(meta.params.len());
                for param in &meta.params {
                    let val = match marshal_arg(&handle, regs, param) {
                        Ok(v) => v,
                        Err(_) => return hooking::HookAction::Passthrough,
                    };
                    arg_pairs.push((param.name.clone(), val));
                }

                let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let ctx = XurHookContext {
                    args: arg_pairs,
                    modified: std::sync::Arc::new(Mutex::new(Vec::new())),
                    signature: signature.clone(),
                    original,
                    int_regs: regs.int_regs,
                    sse_regs: regs.sse_regs,
                    params: meta.params.clone(),
                    return_element,
                    handle: handle.clone(),
                    called: called.clone(),
                };
                let ctx_val = match into_value(&handle, ctx) {
                    Ok(v) => v,
                    Err(_) => return hooking::HookAction::Passthrough,
                };

                let result = match handle.call_closure(&callback, &[ctx_val.clone()]) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("hook error: {}", e.message);
                        return hooking::HookAction::Passthrough;
                    }
                };

                // Write back modified args (unless ctx.call() already did).
                if !called.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Value::Native(ref n) = ctx_val {
                        if let Some(mods) = n.with(|c: &XurHookContext| {
                            xur_runtime::lock(&c.modified).clone()
                        }) {
                            for (idx, val) in mods {
                                if idx < meta.params.len() {
                                    write_arg_to_regs(regs, &meta.params[idx], &val);
                                }
                            }
                        }
                    }
                }

                let lang = handle.lang();
                interpret_hook_result(result, return_element, lang)
            }).map_err(rt_err)?;

            std::mem::forget(hook);
            Ok(Value::Unit)
        });

    m.ty::<XurHookContext>("HookContext")
        .raw_function("arg", |_cx, args| {
            let [this, key] = args else {
                return Err(RuntimeError::new("`arg` takes one argument (index or name)"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `HookContext`"));
            };
            obj.with(|ctx: &XurHookContext| -> Result<Value, RuntimeError> {
                match key {
                    Value::Int(i) => {
                        let idx = *i as usize;
                        ctx.args.get(idx)
                            .map(|(_, v)| v.clone())
                            .ok_or_else(|| RuntimeError::new(format!("arg index {} out of range", idx)))
                    }
                    Value::Str(name) => {
                        ctx.args.iter()
                            .find(|(n, _)| n.as_str() == name.as_ref())
                            .map(|(_, v)| v.clone())
                            .ok_or_else(|| RuntimeError::new(format!("no arg named \"{}\"", name)))
                    }
                    _ => Err(RuntimeError::new("`arg` takes an int index or string name")),
                }
            })
            .ok_or_else(|| RuntimeError::new("expected a `HookContext`"))?
        })
        .raw_function("args", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`args` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `HookContext`"));
            };
            obj.with(|ctx: &XurHookContext| {
                Value::list(ctx.args.iter().map(|(_, v)| v.clone()).collect())
            })
            .ok_or_else(|| RuntimeError::new("expected a `HookContext`"))
        })
        .raw_function("set_arg", |_cx, args| {
            let [this, key, val] = args else {
                return Err(RuntimeError::new("`set_arg` takes a key and a value"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `HookContext`"));
            };
            obj.with(|ctx: &XurHookContext| -> Result<(), RuntimeError> {
                let idx = match key {
                    Value::Int(i) => *i as usize,
                    Value::Str(name) => {
                        ctx.args.iter()
                            .position(|(n, _)| n.as_str() == name.as_ref())
                            .ok_or_else(|| RuntimeError::new(format!("no arg named \"{}\"", name)))?
                    }
                    _ => return Err(RuntimeError::new("`set_arg` key must be int or string")),
                };
                if idx >= ctx.args.len() {
                    return Err(RuntimeError::new(format!("arg index {} out of range", idx)));
                }
                xur_runtime::lock(&ctx.modified).push((idx, val.clone()));
                Ok(())
            })
            .ok_or_else(|| RuntimeError::new("expected a `HookContext`"))??;
            Ok(Value::Unit)
        })
        .raw_function("signature", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`signature` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `HookContext`"));
            };
            obj.with(|ctx: &XurHookContext| Value::str(&ctx.signature))
                .ok_or_else(|| RuntimeError::new("expected a `HookContext`"))
        })
        .raw_function("call", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`call` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `HookContext`"));
            };
            let (original, int_regs, sse_regs, params, modified, return_element, handle, called) =
                obj.with(|ctx: &XurHookContext| {
                    (
                        ctx.original,
                        ctx.int_regs,
                        ctx.sse_regs,
                        ctx.params.clone(),
                        ctx.modified.clone(),
                        ctx.return_element,
                        ctx.handle.clone(),
                        ctx.called.clone(),
                    )
                })
                .ok_or_else(|| RuntimeError::new("expected a `HookContext`"))?;

            if called.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return Err(RuntimeError::new("ctx.call() can only be called once"));
            }

            // Write back any modified args before calling the original.
            {
                let mods = xur_runtime::lock(&modified);
                let mut regs = hooking::SavedRegs { int_regs, sse_regs };
                for (idx, val) in mods.iter() {
                    if *idx < params.len() {
                        write_arg_to_regs(&mut regs, &params[*idx], val);
                    }
                }
            }

            let result = unsafe {
                hooking::call_with_saved_regs(original, int_regs, sse_regs)
            };
            Ok(marshal_call_result(&handle, &result, return_element))
        });

    m.ty::<XurVoidBlockHook>("VoidBlockHook")
        .raw_function("set_blocked", |_cx, args| {
            let [this, Value::Bool(blocked)] = args else {
                return Err(RuntimeError::new("`set_blocked` takes a bool"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `VoidBlockHook`"));
            };
            obj.with(|h: &XurVoidBlockHook| {
                h.0.store(*blocked, std::sync::atomic::Ordering::Relaxed);
            })
            .ok_or_else(|| RuntimeError::new("expected a `VoidBlockHook`"))?;
            Ok(Value::Unit)
        })
        .raw_function("is_blocked", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`is_blocked` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `VoidBlockHook`"));
            };
            obj.with(|h: &XurVoidBlockHook| {
                Value::Bool(h.0.load(std::sync::atomic::Ordering::Relaxed))
            })
            .ok_or_else(|| RuntimeError::new("expected a `VoidBlockHook`"))
        });

    // --- Image methods ---
    m.ty::<XurImage>("Image")
        .method("name", |i: &XurImage| -> String {
            i.image().name().to_owned()
        })
        .raw_function("classes", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`classes` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an `Image`"));
            };
            let list = obj
                .with(|i: &XurImage| -> Result<Vec<Value>, RuntimeError> {
                    i.image()
                        .classes()
                        .map(|c| into_value(cx, XurClass::new(&c)))
                        .collect()
                })
                .ok_or_else(|| RuntimeError::new("expected an `Image`"))??;
            Ok(Value::list(list))
        })
        .raw_function("class_count", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`class_count` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an `Image`"));
            };
            let count = obj
                .with(|i: &XurImage| i.image().class_count())
                .ok_or_else(|| RuntimeError::new("expected an `Image`"))?;
            Ok(Value::Int(count as i64))
        });

    // --- Object methods ---
    m.ty::<XurObject>("Object")
        .method("address", |o: &XurObject| -> i64 { o.0 as i64 })
        .method("is_null", |o: &XurObject| -> bool { o.0 == 0 });

    });
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
