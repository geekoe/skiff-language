use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serde_json::{json, Value};
use skiff_runtime_loader::ArtifactGraph;

use crate::json_utils::value_sha256;

mod file_conversion;
mod file_linker;
mod image;
mod input;
mod interface_tables;
mod link_diagnostics;
mod link_lookup;
mod metadata;
mod operation_validation;
mod package_exports;
mod routes;
mod type_context;

pub use file_conversion::linked_file_unit_from_artifact;
use file_linker::RuntimeFileLinker;
pub use image::LinkedProgramImageBuild;
pub use input::LinkerInput;
use metadata::dynamic_build_id;
use package_exports::{
    package_const_export_addr, package_executable_export_addr, package_slot_dependency_ref_overlay,
    package_slot_id_overlay, package_type_export_addr, validate_package_abi_expectations,
    validate_package_dependencies, validate_package_exports, PackageExportWalker,
};
pub use routes::package_handler_target;
use routes::{RouteIndex, RouteIndexBuilder};
pub use skiff_runtime_linked_program::{LinkOverlay, ResolvedSymbol, SymbolOverlay};
use type_context::runtime_type_context;

use crate::program::{
    addr::{ExecutableAddr, FileAddr, PackageSlot, UnitAddr},
    file_unit::{FileIrIdentity, FileIrRef, FileIrUnit},
    linked::LinkedFileUnit,
    package_unit::PackageUnit,
    service_unit::ServiceUnit,
    types::{PackageSymbolKey, ServiceSymbolKey},
    LinkedProgramImage, RuntimeProgramIdentity, RuntimeTypeContext,
};
use crate::{linker_activation_facts, LinkedImageActivationFacts};

use super::resolver::{ProgramError, ProgramResult};

type ServiceLinkTargetOverlay = HashMap<ServiceSymbolKey, ExecutableAddr>;
const LINKED_PROGRAM_IMAGE_IDENTITY_PREFIX: &str = "skiff-linked-program-image-v1";

pub fn link_runtime_program_image(graph: ArtifactGraph) -> ProgramResult<LinkedProgramImageBuild> {
    build_linked_program_image(LinkerInput::from(graph))
}

#[cfg(any(test, feature = "test-support"))]
pub fn link_runtime_program_image_from_parts(
    service: Arc<ServiceUnit>,
    service_files: Vec<Arc<FileIrUnit>>,
    packages: Vec<Arc<PackageUnit>>,
    package_files: Vec<Vec<Arc<FileIrUnit>>>,
) -> ProgramResult<LinkedProgramImageBuild> {
    build_linked_program_image(LinkerInput::from_legacy_parts(
        service,
        service_files,
        packages,
        package_files,
    ))
}

fn build_linked_program_image(input: LinkerInput) -> ProgramResult<LinkedProgramImageBuild> {
    let plan = LinkPlanBuilder::new(input)
        .canonicalize_input_files()?
        .build_package_slot_indexes()?
        .validate_service_and_package_metadata()?
        .build_route_index()?
        .build_symbol_overlay()?
        .build_type_context()?;
    let linked_files = plan.link_files()?;
    plan.into_image_build(linked_files)
}

struct LinkPlanBuilder {
    service: Arc<ServiceUnit>,
    service_files: Vec<Arc<FileIrUnit>>,
    packages: Vec<Arc<PackageUnit>>,
    package_files: Vec<Vec<Arc<FileIrUnit>>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
}

impl LinkPlanBuilder {
    fn new(input: LinkerInput) -> Self {
        Self {
            service: input.service,
            service_files: input.service_files,
            packages: input.packages,
            package_files: input.package_files,
            service_resources: input.service_resources,
            package_resources: input.package_resources,
        }
    }

