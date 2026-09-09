use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        Span { file, start, end }
    }
    pub fn to(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start,
            end: other.end,
        }
    }
}

pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, path: PathBuf, text: String) -> FileId {
        self.files.push(SourceFile { path, text });
        FileId((self.files.len() - 1) as u32)
    }
    pub fn text(&self, id: FileId) -> &str {
        &self.files[id.0 as usize].text
    }
    pub fn path(&self, id: FileId) -> &Path {
        &self.files[id.0 as usize].path
    }
    /// 1-based (line, column) of the span start. Column counts chars.
    pub fn line_col(&self, span: Span) -> (usize, usize) {
        let text = self.text(span.file);
        let upto = &text[..(span.start as usize).min(text.len())];
        let line = upto.matches('\n').count() + 1;
        let col = upto
            .rsplit('\n')
            .next()
            .map(|s| s.chars().count())
            .unwrap_or(0)
            + 1;
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_returns_sequential_ids_and_keeps_path_and_text() {
        let mut sm = SourceMap::default();
        let a = sm.add("a.medl".into(), "x".to_string());
        let b = sm.add("b.medl".into(), "y".to_string());
        assert_eq!(a, FileId(0));
        assert_eq!(b, FileId(1));
        assert_eq!(sm.path(b).to_str(), Some("b.medl"));
        assert_eq!(sm.text(a), "x");
    }

    #[test]
    fn line_col_is_one_based_and_counts_newlines() {
        let mut sm = SourceMap::default();
        let id = sm.add("a.medl".into(), "ab\ncd\n".to_string());
        assert_eq!(sm.line_col(Span::new(id, 0, 1)), (1, 1));
        assert_eq!(sm.line_col(Span::new(id, 1, 2)), (1, 2));
        assert_eq!(sm.line_col(Span::new(id, 4, 5)), (2, 2));
    }
}
