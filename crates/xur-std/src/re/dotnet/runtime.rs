//! `std::re::dotnet::runtime`: host a .NET runtime with
//! [hao-runtime](https://docs.rs/hao-runtime).
//!
//! ```text
//! let rt = std::re::dotnet::runtime::coreclr()?;
//! rt.load_assembly("MyLib.dll")?;
//! let fp = rt.function_pointer("MyLib", "MyClass", "MyMethod")?;
//! ```

use std::sync::Arc;

use facet::Facet;
use hao_runtime::Runtime;
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, XurStr};

use crate::{error_text, result};

// ---------------------------------------------------------------------------
// Opaque wrapper — Runtime holds Arc<Inner> internally, not Facet-derivable.
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct DotnetRuntime(Arc<Runtime>);

impl std::fmt::Display for DotnetRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runtime({:?})", self.0.backend())
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(super) fn register(registry: &Registry, dotnet: &mut ModuleBuilder<'_>) {
    let mut rt = dotnet.submodule("runtime");

    // --- constructors ---

    rt.raw_function_returns::<DotnetRuntime>("coreclr", |cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`coreclr` takes no arguments"));
        }
        match Runtime::coreclr() {
            Ok(runtime) => {
                let v = into_value(cx, DotnetRuntime(Arc::new(runtime)))?;
                Ok(result(cx, Ok(v)))
            }
            Err(e) => Ok(result(cx, Err(error_text(e)))),
        }
    });

    // --- Runtime methods ---

    rt.ty::<DotnetRuntime>("Runtime")
        .method("backend", |r: &DotnetRuntime| -> String {
            format!("{:?}", r.0.backend())
        })
        .method(
            "load_assembly",
            |r: &DotnetRuntime, path: XurStr| -> Result<(), String> {
                r.0.load_assembly(&*path).map_err(error_text)
            },
        )
        .method(
            "function_pointer",
            |r: &DotnetRuntime, assembly: XurStr, type_name: XurStr, method: XurStr| -> Result<i64, String> {
                r.0.function_pointer(&assembly, &type_name, &method)
                    .map(|p| p as usize as i64)
                    .map_err(error_text)
            },
        )
        .method(
            "method_pointer",
            |r: &DotnetRuntime, assembly: XurStr, type_name: XurStr, method: XurStr| -> Result<i64, String> {
                r.0.method_pointer(&assembly, &type_name, &method)
                    .map(|p| p as usize as i64)
                    .map_err(error_text)
            },
        );

    // Walk the reflected config/descriptor types (Backend, CoreClrOptions, etc.)
    let skip = [];
    crate::re::walk::register(
        registry,
        &mut rt,
        &hao_runtime::reflect::roots(),
        submodule_of,
        &skip,
    );
}

fn submodule_of(shape: &facet::Shape) -> Option<&'static str> {
    let path = shape.module_path?;
    if !path.starts_with("hao_runtime") {
        return None;
    }
    Some("")
}
