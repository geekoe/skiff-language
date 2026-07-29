use skiff_artifact_model::{PackageRefIr, TypeRefIr};
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, FileAddr, InterfaceDeclIr, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, UnitAddr,
};

use super::code_linker::AssemblyCodeLinker;
use crate::{
    linker::call_semantic_validation::{
        local_interface_declaration_abi_ids, package_interface_declaration_id,
        CallSemanticValidationDelegate,
    },
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
    if let Ok(interface_type) = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id) {
        match interface_type {
            TypeRefIr::LocalType { type_index } => {
                return linked_interface_declaration_at(
                    linker,
                    context,
                    source_code_slot,
                    source_file_index,
                    type_index as usize,
                    "local interface declaration for any interface dispatch",
                );
            }
            TypeRefIr::PackageSymbol { symbol } => {
                if symbol.abi_expectation.is_none() {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: interface.interface_abi_id.clone(),
                        expected_kind:
                            "package interface declaration with exact local ABI expectation",
                    });
                }
                let addr = linker
                    .addresses
                    .package_symbol_type_addr(source_code_slot, &symbol)
                    .map_err(|error| {
                        assembly_semantic_error(
                            context,
                            error,
                            "exact package interface declaration owner",
                        )
                    })?;
                let UnitAddr::Package(code_slot) = addr.unit else {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: interface.interface_abi_id.clone(),
                        expected_kind: "package-owned interface declaration",
                    });
                };
                let FileAddr::LoadedFileIndex(file_index) = addr.file else {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: interface.interface_abi_id.clone(),
                        expected_kind: "loaded package interface declaration file",
                    });
                };
                return linked_interface_declaration_at(
                    linker,
                    context,
                    code_slot,
                    file_index,
                    addr.type_index,
                    "package interface declaration for any interface dispatch",
                );
            }
            _ => {}
        }
    }

    let mut matched = None;
    for code_slot in 0..linker.addresses.shared_image().code_slots().len() {
        let files = linker
            .addresses
            .package_files(code_slot)
            .map_err(|error| assembly_semantic_error(context, error, "package code files"))?;
        for (file_index, file) in files.iter().enumerate() {
            for (name, declaration) in &file.declarations.interfaces {
                let abi_ids =
                    interface_declaration_abi_ids(linker, context, code_slot, file, name)?;
                if !abi_ids
                    .iter()
                    .any(|abi_id| abi_id == &interface.interface_abi_id)
                {
                    continue;
                }
                if matched.is_some() {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: interface.interface_abi_id.clone(),
                        expected_kind: "unique interface declaration for any interface dispatch",
                    });
                }
                let mut linked = declaration.clone();
                link_interface_declaration_types(linker, code_slot, file_index, &mut linked)
                    .map_err(|error| {
                        assembly_semantic_error(context, error, "linked interface declaration")
                    })?;
                matched = Some(linked);
            }
        }
    }
    matched.ok_or_else(|| ProgramError::LinkSymbolUnresolved {
        context: context.to_string(),
        symbol: interface.interface_abi_id.clone(),
        expected_kind: "interface declaration for any interface dispatch",
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
    let mut linked = declaration.clone();
    link_interface_declaration_types(linker, code_slot, file_index, &mut linked)
        .map_err(|error| assembly_semantic_error(context, error, "linked interface declaration"))?;
    Ok(linked)
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

fn interface_declaration_abi_ids(
    linker: &AssemblyCodeLinker<'_>,
    context: &str,
    code_slot: usize,
    file: &LinkedFileUnit,
    declaration_name: &str,
) -> ProgramResult<Vec<String>> {
    let mut abi_ids = local_interface_declaration_abi_ids(context, file, declaration_name)?;
    let Some(type_declaration) = file.declarations.types.get(declaration_name) else {
        return Ok(abi_ids);
    };
    let code = linker
        .addresses
        .shared_image()
        .code_slots()
        .get(code_slot)
        .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: code_slot.to_string(),
            expected_kind: "package code slot for interface declaration",
        })?;
    for (export_symbol, export) in &code.artifact().implementation_links.types {
        if export.file.file_ir_identity != file.file_ir_identity
            || export.type_index as usize != type_declaration.type_index
        {
            continue;
        }
        push_unique(
            &mut abi_ids,
            package_interface_declaration_id(
                context,
                PackageRefIr::PackageId {
                    package_id: code.artifact().package_id.clone(),
                },
                export_symbol,
            )?,
        );
        if !export.symbol.is_empty() && export.symbol != *export_symbol {
            push_unique(
                &mut abi_ids,
                package_interface_declaration_id(
                    context,
                    PackageRefIr::PackageId {
                        package_id: code.artifact().package_id.clone(),
                    },
                    &export.symbol,
                )?,
            );
        }
        for binding in &linker
            .addresses
            .shared_image()
            .package_link_plan()
            .package_links
        {
            if binding.package.package_build_id != *code.package_build_id() {
                continue;
            }
            push_unique(
                &mut abi_ids,
                package_interface_declaration_id(
                    context,
                    PackageRefIr::Dependency {
                        dependency_ref: binding.key.package_requirement_alias.clone(),
                    },
                    export_symbol,
                )?,
            );
        }
    }
    Ok(abi_ids)
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

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.contains(&value) {
        items.push(value);
    }
}
