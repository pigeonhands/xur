use hao::dotnet::signature::{MethodSig, TypeSig};
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

use super::{DotnetMethodSig, DotnetTypeSig};

pub(super) fn register(dotnet: &mut ModuleBuilder<'_>) {
    let mut sig = dotnet.submodule("sig");

    sig.function("void", || DotnetTypeSig(TypeSig::VOID));
    sig.function("bool", || DotnetTypeSig(TypeSig::BOOL));
    sig.function("char", || DotnetTypeSig(TypeSig::CHAR));
    sig.function("int32", || DotnetTypeSig(TypeSig::INT32));
    sig.function("int64", || DotnetTypeSig(TypeSig::INT64));
    sig.function("float64", || DotnetTypeSig(TypeSig::FLOAT64));
    sig.function("string", || DotnetTypeSig(TypeSig::STRING));
    sig.function("object", || DotnetTypeSig(TypeSig::OBJECT));
    sig.function("intptr", || DotnetTypeSig(TypeSig::NATIVE_INT));
    sig.function("sz_array", |elem: DotnetTypeSig| {
        DotnetTypeSig(TypeSig::sz_array(elem.0))
    });

    sig.raw_function("method_static", |cx, args| {
        let [Value::Native(ret), Value::List(params)] = args else {
            return Err(RuntimeError::new(
                "`method_static` takes (ret_type, [param_types...])",
            ));
        };
        let ret_sig = ret
            .with(|s: &DotnetTypeSig| s.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a TypeSig for return type"))?;
        let param_sigs = extract_type_sigs(params)?;
        into_value(
            cx,
            DotnetMethodSig(MethodSig::new_static(ret_sig, param_sigs)),
        )
    }).returns::<DotnetMethodSig>();

    sig.raw_function("method_instance", |cx, args| {
        let [Value::Native(ret), Value::List(params)] = args else {
            return Err(RuntimeError::new(
                "`method_instance` takes (ret_type, [param_types...])",
            ));
        };
        let ret_sig = ret
            .with(|s: &DotnetTypeSig| s.0.clone())
            .ok_or_else(|| RuntimeError::new("expected a TypeSig for return type"))?;
        let param_sigs = extract_type_sigs(params)?;
        into_value(
            cx,
            DotnetMethodSig(MethodSig::new_instance(ret_sig, param_sigs)),
        )
    }).returns::<DotnetMethodSig>();
}

fn extract_type_sigs(
    params: &std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
) -> Result<Vec<TypeSig>, RuntimeError> {
    params
        .lock()
        .unwrap()
        .iter()
        .map(|v| match v {
            Value::Native(n) => n
                .with(|s: &DotnetTypeSig| s.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a TypeSig in params")),
            _ => Err(RuntimeError::new("expected a TypeSig in params")),
        })
        .collect()
}
