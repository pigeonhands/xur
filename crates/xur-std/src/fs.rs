//! `std::fs`: files.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use facet::Facet;
use xur_runtime::{Registry, lock};

use crate::error_text;

/// An open file, shared by every Xur value referring to it.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct File(Arc<Mutex<fs::File>>);

impl File {
    fn new(file: fs::File) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }
}

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::fs")
        .function("read_to_string", |path: String| {
            fs::read_to_string(path).map_err(error_text)
        })
        .function("write", |path: String, contents: String| {
            fs::write(path, contents).map_err(error_text)
        })
        .function(
            "append",
            |path: String, contents: String| -> Result<(), String> {
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                    .map_err(error_text)?;
                file.write_all(contents.as_bytes()).map_err(error_text)
            },
        )
        .function("exists", |path: String| Path::new(&path).exists())
        .function("remove", |path: String| {
            fs::remove_file(path).map_err(error_text)
        })
        .function("create_dir_all", |path: String| {
            fs::create_dir_all(path).map_err(error_text)
        })
        .function("read_dir", |path: String| -> Result<Vec<String>, String> {
            let mut names = fs::read_dir(path)
                .map_err(error_text)?
                .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(error_text)?;
            names.sort();
            Ok(names)
        })
        .ty::<File>("File")
        .function("open", |path: String| {
            fs::File::open(path).map(File::new).map_err(error_text)
        })
        .function("create", |path: String| {
            fs::File::create(path).map(File::new).map_err(error_text)
        })
        .method("read_to_string", |file: &File| {
            let mut contents = String::new();
            lock(&file.0)
                .read_to_string(&mut contents)
                .map(|_| contents)
                .map_err(error_text)
        })
        .method("write", |file: &File, contents: String| {
            lock(&file.0)
                .write_all(contents.as_bytes())
                .map_err(error_text)
        });
}
