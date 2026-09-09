use crate::span::{SourceMap, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Lex,
    Parse,
    Load,
    Merge,
    Required,
    Cycle,
    Resolve,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<Span>,
    pub labels: Vec<Label>,
}

impl Error {
    pub fn new(kind: ErrorKind, span: Span, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
            span: Some(span),
            labels: Vec::new(),
        }
    }
    pub fn spanless(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
            span: None,
            labels: Vec::new(),
        }
    }
    pub fn label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }
    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = format!("error: {}\n", self.message);
        if let Some(span) = self.span {
            out.push_str(&format!("  --> {}\n", location(sources, span)));
        }
        for label in &self.labels {
            out.push_str(&format!(
                "  = note: {}: {}\n",
                location(sources, label.span),
                label.message
            ));
        }
        out
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Locations live in the `SourceMap`, so `Display` is the message alone; `render` adds them.
impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub message: String,
    pub span: Span,
}

impl Warning {
    pub fn render(&self, sources: &SourceMap) -> String {
        format!(
            "warning: {}\n  --> {}\n",
            self.message,
            location(sources, self.span)
        )
    }
}

fn location(sources: &SourceMap, span: Span) -> String {
    let (line, col) = sources.line_col(span);
    format!("{}:{}:{}", sources.path(span.file).display(), line, col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{SourceMap, Span};

    #[test]
    fn render_includes_path_line_col_and_labels() {
        let mut sm = SourceMap::default();
        let id = sm.add("a.medl".into(), "x = 1\ny = 2\n".into());
        let err = Error::new(ErrorKind::Resolve, Span::new(id, 6, 7), "boom")
            .label(Span::new(id, 0, 1), "first defined here");
        assert_eq!(
            err.render(&sm),
            "error: boom\n  --> a.medl:2:1\n  = note: a.medl:1:1: first defined here\n"
        );
    }

    #[test]
    fn spanless_error_renders_message_only() {
        let sm = SourceMap::default();
        let err = Error::spanless(ErrorKind::Load, "cannot read `x.medl`");
        assert_eq!(err.render(&sm), "error: cannot read `x.medl`\n");
    }

    /// `Display` is the bare message; `render` is the one that adds locations.
    #[test]
    fn display_and_the_std_error_trait_are_implemented() {
        let err = Error::spanless(ErrorKind::Load, "cannot read `x.medl`");
        assert_eq!(format!("{err}"), err.message);

        fn propagates() -> Result<(), Box<dyn std::error::Error>> {
            Err(Error::spanless(ErrorKind::Load, "boom"))?;
            Ok(())
        }
        assert_eq!(propagates().unwrap_err().to_string(), "boom");
    }

    #[test]
    fn warning_renders_with_location() {
        let mut sm = SourceMap::default();
        let id = sm.add("a.medl".into(), "x = 1".into());
        let w = Warning {
            message: "dead code".into(),
            span: Span::new(id, 4, 5),
        };
        assert_eq!(w.render(&sm), "warning: dead code\n  --> a.medl:1:5\n");
    }
}
