use crate::ast::File;
use crate::error::{Error, ErrorKind};
use crate::parser::parse_source;
use crate::span::SourceMap;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

pub trait SourceLoader {
    /// Read a file's text. `Err` is a human-readable reason.
    fn read(&self, path: &Path) -> Result<String, String>;
}

pub struct FsLoader;

impl SourceLoader for FsLoader {
    fn read(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}

pub struct MemoryLoader(pub HashMap<PathBuf, String>);

impl MemoryLoader {
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        MemoryLoader(
            pairs
                .iter()
                .map(|(p, text)| (normalize(Path::new(p)), text.to_string()))
                .collect(),
        )
    }
}

impl SourceLoader for MemoryLoader {
    fn read(&self, path: &Path) -> Result<String, String> {
        self.0
            .get(&normalize(path))
            .cloned()
            .ok_or_else(|| "no such file".to_string())
    }
}

/// Lexically remove `.` and resolve `..` without touching the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let ends_with_parent =
                    matches!(out.components().next_back(), Some(Component::ParentDir));
                if ends_with_parent || !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn load_file(
    path: &Path,
    loader: &dyn SourceLoader,
    sources: &mut SourceMap,
) -> Result<File, Error> {
    let text = loader.read(path).map_err(|reason| {
        Error::spanless(
            ErrorKind::Load,
            format!("cannot read `{}`: {reason}", path.display()),
        )
    })?;
    let id = sources.add(path.to_path_buf(), text);
    parse_source(sources.text(id), id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::span::{FileId, SourceMap};
    use std::path::Path;

    #[test]
    fn normalize_removes_dot_and_resolves_parent() {
        assert_eq!(
            normalize(Path::new("a/./b/../c.medl")),
            Path::new("a/c.medl")
        );
        assert_eq!(normalize(Path::new("../x.medl")), Path::new("../x.medl"));
        assert_eq!(
            normalize(Path::new("a/../../x.medl")),
            Path::new("../x.medl")
        );
        assert_eq!(normalize(Path::new("x.medl")), Path::new("x.medl"));
    }

    #[test]
    fn memory_loader_reads_registered_files() {
        let loader = MemoryLoader::from_pairs(&[("a/./b.medl", "x = 1")]);
        assert_eq!(loader.read(Path::new("a/b.medl")), Ok("x = 1".to_string()));
        assert_eq!(
            loader.read(Path::new("nope.medl")),
            Err("no such file".to_string())
        );
    }

    #[test]
    fn load_file_registers_source_and_parses() {
        let loader = MemoryLoader::from_pairs(&[("a.medl", "x = 1")]);
        let mut sources = SourceMap::default();
        let file = load_file(Path::new("a.medl"), &loader, &mut sources).unwrap();
        assert_eq!(file.statements.len(), 1);
        assert_eq!(sources.path(file.file), Path::new("a.medl"));
        assert_eq!(sources.text(file.file), "x = 1");
    }

    #[test]
    fn load_file_reports_missing_and_parse_errors() {
        let loader = MemoryLoader::from_pairs(&[("bad.medl", "x = ")]);
        let mut sources = SourceMap::default();
        let err = load_file(Path::new("nope.medl"), &loader, &mut sources).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Load);
        assert_eq!(err.message, "cannot read `nope.medl`: no such file");
        assert_eq!(err.span, None);
        let err = load_file(Path::new("bad.medl"), &loader, &mut sources).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Parse);
        assert_eq!(err.span.unwrap().file, FileId(0));
    }
}
