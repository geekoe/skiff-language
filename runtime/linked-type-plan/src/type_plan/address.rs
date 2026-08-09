use super::*;

pub(crate) fn program_package_type_addr(
    program: ProgramTypeView<'_>,
    symbol: &PackageSymbolRef,
) -> Option<TypeAddr> {
    let resolved = match &symbol.package {
        PackageRefIr::PackageId { package_id } => program
            .link_overlay
            .resolved_package_id_symbol(package_id, &symbol.symbol_path),
        PackageRefIr::Dependency { dependency_ref } => program
            .link_overlay
            .resolved_package_dependency_ref_symbol(dependency_ref, &symbol.symbol_path),
    }?;
    match resolved {
        ResolvedSymbol::Type { addr } => Some(addr.clone()),
        _ => None,
    }
}

pub(crate) fn program_db_object_type_addr(
    program: ProgramTypeView<'_>,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    match unit {
        UnitAddr::Service => {
            let local = program_local_type_addr(program.service_files, unit, symbol)?;
            Ok(local.or_else(|| {
                program
                    .types
                    .exported_service_type(&symbol.module_path, &symbol.symbol)
                    .cloned()
            }))
        }
        UnitAddr::Package(slot) => {
            let Some(files) = program.package_files(*slot) else {
                return Ok(None);
            };
            program_local_type_addr(files, unit, symbol)
        }
    }
}

pub(crate) fn program_publication_type_addr(
    program: ProgramTypeView<'_>,
    unit: &UnitAddr,
    module_path: &str,
    type_index: usize,
) -> Option<TypeAddr> {
    let files = match unit {
        UnitAddr::Service => program.service_files,
        UnitAddr::Package(slot) => program.package_files(*slot)?,
    };
    let (file_index, file) = files
        .iter()
        .enumerate()
        .find(|(_, file)| file.module_path == module_path)?;
    if type_index >= file.types.len() {
        return None;
    }
    Some(TypeAddr {
        unit: unit.clone(),
        file: FileAddr::LoadedFileIndex(file_index),
        type_index,
    })
}

pub(crate) fn program_service_symbol_type_addr(
    program: ProgramTypeView<'_>,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    if let Some(addr) = program
        .types
        .exported_service_type(&symbol.module_path, &symbol.symbol)
        .cloned()
    {
        return Ok(Some(addr));
    }
    let UnitAddr::Package(slot) = unit else {
        return Ok(None);
    };
    let Some(files) = program.package_files(*slot) else {
        return Ok(None);
    };
    program_local_type_addr(files, unit, symbol)
}

pub(crate) fn is_actor_declaration_symbol(
    program: ProgramTypeView<'_>,
    symbol: &ServiceSymbolRef,
) -> bool {
    program
        .service_files
        .iter()
        .chain(program.packages.iter().flat_map(|package| package.files()))
        .flat_map(|file| file.actor_declarations.iter())
        .any(|declaration| declaration.actor_type == *symbol)
}

pub(crate) fn program_local_type_addr(
    files: &[Arc<LinkedFileUnit>],
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<Option<TypeAddr>> {
    let mut resolved = None;
    for (file_index, file) in files.iter().enumerate() {
        if file.module_path != symbol.module_path {
            continue;
        }
        let file_addr = FileAddr::LoadedFileIndex(file_index);
        if let Some(declaration) = file.declarations.types.get(&symbol.symbol) {
            merge_type_addr(
                &mut resolved,
                TypeAddr {
                    unit: unit.clone(),
                    file: file_addr.clone(),
                    type_index: declaration.type_index,
                },
                unit,
                symbol,
            )?;
        }
        if let Some(type_index) = file.link_targets.types.get(&symbol.symbol) {
            merge_type_addr(
                &mut resolved,
                TypeAddr {
                    unit: unit.clone(),
                    file: file_addr.clone(),
                    type_index: *type_index,
                },
                unit,
                symbol,
            )?;
        }
    }
    Ok(resolved)
}

pub(crate) fn merge_type_addr(
    resolved: &mut Option<TypeAddr>,
    candidate: TypeAddr,
    unit: &UnitAddr,
    symbol: &ServiceSymbolRef,
) -> Result<()> {
    match resolved {
        Some(existing) if *existing != candidate => Err(RuntimeError::InvalidArtifact(format!(
            "ambiguous type symbol {}.{} in {unit}: {existing} and {candidate}",
            symbol.module_path, symbol.symbol
        ))),
        Some(_) => Ok(()),
        None => {
            *resolved = Some(candidate);
            Ok(())
        }
    }
}
