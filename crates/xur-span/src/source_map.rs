//! Every source file in a session, after `rustc_span::source_map`.
//!
//! Files are laid out one after another in a single global space of byte
//! positions, so a [`Span`] identifies its file as well as its location and
//! spans from different files never collide.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::Span;

/// A 1-based line and column. The column counts characters, not bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// A loaded source file and the global positions it occupies.
#[derive(Debug)]
pub struct SourceFile {
    /// The name shown in diagnostics, usually the path as given.
    pub name: String,
    pub src: String,
    /// Global position of the file's first byte.
    pub start: usize,
    /// Byte offset (within the file) at which each line starts.
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(name: String, src: String, start: usize) -> Self {
        let line_starts = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            name,
            src,
            start,
            line_starts,
        }
    }

    /// Global position one past the file's last byte.
    pub fn end(&self) -> usize {
        self.start + self.src.len()
    }

    /// The span of the whole file.
    pub fn span(&self) -> Span {
        Span::new(self.start, self.end())
    }

    pub fn contains(&self, pos: usize) -> bool {
        (self.start..=self.end()).contains(&pos)
    }

    /// The source text covered by `span`, which must lie in this file.
    pub fn slice(&self, span: Span) -> &str {
        &self.src[span.start - self.start..span.end - self.start]
    }

    /// The line/column of global position `pos`, which may be the end of
    /// the file.
    pub fn line_col(&self, pos: usize) -> LineCol {
        let offset = pos - self.start;
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let line_start = self.line_starts[line];
        let col = self.src[line_start..offset].chars().count();
        LineCol {
            line: line as u32 + 1,
            col: col as u32 + 1,
        }
    }
}

/// Reads source files; abstracted so tests can supply files from memory
/// (rustc's `FileLoader`).
pub trait FileLoader {
    fn read_file(&self, path: &Path) -> io::Result<String>;
}

/// Reads files from disk.
pub struct RealFileLoader;

impl FileLoader for RealFileLoader {
    fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// Serves files from memory, for tests and for embedding scripts that
/// aren't on disk.
#[derive(Debug, Default)]
pub struct MemoryFileLoader {
    files: HashMap<PathBuf, String>,
}

impl MemoryFileLoader {
    pub fn new<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(path, src)| (PathBuf::from(path), src.to_owned()))
                .collect(),
        }
    }
}

impl FileLoader for MemoryFileLoader {
    fn read_file(&self, path: &Path) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "file not found"))
    }
}

#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<Arc<SourceFile>>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file after all existing ones. A one-byte gap is left between
    /// files so that the end-of-file position of one is not the start of
    /// the next.
    pub fn add_file(&mut self, name: impl Into<String>, src: impl Into<String>) -> Arc<SourceFile> {
        let start = self.files.last().map_or(0, |file| file.end() + 1);
        let file = Arc::new(SourceFile::new(name.into(), src.into(), start));
        self.files.push(Arc::clone(&file));
        file
    }

    pub fn files(&self) -> &[Arc<SourceFile>] {
        &self.files
    }

    /// The file containing global position `pos`.
    pub fn lookup_file(&self, pos: usize) -> &Arc<SourceFile> {
        let index = self.files.partition_point(|file| file.start <= pos) - 1;
        let file = &self.files[index];
        debug_assert!(file.contains(pos), "position {pos} is in no file");
        file
    }

    /// The source text covered by `span`.
    pub fn slice(&self, span: Span) -> &str {
        self.lookup_file(span.start).slice(span)
    }

    /// `name:line:col` for the start of `span`.
    pub fn describe_pos(&self, span: Span) -> String {
        let file = self.lookup_file(span.start);
        format!("{}:{}", file.name, file.line_col(span.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(src: &str) -> Arc<SourceFile> {
        SourceMap::new().add_file("test.xur", src)
    }

    #[test]
    fn line_col_first_line() {
        let file = single("let x");
        assert_eq!(file.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(file.line_col(4), LineCol { line: 1, col: 5 });
    }

    #[test]
    fn line_col_after_newlines() {
        let file = single("a\nbc\n\nd");
        assert_eq!(file.line_col(2), LineCol { line: 2, col: 1 });
        assert_eq!(file.line_col(3), LineCol { line: 2, col: 2 });
        assert_eq!(file.line_col(5), LineCol { line: 3, col: 1 });
        assert_eq!(file.line_col(6), LineCol { line: 4, col: 1 });
    }

    #[test]
    fn line_col_counts_chars_not_bytes() {
        // `α` is two bytes wide.
        assert_eq!(single("α = 1").line_col(3), LineCol { line: 1, col: 3 });
    }

    #[test]
    fn line_col_at_end_of_input() {
        let file = single("a\nb");
        assert_eq!(file.line_col(file.end()), LineCol { line: 2, col: 2 });
    }

    #[test]
    fn files_occupy_disjoint_positions() {
        let mut map = SourceMap::new();
        let a = map.add_file("a.xur", "let a");
        let b = map.add_file("b.xur", "let bb");
        assert_eq!(a.span(), Span::new(0, 5));
        assert_eq!(b.span(), Span::new(6, 12));
        assert_eq!(map.lookup_file(5).name, "a.xur");
        assert_eq!(map.lookup_file(6).name, "b.xur");
        assert_eq!(map.slice(Span::new(10, 12)), "bb");
        assert_eq!(b.line_col(10), LineCol { line: 1, col: 5 });
        assert_eq!(map.describe_pos(Span::new(10, 12)), "b.xur:1:5");
    }
}
