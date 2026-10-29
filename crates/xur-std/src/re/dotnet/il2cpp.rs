//! `std::re::dotnet::il2cpp`: read Unity Il2Cpp games with
//! [hao-il2cpp](https://docs.rs/hao-il2cpp).
//!
//! `il2cpp::open(path)` opens a game folder and gives access to every type,
//! field and method through `TypeView`, `FieldView` and `MethodView`.
//!
//! ```text
//! let game = std::re::dotnet::il2cpp::open("path/to/game")?;
//! for ty in game.types() {
//!     io::println("{}", ty.full_name());
//! }
//! game.write_assemblies("out")?;
//! ```

use std::sync::Arc;

use facet::Facet;
use hao_il2cpp::{FieldView, Game, MethodView, ParameterView, TypeView};
use xur_runtime::convert::into_value;
use xur_runtime::registry::{ModuleBuilder, XurType};
use xur_runtime::{Registry, RuntimeError, Value};

use crate::{error_text, result};

// ---------------------------------------------------------------------------
// Opaque wrappers — Game owns its data; views borrow it with lifetimes that
// can't survive as Xur values, so we Arc the Game and clone into each view.
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct Il2CppGame(Arc<Game>);

impl std::fmt::Display for Il2CppGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Game")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct Il2CppTypeView {
    game: Arc<Game>,
    index: i32,
}

impl std::fmt::Display for Il2CppTypeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self
            .game
            .types()
            .find(|t| t.index() == self.index)
            .map(|t| t.full_name())
            .unwrap_or_default();
        write!(f, "TypeView({})", name)
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct Il2CppFieldInfo {
    name: String,
    type_name: String,
    is_static: bool,
}

impl std::fmt::Display for Il2CppFieldInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_static {
            write!(f, "static ")?;
        }
        write!(f, "{} {}", self.type_name, self.name)
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct Il2CppMethodInfo {
    name: String,
    return_type: String,
    parameters: Vec<(String, String)>,
    is_static: bool,
}

impl std::fmt::Display for Il2CppMethodInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_static {
            write!(f, "static ")?;
        }
        let params: Vec<String> = self
            .parameters
            .iter()
            .map(|(ty, name)| format!("{ty} {name}"))
            .collect();
        write!(
            f,
            "{} {}({})",
            self.return_type,
            self.name,
            params.join(", ")
        )
    }
}

// ---------------------------------------------------------------------------
// Snapshot helpers — extract owned data from borrowed views
// ---------------------------------------------------------------------------

fn snapshot_type(game: &Arc<Game>, view: TypeView<'_>) -> Il2CppTypeView {
    Il2CppTypeView {
        game: Arc::clone(game),
        index: view.index(),
    }
}

fn snapshot_field(view: FieldView<'_>) -> Il2CppFieldInfo {
    Il2CppFieldInfo {
        name: view.name().to_owned(),
        type_name: view.type_name(),
        is_static: view.is_static(),
    }
}

