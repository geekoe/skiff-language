use skiff_artifact_model::{PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, TypeRefIr};
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, FileAddr, InterfaceDeclIr, LinkedInterfaceInstantiationRef,
    TypeAddr, UnitAddr,
};

use super::code_linker::AssemblyCodeLinker;
use crate::{
    linker::call_semantic_validation::CallSemanticValidationDelegate,
    resolver::{ProgramError, ProgramResult},
};

/// Assembly-specific lookup delegate for the linker-owned shared call validator.
pub(super) struct AssemblyCallSemanticDelegate<'linker, 'image> {
    linker: &'linker AssemblyCodeLinker<'image>,
    code_slot: usize,
    file_index: usize,
}

impl<'linker, 'image> AssemblyCallSemanticDelegate<'linker, 'image> {
    pub(super) fn new(
        linker: &'linker AssemblyCodeLinker<'image>,
        code_slot: usize,
        file_index: usize,
    ) -> Self {
        Self {
            linker,
            code_slot,
            file_index,
        }
    }
}

impl CallSemanticValidationDelegate for AssemblyCallSemanticDelegate<'_, '_> {
    fn validate_const_target(&self, context: &str, addr: &ConstAddr) -> ProgramResult<()> {
        self.linker
            .addresses
            .validate_const_addr(addr)
            .map_err(|error| assembly_semantic_error(context, error, "valid const receiver target"))
    }

    fn validate_executable_target(
        &self,
        context: &str,
        addr: &ExecutableAddr,
    ) -> ProgramResult<()> {
        self.linker
            .addresses
            .validate_executable_addr(addr)
            .map_err(|error| {
                assembly_semantic_error(context, error, "valid receiver executable target")
            })
    }

    fn link_interface_declaration(
        &self,
        context: &str,
        interface: &mut LinkedInterfaceInstantiationRef,
    ) -> ProgramResult<InterfaceDeclIr> {
        self.linker
            .link_interface(self.code_slot, self.file_index, interface)
            .map_err(|error| {
                assembly_semantic_error(context, error, "linked interface method target")
            })?;
        linked_interface_declaration(
            self.linker,
            context,
            self.code_slot,
            self.file_index,
            interface,
        )
    }
}

fn linked_interface_declaration(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    source_code_slot: usize,
    source_file_index: usize,
    interface: &LinkedInterfaceInstantiationRef,
) -> ProgramResult<InterfaceDeclIr> {
    let owner = exact_interface_owner(
        linker,
        context,
        source_code_slot,
        source_file_index,
        &interface.interface_abi_id,
    )?;
    linked_interface_declaration_at(
        linker,
        context,
        owner.code_slot,
        owner.file_index,
        owner.type_index,
        "interface declaration at exact owner coordinate",
    )
}

pub(super) struct ExactInterfaceOwner {
    code_slot: usize,
    file_index: usize,
    type_index: usize,
    canonical_abi_id: Option<String>,
}

impl ExactInterfaceOwner {
    pub(super) fn canonical_abi_id(&self) -> Option<&str> {
        self.canonical_abi_id.as_deref()
    }
}

pub(super) fn exact_interface_owner(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    source_code_slot: usize,
    source_file_index: usize,
    interface_abi_id: &str,
) -> ProgramResult<ExactInterfaceOwner> {
    let interface_type = serde_json::from_str::<TypeRefIr>(interface_abi_id).map_err(|error| {
        assembly_semantic_error(
            context,
            anyhow::Error::new(error),
            "canonical typed interface owner",
        )
    })?;
    let addr = match interface_type {
        TypeRefIr::LocalType { type_index } => {
            linker
                .addresses
                .type_addr(source_code_slot, source_file_index, type_index as usize)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => linker.addresses.publication_type_addr(
            source_code_slot,
            &module_path,
            type_index as usize,
        ),
        TypeRefIr::ServiceSymbol { symbol } => linker
            .addresses
            .local_symbol_type_addr(source_code_slot, &symbol),
        TypeRefIr::PackageSymbol { symbol } => {
            if symbol.abi_expectation.is_none() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: interface_abi_id.to_string(),
                    expected_kind: "package interface declaration with exact local ABI expectation",
                });
            }
            linker
                .addresses
                .package_symbol_type_addr(source_code_slot, &symbol)
        }
        _ => {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_abi_id.to_string(),
                expected_kind: "local, publication, service, or exact package interface owner",
            });
        }
    }
    .map_err(|error| {
        assembly_semantic_error(context, error, "exact typed interface declaration owner")
    })?;
    exact_interface_owner_at(linker, context, interface_abi_id, &addr)
}

