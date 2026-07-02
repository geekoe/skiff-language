use std::collections::{BTreeMap as StdBTreeMap, BTreeSet};

use serde::Serialize;

use super::signature_matching::{
    executable_signature_params, signature_type_ref_matches, SignatureTypeRefContext,
};
use crate::{
    context::ProjectedPackageDependency,
    contract::{ContractProjection, ContractProjectionIndex},
    contract_schema::descriptor::RuntimeTypeDescriptorIr,
    error::ProjectionError,
    runtime::{
        service_operation_adapter_symbol, EntryOperationCallable, EntryOperationSpec, GatewayEntry,
        OperationEntryIr, TimeoutEntry,
    },
    typed_artifacts::{
        public_instance_method_operation_abi_id, GatewayConfig, GatewayRoute, GatewayWebSocket,
        InterfaceMethodSignature, OperationConstReceiverRef, OperationMode, OperationParam,
        OperationTargetRef, PackageAbiExpectation, PackageDependencyConstraint, PackageUsedSymbol,
        PackageUsedSymbolKind, PublicInstanceExport, PublicInstanceOperation,
        ServiceConfigMetadata, ServiceOperation,
    },
};
use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref,
    interface_instantiation_ref_for_type_ref, type_ref_abi_key, BlockIr, BoxSourceIr, CallIr,
    CallTargetIr, CanonicalPublicCallableSignature, DbBodyIr, DbChangeOpIr, DbOperationIr,
    DbPredicateIr, DbQueryIr, DbQueryValueIr, DbSelectorIr, ExecutableBody,
    ExecutableDeclarationIr, ExecutableIr, ExecutableKind, ExecutableLinkTargetIr, ExprIr,
    ExprRefIr, FileIrRef, FileIrUnit, FunctionTypeParamIr, InterfaceDeclIr,
    InterfaceInstantiationRef, InterfaceMethodTablePlanIr, LocalReceiverExecutableRef,
    MetadataValue, OperationAbiRef, OperationCallableKind, PackageRefIr, PackageSymbolRef,
    PackageUnit, ParamIr, PatternIr, PublicationOperationKind, ReceiverCallAbi,
    ServiceOperationTarget, ServiceReceiverOperationTarget, ServiceSymbolRef, SlotIr, SlotKind,
    SlotLayout, StmtIr, StmtRefIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::naming::impl_method_declaration_name;
use skiff_compiler_core::package_interface_methods::instantiate_interface_method_signatures;
use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;
use skiff_compiler_projection_input::{
    ExportPublicInstanceInterfaceProjection, ExportPublicInstanceProjection,
    PackageProjectionInput, ProjectionView,
};

fn type_ref_from_runtime_descriptor(descriptor: &RuntimeTypeDescriptorIr) -> TypeRefIr {
    descriptor.to_type_ref_for_service_unit()
}

pub fn service_package_dependency_constraints(
    declared_dependencies: &[ProjectedPackageDependency],
    package_publications: &[PackageProjectionInput],
    service_file_units: &[FileIrUnit],
) -> Vec<PackageDependencyConstraint> {
    let declared_by_id = declared_dependencies
        .iter()
        .map(|dependency| (dependency.id.as_str(), dependency))
        .collect::<StdBTreeMap<_, _>>();
    let mut constraints = package_publications_by_id(package_publications)
        .into_iter()
        .filter(|package| package.manifest().id() != PRELUDE_REGISTRY_ID)
        .filter_map(|package| {
            let declared = declared_by_id.get(package.manifest().id())?;
            let mut dependency = package_dependency_constraint(declared);
            dependency.config = crate::context::empty_dependency_config();
            Some(dependency)
        })
        .collect::<Vec<_>>();
    if file_ir_units_reference_std_package(service_file_units)
        && !constraints
            .iter()
            .any(|dependency| dependency.id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID)
    {
        constraints.push(std_package_dependency_constraint());
    }
    constraints
}

fn package_dependency_constraint(
    dependency: &ProjectedPackageDependency,
) -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias: dependency.effective_alias().to_string(),
        config: dependency.config.clone(),
    }
}

fn std_package_dependency_constraint() -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID.to_string(),
        version: "1.0.0".to_string(),
        alias: "std".to_string(),
        config: crate::context::empty_dependency_config(),
    }
}

fn file_ir_units_reference_std_package<'a>(
    file_ir_units: impl IntoIterator<Item = &'a FileIrUnit>,
) -> bool {
    file_ir_units.into_iter().any(|file| {
        file_unit_references_package(file, skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID)
    })
}

fn file_unit_references_package(file: &FileIrUnit, package_id: &str) -> bool {
    file.external_refs
        .package_symbols
        .iter()
        .any(|symbol| package_symbol_references_package(symbol, package_id))
        || file.type_table.iter().any(|ty| {
            type_descriptor_references_package(&ty.descriptor, package_id)
                || ty
                    .implements
                    .iter()
                    .any(|implemented| type_ref_references_package(implemented, package_id))
        })
        || file.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(&executable.return_type, package_id)
                || executable
                    .self_type
                    .as_ref()
                    .is_some_and(|ty| type_ref_references_package(ty, package_id))
        })
}

fn type_descriptor_references_package(descriptor: &TypeDescriptorIr, package_id: &str) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeDescriptorIr::Alias { target } => type_ref_references_package(target, package_id),
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .any(|variant| type_ref_references_package(variant, package_id)),
        TypeDescriptorIr::Native { .. } => false,
    }
}

fn type_ref_references_package(ty: &TypeRefIr, package_id: &str) -> bool {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            package_symbol_references_package(symbol, package_id)
        }
        TypeRefIr::Native { args, .. } => args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| type_ref_references_package(item, package_id)),
        TypeRefIr::Nullable { inner } => type_ref_references_package(inner, package_id),
        TypeRefIr::AnyInterface { interface } => {
            interface_instantiation_references_package(interface, package_id)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(return_type, package_id)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn interface_instantiation_references_package(
    interface: &InterfaceInstantiationRef,
    package_id: &str,
) -> bool {
    serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .ok()
        .is_some_and(|ty| type_ref_references_package(&ty, package_id))
        || interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id))
}

fn package_symbol_references_package(symbol: &PackageSymbolRef, package_id: &str) -> bool {
    match &symbol.package {
        PackageRefIr::PackageId {
            package_id: candidate,
        } => candidate == package_id,
        PackageRefIr::Dependency { .. } => false,
    }
}

pub fn service_config_metadata(
    package_publications: &[PackageProjectionInput],
    declared_dependencies: &[ProjectedPackageDependency],
) -> ServiceConfigMetadata {
    let package_configs = service_package_configs(package_publications, declared_dependencies)
        .into_iter()
        .map(|(id, entry)| {
            (
                id,
                serde_json::to_value(entry).expect("package config entry serializes"),
            )
        })
        .collect();
    ServiceConfigMetadata {
        values: StdBTreeMap::new(),
        profiles: StdBTreeMap::new(),
        package_configs,
    }
}

/// Per-package config entry embedded in the service assembly's
/// `packageConfigs` map. Field order matches the former `json!`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicePackageConfigEntry {
    package_id: String,
    id: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    direct: bool,
}

pub fn service_package_configs(
    package_publications: &[PackageProjectionInput],
    declared_dependencies: &[ProjectedPackageDependency],
) -> StdBTreeMap<String, ServicePackageConfigEntry> {
    let declared_by_id = declared_dependencies
        .iter()
        .map(|dependency| (dependency.id.as_str(), dependency))
        .collect::<StdBTreeMap<_, _>>();
    package_publications_by_id(package_publications)
        .into_iter()
        .filter(|package| {
            package.manifest().id() != PRELUDE_REGISTRY_ID
                && !package.manifest().provenance().synthetic()
        })
        .map(|package| {
            let declared = declared_by_id.get(package.manifest().id()).copied();
            let entry = ServicePackageConfigEntry {
                package_id: package.manifest().id().to_string(),
                id: package.manifest().id().to_string(),
                version: package.manifest().version().to_string(),
                alias: declared.map(|dependency| dependency.effective_alias().to_string()),
                direct: declared.is_some(),
            };
            (package.manifest().id().to_string(), entry)
        })
        .collect()
}

