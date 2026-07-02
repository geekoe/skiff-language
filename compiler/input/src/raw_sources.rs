use std::fs;

use skiff_compiler_input_model::{
    CompilerRawSourceFile, CompilerSourceRole, RawPublicationSourceGraph, RawSourceFileMeta,
    RawSourceOrigin,
};

use crate::{error::InputAssemblyError, source_tree::SourceTree};

pub fn read_publication_sources(
    source_tree: &SourceTree,
) -> Result<RawPublicationSourceGraph, InputAssemblyError> {
    let files = source_tree
        .sources
        .iter()
        .map(|source| {
            let path = source_tree.root.join(&source.file_path);
            let text = fs::read_to_string(&path).map_err(|source| InputAssemblyError::Read {
                path: path.display().to_string(),
                source,
            })?;
            Ok::<CompilerRawSourceFile, InputAssemblyError>(CompilerRawSourceFile {
                meta: RawSourceFileMeta {
                    relative_path: source.file_path.clone(),
                    module_path: source.module_path.clone(),
                    is_test_file: source.is_test_file,
                    is_generated: false,
                    origin: RawSourceOrigin::Service,
                },
                text,
                role: CompilerSourceRole::Implementation,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RawPublicationSourceGraph {
        root: source_tree.root.clone(),
        files,
    })
}