fn snapshot_method(view: MethodView<'_>) -> Il2CppMethodInfo {
    Il2CppMethodInfo {
        name: view.name().to_owned(),
        return_type: view.return_type_name(),
        parameters: view
            .parameters()
            .map(|p| (p.type_name(), p.name().to_owned()))
            .collect(),
        is_static: view.is_static(),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(super) fn register(registry: &Registry, dotnet: &mut ModuleBuilder<'_>) {
    let mut il2cpp = dotnet.submodule("il2cpp");

    il2cpp.ty::<Il2CppFieldInfo>("Field");
    il2cpp.ty::<Il2CppMethodInfo>("Method");

    // --- open ---

    il2cpp.raw_function_returns::<Il2CppGame>("open", |cx, args| {
        let [Value::Str(path)] = args else {
            return Err(RuntimeError::new("`open` takes a path string"));
        };
        match Game::open(path.as_ref()) {
            Ok(game) => {
                let v = into_value(cx, Il2CppGame(Arc::new(game)))?;
                Ok(result(cx, Ok(v)))
            }
            Err(e) => Ok(result(cx, Err(error_text(e)))),
        }
    });

    il2cpp.raw_function_returns::<Il2CppGame>("from_files", |cx, args| {
        let [Value::Str(metadata), Value::Str(binary)] = args else {
            return Err(RuntimeError::new(
                "`from_files` takes (metadata_path, binary_path)",
            ));
        };
        match Game::from_files(metadata.as_ref(), binary.as_ref()) {
            Ok(game) => {
                let v = into_value(cx, Il2CppGame(Arc::new(game)))?;
                Ok(result(cx, Ok(v)))
            }
            Err(e) => Ok(result(cx, Err(error_text(e)))),
        }
    });

    // --- Game methods ---

    il2cpp
        .ty::<Il2CppGame>("Game")
        .raw_function_returns_xur("types", XurType::LIST, |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`types` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Game`"));
            };
            let list = obj
                .with(
                    |g: &Il2CppGame| -> std::result::Result<Vec<Value>, RuntimeError> {
                        g.0.types()
                            .map(|tv| into_value(cx, snapshot_type(&g.0, tv)))
                            .collect()
                    },
                )
                .ok_or_else(|| RuntimeError::new("expected a `Game`"))??;
            Ok(Value::list(list))
        })
        .raw_function_returns::<Il2CppTypeView>("find_type", |cx, args| {
            let [this, Value::Str(name)] = args else {
                return Err(RuntimeError::new("`find_type` takes a type name string"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Game`"));
            };
            let res = obj
                .with(
                    |g: &Il2CppGame| -> std::result::Result<Value, RuntimeError> {
                        match g.0.find_type(name.as_ref()) {
                            Some(tv) => {
                                let v = into_value(cx, snapshot_type(&g.0, tv))?;
                                Ok(result(cx, Ok(v)))
                            }
                            None => Ok(result(cx, Err(format!("type `{}` not found", name)))),
                        }
                    },
                )
                .ok_or_else(|| RuntimeError::new("expected a `Game`"))??;
            Ok(res)
        })
        .method("image_names", |g: &Il2CppGame| -> Vec<String> {
            g.0.image_names().map(|s| s.to_owned()).collect()
        })
        .raw_function("write_assemblies", |cx, args| {
            let [this, Value::Str(dir)] = args else {
                return Err(RuntimeError::new(
                    "`write_assemblies` takes a directory path",
                ));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `Game`"));
            };
            let res = obj
                .with(|g: &Il2CppGame| {
                    g.0.write_assemblies(dir.as_ref())
                        .map(|paths| {
                            paths
                                .into_iter()
                                .map(|p| Value::str(&p.display().to_string()))
                                .collect::<Vec<_>>()
                        })
                        .map_err(error_text)
                })
                .ok_or_else(|| RuntimeError::new("expected a `Game`"))?;
            match res {
                Ok(paths) => Ok(result(cx, Ok(Value::list(paths)))),
                Err(e) => Ok(result(cx, Err(e))),
            }
        });

    // --- TypeView methods ---

    il2cpp
        .ty::<Il2CppTypeView>("TypeView")
        .method("name", |tv: &Il2CppTypeView| -> String {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .map(|t| t.name().to_owned())
                .unwrap_or_default()
        })
        .method("namespace", |tv: &Il2CppTypeView| -> String {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .map(|t| t.namespace().to_owned())
                .unwrap_or_default()
        })
        .method("full_name", |tv: &Il2CppTypeView| -> String {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .map(|t| t.full_name())
                .unwrap_or_default()
        })
        .method("image_name", |tv: &Il2CppTypeView| -> String {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .map(|t| t.image_name().to_owned())
                .unwrap_or_default()
        })
        .method("flags", |tv: &Il2CppTypeView| -> i64 {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .map(|t| t.flags() as i64)
                .unwrap_or(0)
        })
        .method("base_type_name", |tv: &Il2CppTypeView| -> Option<String> {
            let types: Vec<_> = tv.game.types().collect();
            types
                .iter()
                .find(|t| t.index() == tv.index)
                .and_then(|t| t.base_type_name())
        })
        .raw_function_returns_xur("fields", XurType::LIST, |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`fields` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `TypeView`"));
            };
            let list = obj
                .with(
                    |tv: &Il2CppTypeView| -> std::result::Result<Vec<Value>, RuntimeError> {
                        let types: Vec<_> = tv.game.types().collect();
                        let view = types
                            .iter()
                            .find(|t| t.index() == tv.index)
                            .ok_or_else(|| RuntimeError::new("type not found"))?;
                        view.fields()
                            .map(|fv| into_value(cx, snapshot_field(fv)))
                            .collect()
                    },
                )
                .ok_or_else(|| RuntimeError::new("expected a `TypeView`"))??;
            Ok(Value::list(list))
        })
        .raw_function_returns_xur("methods", XurType::LIST, |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`methods` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a `TypeView`"));
            };
            let list = obj
                .with(
                    |tv: &Il2CppTypeView| -> std::result::Result<Vec<Value>, RuntimeError> {
                        let types: Vec<_> = tv.game.types().collect();
                        let view = types
                            .iter()
                            .find(|t| t.index() == tv.index)
                            .ok_or_else(|| RuntimeError::new("type not found"))?;
                        view.methods()
                            .map(|mv| into_value(cx, snapshot_method(mv)))
                            .collect()
                    },
                )
                .ok_or_else(|| RuntimeError::new("expected a `TypeView`"))??;
            Ok(Value::list(list))
        });

    // --- FieldView / MethodView read-only accessors ---

    il2cpp
        .ty::<Il2CppFieldInfo>("FieldView")
        .method("name", |f: &Il2CppFieldInfo| f.name.clone())
        .method("type_name", |f: &Il2CppFieldInfo| f.type_name.clone())
        .method("is_static", |f: &Il2CppFieldInfo| f.is_static);

    il2cpp
        .ty::<Il2CppMethodInfo>("MethodView")
        .method("name", |m: &Il2CppMethodInfo| m.name.clone())
        .method("return_type", |m: &Il2CppMethodInfo| m.return_type.clone())
        .method("is_static", |m: &Il2CppMethodInfo| m.is_static)
        .method("parameter_names", |m: &Il2CppMethodInfo| -> Vec<String> {
            m.parameters.iter().map(|(_, n)| n.clone()).collect()
        })
        .method(
            "parameter_type_names",
            |m: &Il2CppMethodInfo| -> Vec<String> {
                m.parameters.iter().map(|(t, _)| t.clone()).collect()
            },
        );

    // Walk the remaining reflected shapes (Binary, Metadata, GeneratedAssembly
    // and their reachable structs/enums).
    let skip = [
        <Game as Facet>::SHAPE,
        <TypeView as Facet>::SHAPE,
        <FieldView as Facet>::SHAPE,
        <MethodView as Facet>::SHAPE,
        <ParameterView as Facet>::SHAPE,
    ];
    crate::re::walk::register(
        registry,
        &mut il2cpp,
        hao_il2cpp::reflect::roots(),
        submodule_of,
        &skip,
    );
}

fn submodule_of(shape: &facet::Shape) -> Option<&'static str> {
    let path = shape.module_path?;
    if !path.starts_with("hao_il2cpp") {
        return None;
    }
    let mut segments = path.split("::");
    let _ = segments.next(); // "hao_il2cpp"
    match segments.next() {
        Some("binary") => Some("binary"),
        Some("metadata") => Some("metadata"),
        Some("generate") => Some("generate"),
        Some("game") => Some(""),
        _ => Some(""),
    }
}