    fn canonicalize_input_files(self) -> ProgramResult<CanonicalizedLinkPlanBuilder> {
        let Self {
            service,
            service_files,
            packages,
            package_files,
            service_resources,
            package_resources,
        } = self;

        if packages.len() != package_files.len() {
            return Err(ProgramError::PackageFileSlotMismatch {
                package_count: packages.len(),
                package_file_slot_count: package_files.len(),
            });
        }
        let package_resources = if package_resources.is_empty() {
            vec![crate::program::PublicationResourceTable::default(); packages.len()]
        } else if packages.len() != package_resources.len() {
            return Err(ProgramError::PackageResourceSlotMismatch {
                package_count: packages.len(),
                package_resource_slot_count: package_resources.len(),
            });
        } else {
            package_resources
        };

        let service_files =
            linkable_files_for_unit(UnitAddr::Service, &service.files, service_files)?;
        let service_files_by_identity = file_identity_overlay(&service_files);

        let mut canonical_package_files = Vec::with_capacity(package_files.len());
        let mut package_files_by_identity = HashMap::new();
        for (slot, (package, files)) in packages.iter().zip(package_files).enumerate() {
            let files = linkable_files_for_unit(UnitAddr::Package(slot), &package.files, files)?;
            let files_by_identity = file_identity_overlay(&files);
            validate_package_exports(slot, package, &files)?;
            package_files_by_identity.insert(slot, files_by_identity);
            canonical_package_files.push(files);
        }

        Ok(CanonicalizedLinkPlanBuilder {
            service,
            packages,
            service_resources,
            package_resources,
            canonical_inputs: CanonicalLinkInputs {
                files: RuntimeProgramFiles {
                    service_files,
                    package_files: canonical_package_files,
                },
                service_files_by_identity,
                package_files_by_identity,
            },
        })
    }
}

struct CanonicalizedLinkPlanBuilder {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    canonical_inputs: CanonicalLinkInputs,
}

impl CanonicalizedLinkPlanBuilder {
    fn build_package_slot_indexes(self) -> ProgramResult<IndexedLinkPlanBuilder> {
        let package_slots_by_id = package_slot_id_overlay(&self.packages)?;
        let package_slots_by_dependency_ref = package_slot_dependency_ref_overlay(
            &self.service,
            &self.packages,
            &package_slots_by_id,
        )?;

        Ok(IndexedLinkPlanBuilder {
            service: self.service,
            packages: self.packages,
            service_resources: self.service_resources,
            package_resources: self.package_resources,
            canonical_inputs: self.canonical_inputs,
            package_slot_indexes: PackageSlotIndexes {
                by_id: package_slots_by_id,
                by_dependency_ref: package_slots_by_dependency_ref,
            },
        })
    }
}

struct IndexedLinkPlanBuilder {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    canonical_inputs: CanonicalLinkInputs,
    package_slot_indexes: PackageSlotIndexes,
}

impl IndexedLinkPlanBuilder {
    fn validate_service_and_package_metadata(self) -> ProgramResult<ValidatedLinkPlanBuilder> {
        validate_package_dependencies(
            &self.service,
            &self.packages,
            &self.package_slot_indexes.by_id,
        )?;
        validate_package_abi_expectations(
            &self.service,
            &self.packages,
            &self.package_slot_indexes.by_id,
        )?;
        let activation_facts = linker_activation_facts(
            &self.service,
            &self.packages,
            &self.package_slot_indexes.by_id,
        )?;

        Ok(ValidatedLinkPlanBuilder {
            service: self.service,
            packages: self.packages,
            service_resources: self.service_resources,
            package_resources: self.package_resources,
            canonical_inputs: self.canonical_inputs,
            package_slot_indexes: self.package_slot_indexes,
            activation_facts,
        })
    }
}

struct ValidatedLinkPlanBuilder {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    canonical_inputs: CanonicalLinkInputs,
    package_slot_indexes: PackageSlotIndexes,
    activation_facts: LinkedImageActivationFacts,
}

