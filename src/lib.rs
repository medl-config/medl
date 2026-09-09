pub mod ast;
pub mod builtins;
pub mod error;
pub mod eval;
pub mod graph;
pub mod inputs;
pub mod lexer;
pub mod lint;
pub mod loader;
pub mod merge;
pub mod parser;
pub mod resolve;
pub mod span;
pub mod value;

#[cfg(test)]
mod test_util;

use std::path::Path;

pub use error::{Error, Warning};
pub use value::Value;

#[derive(Debug)]
pub struct Output {
    pub value: Value,
    pub warnings: Vec<Warning>,
}

/// One resolution session. Keeps the `SourceMap` so callers can render errors.
pub struct Medl {
    pub sources: span::SourceMap,
    loader: Box<dyn loader::SourceLoader>,
}

impl Medl {
    pub fn new(loader: Box<dyn loader::SourceLoader>) -> Self {
        Medl {
            sources: span::SourceMap::default(),
            loader,
        }
    }

    /// Spec section 2: merge, required check, graph build, resolve.
    ///
    /// `sources` accumulates every file read, across calls, so that errors from an earlier
    /// resolution still render; create a new `Medl` per resolution if that matters.
    pub fn resolve(&mut self, entry: &Path, inputs: &inputs::Inputs) -> Result<Output, Error> {
        let merged = merge::merge(entry, self.loader.as_ref(), &mut self.sources, inputs)?;
        merge::check_required(&merged)?;
        let order = graph::topo_order(&merged)?;
        let resolved = resolve::resolve(&merged, &order, inputs)?;
        let mut warnings = lint::lint(&merged);
        warnings.extend(resolved.warnings);
        Ok(Output {
            value: resolved.value,
            warnings,
        })
    }
}
