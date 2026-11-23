//! Scene tree traversal — wrappers over hao-godot engine bindings.

use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

use crate::stub::coreclr::{self, ManagedObject};

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.raw_function(
        "scene_tree",
        |cx, _args| match hao_godot::scene::scene_tree() {
            Ok(ptr) if !ptr.is_null() => {
                let v = coreclr::managed_object_value(cx, ptr as usize)?;
                Ok(Value::adt(cx.lang().ok, vec![v]))
            }
            Ok(_) => Ok(Value::adt(
                cx.lang().err,
                vec![Value::str("scene_tree returned null")],
            )),
            Err(e) => Ok(Value::adt(cx.lang().err, vec![Value::str(&e.to_string())])),
        },
    );

    m.raw_function("root", |cx, _args| match hao_godot::scene::root() {
        Ok(ptr) if !ptr.is_null() => {
            let v = coreclr::managed_object_value(cx, ptr as usize)?;
            Ok(Value::adt(cx.lang().ok, vec![v]))
        }
        Ok(_) => Ok(Value::adt(
            cx.lang().err,
            vec![Value::str("root returned null")],
        )),
        Err(e) => Ok(Value::adt(cx.lang().err, vec![Value::str(&e.to_string())])),
    });

    m.raw_function("child", |cx, args| {
        let [Value::Native(node), Value::Int(index)] = args else {
            return Err(RuntimeError::new("`child` takes (node, index)"));
        };
        let addr = node
            .with(|o: &ManagedObject| o.0)
            .ok_or_else(|| RuntimeError::new("expected a ManagedObject"))?;
        match hao_godot::scene::child(addr, *index as i32) {
            Ok(ptr) if !ptr.is_null() => {
                let v = coreclr::managed_object_value(cx, ptr as usize)?;
                Ok(Value::adt(cx.lang().ok, vec![v]))
            }
            Ok(_) => Ok(Value::adt(
                cx.lang().err,
                vec![Value::str("child returned null")],
            )),
            Err(e) => Ok(Value::adt(cx.lang().err, vec![Value::str(&e.to_string())])),
        }
    });

    m.function(
        "child_count",
        |node: &ManagedObject| -> Result<i64, String> {
            hao_godot::scene::child_count(node.0)
                .map(|c| c as i64)
                .map_err(|e| e.to_string())
        },
    );

    m.function(
        "node_name",
        |node: &ManagedObject| -> Result<String, String> {
            let rt = hao_godot::runtime().map_err(|e| e.to_string())?;
            let name_ptr = hao_godot::engine::godot_object::to_string(rt, node.0)
                .map_err(|e| e.to_string())?;
            if name_ptr.is_null() {
                return Ok("?".into());
            }
            Ok(coreclr::read_clr_string(name_ptr as usize))
        },
    );
}
