//! `std::path`: path manipulation.

use std::path::{Path, PathBuf};

use xur_runtime::Registry;

use crate::error_text;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::path")
        .raw_function("join", |_cx, args| {
            if args.is_empty() {
                return Ok(xur_runtime::Value::str(""));
            }
            let mut path = PathBuf::new();
            for arg in args {
                match arg {
                    xur_runtime::Value::Str(s) => path.push(&**s),
                    other => {
                        return Err(xur_runtime::RuntimeError::new(format!(
                            "`path::join` takes strings, got {}",
                            other.kind()
                        )))
                    }
                }
            }
            Ok(xur_runtime::Value::str(&path.to_string_lossy()))
        })
        .function("parent", |path: String| -> Option<String> {
            Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .function("file_name", |path: String| -> Option<String> {
            Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .function("stem", |path: String| -> Option<String> {
            Path::new(&path)
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .function("extension", |path: String| -> Option<String> {
            Path::new(&path)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
        })
        .function("with_extension", |path: String, ext: String| -> String {
            Path::new(&path)
                .with_extension(ext)
                .to_string_lossy()
                .into_owned()
        })
        .function("is_absolute", |path: String| Path::new(&path).is_absolute())
        .function("is_relative", |path: String| Path::new(&path).is_relative())
        .function("is_dir", |path: String| Path::new(&path).is_dir())
        .function("is_file", |path: String| Path::new(&path).is_file())
        .function("exists", |path: String| Path::new(&path).exists())
        .function("absolute", |path: String| -> Result<String, String> {
            std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(error_text)
        })
        .function("components", |path: String| -> Vec<String> {
            Path::new(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        });
}
