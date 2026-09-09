use crate::error::Error;
use crate::graph;
use crate::inputs::Inputs;
use crate::loader::MemoryLoader;
use crate::merge::{self, Merged};
use crate::resolve::{self, Resolved};
use crate::span::SourceMap;
use std::path::Path;

/// Merge an in-memory file set. The first pair is the entry file.
pub fn merge_files(files: &[(&str, &str)], inputs: &Inputs) -> Result<(Merged, SourceMap), Error> {
    let loader = MemoryLoader::from_pairs(files);
    let mut sources = SourceMap::default();
    let merged = merge::merge(Path::new(files[0].0), &loader, &mut sources, inputs)?;
    Ok((merged, sources))
}

pub fn merge_src(files: &[(&str, &str)]) -> Result<Merged, Error> {
    merge_files(files, &Inputs::new()).map(|(m, _)| m)
}

pub fn keypath(s: &str) -> Vec<String> {
    s.split('.').map(str::to_string).collect()
}

pub fn resolve_files(
    files: &[(&str, &str)],
    inputs: &Inputs,
) -> Result<(Resolved, SourceMap), Error> {
    let (merged, sources) = merge_files(files, inputs)?;
    merge::check_required(&merged)?;
    let order = graph::topo_order(&merged)?;
    let resolved = resolve::resolve(&merged, &order, inputs)?;
    Ok((resolved, sources))
}

pub fn resolve_src(src: &str) -> Result<Resolved, Error> {
    resolve_files(&[("main.medl", src)], &Inputs::new()).map(|(r, _)| r)
}