fn package_publications_by_id(
    package_publications: &[PackageProjectionInput],
) -> Vec<&PackageProjectionInput> {
    let mut package_publications = package_publications.iter().collect::<Vec<_>>();
    package_publications.sort_by(|left, right| left.manifest().id().cmp(right.manifest().id()));
    package_publications
}

pub fn service_package_abi_expectations(
    dependencies: &[PackageDependencyConstraint],
    package_units: &[PackageUnit],
    service_file_units: &[FileIrUnit],
) -> Result<Vec<PackageAbiExpectation>, ProjectionError> {
    let package_abi_index = package_units
        .iter()
        .map(|unit| {
            (
                (unit.package_id.clone(), unit.version.clone()),
                unit.abi_identity.clone(),
            )
        })
        .collect::<StdBTreeMap<_, _>>();
    let package_export_kind_index = package_export_kind_index(package_units);
    let dependency_index = dependencies
        .iter()
        .flat_map(|dependency| {
            [
                (
                    dependency.id.clone(),
                    (dependency.id.clone(), dependency.version.clone()),
                ),
                (
                    dependency.alias.clone(),
                    (dependency.id.clone(), dependency.version.clone()),
                ),
            ]
        })
        .collect::<StdBTreeMap<_, _>>();
    let mut used_by_package = StdBTreeMap::<(String, String), Vec<PackageUsedSymbol>>::new();

    for unit in service_file_units {
        collect_package_used_symbols_from_file_unit(
            unit,
            &dependency_index,
            &package_export_kind_index,
            &mut used_by_package,
        )?;
    }

    Ok(dependencies
        .iter()
        .filter_map(|dependency| {
            let key = (dependency.id.clone(), dependency.version.clone());
            let used_symbols = used_by_package.remove(&key).unwrap_or_default();
            (!used_symbols.is_empty()).then(|| PackageAbiExpectation {
                id: dependency.id.clone(),
                version: dependency.version.clone(),
                abi_identity: package_abi_index
                    .get(&key)
                    .cloned()
                    .expect("resolved service package dependency must have a PackageUnit"),
                used_symbols,
            })
        })
        .collect())
}

fn package_export_kind_index(
    package_units: &[PackageUnit],
) -> StdBTreeMap<(String, String, String), PackageUsedSymbolKind> {
    let mut index = StdBTreeMap::new();
    for unit in package_units {
        let package_id = &unit.package_id;
        let version = &unit.version;
        let exports = &unit.implementation_links;
        let symbol_kinds: [(Vec<&String>, PackageUsedSymbolKind); 4] = [
            (exports.types.keys().collect(), PackageUsedSymbolKind::Type),
            (
                exports.constants.keys().collect(),
                PackageUsedSymbolKind::Const,
            ),
            (
                exports.functions.keys().collect(),
                PackageUsedSymbolKind::Function,
            ),
            (
                exports.impl_methods.keys().collect(),
                PackageUsedSymbolKind::ImplMethod,
            ),
        ];
        for (symbols, kind) in symbol_kinds {
            for symbol_path in symbols {
                index.insert(
                    (package_id.clone(), version.clone(), symbol_path.clone()),
                    kind,
                );
            }
        }
    }
    index
}