impl ValidatedLinkPlanBuilder {
    fn build_route_index(self) -> ProgramResult<RoutedLinkPlanBuilder> {
        let service_link_targets =
            service_link_target_overlay(&self.canonical_inputs.files.service_files)?;
        let route_index = RouteIndexBuilder::build(
            &self.service.operations,
            &self.service.spawn_targets,
            &service_link_targets,
            &self.canonical_inputs.files.service_files,
            &self.packages,
            &self.canonical_inputs.files.package_files,
        )?;

        Ok(RoutedLinkPlanBuilder {
            service: self.service,
            packages: self.packages,
            service_resources: self.service_resources,
            package_resources: self.package_resources,
            canonical_inputs: self.canonical_inputs,
            package_slot_indexes: self.package_slot_indexes,
            activation_facts: self.activation_facts,
            service_link_targets,
            route_index,
        })
    }
}

struct RoutedLinkPlanBuilder {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    canonical_inputs: CanonicalLinkInputs,
    package_slot_indexes: PackageSlotIndexes,
    activation_facts: LinkedImageActivationFacts,
    service_link_targets: ServiceLinkTargetOverlay,
    route_index: RouteIndex,
}

impl RoutedLinkPlanBuilder {
    fn build_symbol_overlay(self) -> ProgramResult<OverlayLinkPlanBuilder> {
        let Self {
            service,
            packages,
            service_resources,
            package_resources,
            canonical_inputs,
            package_slot_indexes,
            activation_facts,
            service_link_targets,
            route_index,
        } = self;

        let mut symbols =
            service_symbol_overlay(&service_link_targets, &canonical_inputs.files.service_files)?;
        extend_package_symbol_overlay(
            &mut symbols,
            &packages,
            &canonical_inputs.files.package_files,
        )?;

        let PackageSlotIndexes {
            by_id,
            by_dependency_ref,
        } = package_slot_indexes;
        let CanonicalLinkInputs {
            files,
            service_files_by_identity,
            package_files_by_identity,
        } = canonical_inputs;
        let link_overlay = LinkOverlay {
            symbols,
            package_slots_by_id: by_id,
            package_slots_by_dependency_ref: by_dependency_ref,
            service_files_by_identity,
            package_files_by_identity,
        };

        Ok(OverlayLinkPlanBuilder {
            service,
            packages,
            service_resources,
            package_resources,
            files,
            activation_facts,
            route_index,
            link_overlay,
        })
    }
}

struct OverlayLinkPlanBuilder {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    files: RuntimeProgramFiles,
    activation_facts: LinkedImageActivationFacts,
    route_index: RouteIndex,
    link_overlay: LinkOverlay,
}

impl OverlayLinkPlanBuilder {
    fn build_type_context(self) -> ProgramResult<LinkPlan> {
        let original_types = runtime_type_context(
            &self.files.service_files,
            &self.packages,
            &self.files.package_files,
        )?;
        let build_id = dynamic_build_id(&self.service, &self.packages)?;

        Ok(LinkPlan {
            service: self.service,
            packages: self.packages,
            service_resources: self.service_resources,
            package_resources: self.package_resources,
            files: self.files,
            activation_facts: self.activation_facts,
            route_index: self.route_index,
            build_id,
            link_overlay: self.link_overlay,
            original_types,
        })
    }
}

struct CanonicalLinkInputs {
    files: RuntimeProgramFiles,
    service_files_by_identity: HashMap<FileIrIdentity, FileAddr>,
    package_files_by_identity: HashMap<PackageSlot, HashMap<FileIrIdentity, FileAddr>>,
}

struct PackageSlotIndexes {
    by_id: HashMap<String, PackageSlot>,
    by_dependency_ref: HashMap<String, PackageSlot>,
}

struct RuntimeProgramFiles {
    service_files: Vec<Arc<LinkedFileUnit>>,
    package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
}

