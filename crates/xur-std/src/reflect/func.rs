//! `std::reflect::func`: inspecting function signatures at runtime.

use xur_runtime::{RunResult, RuntimeError, Value};

pub(super) fn register(module: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    let mut func = module.submodule("func");
    func.named_raw_function_returns::<String>("name", &["f"], |cx, args| {
        let [value] = args else { return arity("name", "f") };
        let info = cx
            .fn_info(value)
            .ok_or_else(|| RuntimeError::new("`reflect::func::name` expects a function"))?;
        Ok(Value::str(&info.name))
    })
    .named_raw_function_returns::<i64>("params", &["f"], |cx, args| {
        let [value] = args else { return arity("params", "f") };
        let info = cx
            .fn_info(value)
            .ok_or_else(|| RuntimeError::new("`reflect::func::params` expects a function"))?;
        Ok(Value::Int(info.param_count as i64))
    })
    .named_raw_function("param_names", &["f"], |cx, args| {
        let [value] = args else { return arity("param_names", "f") };
        let info = cx
            .fn_info(value)
            .ok_or_else(|| RuntimeError::new("`reflect::func::param_names` expects a function"))?;
        let names: Vec<Value> = info
            .param_names
            .iter()
            .map(|n| match n {
                Some(name) => Value::str(name),
                None => Value::str("_"),
            })
            .collect();
        Ok(Value::list(names))
    })
    .named_raw_function("param_types", &["f"], |cx, args| {
        let [value] = args else { return arity("param_types", "f") };
        let info = cx
            .fn_info(value)
            .ok_or_else(|| RuntimeError::new("`reflect::func::param_types` expects a function"))?;
        let types: Vec<Value> = info
            .param_types
            .iter()
            .map(|t| match t {
                Some(ty) => Value::Type(Box::new(ty.clone())),
                None => Value::Unit,
            })
            .collect();
        Ok(Value::list(types))
    })
    .named_raw_function("return_type", &["f"], |cx, args| {
        let [value] = args else { return arity("return_type", "f") };
        let info = cx
            .fn_info(value)
            .ok_or_else(|| RuntimeError::new("`reflect::func::return_type` expects a function"))?;
        match info.return_type {
            Some(ty) => Ok(Value::Type(Box::new(ty))),
            None => Ok(Value::Unit),
        }
    });
}

fn arity(name: &str, params: &str) -> RunResult<Value> {
    Err(RuntimeError::new(format!(
        "`reflect::func::{name}` takes ({params})"
    )))
}