fn exact_interface_owner_at(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    interface_abi_id: &str,
    addr: &TypeAddr,
) -> ProgramResult<ExactInterfaceOwner> {
    let UnitAddr::Package(code_slot) = addr.unit else {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_abi_id.to_string(),
            expected_kind: "package-owned interface declaration",
        });
    };
    let FileAddr::LoadedFileIndex(file_index) = addr.file else {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_abi_id.to_string(),
            expected_kind: "loaded package interface declaration file",
        });
    };
    interface_declaration_at(
        linker,
        context,
        code_slot,
        file_index,
        addr.type_index,
        "unique interface declaration at exact owner coordinate",
    )?;
    let canonical_abi_id = canonical_package_interface_abi_id(
        linker,
        context,
        code_slot,
        file_index,
        addr.type_index,
    )?;
    Ok(ExactInterfaceOwner {
        code_slot,
        file_index,
        type_index: addr.type_index,
        canonical_abi_id,
    })
}

fn linked_interface_declaration_at(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    code_slot: usize,
    file_index: usize,
    type_index: usize,
    expected_kind: &'static str,
) -> ProgramResult<InterfaceDeclIr> {
    let mut linked = interface_declaration_at(
        linker,
        context,
        code_slot,
        file_index,
        type_index,
        expected_kind,
    )?
    .clone();
    link_interface_declaration_types(linker, code_slot, file_index, &mut linked)
        .map_err(|error| assembly_semantic_error(context, error, "linked interface declaration"))?;
    Ok(linked)
}

fn interface_declaration_at<'a>(
    linker: &'a AssemblyCodeLinker<'_>,
    context: &str,
    code_slot: usize,
    file_index: usize,
    type_index: usize,
    expected_kind: &'static str,
) -> ProgramResult<&'a InterfaceDeclIr> {
    let files = linker
        .addresses
        .package_files(code_slot)
        .map_err(|error| assembly_semantic_error(context, error, "package code files"))?;
    let file = files
        .get(file_index)
        .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: file_index.to_string(),
            expected_kind: "interface declaration source file",
        })?;
    let mut matches = file
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index)
        .filter_map(|(name, _)| file.declarations.interfaces.get(name));
    let declaration = matches
        .next()
        .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: type_index.to_string(),
            expected_kind,
        })?;
    if matches.next().is_some() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: type_index.to_string(),
            expected_kind: "unique interface declaration at exact owner coordinate",
        });
    }
    if !matches!(
        file.types.get(type_index).map(|ty| &ty.descriptor),
        Some(skiff_runtime_linked_program::LinkedTypeDescriptor::Interface)
    ) {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: type_index.to_string(),
            expected_kind: "interface type descriptor at exact owner coordinate",
        });
    }
    Ok(declaration)
}

