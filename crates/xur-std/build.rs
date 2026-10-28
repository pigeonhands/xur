use std::fs;
use std::path::PathBuf;

fn main() {
    let prelude_dir = PathBuf::from("prelude");
    println!("cargo:rerun-if-changed=prelude");

    let mut files: Vec<_> = fs::read_dir(&prelude_dir)
        .expect("prelude/ directory must exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "xur"))
        .collect();
    files.sort_by_key(|e| e.file_name());

    let mut content = String::new();
    for file in &files {
        content.push_str(&fs::read_to_string(file.path()).unwrap());
        content.push('\n');
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("prelude.rs");
    fs::write(out, format!("pub const PRELUDE: &str = {:?};\n", content)).unwrap();
}
