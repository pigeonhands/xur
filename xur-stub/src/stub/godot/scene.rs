//! Scene tree traversal — wrappers over hao-godot engine bindings.

use xur_runtime::registry::ModuleBuilder;

use super::GodotError;
use crate::stub::coreclr::{self, ManagedObject};

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.function("scene_tree", || -> Result<ManagedObject, GodotError> {
        let ptr = hao_godot::scene::scene_tree().map_err(|e| GodotError::Failed(e.to_string()))?;
        if ptr.is_null() {
            return Err(GodotError::NullPointer);
        }
        Ok(ManagedObject(ptr as usize))
    });

    m.function("root", || -> Result<ManagedObject, GodotError> {
        let ptr = hao_godot::scene::root().map_err(|e| GodotError::Failed(e.to_string()))?;
        if ptr.is_null() {
            return Err(GodotError::NullPointer);
        }
        Ok(ManagedObject(ptr as usize))
    });

    m.named_function(
        "child",
        &["node", "index"],
        |node: &ManagedObject, index: i64| -> Result<ManagedObject, GodotError> {
            let ptr = hao_godot::scene::child(node.0, index as i32)
                .map_err(|e| GodotError::Failed(e.to_string()))?;
            if ptr.is_null() {
                return Err(GodotError::NullPointer);
            }
            Ok(ManagedObject(ptr as usize))
        },
    );

    m.named_function(
        "child_count",
        &["node"],
        |node: &ManagedObject| -> Result<i64, GodotError> {
            hao_godot::scene::child_count(node.0)
                .map(|c| c as i64)
                .map_err(|e| GodotError::Failed(e.to_string()))
        },
    );

    m.named_function(
        "node_name",
        &["node"],
        |node: &ManagedObject| -> Result<String, GodotError> {
            let rt = hao_godot::runtime().map_err(|e| GodotError::Failed(e.to_string()))?;
            let name_ptr = hao_godot::engine::godot_object::to_string(rt, node.0)
                .map_err(|e| GodotError::Failed(e.to_string()))?;
            if name_ptr.is_null() {
                return Ok("?".into());
            }
            coreclr::read_clr_string(name_ptr as usize).map_err(GodotError::Failed)
        },
    );
}