fn canonical_package_interface_abi_id(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    code_slot: usize,
    file_index: usize,
    type_index: usize,
) -> ProgramResult<Option<String>> {
    let code = linker
        .addresses
        .shared_image()
        .code_slots()
        .get(code_slot)
        .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: code_slot.to_string(),
            expected_kind: "package code for exact interface owner",
        })?;
    let file = linker
        .addresses
        .package_files(code_slot)
        .map_err(|error| assembly_semantic_error(context, error, "package code files"))?
        .get(file_index)
        .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: file_index.to_string(),
            expected_kind: "loaded package interface declaration file",
        })?;
    let mut owner_declarations = file
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index);
    let (owner_name, owner_declaration) =
        owner_declarations
            .next()
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: type_index.to_string(),
                expected_kind: "canonical implementation interface declaration",
            })?;
    if owner_declarations.next().is_some() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: type_index.to_string(),
            expected_kind: "unique canonical implementation interface declaration",
        });
    }
    let canonical_source_path = format!("{}.{}", file.module_path, owner_name);
    if owner_declaration.symbol != canonical_source_path {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: owner_declaration.symbol.clone(),
            expected_kind: "canonical implementation interface source path",
        });
    }

    let exact_owner_coordinate = |export: &skiff_artifact_model::TypeExport| {
        export.type_index as usize == type_index
            && export.file.file_ir_identity == file.file_ir_identity
            && export.file.module_path == file.module_path
            && export.file.source_ast_hash.as_deref() == Some(file.source_ast_hash.as_str())
    };
    let collect_exact_interface_exports =
        |symbols: &std::collections::BTreeMap<String, PackageLocalAbiSymbol>,
         surface: &'static str|
         -> ProgramResult<Vec<(String, skiff_artifact_model::TypeExport)>> {
            let mut exact = Vec::new();
            for (symbol_path, symbol) in symbols {
                let PackageLocalAbiSymbol::Type {
                    descriptor,
                    is_alias,
                    is_interface: true,
                    ..
                } = symbol
                else {
                    continue;
                };
                let export = code
                    .artifact()
                    .implementation_links
                    .types
                    .get(symbol_path)
                    .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: symbol_path.clone(),
                        expected_kind: match surface {
                            "implementation" => "implementation package interface export",
                            _ => "public package interface export",
                        },
                    })?;
                if !export.is_interface
                    || *is_alias && surface == "implementation"
                    || !matches!(
                        export.descriptor,
                        Some(skiff_artifact_model::TypeDescriptorIr::Interface)
                    )
                    || !matches!(
                        descriptor,
                        skiff_artifact_model::TypeDescriptorIr::Interface
                    )
                {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: symbol_path.clone(),
                        expected_kind: match surface {
                            "implementation" => "non-alias implementation package interface export",
                            _ => "public package interface export",
                        },
                    });
                }
                if exact_owner_coordinate(export) {
                    exact.push((symbol_path.clone(), export.clone()));
                }
            }
            Ok(exact)
        };

    let implementation_exports = collect_exact_interface_exports(
        &code.artifact().package_local_abi.implementation_symbols,
        "implementation",
    )?;
    let (implementation_path, implementation_export) = match implementation_exports.as_slice() {
        [only] => only,
        [] | [_, _, ..] => {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{}:{}:{}",
                    code.artifact().package_id,
                    file.file_ir_identity,
                    type_index
                ),
                expected_kind:
                    "unique implementation package interface export at exact owner coordinate",
            });
        }
    };
    let unqualified_symbol_has_exact_public_collision = implementation_export.symbol.as_str()
        == owner_name.as_str()
        && matches!(
            code.artifact()
                .package_local_abi
                .public_symbols
                .get(&canonical_source_path),
            Some(PackageLocalAbiSymbol::Type {
                local_type_id,
                descriptor: skiff_artifact_model::TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                ..
            }) if local_type_id == &format!("type:{canonical_source_path}")
        )
        && code
            .artifact()
            .implementation_links
            .types
            .get(&canonical_source_path)
            .is_some_and(|public_export| {
                public_export == implementation_export
                    && public_export.is_interface
                    && matches!(
                        public_export.descriptor,
                        Some(skiff_artifact_model::TypeDescriptorIr::Interface)
                    )
                    && exact_owner_coordinate(public_export)
            });
    let implementation_symbol_matches_owner = implementation_export.symbol.as_str()
        == canonical_source_path.as_str()
        || unqualified_symbol_has_exact_public_collision;
    if implementation_path != &canonical_source_path || !implementation_symbol_matches_owner {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: implementation_export.symbol.clone(),
            expected_kind: "canonical implementation interface source export",
        });
    }
    let Some(PackageLocalAbiSymbol::Type {
        local_type_id,
        is_alias: false,
        is_interface: true,
        ..
    }) = code
        .artifact()
        .package_local_abi
        .implementation_symbols
        .get(implementation_path)
    else {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: implementation_path.clone(),
            expected_kind: "canonical implementation interface symbol",
        });
    };
    if local_type_id
        != &format!(
            "type:{}:top-level:{}",
            code.artifact().package_id,
            canonical_source_path
        )
    {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: local_type_id.clone(),
            expected_kind: "canonical implementation interface Local ABI identity",
        });
    }

    let public_exports = collect_exact_interface_exports(
        &code.artifact().package_local_abi.public_symbols,
        "public",
    )?;
    let symbol_path = match public_exports.as_slice() {
        [(public_path, public_export)] => {
            if public_export.symbol.as_str() != owner_name.as_str() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: public_export.symbol.clone(),
                    expected_kind: "canonical public interface source export",
                });
            }
            let Some(PackageLocalAbiSymbol::Type { local_type_id, .. }) = code
                .artifact()
                .package_local_abi
                .public_symbols
                .get(public_path)
            else {
                unreachable!("the public export collector only returns interface type symbols")
            };
            if local_type_id != &format!("type:{public_path}") {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: local_type_id.clone(),
                    expected_kind: "canonical public interface Local ABI identity",
                });
            }
            public_path
        }
        [] => implementation_path,
        [_, _, ..] => {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{}:{}:{}",
                    code.artifact().package_id,
                    file.file_ir_identity,
                    type_index
                ),
                expected_kind: "unique public package interface export at exact owner coordinate",
            });
        }
    };
    let canonical_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: code.artifact().package_id.clone(),
            },
            symbol_path: symbol_path.clone(),
            abi_expectation: Some(code.local_abi_identity().as_str().to_string()),
        },
    };
    Ok(Some(skiff_artifact_identity::type_ref_abi_key(
        &canonical_type,
    )))
}

fn link_interface_declaration_types(
    linker: &AssemblyCodeLinker<'_>,
    code_slot: usize,
    file_index: usize,
    declaration: &mut InterfaceDeclIr,
) -> anyhow::Result<()> {
    for operation in &mut declaration.operations {
        for param in &mut operation.params {
            linker.link_type_ref(code_slot, file_index, &mut param.ty)?;
        }
        linker.link_type_ref(code_slot, file_index, &mut operation.return_type)?;
        if let Some(implicit_self) = &mut operation.implicit_self {
            linker.link_type_ref(code_slot, file_index, implicit_self)?;
        }
    }
    Ok(())
}

fn assembly_semantic_error(
    context: &str,
    error: anyhow::Error,
    expected_kind: &'static str,
) -> ProgramError {
    ProgramError::LinkSymbolUnresolved {
        context: context.to_string(),
        symbol: error.to_string(),
        expected_kind,
    }
}