struct LinkPlan {
    service: Arc<ServiceUnit>,
    packages: Vec<Arc<PackageUnit>>,
    service_resources: crate::program::PublicationResourceTable,
    package_resources: Vec<crate::program::PublicationResourceTable>,
    files: RuntimeProgramFiles,
    activation_facts: LinkedImageActivationFacts,
    route_index: RouteIndex,
    build_id: String,
    link_overlay: LinkOverlay,
    original_types: RuntimeTypeContext,
}

impl LinkPlan {
    fn link_files(&self) -> ProgramResult<RuntimeProgramFiles> {
        let linker = RuntimeFileLinker::new(
            &self.service,
            &self.link_overlay,
            &self.original_types,
            &self.packages,
            &self.files.service_files,
            &self.files.package_files,
        );
        let service_files = linker.link_files(UnitAddr::Service, &self.files.service_files)?;
        let package_files = self
            .files
            .package_files
            .iter()
            .enumerate()
            .map(|(slot, files)| linker.link_files(UnitAddr::Package(slot), files))
            .collect::<ProgramResult<Vec<_>>>()?;

        Ok(RuntimeProgramFiles {
            service_files,
            package_files,
        })
    }

    fn into_image_build(
        self,
        linked_files: RuntimeProgramFiles,
    ) -> ProgramResult<LinkedProgramImageBuild> {
        let types = runtime_type_context(
            &linked_files.service_files,
            &self.packages,
            &linked_files.package_files,
        )?;

        let image = LinkedProgramImage {
            service_files: linked_files.service_files,
            packages: self.packages.clone(),
            package_files: linked_files.package_files,
            service_resources: self.service_resources,
            package_resources: self.package_resources,
            routes: self.route_index.routes,
            spawn_routes: self.route_index.spawn_routes,
            operations: self.route_index.operations,
            operation_receivers: self.route_index.operation_receivers,
            link_overlay: self.link_overlay,
            types,
        };
        let identity =
            RuntimeProgramIdentity::new(self.build_id, linked_program_image_identity(&image)?);
        Ok(LinkedProgramImageBuild::new(
            identity,
            image,
            self.activation_facts,
        ))
    }
}

fn linked_program_image_identity(image: &LinkedProgramImage) -> ProgramResult<String> {
    let service_files = image
        .service_files
        .iter()
        .map(|file| linked_file_identity_value(file.as_ref()))
        .collect::<ProgramResult<Vec<_>>>()?;
    let packages = image
        .packages
        .iter()
        .map(|package| {
            serde_json::to_value(package.as_ref()).map_err(|error| {
                ProgramError::LinkedProgramImageIdentityFailed {
                    message: format!("package unit serialization failed: {error}"),
                }
            })
        })
        .collect::<ProgramResult<Vec<_>>>()?;
    let package_files = image
        .package_files
        .iter()
        .map(|files| {
            files
                .iter()
                .map(|file| linked_file_identity_value(file.as_ref()))
                .collect::<ProgramResult<Vec<_>>>()
        })
        .collect::<ProgramResult<Vec<_>>>()?;
    let value = json!({
        "format": LINKED_PROGRAM_IMAGE_IDENTITY_PREFIX,
        "serviceFiles": service_files,
        "packages": packages,
        "packageFiles": package_files,
        "routes": image.routes,
        "operations": image.operations,
        "operationReceivers": image.operation_receivers,
    });
    let hash =
        value_sha256(&value).map_err(|error| ProgramError::LinkedProgramImageIdentityFailed {
            message: error.to_string(),
        })?;
    Ok(format!(
        "{LINKED_PROGRAM_IMAGE_IDENTITY_PREFIX}:sha256:{hash}"
    ))
}

fn linked_file_identity_value(file: &LinkedFileUnit) -> ProgramResult<Value> {
    serde_json::to_value(file).map_err(|error| ProgramError::LinkedProgramImageIdentityFailed {
        message: format!("linked file serialization failed: {error}"),
    })
}

