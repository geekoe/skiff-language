use std::sync::Arc;

use skiff_runtime_linked_program::{
    FileAddr, LinkedFileUnit, PackageSymbolKey, RuntimeTypeContext, TypeAddr, UnitAddr,
};

pub(super) fn build_execution_type_index(
    shared: &skiff_runtime_linked_program::SharedPackageLinkedImage,
    files: &[Vec<Arc<LinkedFileUnit>>],
) -> anyhow::Result<RuntimeTypeContext> {
    let mut types = RuntimeTypeContext::default();

    for (code_slot, (code, package_files)) in shared.code_slots().iter().zip(files).enumerate() {
        for (file_index, file) in package_files.iter().enumerate() {
            for (type_index, declaration) in file.types.iter().enumerate() {
                let addr = TypeAddr {
                    unit: UnitAddr::Package(code_slot),
                    file: FileAddr::LoadedFileIndex(file_index),
                    type_index,
                };
                if types
                    .descriptors
                    .insert(addr, declaration.clone())
                    .is_some()
                {
                    anyhow::bail!("duplicate assembly type address");
                }
            }
        }

        for (symbol, export) in &code.artifact().implementation_links.types {
            let addr = execution_type_export_addr(code_slot, package_files, export)?;
            let key = PackageSymbolKey::new(code_slot, symbol.clone());
            if types
                .exported_types
                .insert_package(key, addr.clone())
                .is_some()
            {
                anyhow::bail!("duplicate package type export {symbol}");
            }
            if code.artifact().package_id == "skiff.run/std" {
                let std_key = PackageSymbolKey::new(code_slot, format!("std.{symbol}"));
                if types
                    .exported_types
                    .insert_package(std_key, addr.clone())
                    .is_some()
                {
                    anyhow::bail!("duplicate std package type export {symbol}");
                }
            }
        }
    }
    Ok(types)
}

fn execution_type_export_addr(
    code_slot: usize,
    files: &[Arc<LinkedFileUnit>],
    export: &skiff_artifact_model::TypeExport,
) -> anyhow::Result<TypeAddr> {
    let file_index = execution_file_index(files, &export.file)?;
    let file = &files[file_index];
    let type_index = export.type_index as usize;
    if type_index >= file.types.len() {
        anyhow::bail!("type export index is out of bounds");
    }
    Ok(TypeAddr {
        unit: UnitAddr::Package(code_slot),
        file: FileAddr::LoadedFileIndex(file_index),
        type_index,
    })
}

fn execution_file_index(
    files: &[Arc<LinkedFileUnit>],
    file_ref: &skiff_artifact_model::FileIrRef,
) -> anyhow::Result<usize> {
    let mut matches = files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.file_ir_identity == file_ref.file_ir_identity);
    let (file_index, file) = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("export File IR is not loaded"))?;
    if matches.next().is_some()
        || file.module_path != file_ref.module_path
        || file_ref
            .source_ast_hash
            .as_deref()
            .is_some_and(|hash| hash != file.source_ast_hash)
    {
        anyhow::bail!("export File IR ref does not exactly match loaded code");
    }
    Ok(file_index)
}
