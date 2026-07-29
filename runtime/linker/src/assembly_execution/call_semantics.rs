use skiff_artifact_model::{PackageRefIr, PackageSymbolRef, TypeRefIr};
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
    let mut exports = code
        .artifact()
        .implementation_links
        .types
        .iter()
        .filter(|(_, export)| {
            export.type_index as usize == type_index
                && export.file.file_ir_identity == file.file_ir_identity
                && export.file.module_path == file.module_path
                && export
                    .file
                    .source_ast_hash
                    .as_deref()
                    .is_none_or(|hash| hash == file.source_ast_hash)
        });
    let Some((symbol_path, export)) = exports.next() else {
        // Private interfaces have no package export and therefore cannot cross
        // the package boundary. Their typed local/publication spelling remains
        // exact within the package instead of inventing a public ABI identity.
        return Ok(None);
    };
    if exports.next().is_some() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{}:{}:{}",
                code.artifact().package_id,
                file.file_ir_identity,
                type_index
            ),
            expected_kind: "unique package export for exact interface owner",
        });
    }
    if !export.is_interface {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: symbol_path.clone(),
            expected_kind: "package interface export at exact owner coordinate",
        });
    }
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