fn canonical_files_for_unit(
    unit: UnitAddr,
    refs: &[FileIrRef],
    files: Vec<Arc<FileIrUnit>>,
) -> ProgramResult<Vec<Arc<FileIrUnit>>> {
    let mut files_by_identity = HashMap::new();
    for file in files {
        let identity = file.file_ir_identity.clone();
        if files_by_identity.insert(identity.clone(), file).is_some() {
            return Err(ProgramError::LoadedFileIdentityDuplicate {
                unit: unit.clone(),
                identity,
            });
        }
    }

    let mut declared_identities = HashSet::new();
    let mut canonical_files = Vec::with_capacity(refs.len());
    for file_ref in refs {
        let file = files_by_identity
            .get(&file_ref.file_ir_identity)
            .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                unit: unit.clone(),
                identity: file_ref.file_ir_identity.clone(),
            })?;

        if file_ref.module_path != file.module_path {
            return Err(ProgramError::FileRefModulePathMismatch {
                unit: unit.clone(),
                identity: file_ref.file_ir_identity.clone(),
                expected: file_ref.module_path.clone(),
                actual: file.module_path.clone(),
            });
        }

        declared_identities.insert(file_ref.file_ir_identity.clone());
        canonical_files.push(Arc::clone(file));
    }

    for identity in files_by_identity.keys() {
        if !declared_identities.contains(identity) {
            return Err(ProgramError::LoadedFileIdentityNotDeclared {
                unit: unit.clone(),
                identity: identity.clone(),
            });
        }
    }

    Ok(canonical_files)
}

fn linkable_files_for_unit(
    unit: UnitAddr,
    refs: &[FileIrRef],
    files: Vec<Arc<FileIrUnit>>,
) -> ProgramResult<Vec<Arc<LinkedFileUnit>>> {
    canonical_files_for_unit(unit, refs, files)?
        .iter()
        .map(|file| {
            linked_file_unit_from_artifact(file.as_ref())
                .map(Arc::new)
                .map_err(|error| ProgramError::LinkedFileConversionFailed {
                    identity: file.file_ir_identity.clone(),
                    error: error.to_string(),
                })
        })
        .collect()
}

fn file_identity_overlay(files: &[Arc<LinkedFileUnit>]) -> HashMap<FileIrIdentity, FileAddr> {
    let mut files_by_identity = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        files_by_identity
            .entry(file.file_ir_identity.clone())
            .or_insert_with(|| FileAddr::LoadedFileIndex(index));
    }
    files_by_identity
}

fn service_link_target_overlay(
    service_files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<ServiceLinkTargetOverlay> {
    let mut link_targets = HashMap::new();
    for (file_index, file) in service_files.iter().enumerate() {
        let addr_for_link_target = |executable| ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(file_index),
            executable,
        };

        for (symbol, executable) in file.link_targets.executable_link_targets() {
            let addr = addr_for_link_target(*executable);
            if *executable >= file.executables.len() {
                return Err(ProgramError::ExecutableIndexOutOfBounds {
                    unit: UnitAddr::Service,
                    file: addr.file,
                    index: *executable,
                    executable_count: file.executables.len(),
                });
            }
            let key = ServiceSymbolKey::new(file.module_path.clone(), symbol.clone());
            if let Some(first_addr) = link_targets.insert(key.clone(), addr.clone()) {
                return Err(ProgramError::ServiceLinkTargetDuplicate {
                    module_path: key.module_path,
                    symbol: key.symbol,
                    first_addr,
                    duplicate_addr: addr,
                });
            }
        }
    }
    Ok(link_targets)
}

