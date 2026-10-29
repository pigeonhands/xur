//! `std::path`: path manipulation.

use std::path::{Path, PathBuf};

use xur_runtime::Registry;

use crate::error_text;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::path")
        .named_raw_function("join", &["parts"], |_cx, args| {
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
        }).returns::<String>()
        .named_function("parent", &["path"], |path: String| -> Option<String> {
            Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .named_function("file_name", &["path"], |path: String| -> Option<String> {
            Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .named_function("stem", &["path"], |path: String| -> Option<String> {
            Path::new(&path)
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .named_function("extension", &["path"], |path: String| -> Option<String> {
            Path::new(&path)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
        })
        .named_function("with_extension", &["path", "ext"], |path: String, ext: String| -> String {
            Path::new(&path)
                .with_extension(ext)
                .to_string_lossy()
                .into_owned()
        })
        .named_function("is_absolute", &["path"], |path: String| Path::new(&path).is_absolute())
        .named_function("is_relative", &["path"], |path: String| Path::new(&path).is_relative())
        .named_function("is_dir", &["path"], |path: String| Path::new(&path).is_dir())
        .named_function("is_file", &["path"], |path: String| Path::new(&path).is_file())
        .named_function("exists", &["path"], |path: String| Path::new(&path).exists())
        .named_function("absolute", &["path"], |path: String| -> Result<String, String> {
            std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(error_text)
        })
        .named_function("components", &["path"], |path: String| -> Vec<String> {
            Path::new(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        });
}
