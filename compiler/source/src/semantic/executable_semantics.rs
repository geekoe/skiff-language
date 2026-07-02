use std::collections::BTreeMap;

use crate::shared::error::Result;

use super::{ExecutableIndex, SemanticPublication};

/// Per-module executable indexes for a publication.
///
/// This used to live alongside the `process` semantic model. Processes have been
/// removed from the language, but the executable index map is still needed by the
/// rest of the compiler, so it is kept here under a process-free name.
#[derive(Debug, Clone)]
pub struct ExecutableSemantics<'a> {
    executable_indexes: BTreeMap<&'a str, ExecutableIndex>,
}

impl ExecutableSemantics<'_> {
    pub fn executable_index(&self, module_path: &str) -> Option<&ExecutableIndex> {
        self.executable_indexes.get(module_path)
    }
}

pub fn build_executable_semantics<'a>(
    publication: &'a SemanticPublication<'a>,
) -> Result<ExecutableSemantics<'a>> {
    let executable_indexes = publication
        .sources
        .iter()
        .map(|source| Ok((source.module_path, ExecutableIndex::source_index(source)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(ExecutableSemantics { executable_indexes })
}