fn service_symbol_overlay(
    link_targets: &ServiceLinkTargetOverlay,
    service_files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<SymbolOverlay> {
    let mut symbols = SymbolOverlay::default();
    for (key, addr) in link_targets {
        symbols.insert_service(
            key.clone(),
            ResolvedSymbol::Executable { addr: addr.clone() },
        );
    }
    for (file_index, file) in service_files.iter().enumerate() {
        let mut constant_targets = file.link_targets.constants.clone();
        for (symbol, declaration) in &file.declarations.constants {
            constant_targets
                .entry(symbol.clone())
                .or_insert(declaration.const_index);
        }
        for (symbol, const_index) in constant_targets {
            let key = ServiceSymbolKey::new(file.module_path.clone(), symbol.clone());
            let resolved = ResolvedSymbol::Constant {
                unit: UnitAddr::Service,
                file: FileAddr::LoadedFileIndex(file_index),
                const_index,
            };
            if let Some(previous) = symbols.insert_service(key.clone(), resolved) {
                return Err(ProgramError::LinkSymbolKindMismatch {
                    context: "service symbol overlay".to_string(),
                    symbol: key.to_string(),
                    expected_kind: "unique service symbol",
                    actual_kind: previous.export_kind(),
                });
            }
        }
    }
    Ok(symbols)
}

fn extend_package_symbol_overlay(
    symbols: &mut SymbolOverlay,
    packages: &[Arc<PackageUnit>],
    package_files: &[Vec<Arc<LinkedFileUnit>>],
) -> ProgramResult<()> {
    let mut package_symbol_kinds = HashMap::new();
    for exports in PackageExportWalker::all(packages, package_files)? {
        for item in exports.executable_exports() {
            let addr = package_executable_export_addr(exports.slot, item.export, exports.files)?;
            insert_package_symbol_overlay(
                symbols,
                &mut package_symbol_kinds,
                exports.slot,
                item.symbol,
                item.kind,
                ResolvedSymbol::Executable { addr },
            )?;
        }
        for item in exports.const_exports() {
            let (file, const_index) =
                package_const_export_addr(exports.slot, item.export, exports.files)?;
            insert_package_symbol_overlay(
                symbols,
                &mut package_symbol_kinds,
                exports.slot,
                item.symbol,
                item.kind,
                ResolvedSymbol::Constant {
                    unit: UnitAddr::Package(exports.slot),
                    file,
                    const_index,
                },
            )?;
        }
        for item in exports.type_exports() {
            let addr = package_type_export_addr(exports.slot, item.export, exports.files)?;
            insert_package_symbol_overlay(
                symbols,
                &mut package_symbol_kinds,
                exports.slot,
                item.symbol,
                item.kind,
                ResolvedSymbol::Type { addr: addr.clone() },
            )?;
        }
    }
    Ok(())
}

fn insert_package_symbol_overlay(
    symbols: &mut SymbolOverlay,
    package_symbol_kinds: &mut HashMap<PackageSymbolKey, &'static str>,
    package_slot: PackageSlot,
    symbol: &str,
    kind: &'static str,
    resolved: ResolvedSymbol,
) -> ProgramResult<()> {
    let key = PackageSymbolKey::new(package_slot, symbol);
    if let Some(existing) = symbols.get_package(package_slot, symbol) {
        return Err(ProgramError::PackageExportDuplicateSymbol {
            package_slot,
            symbol: symbol.to_string(),
            first_kind: package_symbol_kinds
                .get(&key)
                .copied()
                .unwrap_or_else(|| existing.export_kind()),
            duplicate_kind: kind,
        });
    }
    package_symbol_kinds.insert(key.clone(), kind);
    symbols.insert_package(key, resolved);
    Ok(())
}

fn validate_executable_index(
    unit: UnitAddr,
    file: FileAddr,
    index: usize,
    file_unit: &LinkedFileUnit,
) -> ProgramResult<()> {
    if index >= file_unit.executables.len() {
        return Err(ProgramError::ExecutableIndexOutOfBounds {
            unit,
            file,
            index,
            executable_count: file_unit.executables.len(),
        });
    }
    Ok(())
}
