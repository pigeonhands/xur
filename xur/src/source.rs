use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use xur_span::{SourceFile, SourceMap};

/// Reads `path` into the source map, naming it by the path as given.
pub fn load(source_map: &mut SourceMap, path: &Path) -> Result<Arc<SourceFile>> {
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    Ok(source_map.add_file(path.display().to_string(), src))
}