fn collect_package_used_symbols_from_file_unit(
    unit: &FileIrUnit,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    for ty in &unit.type_table {
        collect_package_used_symbols_from_type_descriptor(
            &ty.descriptor,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
        for implemented in &ty.implements {
            collect_package_used_symbols_from_type_ref(
                implemented,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
    }
    for interface in unit.declarations.interfaces.values() {
        collect_package_used_symbols_from_interface(
            interface,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for constant in unit.declarations.constants.values() {
        collect_package_used_symbols_from_type_ref(
            &constant.ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for db in unit.declarations.db.values() {
        collect_package_used_symbols_from_type_ref(
            &db.type_ref,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
        collect_package_used_symbols_from_type_ref(
            &db.key.ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
        for field in &db.fields {
            collect_package_used_symbols_from_type_ref(
                &field.ty,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
    }
    for constant in &unit.constants {
        collect_package_used_symbols_from_type_ref(
            &constant.ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
        collect_package_used_symbols_from_body(
            &constant.body,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for executable in &unit.executables {
        collect_package_used_symbols_from_executable(
            executable,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_interface(
    interface: &InterfaceDeclIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    for operation in &interface.operations {
        for param in &operation.params {
            collect_package_used_symbols_from_type_ref(
                &param.ty,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        collect_package_used_symbols_from_type_ref(
            &operation.return_type,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
        if let Some(implicit_self) = &operation.implicit_self {
            collect_package_used_symbols_from_type_ref(
                implicit_self,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
    }
    Ok(())
}

fn collect_package_used_symbols_from_executable(
    executable: &ExecutableIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    for param in &executable.params {
        collect_package_used_symbols_from_type_ref(
            &param.ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    collect_package_used_symbols_from_type_ref(
        &executable.return_type,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    if let Some(self_type) = &executable.self_type {
        collect_package_used_symbols_from_type_ref(
            self_type,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    collect_package_used_symbols_from_body(
        &executable.body,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )
}

fn collect_package_used_symbols_from_body(
    body: &ExecutableBody,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    for expression in &body.expressions {
        collect_package_used_symbols_from_expr(
            expression,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for statement in &body.statements {
        collect_package_used_symbols_from_stmt(
            statement,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_stmt(
    statement: &StmtIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if let StmtIr::Match { arms, .. } = statement {
        for arm in arms {
            collect_package_used_symbols_from_pattern(
                &arm.pattern,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
    }
    Ok(())
}

fn collect_package_used_symbols_from_expr(
    expression: &ExprIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match expression {
        ExprIr::Construct { type_ref, .. } => {
            collect_package_used_symbols_from_type_ref(
                type_ref,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::InterfaceBox {
            interface, source, ..
        } => {
            collect_package_used_symbols_from_interface_instantiation(
                interface,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
            collect_package_used_symbols_from_box_source(
                source,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::Call { call } => {
            collect_package_used_symbols_from_call(
                call,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::Catch { catch_type, .. } => {
            if let Some(catch_type) = catch_type {
                collect_package_used_symbols_from_type_ref(
                    catch_type,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        ExprIr::DbOperation { operation } => {
            collect_package_used_symbols_from_db_operation(
                operation,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::DbQuery { query } => {
            collect_package_used_symbols_from_db_query(
                query,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::DbTransaction { transaction } => {
            collect_package_used_symbols_from_type_ref(
                &transaction.result_type,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::DbLeaseClaim { claim } => {
            collect_package_used_symbols_from_type_ref(
                &claim.target.type_ref,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
            collect_package_used_symbols_from_type_ref(
                &claim.result_type,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::DbLeaseRead { read } => {
            collect_package_used_symbols_from_type_ref(
                &read.target.type_ref,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
            collect_package_used_symbols_from_type_ref(
                &read.result_type,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        ExprIr::Literal { .. }
        | ExprIr::LoadSlot { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::Field { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
        | ExprIr::Unary { .. }
        | ExprIr::Binary { .. }
        | ExprIr::Throw { .. }
        | ExprIr::Rethrow { .. }
        | ExprIr::ValueBlock { .. } => {}
    }
    Ok(())
}

fn collect_package_used_symbols_from_call(
    call: &CallIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match &call.target {
        CallTargetIr::PackageSymbol {
            package_ref,
            operation,
        } => {
            record_package_used_operation(
                package_ref,
                operation,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        CallTargetIr::InterfaceMethod { interface, .. } => {
            collect_package_used_symbols_from_interface_instantiation(
                interface,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        CallTargetIr::LocalExecutable { .. }
        | CallTargetIr::ExternalServiceSymbol { .. }
        | CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => {}
    }
    for ty in call.type_args.values() {
        collect_package_used_symbols_from_type_ref(
            ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for metadata in call.metadata.values() {
        collect_package_used_symbols_from_metadata(
            metadata,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_interface_instantiation(
    interface: &InterfaceInstantiationRef,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if let Ok(interface_ty) = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id) {
        collect_package_used_symbols_from_type_ref(
            &interface_ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    for arg in &interface.canonical_type_args {
        collect_package_used_symbols_from_type_ref(
            arg,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_box_source(
    source: &BoxSourceIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match source {
        BoxSourceIr::Local {
            concrete_type,
            method_table,
        } => {
            collect_package_used_symbols_from_type_ref(
                concrete_type,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
            collect_package_used_symbols_from_interface_method_table(
                method_table,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        BoxSourceIr::Remote { .. } => {}
    }
    Ok(())
}

fn collect_package_used_symbols_from_interface_method_table(
    method_table: &InterfaceMethodTablePlanIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    collect_package_used_symbols_from_interface_instantiation(
        &method_table.interface,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    collect_package_used_symbols_from_type_ref(
        &method_table.concrete_type,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    for slot in &method_table.slots {
        for param in &slot.signature.params {
            collect_package_used_symbols_from_type_ref(
                &param.ty,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        collect_package_used_symbols_from_type_ref(
            &slot.signature.return_type,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_db_operation(
    operation: &DbOperationIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    collect_package_used_symbols_from_type_ref(
        &operation.target.type_ref,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    collect_package_used_symbols_from_type_ref(
        &operation.result_type,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    if let Some(selector) = &operation.selector {
        collect_package_used_symbols_from_db_selector(
            selector,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    if let Some(query) = &operation.query {
        collect_package_used_symbols_from_db_query_ir(
            query,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    if let Some(body) = &operation.body {
        collect_package_used_symbols_from_db_body(
            body,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    if let Some(body) = &operation.insert_body {
        collect_package_used_symbols_from_db_body(
            body,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    if let Some(change) = &operation.change {
        for change_op in &change.ops {
            match change_op {
                DbChangeOpIr::Set { .. }
                | DbChangeOpIr::Inc { .. }
                | DbChangeOpIr::Unset { .. }
                | DbChangeOpIr::AddToSet { .. }
                | DbChangeOpIr::Remove { .. } => {}
            }
        }
    }
    Ok(())
}

fn collect_package_used_symbols_from_db_query(
    query: &DbQueryValueIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    collect_package_used_symbols_from_type_ref(
        &query.target.type_ref,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    collect_package_used_symbols_from_type_ref(
        &query.result_type,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )?;
    collect_package_used_symbols_from_db_query_ir(
        &query.query,
        dependency_index,
        package_export_kind_index,
        used_by_package,
    )
}

fn collect_package_used_symbols_from_db_selector(
    selector: &DbSelectorIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if let DbSelectorIr::Query { query } = selector {
        collect_package_used_symbols_from_db_query_ir(
            query,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_db_query_ir(
    query: &DbQueryIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    for predicate in &query.where_clauses {
        collect_package_used_symbols_from_db_predicate(
            predicate,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_db_predicate(
    predicate: &DbPredicateIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match predicate {
        DbPredicateIr::And { predicates } | DbPredicateIr::Or { predicates } => {
            for predicate in predicates {
                collect_package_used_symbols_from_db_predicate(
                    predicate,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        DbPredicateIr::Not { predicate } | DbPredicateIr::Conditional { predicate, .. } => {
            collect_package_used_symbols_from_db_predicate(
                predicate,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        DbPredicateIr::Compare { .. } | DbPredicateIr::Regex { .. } => {}
    }
    Ok(())
}

fn collect_package_used_symbols_from_db_body(
    body: &DbBodyIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match body {
        DbBodyIr::ObjectFields { .. } | DbBodyIr::Values { .. } => {
            let _ = (dependency_index, package_export_kind_index, used_by_package);
        }
    }
    Ok(())
}

fn collect_package_used_symbols_from_pattern(
    pattern: &PatternIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if let PatternIr::Type { ty } = pattern {
        collect_package_used_symbols_from_type_ref(
            ty,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    Ok(())
}

fn collect_package_used_symbols_from_type_descriptor(
    descriptor: &TypeDescriptorIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values() {
                collect_package_used_symbols_from_type_ref(
                    field,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        TypeDescriptorIr::Alias { target } => {
            collect_package_used_symbols_from_type_ref(
                target,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        TypeDescriptorIr::Union { variants } => {
            for variant in variants {
                collect_package_used_symbols_from_type_ref(
                    variant,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        TypeDescriptorIr::Native { .. } => {}
    }
    Ok(())
}

fn collect_package_used_symbols_from_type_ref(
    ty: &TypeRefIr,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => record_package_used_symbol(
            symbol,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?,
        TypeRefIr::Native { args, .. } => {
            for arg in args {
                collect_package_used_symbols_from_type_ref(
                    arg,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_package_used_symbols_from_type_ref(
                    field,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_package_used_symbols_from_type_ref(
                    item,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        TypeRefIr::Nullable { inner } => collect_package_used_symbols_from_type_ref(
            inner,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?,
        TypeRefIr::AnyInterface { interface } => {
            collect_package_used_symbols_from_interface_instantiation(
                interface,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_package_used_symbols_from_type_ref(
                    &param.ty,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
            collect_package_used_symbols_from_type_ref(
                return_type,
                dependency_index,
                package_export_kind_index,
                used_by_package,
            )?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn collect_package_used_symbols_from_metadata(
    metadata: &MetadataValue,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if let Some(symbol) = package_symbol_from_metadata(metadata) {
        record_package_used_symbol(
            &symbol,
            dependency_index,
            package_export_kind_index,
            used_by_package,
        )?;
    }
    match metadata {
        MetadataValue::Array(items) => {
            for item in items {
                collect_package_used_symbols_from_metadata(
                    item,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        MetadataValue::Object(object) => {
            for item in object.values() {
                collect_package_used_symbols_from_metadata(
                    item,
                    dependency_index,
                    package_export_kind_index,
                    used_by_package,
                )?;
            }
        }
        MetadataValue::Null
        | MetadataValue::Bool(_)
        | MetadataValue::Number(_)
        | MetadataValue::String(_) => {}
    }
    Ok(())
}

fn package_symbol_from_metadata(metadata: &MetadataValue) -> Option<PackageSymbolRef> {
    let MetadataValue::Object(object) = metadata else {
        return None;
    };
    if !matches!(
        object.get("kind"),
        Some(MetadataValue::String(kind)) if kind == "packageSymbol"
    ) {
        return None;
    }
    let Some(MetadataValue::Object(symbol)) = object.get("symbol") else {
        return None;
    };
    package_symbol_ref_from_metadata_object(symbol)
}

fn package_symbol_ref_from_metadata_object(
    object: &StdBTreeMap<String, MetadataValue>,
) -> Option<PackageSymbolRef> {
    let package = match object.get("package")? {
        MetadataValue::Object(package) => package_ref_from_metadata_object(package)?,
        _ => return None,
    };
    let symbol_path = match object.get("symbolPath")? {
        MetadataValue::String(symbol_path) => symbol_path.clone(),
        _ => return None,
    };
    let abi_expectation = match object.get("abiExpectation") {
        Some(MetadataValue::String(abi_expectation)) => Some(abi_expectation.clone()),
        Some(MetadataValue::Null) | None => None,
        _ => return None,
    };
    Some(PackageSymbolRef {
        package,
        symbol_path,
        abi_expectation,
    })
}

fn package_ref_from_metadata_object(
    object: &StdBTreeMap<String, MetadataValue>,
) -> Option<PackageRefIr> {
    match object.get("kind")? {
        MetadataValue::String(kind) if kind == "dependency" => {
            let MetadataValue::String(dependency_ref) = object.get("dependencyRef")? else {
                return None;
            };
            Some(PackageRefIr::Dependency {
                dependency_ref: dependency_ref.clone(),
            })
        }
        MetadataValue::String(kind) if kind == "packageId" => {
            let MetadataValue::String(package_id) = object.get("packageId")? else {
                return None;
            };
            Some(PackageRefIr::PackageId {
                package_id: package_id.clone(),
            })
        }
        _ => None,
    }
}

fn record_package_used_symbol(
    symbol: &PackageSymbolRef,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if symbol.symbol_path.is_empty() {
        return Ok(());
    }
    let package_key = match &symbol.package {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref,
        PackageRefIr::PackageId { package_id } => package_id,
    };
    let Some((id, version)) = dependency_index.get(package_key.as_str()) else {
        return Ok(());
    };
    let symbol_path = symbol.symbol_path.as_str();
    let Some(kind) = package_export_kind_index
        .get(&(id.clone(), version.clone(), symbol_path.to_string()))
        .copied()
    else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "package dependency {id}@{version} does not export symbol {symbol_path}"
            ),
        });
    };

    let symbols = used_by_package
        .entry((id.clone(), version.clone()))
        .or_default();
    if !symbols
        .iter()
        .any(|symbol| symbol.symbol_path == symbol_path && symbol.kind == kind)
    {
        symbols.push(PackageUsedSymbol {
            symbol_path: symbol_path.to_string(),
            kind,
        });
    }
    Ok(())
}

fn record_package_used_operation(
    package_ref: &PackageRefIr,
    operation: &OperationAbiRef,
    dependency_index: &StdBTreeMap<String, (String, String)>,
    package_export_kind_index: &StdBTreeMap<(String, String, String), PackageUsedSymbolKind>,
    used_by_package: &mut StdBTreeMap<(String, String), Vec<PackageUsedSymbol>>,
) -> Result<(), ProjectionError> {
    if operation.public_path.is_empty() {
        return Ok(());
    }
    let package_key = match package_ref {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref,
        PackageRefIr::PackageId { package_id } => package_id,
    };
    let Some((id, version)) = dependency_index.get(package_key.as_str()) else {
        return Ok(());
    };
    let symbol_path = operation.public_path.as_str();
    let kind = package_export_kind_index
        .get(&(id.clone(), version.clone(), symbol_path.to_string()))
        .copied()
        .unwrap_or(PackageUsedSymbolKind::Function);
    let symbols = used_by_package
        .entry((id.clone(), version.clone()))
        .or_default();
    if !symbols
        .iter()
        .any(|symbol| symbol.symbol_path == symbol_path && symbol.kind == kind)
    {
        symbols.push(PackageUsedSymbol {
            symbol_path: symbol_path.to_string(),
            kind,
        });
    }
    Ok(())
}

pub fn service_unit_public_instances(
    input: ProjectionView<'_>,
    contract: &ContractProjection,
    index: &ContractProjectionIndex<'_>,
    package_dependencies: &[ProjectedPackageDependency],
) -> Result<Vec<PublicInstanceExport>, ProjectionError> {
    let signature_context =
        SignatureTypeRefContext::from_package_dependencies(package_dependencies);
    let mut names_by_source = StdBTreeMap::<String, String>::new();
    let mut instances = Vec::new();

    for public_instance in input.source().export_bindings().public_instances().values() {
        let public_instance_key = public_instance.public_path.as_str();
        let module_path = public_instance.source_module.as_str();
        let const_name = public_instance.source_symbol.as_str();
        let unit = index.unit_by_module_path(module_path).ok_or_else(|| {
            ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{public_instance_key}` const selector points to missing module {module_path}"
                ),
            }
        })?;
        let constant =
            unit.declarations
                .constants
                .get(const_name)
                .ok_or_else(|| ProjectionError::ContractValidation {
                    message: format!(
                        "public instance `{public_instance_key}` const selector points to missing const {module_path}.{const_name}"
                    ),
                })?;
        let receiver_const = operation_const_receiver_ref(unit, const_name)?;
        let receiver = public_instance_receiver_type_ref(index, module_path, &constant.ty).ok_or_else(
            || ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{public_instance_key}` const {module_path}.{const_name} must have an explicit nominal receiver type"
                ),
            },
        )?;
        let receiver_decl = index
            .type_decl_by_module_local_name(&receiver.module_path, &receiver.symbol)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{public_instance_key}` receiver type {}.{} is missing",
                    receiver.module_path, receiver.symbol
                ),
            })?;
        let implemented_interfaces = public_instance_listed_interfaces(
            contract,
            index,
            &receiver,
            receiver_decl,
            public_instance,
        )?;
        if implemented_interfaces.is_empty() {
            return Err(ProjectionError::ContractValidation {
                message: format!("public instance `{public_instance_key}` exposes no interfaces"),
            });
        }

        let source = format!("{}.{}", unit.module_path, const_name);
        if let Some(existing) =
            names_by_source.insert(public_instance_key.to_string(), source.clone())
        {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "duplicate public instance `{public_instance_key}` exported by both {existing} and {source}"
                ),
            });
        }

        let operations = public_instance_operations(
            index,
            &signature_context,
            public_instance_key,
            &receiver,
            &receiver_const,
            &implemented_interfaces,
        )?;
        instances.push(PublicInstanceExport {
            name: public_instance_key.to_string(),
            module_path: unit.module_path.clone(),
            declared_receiver_type: receiver_type_ref(&receiver),
            implemented_interfaces: implemented_interfaces
                .iter()
                .map(|interface| interface.ty.clone())
                .collect(),
            operations,
        });
    }

    Ok(instances)
}

fn public_instance_receiver_type_ref(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    let symbol = service_symbol_for_nominal_type_ref(index, module_path, ty)?;
    let decl = index.type_decl_by_module_local_name(&symbol.module_path, &symbol.symbol)?;
    is_public_instance_receiver_decl(index, &symbol.module_path, &symbol.symbol, decl)
        .then_some(symbol)
}

fn service_symbol_for_nominal_type_ref(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let decl = index
                .unit_by_module_path(module_path)?
                .type_table
                .get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            if let Some(source_key) =
                index.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
            {
                index.type_decl_by_module_local_name(
                    source_key.module_path(),
                    source_key.symbol(),
                )?;
                return Some(ServiceSymbolRef {
                    module_path: source_key.module_path().to_string(),
                    symbol: source_key.symbol().to_string(),
                });
            }
            let source_module = index.source_module_for_reference_module(&symbol.module_path);
            index.type_decl_by_module_local_name(source_module, &symbol.symbol)?;
            Some(ServiceSymbolRef {
                module_path: source_module.to_string(),
                symbol: symbol.symbol.clone(),
            })
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => None,
    }
}

fn is_public_instance_receiver_decl(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    local_name: &str,
    decl: &TypeDeclIr,
) -> bool {
    !matches!(decl.descriptor, TypeDescriptorIr::Alias { .. })
        && index
            .interface_decl_by_module_local_name(module_path, local_name)
            .is_none()
}

#[derive(Debug, Clone)]
struct PublicImplementedInterface {
    ty: TypeRefIr,
    instantiation: InterfaceInstantiationRef,
    methods: PublicImplementedInterfaceMethods,
}

#[derive(Debug, Clone)]
enum PublicImplementedInterfaceMethods {
    Local {
        methods: Vec<InterfaceMethodSignature>,
    },
    Package {
        methods: Vec<InterfaceMethodSignature>,
    },
}

fn public_instance_listed_interfaces(
    contract: &ContractProjection,
    index: &ContractProjectionIndex<'_>,
    receiver: &ServiceSymbolRef,
    receiver_decl: &TypeDeclIr,
    public_instance: &ExportPublicInstanceProjection,
) -> Result<Vec<PublicImplementedInterface>, ProjectionError> {
    let mut seen = BTreeSet::new();
    let mut implemented = Vec::new();
    for listed_interface in &public_instance.interfaces {
        if let Some(interface_decl) = index.interface_decl_by_module_local_name(
            &listed_interface.source_module,
            &listed_interface.source_symbol,
        ) {
            let interface_symbol = ServiceSymbolRef {
                module_path: listed_interface.source_module.clone(),
                symbol: listed_interface.source_symbol.clone(),
            };
            if !receiver_decl.implements.iter().any(|implemented_ty| {
                public_interface_symbol_for_type_ref(index, &receiver.module_path, implemented_ty)
                    .is_some_and(|implemented_symbol| implemented_symbol == interface_symbol)
            }) {
                return Err(ProjectionError::ContractValidation {
                    message: format!(
                        "public instance `{}` receiver {}.{} does not explicitly implement listed interface {}.{}",
                        public_instance.public_path,
                        receiver.module_path,
                        receiver.symbol,
                        listed_interface.source_module,
                        listed_interface.source_symbol
                    ),
                });
            }
            let interface_ty = service_public_interface_type_ref(contract, index, listed_interface);
            let interface_display =
                service_public_instance_interface_display_name(&interface_ty, listed_interface);
            let instantiation = service_public_instance_interface_instantiation_ref(
                index,
                receiver,
                &interface_ty,
                &interface_display,
                listed_interface,
            )?;
            let interface_key =
                serde_json::to_string(&instantiation).expect("interface ref must serialize");
            if !seen.insert(interface_key) {
                return Err(ProjectionError::ContractValidation {
                    message: format!(
                        "public instance `{}` duplicate interface selector {}.{}",
                        public_instance.public_path,
                        listed_interface.source_module,
                        listed_interface.source_symbol
                    ),
                });
            }
            let methods = service_interface_method_signatures(
                index,
                &listed_interface.source_module,
                interface_decl,
            );
            let methods = instantiate_interface_method_signatures(
                methods,
                &interface_decl.type_params,
                &instantiation.canonical_type_args,
            )
            .map_err(|error| ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{}` interface {}.{} expects {} type arguments but got {}",
                    public_instance.public_path,
                    listed_interface.source_module,
                    listed_interface.source_symbol,
                    error.expected_type_args,
                    error.actual_type_args
                ),
            })?;
            implemented.push(PublicImplementedInterface {
                ty: interface_ty,
                instantiation,
                methods: PublicImplementedInterfaceMethods::Local { methods },
            });
            continue;
        }

        let Some((package_interface_identity, package_interface_methods)) =
            public_instance_listed_package_interface(receiver, receiver_decl, listed_interface)
        else {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{}` listed interface selector {}.{} does not resolve to a local or package interface implemented by receiver {}.{}",
                    public_instance.public_path,
                    listed_interface.source_module,
                    listed_interface.source_symbol,
                    receiver.module_path,
                    receiver.symbol
                ),
            });
        };
        let instantiation = interface_instantiation_ref_for_type_ref(&package_interface_identity);
        let interface_key =
            serde_json::to_string(&instantiation).expect("interface ref must serialize");
        if !seen.insert(interface_key) {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{}` duplicate interface selector {}.{}",
                    public_instance.public_path,
                    listed_interface.source_module,
                    listed_interface.source_symbol
                ),
            });
        }
        implemented.push(PublicImplementedInterface {
            ty: package_interface_identity,
            instantiation,
            methods: PublicImplementedInterfaceMethods::Package {
                methods: package_interface_methods,
            },
        });
    }
    Ok(implemented)
}

fn public_interface_symbol_for_type_ref(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    let symbol = service_symbol_for_nominal_type_ref(index, module_path, ty)?;
    index.interface_decl_by_module_local_name(&symbol.module_path, &symbol.symbol)?;
    Some(symbol)
}

fn service_public_interface_type_ref(
    contract: &ContractProjection,
    index: &ContractProjectionIndex<'_>,
    interface: &ExportPublicInstanceInterfaceProjection,
) -> TypeRefIr {
    contract
        .interfaces
        .iter()
        .find_map(|(public_name, projected)| {
            (projected.source_module == interface.source_module
                && projected.source_name == interface.source_symbol)
                .then(|| public_interface_type_ref(public_name))
        })
        .unwrap_or_else(|| TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: index
                    .source_module_for_reference_module(&interface.source_module)
                    .to_string(),
                symbol: interface.source_symbol.clone(),
            },
        })
}

fn service_public_instance_interface_display_name(
    interface_ty: &TypeRefIr,
    interface: &ExportPublicInstanceInterfaceProjection,
) -> String {
    match interface_ty {
        TypeRefIr::ServiceSymbol { symbol } if symbol.module_path.is_empty() => {
            symbol.symbol.clone()
        }
        _ => format!("{}.{}", interface.source_module, interface.source_symbol),
    }
}

fn public_instance_listed_package_interface(
    receiver: &ServiceSymbolRef,
    _receiver_decl: &TypeDeclIr,
    listed_interface: &ExportPublicInstanceInterfaceProjection,
) -> Option<(TypeRefIr, Vec<InterfaceMethodSignature>)> {
    let _ = receiver;
    listed_interface
        .receiver_implements_package_interface
        .then(|| {
            Some((
                listed_interface.package_interface_identity.clone()?,
                listed_interface.package_interface_methods.clone(),
            ))
        })?
}

fn package_interface_identities_match(left: &TypeRefIr, right: &TypeRefIr) -> bool {
    let (
        TypeRefIr::PackageSymbol {
            symbol: left_symbol,
        },
        TypeRefIr::PackageSymbol {
            symbol: right_symbol,
        },
    ) = (left, right)
    else {
        return false;
    };
    left_symbol.package == right_symbol.package
        && (left_symbol.symbol_path == right_symbol.symbol_path
            || left_symbol
                .symbol_path
                .rsplit_once('.')
                .is_some_and(|(_, left_name)| {
                    right_symbol
                        .symbol_path
                        .rsplit_once('.')
                        .is_some_and(|(_, right_name)| left_name == right_name)
                }))
}

fn public_instance_package_interface_matches_selector(
    interface_identity: &TypeRefIr,
    selector_path: &str,
) -> bool {
    let TypeRefIr::PackageSymbol { symbol } = interface_identity else {
        return false;
    };
    symbol.symbol_path == selector_path
        || symbol
            .symbol_path
            .rsplit_once('.')
            .is_some_and(|(_, symbol_name)| {
                selector_path
                    .rsplit_once('.')
                    .is_some_and(|(_, selector_name)| symbol_name == selector_name)
            })
}

fn service_interface_method_signatures(
    index: &ContractProjectionIndex<'_>,
    module_path: &str,
    interface: &InterfaceDeclIr,
) -> Vec<InterfaceMethodSignature> {
    interface
        .operations
        .iter()
        .map(|operation| {
            let explicit_self = operation
                .params
                .first()
                .filter(|param| param.name == "self");
            InterfaceMethodSignature {
                name: operation.name.clone(),
                type_params: operation.type_params.clone(),
                params: operation
                    .params
                    .iter()
                    .skip(usize::from(explicit_self.is_some()))
                    .map(|param| FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: canonical_service_interface_type_arg(index, module_path, &param.ty),
                    })
                    .collect(),
                return_type: canonical_service_interface_type_arg(
                    index,
                    module_path,
                    &operation.return_type,
                ),
                is_native: operation.is_native,
                is_provider: operation.is_provider,
                is_static: operation.is_static,
                implicit_self: operation
                    .implicit_self
                    .as_ref()
                    .or_else(|| explicit_self.map(|param| &param.ty))
                    .map(|ty| canonical_service_interface_type_arg(index, module_path, ty)),
            }
        })
        .collect()
}

fn service_public_instance_interface_instantiation_ref(
    index: &ContractProjectionIndex<'_>,
    receiver: &ServiceSymbolRef,
    interface_ty: &TypeRefIr,
    public_name: &str,
    listed_interface: &ExportPublicInstanceInterfaceProjection,
) -> Result<InterfaceInstantiationRef, ProjectionError> {
    if !listed_interface.implements_interface {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "public instance receiver {}.{} does not explicitly implement public interface {public_name}",
                receiver.module_path, receiver.symbol
            ),
        });
    }
    let canonical_type_args = listed_interface
        .canonical_type_args
        .iter()
        .map(|arg| canonical_service_interface_type_arg(index, &receiver.module_path, arg))
        .collect();
    Ok(interface_instantiation_ref(
        interface_ty.clone(),
        canonical_type_args,
    ))
}

fn canonical_service_interface_type_arg(
    index: &ContractProjectionIndex<'_>,
    context_module: &str,
    ty: &TypeRefIr,
) -> TypeRefIr {
    match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| canonical_service_interface_type_arg(index, context_module, arg))
                .collect(),
        },
        TypeRefIr::LocalType { .. } => {
            service_symbol_for_nominal_type_ref(index, context_module, ty)
                .map(|symbol| TypeRefIr::ServiceSymbol { symbol })
                .unwrap_or_else(|| ty.clone())
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            service_symbol_for_nominal_type_ref(index, context_module, ty)
                .map(|symbol| TypeRefIr::ServiceSymbol { symbol })
                .unwrap_or_else(|| {
                    let source_module =
                        index.source_module_for_reference_module(&symbol.module_path);
                    TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: source_module.to_string(),
                            symbol: symbol.symbol.clone(),
                        },
                    }
                })
        }
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    (
                        name.clone(),
                        canonical_service_interface_type_arg(index, context_module, field_ty),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| canonical_service_interface_type_arg(index, context_module, item))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(canonical_service_interface_type_arg(
                index,
                context_module,
                inner,
            )),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| canonical_service_interface_type_arg(index, context_module, arg))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: canonical_service_interface_type_arg(index, context_module, &param.ty),
                })
                .collect(),
            return_type: Box::new(canonical_service_interface_type_arg(
                index,
                context_module,
                return_type,
            )),
        },
        TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn public_interface_type_ref(public_name: &str) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: String::new(),
            symbol: public_name.to_string(),
        },
    }
}

fn receiver_type_ref(receiver: &ServiceSymbolRef) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: receiver.clone(),
    }
}

fn public_instance_operations(
    index: &ContractProjectionIndex<'_>,
    signature_context: &SignatureTypeRefContext,
    public_instance_name: &str,
    receiver: &ServiceSymbolRef,
    receiver_const: &OperationConstReceiverRef,
    implemented_interfaces: &[PublicImplementedInterface],
) -> Result<Vec<PublicInstanceOperation>, ProjectionError> {
    let mut operations = Vec::new();
    let mut operation_keys = StdBTreeMap::<String, PublicInstanceOperationKey>::new();

    for implemented in implemented_interfaces {
        let projected = match &implemented.methods {
            PublicImplementedInterfaceMethods::Local { methods }
            | PublicImplementedInterfaceMethods::Package { methods } => {
                public_instance_interface_operations(
                    index,
                    signature_context,
                    public_instance_name,
                    receiver,
                    receiver_const,
                    &implemented.instantiation,
                    methods,
                )?
            }
        };

        for operation in projected {
            let key = PublicInstanceOperationKey::from(&operation);
            if let Some(existing) = operation_keys.get(&operation.operation.display_name) {
                if existing != &key {
                    return Err(ProjectionError::ContractValidation {
                        message: format!(
                            "public instance `{public_instance_name}` derives conflicting operation `{}` from multiple interfaces",
                            operation.operation.display_name
                        ),
                    });
                }
            } else {
                operation_keys.insert(operation.operation.display_name.clone(), key);
                operations.push(operation);
            }
        }
    }

    Ok(operations)
}

fn public_instance_interface_operations(
    index: &ContractProjectionIndex<'_>,
    signature_context: &SignatureTypeRefContext,
    public_instance_name: &str,
    receiver: &ServiceSymbolRef,
    receiver_const: &OperationConstReceiverRef,
    interface: &InterfaceInstantiationRef,
    methods: &[InterfaceMethodSignature],
) -> Result<Vec<PublicInstanceOperation>, ProjectionError> {
    let receiver_unit =
        index
            .unit_by_module_path(&receiver.module_path)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "public instance `{public_instance_name}` receiver module {} is missing from the projection index",
                    receiver.module_path
                ),
            })?;
    methods
        .iter()
        .map(|method| {
            let target_symbol = impl_method_declaration_name(&receiver.symbol, &method.name);
            let executable_index =
                service_local_executable_index(receiver_unit, &target_symbol).ok_or_else(|| {
                    ProjectionError::ContractValidation {
                        message: format!(
                            "public instance `{public_instance_name}` receiver {}.{} is missing implementation method {}",
                            receiver.module_path, receiver.symbol, method.name
                        ),
                    }
                })?;
            let executable = receiver_unit
                .executables
                .get(executable_index as usize)
                .ok_or_else(|| ProjectionError::ContractValidation {
                    message: format!(
                        "public instance `{public_instance_name}` receiver {}.{} method {} points to missing executable index {}",
                        receiver.module_path,
                        receiver.symbol,
                        method.name,
                        executable_index
                    ),
                })?;
            validate_public_instance_impl_method_signature(
                Some(index),
                public_instance_name,
                receiver,
                method,
                executable,
                signature_context,
            )?;
            let public_signature = public_callable_signature_from_interface_method(method);
            let operation_ref = public_instance_operation_ref(
                public_instance_name,
                interface,
                &method.name,
                &public_signature,
            );
            Ok(PublicInstanceOperation {
                operation: operation_ref.clone(),
                receiver_executable: local_receiver_executable_ref(
                    receiver_const,
                    receiver_unit,
                    &target_symbol,
                    executable_index,
                    operation_ref
                        .method_abi_id
                        .as_deref()
                        .unwrap_or(operation_ref.operation_abi_id.as_str()),
                    OperationCallableKind::ImplMethod,
                ),
            })
        })
        .collect()
}

fn service_local_executable_index(file: &FileIrUnit, symbol: &str) -> Option<u32> {
    file.link_targets
        .executables
        .get(symbol)
        .map(|target| target.executable_index)
        .or_else(|| {
            file.declarations
                .executables
                .get(symbol)
                .map(|declaration| declaration.executable_index)
        })
        .or_else(|| {
            file.executables
                .iter()
                .position(|executable| {
                    executable.symbol == symbol
                        || executable.symbol.ends_with(&format!(".{symbol}"))
                })
                .map(|index| index as u32)
        })
}

fn validate_public_instance_impl_method_signature(
    index: Option<&ContractProjectionIndex<'_>>,
    public_instance_name: &str,
    receiver: &ServiceSymbolRef,
    method: &InterfaceMethodSignature,
    executable: &ExecutableIr,
    signature_context: &SignatureTypeRefContext,
) -> Result<(), ProjectionError> {
    let expected_mode = operation_mode_for_type_ref(&method.return_type);
    let actual_mode = operation_mode_for_type_ref(&executable.return_type);
    let actual_params = executable_signature_params(&executable.params);
    let params_match = actual_params.len() == method.params.len()
        && actual_params
            .iter()
            .zip(method.params.iter())
            .all(|(actual, expected)| {
                let actual_ty = canonical_public_instance_signature_type_arg(
                    index,
                    &receiver.module_path,
                    &actual.ty,
                );
                actual.name == expected.name
                    && signature_type_ref_matches(&actual_ty, &expected.ty, signature_context)
            });
    let actual_return = canonical_public_instance_signature_type_arg(
        index,
        &receiver.module_path,
        &executable.return_type,
    );
    let return_type_match =
        signature_type_ref_matches(&actual_return, &method.return_type, signature_context);
    if actual_mode != expected_mode || !params_match || !return_type_match {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "public instance `{public_instance_name}` receiver {}.{} method {} signature does not match implemented interface method; expected params {:?} return {:?}, got params {:?} return {:?}",
                receiver.module_path,
                receiver.symbol,
                method.name,
                method.params,
                method.return_type,
                executable.params,
                executable.return_type
            ),
        });
    }
    Ok(())
}

fn canonical_public_instance_signature_type_arg(
    index: Option<&ContractProjectionIndex<'_>>,
    module_path: &str,
    ty: &TypeRefIr,
) -> TypeRefIr {
    index
        .map(|index| canonical_service_interface_type_arg(index, module_path, ty))
        .unwrap_or_else(|| ty.clone())
}

fn public_instance_operation_ref(
    public_instance_name: &str,
    interface: &InterfaceInstantiationRef,
    method_name: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    let public_path = format!("{public_instance_name}.{method_name}");
    let method_abi_id = canonical_interface_method_abi_id(&interface, method_name);
    OperationAbiRef {
        operation_abi_id: public_instance_method_operation_abi_id(
            &public_path,
            public_instance_name,
            interface,
            &method_abi_id,
            public_signature,
            &[],
            &StdBTreeMap::new(),
        ),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: public_path.clone(),
        public_instance_key: Some(public_instance_name.to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some(method_abi_id),
        display_name: public_path,
    }
}

fn public_callable_signature_from_interface_method(
    method: &InterfaceMethodSignature,
) -> CanonicalPublicCallableSignature {
    CanonicalPublicCallableSignature {
        params: method.params.clone(),
        return_type: method.return_type.clone(),
        may_suspend: matches!(
            operation_mode_for_type_ref(&method.return_type),
            OperationMode::ServerStream
        ),
    }
}

fn operation_const_receiver_ref(
    unit: &FileIrUnit,
    const_name: &str,
) -> Result<OperationConstReceiverRef, ProjectionError> {
    let const_decl = unit.declarations.constants.get(const_name).ok_or_else(|| {
        ProjectionError::ContractValidation {
            message: format!(
                "public instance receiver {}.{} does not resolve to a const",
                unit.module_path, const_name
            ),
        }
    })?;
    Ok(OperationConstReceiverRef {
        file_ref: file_ref_for_unit(unit),
        const_index: const_decl.const_index,
        const_abi_id: format!("const:{}.{}", unit.module_path, const_name),
        const_type_abi_id: type_ref_abi_key(&const_decl.ty),
    })
}

fn local_receiver_executable_ref(
    receiver_const: &OperationConstReceiverRef,
    unit: &FileIrUnit,
    executable_symbol: &str,
    executable_index: u32,
    method_abi_id: &str,
    callable_kind: OperationCallableKind,
) -> LocalReceiverExecutableRef {
    LocalReceiverExecutableRef {
        receiver: receiver_const.clone(),
        executable_target: operation_target_ref(
            unit,
            executable_symbol,
            executable_index,
            callable_kind,
        ),
        method_abi_id: method_abi_id.to_string(),
        receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PublicInstanceOperationKey {
    operation_abi_id: String,
    receiver_executable: LocalReceiverExecutableRef,
}

impl From<&PublicInstanceOperation> for PublicInstanceOperationKey {
    fn from(operation: &PublicInstanceOperation) -> Self {
        Self {
            operation_abi_id: operation.operation.operation_abi_id.clone(),
            receiver_executable: operation.receiver_executable.clone(),
        }
    }
}

fn operation_mode_for_type_ref(return_type: &TypeRefIr) -> OperationMode {
    match return_type {
        TypeRefIr::Native { name, args }
            if name.rsplit('.').next() == Some("Stream") && args.len() == 1 =>
        {
            OperationMode::ServerStream
        }
        _ => OperationMode::Unary,
    }
}

pub fn ensure_service_operation_adapters(
    file_units: &mut [FileIrUnit],
    contract_projection: &ContractProjection,
    entry_operations: &[EntryOperationSpec],
) -> Result<(), ProjectionError> {
    let mut adapter_requests = StdBTreeMap::<(String, String, String, String), ()>::new();
    for binding in contract_projection.api_bindings.values() {
        for operation in binding.operations.values() {
            adapter_requests.insert(
                (
                    operation.module_path.clone(),
                    operation.type_name.clone(),
                    operation.method_name.clone(),
                    operation.executable_symbol.clone(),
                ),
                (),
            );
        }
    }
    for operation in entry_operations {
        if let EntryOperationCallable::ImplMethod { type_name, method } = &operation.callable {
            adapter_requests.insert(
                (
                    operation.implementation_module.clone(),
                    type_name.clone(),
                    method.clone(),
                    impl_method_declaration_name(type_name, method),
                ),
                (),
            );
        }
    }

    for ((module_path, type_name, method_name, executable_symbol), ()) in adapter_requests {
        let unit = file_units
            .iter_mut()
            .find(|unit| unit.module_path == module_path)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "service operation adapter {}.{} references missing module {}",
                    type_name, method_name, module_path
                ),
            })?;
        ensure_service_operation_adapter(unit, &type_name, &method_name, &executable_symbol)?;
    }
    Ok(())
}

fn ensure_service_operation_adapter(
    unit: &mut FileIrUnit,
    type_name: &str,
    method_name: &str,
    executable_symbol: &str,
) -> Result<(), ProjectionError> {
    let adapter_symbol = service_operation_adapter_symbol(type_name, method_name);
    if unit.declarations.executables.contains_key(&adapter_symbol) {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} cannot use reserved executable symbol {}",
                unit.module_path, executable_symbol, adapter_symbol
            ),
        });
    }
    let executable_decl = unit
        .declarations
        .executables
        .get(executable_symbol)
        .ok_or_else(|| ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} references missing impl method {}",
                unit.module_path, adapter_symbol, executable_symbol
            ),
        })?;
    let impl_executable_index = executable_decl.executable_index;
    let impl_executable = unit
        .executables
        .get(impl_executable_index as usize)
        .ok_or_else(|| ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} references missing executable index {}",
                unit.module_path, adapter_symbol, impl_executable_index
            ),
        })?;
    if impl_executable.kind != ExecutableKind::ImplMethod {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} target {} has kind {:?}, expected implMethod",
                unit.module_path, adapter_symbol, executable_symbol, impl_executable.kind
            ),
        });
    }
    let impl_params = impl_executable.params.clone();
    let impl_return_type = impl_executable.return_type.clone();
    let impl_may_suspend = impl_executable.may_suspend;
    let Some(self_param) = impl_params.first() else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} target {} has no explicit self parameter",
                unit.module_path, adapter_symbol, executable_symbol
            ),
        });
    };
    if self_param.name != "self" {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} target {} first parameter is {}, expected self",
                unit.module_path, adapter_symbol, executable_symbol, self_param.name
            ),
        });
    }
    let TypeRefIr::LocalType { type_index } = self_param.ty else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} target {} self type must be a local service type",
                unit.module_path, adapter_symbol, executable_symbol
            ),
        });
    };
    let self_type = unit.type_table.get(type_index as usize).ok_or_else(|| {
        ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} target {} self type index {} is missing",
                unit.module_path, adapter_symbol, executable_symbol, type_index
            ),
        }
    })?;
    match &self_type.descriptor {
        TypeDescriptorIr::Record { fields } if fields.is_empty() => {}
        TypeDescriptorIr::Record { .. } => {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "service operation adapter {}.{} cannot synthesize receiver for non-empty record type {}",
                    unit.module_path, adapter_symbol, self_type.name
                ),
            });
        }
        _ => {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "service operation adapter {}.{} cannot synthesize receiver for non-record type {}",
                    unit.module_path, adapter_symbol, self_type.name
                ),
            });
        }
    }

    let adapter_executable_index =
        u32::try_from(unit.executables.len()).map_err(|_| ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} executable index overflow",
                unit.module_path, adapter_symbol
            ),
        })?;
    let adapter_params = impl_params
        .iter()
        .skip(1)
        .enumerate()
        .map(|(slot, param)| ParamIr {
            name: param.name.clone(),
            slot: slot as u32,
            ty: param.ty.clone(),
        })
        .collect::<Vec<_>>();
    let mut expressions = Vec::new();
    expressions.push(ExprIr::Construct {
        type_ref: TypeRefIr::LocalType { type_index },
        fields: StdBTreeMap::new(),
    });
    let mut call_args = vec![ExprRefIr { expression: 0 }];
    for (index, _param) in adapter_params.iter().enumerate() {
        let expression =
            u32::try_from(expressions.len()).map_err(|_| ProjectionError::ContractValidation {
                message: format!(
                    "service operation adapter {}.{} expression index overflow",
                    unit.module_path, adapter_symbol
                ),
            })?;
        expressions.push(ExprIr::LoadSlot { slot: index as u32 });
        call_args.push(ExprRefIr { expression });
    }
    let call_expression =
        u32::try_from(expressions.len()).map_err(|_| ProjectionError::ContractValidation {
            message: format!(
                "service operation adapter {}.{} call expression index overflow",
                unit.module_path, adapter_symbol
            ),
        })?;
    expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::LocalExecutable {
                executable_index: impl_executable_index,
            },
            args: call_args,
            type_args: StdBTreeMap::new(),
            metadata: StdBTreeMap::new(),
        },
    });
    let slots = adapter_params
        .iter()
        .map(|param| SlotIr {
            index: param.slot,
            name: param.name.clone(),
            kind: SlotKind::Param,
        })
        .collect::<Vec<_>>();
    unit.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: adapter_symbol.clone(),
        type_params: Vec::new(),
        params: adapter_params,
        return_type: impl_return_type,
        self_type: None,
        slots: SlotLayout {
            frame_size: slots.len() as u32,
            slots,
        },
        may_suspend: impl_may_suspend,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr {
                    expression: call_expression,
                }),
            }],
            expressions,
        },
        source_span: None,
    });
    unit.declarations.executables.insert(
        adapter_symbol.clone(),
        ExecutableDeclarationIr {
            executable_index: adapter_executable_index,
            symbol: format!("{}.{}", unit.module_path, adapter_symbol),
            source_span: None,
        },
    );
    unit.link_targets.executables.insert(
        adapter_symbol,
        ExecutableLinkTargetIr {
            executable_index: adapter_executable_index,
        },
    );
    Ok(())
}

pub fn service_unit_operations(
    operation_entries: &[OperationEntryIr],
    file_units: &[FileIrUnit],
    public_instances: &[PublicInstanceExport],
    timeout: Option<&TimeoutEntry>,
) -> Result<Vec<ServiceOperation>, ProjectionError> {
    let module_files = file_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<StdBTreeMap<_, _>>();
    let default_timeout = timeout.and_then(|timeout| timeout.default_ms);
    let public_instance_operations = public_instances
        .iter()
        .flat_map(|public_instance| {
            public_instance.operations.iter().map(|operation| {
                (
                    operation.operation.operation_abi_id.as_str(),
                    operation.operation.clone(),
                )
            })
        })
        .collect::<StdBTreeMap<_, _>>();

    let mut operations = Vec::new();
    for entry in operation_entries {
        let Some(_entrypoint) = entry.entrypoint.as_deref() else {
            continue;
        };
        let module_path = entry.implementation.module_path.as_str();
        let symbol = entry.implementation.symbol.as_str();
        let file = module_files.get(module_path).copied().ok_or_else(|| {
            ProjectionError::ContractValidation {
                message: format!(
                    "operation {} target references unknown module {module_path}",
                    entry.operation
                ),
            }
        })?;
        let executable_index = if let Some(index) = entry.implementation.executable_index {
            u32::try_from(index).map_err(|_| ProjectionError::ContractValidation {
                message: format!(
                    "operation {} target {}.{} executable index {} is out of range",
                    entry.operation, module_path, symbol, index
                ),
            })?
        } else {
            executable_index_by_identity_and_symbol(file_units, &file.file_ir_identity, symbol)
                .ok_or_else(|| ProjectionError::ContractValidation {
                    message: format!(
                        "operation {} target {}.{} does not resolve to an executable index",
                        entry.operation, module_path, symbol
                    ),
                })?
        };
        if file.executables.get(executable_index as usize).is_none() {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "operation {} target {}.{} points to missing executable index {}",
                    entry.operation, module_path, symbol, executable_index
                ),
            });
        }
        let receiver = entry
            .implementation
            .receiver_const
            .as_ref()
            .map(|receiver| {
                let receiver_file = module_files
                    .get(receiver.module_path.as_str())
                    .copied()
                    .ok_or_else(|| ProjectionError::ContractValidation {
                        message: format!(
                            "operation {} receiver references unknown module {}",
                            entry.operation, receiver.module_path
                        ),
                    })?;
                let const_decl = receiver_file
                    .declarations
                    .constants
                    .get(&receiver.const_name)
                    .ok_or_else(|| ProjectionError::ContractValidation {
                        message: format!(
                            "operation {} receiver {}.{} does not resolve to a const",
                            entry.operation, receiver.module_path, receiver.const_name
                        ),
                    })?;
                Ok::<OperationConstReceiverRef, ProjectionError>(OperationConstReceiverRef {
                    file_ref: file_ref_for_unit(receiver_file),
                    const_index: const_decl.const_index,
                    const_abi_id: format!("const:{}.{}", receiver.module_path, receiver.const_name),
                    const_type_abi_id: type_ref_abi_key(&const_decl.ty),
                })
            })
            .transpose()?;
        let _timeout_ms = timeout
            .and_then(|timeout| timeout.methods.get(&entry.operation).copied())
            .or(default_timeout);
        let callable_kind = if receiver.is_some() {
            OperationCallableKind::ImplMethod
        } else {
            OperationCallableKind::PublicFunction
        };
        let operation = public_instance_operations
            .get(entry.operation_abi_id.as_str())
            .cloned()
            .unwrap_or_else(|| OperationAbiRef {
                operation_abi_id: entry.operation_abi_id.clone(),
                kind: PublicationOperationKind::PublicFunction,
                public_path: entry.operation.clone(),
                public_instance_key: None,
                interface: None,
                method_abi_id: None,
                display_name: entry.operation.clone(),
            });
        let executable = operation_target_ref(file, symbol, executable_index, callable_kind);
        let service_operation = if let Some(receiver) = receiver {
            let method_abi_id = operation
                .method_abi_id
                .clone()
                .unwrap_or_else(|| operation.operation_abi_id.clone());
            ServiceOperation::LocalReceiverExecutable(ServiceReceiverOperationTarget {
                operation: operation.clone(),
                receiver_executable: LocalReceiverExecutableRef {
                    receiver,
                    executable_target: executable,
                    method_abi_id,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            })
        } else {
            ServiceOperation::LocalExecutable(ServiceOperationTarget {
                operation,
                executable,
            })
        };
        operations.push(service_operation);
    }
    Ok(operations)
}

fn file_ref_for_unit(unit: &FileIrUnit) -> FileIrRef {
    FileIrRef::new(unit.file_ir_identity.clone(), unit.module_path.clone())
}

fn operation_target_ref(
    unit: &FileIrUnit,
    symbol: &str,
    executable_index: u32,
    callable_kind: OperationCallableKind,
) -> OperationTargetRef {
    OperationTargetRef {
        file_ref: file_ref_for_unit(unit),
        executable_index,
        callable_abi_id: format!("callable:{}.{}", unit.module_path, symbol),
        callable_kind,
    }
}

fn operation_params_from_entry(entry: &OperationEntryIr) -> Vec<OperationParam> {
    entry
        .parameters
        .iter()
        .map(|param| OperationParam {
            name: param.name.clone(),
            ty: type_ref_from_runtime_descriptor(&param.ty),
        })
        .collect()
}

fn executable_index_by_identity_and_symbol(
    files: &[FileIrUnit],
    file_identity: &str,
    symbol: &str,
) -> Option<u32> {
    files
        .iter()
        .find(|file| file.file_ir_identity == file_identity)?
        .link_targets
        .executables
        .get(symbol)
        .map(|export| export.executable_index)
}

pub fn service_unit_gateway(gateway: &GatewayEntry) -> GatewayConfig {
    let mut config = GatewayConfig::default();
    for route in gateway.http_routes() {
        config.routes.insert(
            route.path.clone(),
            GatewayRoute {
                operation: route.operation.clone(),
                operation_abi_id: route.operation_abi_id.clone().unwrap_or_default(),
                method: route.method.clone(),
                path: route.path.clone(),
            },
        );
    }
    if let Some(websocket) = gateway.websocket_default() {
        config.web_sockets.insert(
            "default".to_string(),
            GatewayWebSocket {
                path: websocket.path.map(ToString::to_string),
                operation: websocket.receive_operation.to_string(),
                operation_abi_id: websocket.receive_operation_abi_id.to_string(),
                connect_operation: websocket.connect_operation.map(ToString::to_string),
                connect_operation_abi_id: websocket
                    .connect_operation_abi_id
                    .map(ToString::to_string),
                routes: Vec::new(),
            },
        );
    }
    config
}
