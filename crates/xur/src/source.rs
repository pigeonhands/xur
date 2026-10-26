use std::path::Path;

use anyhow::{Context, Result};

/// A source file loaded into memory.
pub struct SourceFile {
    /// The path as given by the user, used in diagnostics.
    pub name: String,
    pub text: String,
}

impl SourceFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read `{}`", path.display()))?;
        Ok(Self {
            name: path.display().to_string(),
            text,
        })
    }
}
