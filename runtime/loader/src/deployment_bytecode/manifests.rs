use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, host_effect_registry_identity,
    intrinsic_registry_identity, native_value_lifecycle_registry_identity,
    value_lifecycle_policy_identity, BytecodeConstantRef, BytecodeFunctionStatementManifest,
    BytecodePoolEntry, BytecodeRelocation, BytecodeSpecialization, ContractOperationId,
    HostEffectSignature, InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr,
    NominalTypeRefBaseIr, OperationCallableKind, PackageArtifact, PackageArtifactRef,
    PackageBuildId, PackageCallableId, PackageCallableSignature, PackageExecutableCoordinate,
    PackageLocalAbiSymbol, PackageRefIr, PackageRequirement, PackageSymbolRef, PackageTypeRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceRequirementKey, TypeRefIr,
    ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::{
    DeploymentBytecodeHydrationError, DeploymentBytecodeManifestKind, HydratedBytecodePackage,
    HydratedServiceDependency,
};

mod package_types;
mod schema_closure;
mod synthetic;

use package_types::validate_package_type;
use schema_closure::validate_bytecode_schema_closure;
use synthetic::SyntheticCallbackIndex;

#[derive(Debug)]
pub(super) struct HydratedPackageManifests {
    ordinary_functions: BTreeMap<PackageExecutableCoordinate, String>,
    callable_functions: BTreeMap<PackageCallableId, String>,
    canonical_implementation_callables: CanonicalImplementationCallableIndex,
    synthetic_callbacks: SyntheticCallbackIndex,
    constant_roots: BTreeSet<String>,
}

#[derive(Debug)]
struct CanonicalImplementationCallableIndex {
    by_executable: BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
    by_function_key: BTreeMap<String, PackageCallableId>,
    by_callable: BTreeMap<PackageCallableId, String>,
}

impl HydratedPackageManifests {
    pub(super) fn checked(
        reference: &PackageArtifactRef,
        artifact: &PackageArtifact,
        bytecode: &ValidatedBytecodeArtifact,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        validate_header(reference, bytecode)?;
        let ordinary_functions = validate_function_origins(reference, artifact, bytecode)?;
        let callable_functions =
            validate_callable_manifests(reference, artifact, bytecode, &ordinary_functions)?;
        let canonical_implementation_callables = CanonicalImplementationCallableIndex::checked(
            reference,
            artifact,
            bytecode,
            &ordinary_functions,
        )?;
        let synthetic_callbacks = SyntheticCallbackIndex::checked(
            reference,
            artifact,
            bytecode,
            &ordinary_functions,
            &canonical_implementation_callables,
        )?;
        validate_statement_attribution_manifest(reference, artifact, bytecode)?;
        validate_actor_manifests(reference, artifact, bytecode, &callable_functions)?;
        validate_conformance_manifests(reference, bytecode, artifact, &callable_functions)?;
        let constant_roots = validate_constant_roots(reference, artifact, bytecode)?;
        Ok(Self {
            ordinary_functions,
            callable_functions,
            canonical_implementation_callables,
            synthetic_callbacks,
            constant_roots,
        })
    }

    pub(super) fn function_key_for_executable(
        &self,
        executable: &PackageExecutableCoordinate,
    ) -> Option<&str> {
        self.ordinary_functions.get(executable).map(String::as_str)
    }

    pub(super) fn function_key_for_callable(&self, callable: &PackageCallableId) -> Option<&str> {
        self.callable_functions.get(callable).map(String::as_str)
    }

    pub(super) fn canonical_implementation_callable_for_executable(
        &self,
        executable: &PackageExecutableCoordinate,
    ) -> Option<&PackageCallableId> {
        self.canonical_implementation_callables
            .by_executable
            .get(executable)
    }

    pub(super) fn canonical_implementation_callable_for_function_key(
        &self,
        function_key: &str,
    ) -> Option<&PackageCallableId> {
        self.canonical_implementation_callables
            .by_function_key
            .get(function_key)
    }

    pub(super) fn function_key_for_canonical_implementation_callable(
        &self,
        callable: &PackageCallableId,
    ) -> Option<&str> {
        self.canonical_implementation_callables
            .by_callable
            .get(callable)
            .map(String::as_str)
    }

    pub(super) fn function_key_for_synthetic_callback(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&str> {
        self.synthetic_callbacks
            .function_key_for_site(owner, site_ordinal)
    }

    pub(super) fn synthetic_callback_callable(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&PackageCallableId> {
        self.synthetic_callbacks
            .callable_for_site(owner, site_ordinal)
    }

    pub(super) fn function_key_for_synthetic_callback_callable(
        &self,
        callable: &PackageCallableId,
    ) -> Option<&str> {
        self.synthetic_callbacks.function_key_for_callable(callable)
    }

    pub(super) fn canonical_effect_callable_for_function_key(
        &self,
        function_key: &str,
    ) -> Option<&PackageCallableId> {
        self.canonical_implementation_callable_for_function_key(function_key)
            .or_else(|| self.synthetic_callbacks.callable_for_function(function_key))
    }
}

pub(super) fn validate_deployment_manifests(
    deployment: &ServiceDeployment,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    service_dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    validate_unique_package_ids(packages)?;
    validate_bytecode_schema_closure(packages)?;
    validate_deployment_entry_callables(deployment, packages)?;
    for package in packages.values() {
        validate_package_manifest_type_refs(package, deployment, packages)?;
        if package.has_bytecode() {
            validate_package_bytecode_refs(
                package,
                deployment,
                contracts,
                service_dependencies,
                packages,
            )?;
        }
    }
    Ok(())
}

fn validate_header(
    reference: &PackageArtifactRef,
    bytecode: &ValidatedBytecodeArtifact,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let artifact = bytecode.artifact();
    let view = bytecode.view();
    let opcode_fingerprint = skiff_artifact_model::bytecode::opcodes::opcode_table_fingerprint();
    if artifact.magic.as_str() != BYTECODE_MAGIC
        || artifact.schema_version.as_str() != BYTECODE_SCHEMA_VERSION
        || view.schema_version() != BYTECODE_SCHEMA_VERSION
        || view.schema_version() != artifact.schema_version.as_str()
        || artifact.isa_version.as_str() != BYTECODE_ISA_VERSION
        || view.isa_version() != BYTECODE_ISA_VERSION
        || view.isa_version() != artifact.isa_version.as_str()
        || artifact.opcode_table_fingerprint.as_str() != opcode_fingerprint.as_str()
        || view.opcode_table_fingerprint() != opcode_fingerprint.as_str()
        || view.opcode_table_fingerprint() != artifact.opcode_table_fingerprint.as_str()
        || &artifact.native_value_lifecycle_registry != native_value_lifecycle_registry_identity()
        || view.native_value_lifecycle_registry() != native_value_lifecycle_registry_identity()
        || view.native_value_lifecycle_registry() != &artifact.native_value_lifecycle_registry
        || &artifact.value_lifecycle_policy != value_lifecycle_policy_identity()
        || view.value_lifecycle_policy() != value_lifecycle_policy_identity()
        || view.value_lifecycle_policy() != &artifact.value_lifecycle_policy
        || &artifact.host_effect_registry != host_effect_registry_identity()
        || view.host_effect_registry() != host_effect_registry_identity()
        || view.host_effect_registry() != &artifact.host_effect_registry
        || &artifact.intrinsic_registry != intrinsic_registry_identity()
        || view.intrinsic_registry() != intrinsic_registry_identity()
        || view.intrinsic_registry() != &artifact.intrinsic_registry
        || view.bytecode_identity() != bytecode.reference().bytecode_identity.as_str()
        || artifact.bytecode_identity.as_str() != bytecode.reference().bytecode_identity.as_str()
        || view.bytecode_identity() != artifact.bytecode_identity.as_str()
    {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Header,
            "admitted v6 header/view/reference facts are not exact".to_string(),
        );
    }
    Ok(())
}

fn validate_statement_attribution_manifest(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let mut functions = bytecode
        .view()
        .functions()
        .iter()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    functions.sort_by(|left, right| left.origin.cmp(&right.origin));

    let derived = derive_bytecode_statement_manifest_identity(&artifact.package_id, &functions)
        .map_err(|error| {
            manifest_mismatch(
                reference,
                DeploymentBytecodeManifestKind::StatementAttribution,
                format!("statement attribution identity cannot be derived: {error}"),
            )
        })?;
    if derived != artifact.bytecode_statement_manifest_identity {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::StatementAttribution,
            format!(
                "package declares {}, but admitted bytecode functions derive {derived}",
                artifact.bytecode_statement_manifest_identity
            ),
        );
    }
    Ok(())
}

fn validate_function_origins(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
) -> Result<BTreeMap<PackageExecutableCoordinate, String>, DeploymentBytecodeHydrationError> {
    let file_owners = artifact
        .files
        .iter()
        .map(|file| (file.file_ir_identity.clone(), file.module_path.clone()))
        .collect::<BTreeSet<_>>();
    let mut ordinary = BTreeMap::new();
    for function in bytecode.view().functions() {
        let owner = function.origin.owner_executable();
        if !file_owners.contains(&(owner.file_ir_identity.clone(), owner.module_path.clone())) {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::FunctionOrigin,
                format!(
                    "function {:?} owner ({:?}, {:?}) is absent from the exact package file manifest",
                    function.function_key, owner.file_ir_identity, owner.module_path
                ),
            );
        }
        let Some(executable) = function.origin.ordinary_executable() else {
            continue;
        };
        if let Some(previous) = ordinary.insert(executable.clone(), function.function_key.clone()) {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::FunctionOrigin,
                format!(
                    "ordinary executable {executable:?} is owned by both {previous:?} and {:?}",
                    function.function_key
                ),
            );
        }
    }
    Ok(ordinary)
}

fn validate_callable_manifests(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
    ordinary: &BTreeMap<PackageExecutableCoordinate, String>,
) -> Result<BTreeMap<PackageCallableId, String>, DeploymentBytecodeHydrationError> {
    let functions = bytecode
        .view()
        .functions()
        .iter()
        .map(|function| (function.function_key.as_str(), function))
        .collect::<BTreeMap<_, _>>();
    let mut callables = BTreeMap::new();
    let mut callable_coordinates = BTreeSet::new();
    for (callable_id, fact) in &artifact.callable_links {
        if callable_id != &fact.callable_id
            || fact.target.callable_abi_id.as_str() != callable_id.as_str()
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "callableLinks key {callable_id} disagrees with nested id {} or target ABI {:?}",
                    fact.callable_id, fact.target.callable_abi_id
                ),
            );
        }
        let coordinate = target_coordinate(&fact.target);
        callable_coordinates.insert(coordinate.clone());
        let function_key = ordinary.get(&coordinate).ok_or_else(|| {
            manifest_mismatch(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "callable {callable_id} target {coordinate:?} has no ordinary bytecode function origin"
                ),
            )
        })?;
        let function = functions.get(function_key.as_str()).copied().ok_or_else(|| {
            manifest_mismatch(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "callable {callable_id} resolved function {function_key:?} is absent from the admitted view"
                ),
            )
        })?;
        let expects_self = matches!(
            fact.target.callable_kind,
            OperationCallableKind::ReceiverMethod | OperationCallableKind::ImplMethod
        );
        if function.self_type_ref.is_some() != expects_self {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::SelfType,
                format!(
                    "callable {callable_id} kind {:?} and function {function_key:?} selfTypeRef presence disagree",
                    fact.target.callable_kind
                ),
            );
        }
        let signature = callable_signature(reference, artifact, callable_id)?;
        if function.type_parameters.as_slice() != signature.type_params.as_slice()
            || function.frame_layout.parameter_slots.len() != signature.parameters.len()
            || function
                .frame_layout
                .parameter_slots
                .iter()
                .zip(&signature.parameters)
                .any(|(slot, parameter)| slot.mode != parameter.mode)
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "callable {callable_id} type parameters or parameter calling convention disagree with function {function_key:?}"
                ),
            );
        }
        if expects_self
            && signature
                .parameters
                .first()
                .is_none_or(|parameter| parameter.name != "self")
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::SelfType,
                format!("receiver callable {callable_id} has no exact leading self parameter"),
            );
        }
        if expects_self {
            validate_callable_self_type(reference, artifact, bytecode, function, signature)?;
        }
        callables.insert(callable_id.clone(), function_key.clone());
    }
    let ordinary_coordinates = ordinary.keys().cloned().collect::<BTreeSet<_>>();
    if callable_coordinates != ordinary_coordinates {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::FunctionOrigin,
            format!(
                "ordinary bytecode origins are {ordinary_coordinates:?}, but package callable targets are {callable_coordinates:?}"
            ),
        );
    }
    Ok(callables)
}

impl CanonicalImplementationCallableIndex {
    fn checked(
        reference: &PackageArtifactRef,
        artifact: &PackageArtifact,
        bytecode: &ValidatedBytecodeArtifact,
        ordinary: &BTreeMap<PackageExecutableCoordinate, String>,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        let mut index = Self {
            by_executable: BTreeMap::new(),
            by_function_key: BTreeMap::new(),
            by_callable: BTreeMap::new(),
        };
        for symbol in artifact.package_local_abi.implementation_symbols.values() {
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
                continue;
            };
            index.insert_implementation(reference, artifact, ordinary, callable_id)?;
        }
        index.validate_effect_summary_owners(reference, artifact, bytecode, ordinary)?;
        Ok(index)
    }

    fn insert_implementation(
        &mut self,
        reference: &PackageArtifactRef,
        artifact: &PackageArtifact,
        ordinary: &BTreeMap<PackageExecutableCoordinate, String>,
        callable_id: &PackageCallableId,
    ) -> Result<(), DeploymentBytecodeHydrationError> {
        let fact = artifact.callable_links.get(callable_id).ok_or_else(|| {
            manifest_mismatch(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!("canonical implementation callable {callable_id} has no callable link"),
            )
        })?;
        if !matches!(
            fact.target.callable_kind,
            OperationCallableKind::InternalFunction | OperationCallableKind::ImplMethod
        ) {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "implementation callable {callable_id} has non-implementation target kind {:?}",
                    fact.target.callable_kind
                ),
            );
        }
        let coordinate = target_coordinate(&fact.target);
        let function_key = ordinary.get(&coordinate).ok_or_else(|| {
            manifest_mismatch(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "canonical implementation callable {callable_id} targets {coordinate:?}, which is not an ordinary bytecode function origin"
                ),
            )
        })?;
        if let Some(previous) = self
            .by_executable
            .insert(coordinate.clone(), callable_id.clone())
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "ordinary executable {coordinate:?} has ambiguous canonical implementation owners {previous} and {callable_id}"
                ),
            );
        }
        if let Some(previous) = self
            .by_function_key
            .insert(function_key.clone(), callable_id.clone())
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "ordinary function {function_key:?} has ambiguous canonical implementation owners {previous} and {callable_id}"
                ),
            );
        }
        if let Some(previous) = self
            .by_callable
            .insert(callable_id.clone(), function_key.clone())
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Callable,
                format!(
                    "canonical implementation callable {callable_id} ambiguously owns functions {previous:?} and {function_key:?}"
                ),
            );
        }
        Ok(())
    }

    fn validate_effect_summary_owners(
        &self,
        reference: &PackageArtifactRef,
        artifact: &PackageArtifact,
        bytecode: &ValidatedBytecodeArtifact,
        ordinary: &BTreeMap<PackageExecutableCoordinate, String>,
    ) -> Result<(), DeploymentBytecodeHydrationError> {
        let functions = bytecode
            .view()
            .functions()
            .iter()
            .map(|function| (function.function_key.as_str(), function))
            .collect::<BTreeMap<_, _>>();
        for (coordinate, function_key) in ordinary {
            let canonical = self.by_executable.get(coordinate).ok_or_else(|| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::Callable,
                    format!(
                        "ordinary function {function_key:?} at {coordinate:?} has no canonical implementation callable owner"
                    ),
                )
            })?;
            let function = functions.get(function_key.as_str()).copied().ok_or_else(|| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::FunctionOrigin,
                    format!(
                        "ordinary function {function_key:?} is absent from the admitted bytecode view"
                    ),
                )
            })?;
            if &function.effect_summary_ref != canonical {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::Callable,
                    format!(
                        "ordinary function {function_key:?} effectSummaryRef {} is not its canonical implementation owner {canonical}",
                        function.effect_summary_ref
                    ),
                );
            }
            if !artifact.callable_semantic_facts.contains_key(canonical) {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::Callable,
                    format!(
                        "canonical implementation callable {canonical} for function {function_key:?} has no callableSemanticFacts row"
                    ),
                );
            }
        }
        Ok(())
    }
}

fn callable_signature<'a>(
    reference: &PackageArtifactRef,
    artifact: &'a PackageArtifact,
    callable: &PackageCallableId,
) -> Result<&'a PackageCallableSignature, DeploymentBytecodeHydrationError> {
    let mut matches = artifact
        .package_local_abi
        .public_symbols
        .values()
        .chain(artifact.package_local_abi.implementation_symbols.values())
        .filter_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id,
                signature,
            } if callable_id == callable => Some(signature),
            _ => None,
        });
    let selected = matches.next().ok_or_else(|| {
        manifest_mismatch(
            reference,
            DeploymentBytecodeManifestKind::Callable,
            format!("callable {callable} has no exact package-local ABI signature"),
        )
    })?;
    if matches.next().is_some() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Callable,
            format!("callable {callable} has ambiguous package-local ABI signatures"),
        );
    }
    Ok(selected)
}

fn validate_callable_self_type(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
    function: &skiff_artifact_model::bytecode::ValidatedFunction,
    signature: &PackageCallableSignature,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let Some(self_type_ref) = function.self_type_ref else {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::SelfType,
            format!("function {:?} has no selfTypeRef", function.function_key),
        );
    };
    let Some(BytecodePoolEntry::TypeRef { ty, .. }) =
        bytecode.view().pools().types.get(self_type_ref as usize)
    else {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::SelfType,
            format!(
                "function {:?} selfTypeRef has no exact type row",
                function.function_key
            ),
        );
    };
    let Some(PackageTypeRef::Local {
        local_type: expected,
    }) = signature.parameters.first().map(|parameter| &parameter.ty)
    else {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::SelfType,
            format!(
                "function {:?} receiver has no package-local ABI type",
                function.function_key
            ),
        );
    };
    let actual = normalize_path_free_type(reference, artifact, ty)?;
    if &actual != expected {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::SelfType,
            format!(
                "function {:?} receiver type disagrees with its package-local ABI",
                function.function_key
            ),
        );
    }
    Ok(())
}

fn normalize_path_free_type(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    ty: &TypeRefIr,
) -> Result<TypeRefIr, DeploymentBytecodeHydrationError> {
    Ok(match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|argument| normalize_path_free_type(reference, artifact, argument))
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::LocalType { type_index } => {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::SelfType,
                format!("receiver retains ownerless local type index {type_index}"),
            );
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PackageSymbol {
            symbol: package_symbol_for_export(
                reference,
                exact_publication_type_export(reference, artifact, module_path, *type_index)?,
            ),
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: package_symbol_for_export(
                reference,
                exact_service_symbol_type_export(reference, artifact, symbol)?,
            ),
        },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: normalize_path_free_nominal_base(reference, artifact, base)?,
            arguments: arguments
                .iter()
                .map(|argument| normalize_path_free_type(reference, artifact, argument))
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        normalize_path_free_type(reference, artifact, field)?,
                    ))
                })
                .collect::<Result<_, DeploymentBytecodeHydrationError>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| normalize_path_free_type(reference, artifact, item))
                .collect::<Result<_, _>>()?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(normalize_path_free_type(reference, artifact, inner)?),
        },
        TypeRefIr::Literal { value } => TypeRefIr::Literal {
            value: value.clone(),
        },
        TypeRefIr::TypeParam { name } => TypeRefIr::TypeParam { name: name.clone() },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|argument| normalize_path_free_type(reference, artifact, argument))
                    .collect::<Result<_, _>>()?,
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|parameter| {
                    Ok(skiff_artifact_model::FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: normalize_path_free_type(reference, artifact, &parameter.ty)?,
                    })
                })
                .collect::<Result<_, DeploymentBytecodeHydrationError>>()?,
            return_type: Box::new(normalize_path_free_type(reference, artifact, return_type)?),
        },
    })
}

fn normalize_path_free_nominal_base(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    base: &NominalTypeRefBaseIr,
) -> Result<NominalTypeRefBaseIr, DeploymentBytecodeHydrationError> {
    Ok(match base {
        NominalTypeRefBaseIr::LocalType { type_index } => {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::SelfType,
                format!("receiver retains ownerless local nominal type index {type_index}"),
            );
        }
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => NominalTypeRefBaseIr::PackageSymbol {
            symbol: package_symbol_for_export(
                reference,
                exact_publication_type_export(reference, artifact, module_path, *type_index)?,
            ),
        },
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => NominalTypeRefBaseIr::PackageSymbol {
            symbol: package_symbol_for_export(
                reference,
                exact_service_symbol_type_export(reference, artifact, symbol)?,
            ),
        },
        NominalTypeRefBaseIr::PackageSymbol { symbol } => NominalTypeRefBaseIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => NominalTypeRefBaseIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
    })
}

fn package_symbol_for_export(
    reference: &PackageArtifactRef,
    export: &skiff_artifact_model::TypeExport,
) -> PackageSymbolRef {
    PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: reference.package_id.clone(),
        },
        symbol_path: format!("{}.{}", export.file.module_path, export.symbol),
        abi_expectation: None,
    }
}

fn validate_actor_manifests(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
    callables: &BTreeMap<PackageCallableId, String>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let declared = artifact
        .implementation_links
        .types
        .values()
        .filter(|export| export.actor.is_some())
        .map(|export| (export.file.module_path.as_str(), export.symbol.as_str()))
        .collect::<BTreeSet<_>>();
    let implemented = artifact
        .actor_implementations
        .iter()
        .map(|row| (row.actor.module_path.as_str(), row.actor.symbol.as_str()))
        .collect::<BTreeSet<_>>();
    if declared != implemented || implemented.len() != artifact.actor_implementations.len() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Actor,
            "actor ABI roots and implementation manifests are not one-to-one".to_string(),
        );
    }
    for row in &artifact.actor_implementations {
        let actor_abi = actor_abi(reference, artifact, &row.actor)?;
        if actor_abi.abi.actor_name != row.actor.symbol {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Actor,
                format!(
                    "actor {} symbol and ABI actorName {:?} disagree",
                    row.actor.symbol_path(),
                    actor_abi.abi.actor_name
                ),
            );
        }
        let computed_abi =
            skiff_artifact_identity::actor_abi_identity(&actor_abi.abi).map_err(|error| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::Actor,
                    format!(
                        "actor {} ABI cannot be canonicalized: {error}",
                        row.actor.symbol_path()
                    ),
                )
            })?;
        if computed_abi != actor_abi.actor_abi_identity {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Actor,
                format!(
                    "actor {} declared ABI identity does not match its exact ABI",
                    row.actor.symbol_path()
                ),
            );
        }
        if row.actor_implementation_identity.as_str().trim().is_empty() {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Actor,
                format!(
                    "actor {} has an empty implementation identity",
                    row.actor.symbol_path()
                ),
            );
        }
        for method in &actor_abi.abi.public_methods {
            let computed = skiff_artifact_identity::actor_method_identity(
                &row.actor.module_path,
                &actor_abi.abi.actor_name,
                &method.name,
            )
            .map_err(|error| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::Actor,
                    format!(
                        "actor {} method {:?} identity cannot be derived: {error}",
                        row.actor.symbol_path(),
                        method.name
                    ),
                )
            })?;
            if computed != method.method_identity {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::Actor,
                    format!(
                        "actor {} method {:?} has a non-canonical identity",
                        row.actor.symbol_path(),
                        method.name
                    ),
                );
            }
        }
        let declared_methods = actor_abi
            .abi
            .public_methods
            .iter()
            .map(|method| method.method_identity.clone())
            .collect::<BTreeSet<_>>();
        let implemented_methods = row.methods.keys().cloned().collect::<BTreeSet<_>>();
        if declared_methods != implemented_methods {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::Actor,
                format!(
                    "actor {} ABI method identities do not equal its implementation manifest",
                    row.actor.symbol_path()
                ),
            );
        }
        match (&actor_abi.abi.create, &row.create) {
            (None, None) => {}
            (Some(_), Some(create)) => {
                let computed = skiff_artifact_identity::actor_method_identity(
                    &row.actor.module_path,
                    &actor_abi.abi.actor_name,
                    "create",
                )
                .map_err(|error| {
                    manifest_mismatch(
                        reference,
                        DeploymentBytecodeManifestKind::Actor,
                        format!(
                            "actor {} create identity cannot be derived: {error}",
                            row.actor.symbol_path()
                        ),
                    )
                })?;
                if computed != create.method_identity
                    || row.methods.contains_key(&create.method_identity)
                {
                    return manifest_error(
                        reference,
                        DeploymentBytecodeManifestKind::Actor,
                        format!(
                            "actor {} create binding has a non-canonical or public method identity",
                            row.actor.symbol_path()
                        ),
                    );
                }
                validate_receiver_callable(
                    reference,
                    bytecode,
                    callables,
                    &create.package_callable_id,
                    DeploymentBytecodeManifestKind::Actor,
                )?;
            }
            _ => {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::Actor,
                    format!(
                        "actor {} create ABI and implementation presence disagree",
                        row.actor.symbol_path()
                    ),
                );
            }
        }
        for callable in row.methods.values() {
            validate_receiver_callable(
                reference,
                bytecode,
                callables,
                callable,
                DeploymentBytecodeManifestKind::Actor,
            )?;
        }
    }
    Ok(())
}

fn validate_conformance_manifests(
    reference: &PackageArtifactRef,
    bytecode: &ValidatedBytecodeArtifact,
    artifact: &PackageArtifact,
    callables: &BTreeMap<PackageCallableId, String>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for row in &artifact.local_interface_conformances {
        for callable in &row.methods {
            validate_receiver_callable(
                reference,
                bytecode,
                callables,
                callable,
                DeploymentBytecodeManifestKind::InterfaceConformance,
            )?;
        }
    }
    Ok(())
}

fn validate_receiver_callable(
    reference: &PackageArtifactRef,
    bytecode: &ValidatedBytecodeArtifact,
    callables: &BTreeMap<PackageCallableId, String>,
    callable: &PackageCallableId,
    kind: DeploymentBytecodeManifestKind,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let function_key = callables.get(callable).ok_or_else(|| {
        manifest_mismatch(
            reference,
            kind,
            format!("callable {callable} is absent from the checked callable manifest"),
        )
    })?;
    let function = bytecode
        .view()
        .functions()
        .iter()
        .find(|function| &function.function_key == function_key)
        .ok_or_else(|| {
            manifest_mismatch(
                reference,
                kind,
                format!(
                    "receiver callable {callable} resolved function {function_key:?} is absent from the admitted view"
                ),
            )
        })?;
    if function.self_type_ref.is_none() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::SelfType,
            format!("receiver callable {callable} targets unbound function {function_key:?}"),
        );
    }
    Ok(())
}

fn actor_abi<'a>(
    reference: &PackageArtifactRef,
    artifact: &'a PackageArtifact,
    actor: &skiff_artifact_model::ServiceSymbolRef,
) -> Result<&'a skiff_artifact_model::PackageActorAbi, DeploymentBytecodeHydrationError> {
    let source_path = actor.symbol_path();
    let mut matches = artifact
        .implementation_links
        .types
        .values()
        .filter(|export| {
            export.file.module_path == actor.module_path && export.symbol == actor.symbol
        })
        .filter_map(|export| export.actor.as_ref());
    let selected = matches.next().ok_or_else(|| {
        manifest_mismatch(
            reference,
            DeploymentBytecodeManifestKind::Actor,
            format!(
                "actor {} has no path-free implementation type ABI",
                actor.symbol_path()
            ),
        )
    })?;
    if matches.next().is_some() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Actor,
            format!(
                "actor {} has ambiguous implementation type ABIs",
                actor.symbol_path()
            ),
        );
    }
    let Some(PackageLocalAbiSymbol::Type {
        actor: Some(local_abi),
        ..
    }) = artifact
        .package_local_abi
        .implementation_symbols
        .get(&source_path)
    else {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Actor,
            format!("actor {source_path} has no exact package-local ABI row"),
        );
    };
    if local_abi != selected {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::Actor,
            format!("actor {source_path} package-local and implementation-link ABI facts disagree"),
        );
    }
    Ok(selected)
}

fn validate_constant_roots(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
) -> Result<BTreeSet<String>, DeploymentBytecodeHydrationError> {
    let file_owners = artifact
        .files
        .iter()
        .map(|file| (file.file_ir_identity.as_str(), file.module_path.as_str()))
        .collect::<BTreeSet<_>>();
    let mut coordinates = BTreeMap::<(String, String, u32), String>::new();
    for (source_path, export) in &artifact.implementation_links.constants {
        if export.symbol.is_empty()
            || !file_owners.contains(&(
                export.file.file_ir_identity.as_str(),
                export.file.module_path.as_str(),
            ))
        {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::ConstantRoot,
                format!(
                    "constant index {} has no canonical symbol or exact package file owner",
                    export.const_index
                ),
            );
        }
        let coordinate = (
            export.file.file_ir_identity.clone(),
            export.file.module_path.clone(),
            export.const_index,
        );
        let root = format!("{}.{}", export.file.module_path, export.symbol);
        if source_path != &root {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::ConstantRoot,
                format!(
                    "implementation constant key {source_path:?} disagrees with canonical root {root:?}"
                ),
            );
        }
        if let Some(previous) = coordinates.insert(coordinate, root.clone()) {
            if previous != root {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::ConstantRoot,
                    format!(
                        "one implementation constant coordinate has roots {previous:?} and {root:?}"
                    ),
                );
            }
        }
    }
    let mut root_owners = BTreeMap::new();
    for (coordinate, root) in coordinates {
        if let Some(previous) = root_owners.insert(root.clone(), coordinate.clone()) {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::ConstantRoot,
                format!(
                    "constant root {root:?} is owned by coordinates {previous:?} and {coordinate:?}"
                ),
            );
        }
    }
    let expected = root_owners.into_keys().collect::<BTreeSet<_>>();
    let actual = bytecode
        .view()
        .constant_roots()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected != actual {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!("constant root set is {actual:?}, expected {expected:?}"),
        );
    }
    Ok(actual)
}

fn validate_unique_package_ids(
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let mut owners = BTreeMap::<&str, &PackageArtifactRef>::new();
    for package in packages.values() {
        if let Some(previous) =
            owners.insert(package.reference().package_id.as_str(), package.reference())
        {
            return manifest_error(
                package.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "package id {:?} is owned by both {} and {}",
                    package.reference().package_id,
                    previous.package_build_id,
                    package.reference().package_build_id
                ),
            );
        }
    }
    Ok(())
}

fn validate_deployment_entry_callables(
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let implementation = packages
        .get(&deployment.implementation.package_build_id)
        .filter(|package| package.reference() == &deployment.implementation)
        .ok_or_else(|| {
            manifest_mismatch(
                &deployment.implementation,
                DeploymentBytecodeManifestKind::PackageReference,
                "implementation package is absent from the exact hydrated closure".to_string(),
            )
        })?;
    for binding in &deployment.operation_bindings {
        require_deployment_callable(
            implementation,
            &binding.package_callable_id,
            &format!("deployment operation {}", binding.contract_operation_id),
        )?;
    }
    for (entry_key, entry) in &deployment.gateway_entries {
        for (role, callable) in [
            ("handler", entry.handler.as_ref()),
            ("pre", entry.pre.as_ref()),
            ("guard", entry.guard.as_ref()),
            ("closeHandler", entry.close_handler.as_ref()),
        ] {
            if let Some(callable) = callable {
                require_deployment_callable(
                    implementation,
                    callable,
                    &format!("gateway entry {entry_key} {role}"),
                )?;
            }
        }
    }
    Ok(())
}

fn require_deployment_callable(
    implementation: &HydratedBytecodePackage,
    callable: &PackageCallableId,
    owner: &str,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if implementation.function_key_for_callable(callable).is_none() {
        return manifest_error(
            implementation.reference(),
            DeploymentBytecodeManifestKind::Callable,
            format!("{owner} targets missing bytecode callable {callable}"),
        );
    }
    Ok(())
}

fn validate_package_manifest_type_refs(
    package: &HydratedBytecodePackage,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for row in &package.artifact().local_interface_conformances {
        validate_type_ref(package, &row.receiver, deployment, packages)?;
        validate_interface_ref(package, &row.interface, deployment, packages)?;
    }
    for row in &package.artifact().actor_implementations {
        let actor = actor_abi(package.reference(), package.artifact(), &row.actor)?;
        validate_type_ref(package, &actor.abi.actor_id_type, deployment, packages)?;
        for field in &actor.abi.fields {
            validate_type_ref(package, &field.ty, deployment, packages)?;
        }
        if let Some(create) = &actor.abi.create {
            for parameter in &create.parameters {
                validate_type_ref(package, &parameter.ty, deployment, packages)?;
            }
        }
        for method in &actor.abi.public_methods {
            for parameter in &method.parameters {
                validate_type_ref(package, &parameter.ty, deployment, packages)?;
            }
            validate_type_ref(package, &method.return_type, deployment, packages)?;
        }
    }
    Ok(())
}

fn validate_package_bytecode_refs(
    package: &HydratedBytecodePackage,
    deployment: &ServiceDeployment,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    service_dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let Some(bytecode) = package.bytecode() else {
        return Ok(());
    };
    let view = bytecode.view();
    for entry in &view.pools().types {
        if let BytecodePoolEntry::TypeRef { ty, .. } = entry {
            validate_type_ref(package, ty, deployment, packages)?;
        }
    }
    for entry in &view.pools().constants {
        let BytecodePoolEntry::ConstantRef {
            reference, plan, ..
        } = entry
        else {
            continue;
        };
        validate_plan(package, plan, deployment, packages)?;
        if let BytecodeConstantRef::PackageSymbol { symbol } = reference {
            validate_package_constant(package, symbol, deployment, packages)?;
        }
    }
    for entry in &view.pools().shapes {
        let BytecodePoolEntry::ShapeRef { shape } = entry else {
            continue;
        };
        for field in &shape.fields {
            validate_plan(package, &field.plan, deployment, packages)?;
        }
    }
    for entry in &view.pools().resume {
        let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry else {
            continue;
        };
        for plan in &descriptor.result_plans {
            validate_plan(package, plan, deployment, packages)?;
        }
    }
    for entry in &view.pools().callback_capture {
        let BytecodePoolEntry::CallbackCaptureLayout(layout) = entry else {
            continue;
        };
        for capture in &layout.captures {
            validate_plan(package, &capture.plan, deployment, packages)?;
        }
    }
    for entry in &view.pools().effects {
        let BytecodePoolEntry::HostEffectRef(effect) = entry else {
            continue;
        };
        validate_host_signature(package, &effect.signature, deployment, packages)?;
    }
    for function in view.functions() {
        for parameter in &function.frame_layout.parameter_slots {
            validate_plan(package, &parameter.plan, deployment, packages)?;
        }
        for plan in &function.frame_layout.result_plans {
            validate_plan(package, plan, deployment, packages)?;
        }
        for plan in &function.frame_layout.slot_plans {
            validate_plan(package, plan, deployment, packages)?;
        }
        for relocation in &function.relocations {
            validate_relocation(
                package,
                relocation,
                deployment,
                contracts,
                service_dependencies,
                packages,
            )?;
        }
    }
    Ok(())
}

fn validate_relocation(
    package: &HydratedBytecodePackage,
    relocation: &BytecodeRelocation,
    deployment: &ServiceDeployment,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    service_dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match relocation {
        BytecodeRelocation::LocalExecutableRef {
            function_key,
            specialization,
        } => {
            validate_specialization(package, specialization, deployment, packages)?;
            let Some(package_bytecode) = package.bytecode() else {
                return manifest_error(
                    package.reference(),
                    DeploymentBytecodeManifestKind::Callable,
                    "local callable target uses a type-only caller package".to_string(),
                );
            };
            let target = package_bytecode
                .view()
                .functions()
                .iter()
                .find(|function| &function.function_key == function_key)
                .ok_or_else(|| {
                    manifest_mismatch(
                        package.reference(),
                        DeploymentBytecodeManifestKind::Callable,
                        format!("local callable target {function_key:?} is absent"),
                    )
                })?;
            validate_specialization_shape(
                package.reference(),
                specialization,
                target.type_parameters.len(),
                target.self_type_ref.is_some(),
                &format!("local callable {function_key:?}"),
            )?;
        }
        BytecodeRelocation::PackageCallableRef {
            package_ref,
            package_callable_id,
            specialization,
        } => {
            validate_specialization(package, specialization, deployment, packages)?;
            if !matches!(package_ref, PackageRefIr::Dependency { .. }) {
                return manifest_error(
                    package.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!(
                        "package callable {package_callable_id} does not retain a caller-relative dependency alias"
                    ),
                );
            }
            let target = resolve_package_ref(package, package_ref, deployment, packages)?;
            let function_key = target
                .function_key_for_callable(package_callable_id)
                .ok_or_else(|| {
                    manifest_mismatch(
                        package.reference(),
                        DeploymentBytecodeManifestKind::Callable,
                        format!(
                            "package callable {package_callable_id} is absent from target package {}",
                            target.reference().package_build_id
                        ),
                    )
                })?;
            let Some(target_bytecode) = target.bytecode() else {
                return manifest_error(
                    package.reference(),
                    DeploymentBytecodeManifestKind::Callable,
                    format!(
                        "package callable {package_callable_id} target {} is type-only",
                        target.reference().package_build_id
                    ),
                );
            };
            let function = target_bytecode
                .view()
                .functions()
                .iter()
                .find(|function| function.function_key == function_key)
                .ok_or_else(|| {
                    manifest_mismatch(
                        package.reference(),
                        DeploymentBytecodeManifestKind::Callable,
                        format!(
                            "package callable {package_callable_id} resolved function {function_key:?} is absent from the admitted target view"
                        ),
                    )
                })?;
            validate_specialization_shape(
                package.reference(),
                specialization,
                function.type_parameters.len(),
                function.self_type_ref.is_some(),
                &format!("package callable {package_callable_id}"),
            )?;
        }
        BytecodeRelocation::ServiceOperationRef { service_call } => {
            validate_service_operation(
                package,
                service_call.service_requirement_slot,
                &service_call.contract_operation_id,
                &service_call.expected_protocol_identity,
                contracts,
                service_dependencies,
            )?;
        }
        BytecodeRelocation::ActorMethodRef {
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        } => validate_actor_relocation(
            package,
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        )?,
        BytecodeRelocation::InterfaceRequirementRef { interface } => {
            validate_interface_ref(package, interface, deployment, packages)?;
        }
        BytecodeRelocation::LocalInterfaceRef { interface } => {
            validate_interface_ref(package, &interface.interface, deployment, packages)?;
            validate_type_ref(package, &interface.concrete_type, deployment, packages)?;
            for method in &interface.methods {
                validate_interface_signature(package, &method.signature, deployment, packages)?;
            }
            validate_local_interface_relocation(package, interface)?;
        }
        BytecodeRelocation::RemoteInterfaceRef { interface } => {
            validate_interface_ref(package, &interface.interface, deployment, packages)?;
            for method in &interface.methods {
                validate_interface_signature(package, &method.signature, deployment, packages)?;
            }
            validate_remote_interface(package, interface, contracts, service_dependencies)?;
        }
        BytecodeRelocation::HostEffectRef(effect) => {
            validate_host_signature(package, &effect.signature, deployment, packages)?;
        }
        BytecodeRelocation::IntrinsicRef { intrinsic } => {
            validate_host_signature(package, &intrinsic.signature, deployment, packages)?;
        }
        BytecodeRelocation::TypeRef { ty } => {
            validate_type_ref(package, ty, deployment, packages)?;
        }
        BytecodeRelocation::SyntheticCallbackRef { .. }
        | BytecodeRelocation::ShapeRef { .. }
        | BytecodeRelocation::FrozenConstantRef { .. } => {}
    }
    Ok(())
}

fn validate_package_constant(
    caller: &HydratedBytecodePackage,
    symbol: &PackageSymbolRef,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if !matches!(&symbol.package, PackageRefIr::Dependency { .. }) {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "package constant {:?} does not retain a caller-relative dependency alias",
                symbol.symbol_path
            ),
        );
    }
    let target = resolve_package_ref(caller, &symbol.package, deployment, packages)?;
    validate_abi_expectation(caller, symbol, target)?;
    if !matches!(
        target
            .artifact()
            .package_local_abi
            .public_symbols
            .get(&symbol.symbol_path)
            .or_else(|| {
                target
                    .artifact()
                    .package_local_abi
                    .implementation_symbols
                    .get(&symbol.symbol_path)
            }),
        Some(PackageLocalAbiSymbol::Constant { .. })
    ) {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} has no exact target package ABI symbol",
                symbol.symbol_path
            ),
        );
    }
    let export = target
        .artifact()
        .implementation_links
        .constants
        .get(&symbol.symbol_path)
        .ok_or_else(|| {
            manifest_mismatch(
                caller.reference(),
                DeploymentBytecodeManifestKind::ConstantRoot,
                format!(
                    "package constant {:?} is absent from target package {}",
                    symbol.symbol_path,
                    target.reference().package_build_id
                ),
            )
        })?;
    let root = format!("{}.{}", export.file.module_path, export.symbol);
    let Some(target_bytecode) = target.bytecode() else {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} resolves to type-only package {}",
                symbol.symbol_path,
                target.reference().package_build_id
            ),
        );
    };
    let Some(pool_index) = target_bytecode.view().constant_roots().get(&root) else {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} resolves to missing admitted root {root:?}",
                symbol.symbol_path
            ),
        );
    };
    if !target
        .manifests
        .as_ref()
        .is_some_and(|manifests| manifests.constant_roots.contains(&root))
    {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} is outside the checked root manifest",
                symbol.symbol_path
            ),
        );
    }
    let pools = target_bytecode.view().pools();
    let Some(BytecodePoolEntry::ConstantRef {
        reference: BytecodeConstantRef::LocalNode { node_index },
        type_ref,
        ..
    }) = pools.constants.get(*pool_index as usize)
    else {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} root does not select an exact local node",
                symbol.symbol_path
            ),
        );
    };
    let Some(BytecodePoolEntry::TypeRef { .. }) = pools.types.get(*type_ref as usize) else {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} root has no exact type row",
                symbol.symbol_path
            ),
        );
    };
    if target_bytecode
        .view()
        .frozen_constant_graph()
        .nodes
        .get(*node_index as usize)
        .is_none()
    {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::ConstantRoot,
            format!(
                "package constant {:?} root frozen node disagrees with its exact owner",
                symbol.symbol_path
            ),
        );
    }
    Ok(())
}

fn validate_actor_relocation(
    package: &HydratedBytecodePackage,
    actor: &skiff_artifact_model::ServiceSymbolRef,
    actor_abi_identity: &skiff_artifact_model::ActorAbiIdentity,
    actor_implementation_identity: &skiff_artifact_model::ActorImplementationIdentity,
    method_identity: &skiff_artifact_model::ActorMethodIdentity,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let row = package
        .artifact()
        .actor_implementations
        .iter()
        .find(|row| {
            &row.actor == actor
                && &row.actor_implementation_identity == actor_implementation_identity
        })
        .ok_or_else(|| {
            manifest_mismatch(
                package.reference(),
                DeploymentBytecodeManifestKind::Actor,
                format!(
                    "actor {} implementation {:?} is absent",
                    actor.symbol_path(),
                    actor_implementation_identity
                ),
            )
        })?;
    let abi = actor_abi(package.reference(), package.artifact(), actor)?;
    if &abi.actor_abi_identity != actor_abi_identity {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::Actor,
            format!("actor {} ABI identity does not match", actor.symbol_path()),
        );
    }
    let callable = row.methods.get(method_identity).ok_or_else(|| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::Actor,
            format!(
                "actor {} method {:?} is absent from its implementation manifest",
                actor.symbol_path(),
                method_identity
            ),
        )
    })?;
    let Some(bytecode) = package.bytecode() else {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::Actor,
            "actor relocation targets a type-only package".to_string(),
        );
    };
    let Some(manifests) = package.manifests.as_ref() else {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::Actor,
            "actor relocation targets a package without bytecode manifests".to_string(),
        );
    };
    validate_receiver_callable(
        package.reference(),
        bytecode,
        &manifests.callable_functions,
        callable,
        DeploymentBytecodeManifestKind::Actor,
    )
}

fn validate_local_interface_relocation(
    package: &HydratedBytecodePackage,
    interface: &skiff_artifact_model::LocalInterfaceRef,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for method in &interface.methods {
        let expected = skiff_artifact_identity::canonical_interface_method_abi_id(
            &interface.interface,
            &method.method_name,
        );
        if method.method_abi_id != expected {
            return manifest_error(
                package.reference(),
                DeploymentBytecodeManifestKind::InterfaceConformance,
                format!(
                    "local interface {:?} slot {} has non-canonical method ABI identity",
                    interface.interface, method.slot
                ),
            );
        }
    }
    let mut matches = package
        .artifact()
        .local_interface_conformances
        .iter()
        .filter(|row| row.interface == interface.interface)
        .filter(|row| {
            row.methods.len() == interface.methods.len()
                && row.methods.iter().enumerate().all(|(slot, callable)| {
                    interface.methods.get(slot).is_some_and(|method| {
                        method.slot == slot as u32
                            && package.function_key_for_callable(callable)
                                == Some(method.function_key.as_str())
                    })
                })
        })
        .filter(|row| !row.type_parameters.is_empty() || row.receiver == interface.concrete_type);
    if matches.next().is_none() || matches.next().is_some() {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::InterfaceConformance,
            format!(
                "local interface {:?} method table has no unique exact conformance authority",
                interface.interface
            ),
        );
    }
    Ok(())
}

fn validate_remote_interface(
    package: &HydratedBytecodePackage,
    interface: &skiff_artifact_model::RemoteInterfaceRef,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let key = ServiceRequirementKey {
        caller_package_build_id: package.reference().package_build_id.clone(),
        service_requirement_slot: interface.service_requirement_slot,
    };
    let dependency = dependencies.get(&key).ok_or_else(|| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::RemoteInterface,
            format!("remote interface references unresolved symbolic service slot {key:?}"),
        )
    })?;
    if dependency.contract().service_protocol_identity != interface.callee_protocol_identity {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::RemoteInterface,
            "remote interface protocol identity disagrees with the selected consumer contract"
                .to_string(),
        );
    }
    let contract = contracts.get(dependency.contract()).ok_or_else(|| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::RemoteInterface,
            "selected consumer contract is absent from hydration".to_string(),
        )
    })?;
    let instance = contract
        .public_instances
        .get(&interface.public_instance_key)
        .ok_or_else(|| {
            manifest_mismatch(
                package.reference(),
                DeploymentBytecodeManifestKind::RemoteInterface,
                format!(
                    "public instance {:?} is absent from the selected consumer contract",
                    interface.public_instance_key
                ),
            )
        })?;
    let declared = instance
        .interfaces
        .iter()
        .find(|declared| declared.interface == interface.interface)
        .ok_or_else(|| {
            manifest_mismatch(
                package.reference(),
                DeploymentBytecodeManifestKind::RemoteInterface,
                format!(
                    "public instance {:?} does not declare interface {:?}",
                    interface.public_instance_key, interface.interface
                ),
            )
        })?;
    if declared.methods.len() != interface.methods.len()
        || declared.methods.iter().enumerate().any(|(slot, expected)| {
            interface.methods.get(slot).is_none_or(|actual| {
                actual.slot != slot as u32
                    || actual.method_abi_id != expected.method_abi_id
                    || actual.contract_operation_id != expected.contract_operation_id
                    || !dependency
                        .used_operations()
                        .contains(&actual.contract_operation_id)
                    || !contract
                        .operations
                        .contains_key(&actual.contract_operation_id)
            })
        })
    {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::RemoteInterface,
            format!(
                "remote interface {:?} method table disagrees with provider-free contract facts",
                interface.interface
            ),
        );
    }
    Ok(())
}

fn validate_service_operation(
    package: &HydratedBytecodePackage,
    slot: u32,
    operation: &ContractOperationId,
    protocol: &skiff_artifact_model::ServiceProtocolIdentity,
    contracts: &BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    dependencies: &BTreeMap<ServiceRequirementKey, HydratedServiceDependency>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let key = ServiceRequirementKey {
        caller_package_build_id: package.reference().package_build_id.clone(),
        service_requirement_slot: slot,
    };
    let dependency = dependencies.get(&key).ok_or_else(|| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::ServiceOperation,
            format!("bytecode references unresolved symbolic service slot {key:?}"),
        )
    })?;
    let contract = contracts.get(dependency.contract()).ok_or_else(|| {
        manifest_mismatch(
            package.reference(),
            DeploymentBytecodeManifestKind::ServiceOperation,
            "selected symbolic service contract is absent from hydration".to_string(),
        )
    })?;
    if &dependency.contract().service_protocol_identity != protocol
        || !dependency.used_operations().contains(operation)
        || !contract.operations.contains_key(operation)
    {
        return manifest_error(
            package.reference(),
            DeploymentBytecodeManifestKind::ServiceOperation,
            format!(
                "service slot {key:?} does not authorize operation {operation} with protocol {protocol}"
            ),
        );
    }
    Ok(())
}

fn validate_specialization(
    package: &HydratedBytecodePackage,
    specialization: &BytecodeSpecialization,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for ty in &specialization.type_arguments {
        validate_type_ref(package, ty, deployment, packages)?;
    }
    if let Some(receiver) = &specialization.concrete_receiver {
        validate_type_ref(package, receiver, deployment, packages)?;
    }
    Ok(())
}

fn validate_specialization_shape(
    package: &PackageArtifactRef,
    specialization: &BytecodeSpecialization,
    type_parameter_count: usize,
    expects_receiver: bool,
    target: &str,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if specialization.type_arguments.len() != type_parameter_count
        || specialization.concrete_receiver.is_some() != expects_receiver
    {
        return manifest_error(
            package,
            DeploymentBytecodeManifestKind::SelfType,
            format!("{target} specialization does not match its exact function manifest"),
        );
    }
    Ok(())
}

fn validate_host_signature(
    package: &HydratedBytecodePackage,
    signature: &HostEffectSignature,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for ty in signature
        .parameter_types
        .iter()
        .chain(&signature.result_types)
    {
        validate_type_ref(package, ty, deployment, packages)?;
    }
    for plan in signature
        .parameter_plans
        .iter()
        .chain(&signature.result_plans)
    {
        validate_plan(package, plan, deployment, packages)?;
    }
    Ok(())
}

fn validate_interface_signature(
    package: &HydratedBytecodePackage,
    signature: &InterfaceMethodSlotSignatureIr,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for parameter in &signature.params {
        validate_type_ref(package, &parameter.ty, deployment, packages)?;
    }
    validate_type_ref(package, &signature.return_type, deployment, packages)
}

fn validate_interface_ref(
    package: &HydratedBytecodePackage,
    interface: &InterfaceInstantiationRef,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    for argument in &interface.canonical_type_args {
        validate_type_ref(package, argument, deployment, packages)?;
    }
    Ok(())
}

fn validate_plan(
    package: &HydratedBytecodePackage,
    plan: &ValueTransferPlan,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if let ValueTransferPlan::FromType { ty } = plan {
        validate_type_ref(package, ty, deployment, packages)?;
    }
    Ok(())
}

fn validate_type_ref(
    caller: &HydratedBytecodePackage,
    ty: &TypeRefIr,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            validate_package_type(caller, symbol, deployment, packages)?;
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            validate_nominal_base(caller, base, deployment, packages)?;
            for argument in arguments {
                validate_type_ref(caller, argument, deployment, packages)?;
            }
        }
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                validate_type_ref(caller, argument, deployment, packages)?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                validate_type_ref(caller, field, deployment, packages)?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                validate_type_ref(caller, item, deployment, packages)?;
            }
        }
        TypeRefIr::Nullable { inner } => {
            validate_type_ref(caller, inner, deployment, packages)?;
        }
        TypeRefIr::AnyInterface { interface } => {
            validate_interface_ref(caller, interface, deployment, packages)?;
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                validate_type_ref(caller, &parameter.ty, deployment, packages)?;
            }
            validate_type_ref(caller, return_type, deployment, packages)?;
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => validate_publication_type(caller, module_path, *type_index)?,
        TypeRefIr::ServiceSymbol { symbol } => {
            validate_service_symbol_type(caller, symbol)?;
        }
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => validate_package_schema_type(
            caller,
            package_id,
            stable_schema_key,
            package_schema_type_id,
            packages,
        )?,
        TypeRefIr::LocalType { type_index } => {
            return manifest_error(
                caller.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "bytecode retains ownerless local type index {type_index}; a path-free package manifest is required"
                ),
            );
        }
        TypeRefIr::DbObjectSymbol { symbol } => {
            return manifest_error(
                caller.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "bytecode DB object {} has no self-contained package manifest",
                    symbol.symbol_path()
                ),
            );
        }
        TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn validate_nominal_base(
    caller: &HydratedBytecodePackage,
    base: &NominalTypeRefBaseIr,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match base {
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            validate_package_type(caller, symbol, deployment, packages)
        }
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => validate_publication_type(caller, module_path, *type_index),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            validate_service_symbol_type(caller, symbol)
        }
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => validate_package_schema_type(
            caller,
            package_id,
            stable_schema_key,
            package_schema_type_id,
            packages,
        ),
        NominalTypeRefBaseIr::LocalType { type_index } => manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "bytecode retains ownerless local nominal type index {type_index}; a path-free package manifest is required"
            ),
        ),
    }
}

fn validate_publication_type(
    caller: &HydratedBytecodePackage,
    module_path: &str,
    type_index: u32,
) -> Result<(), DeploymentBytecodeHydrationError> {
    exact_publication_type_export(
        caller.reference(),
        caller.artifact(),
        module_path,
        type_index,
    )
    .map(|_| ())
}

fn exact_publication_type_export<'a>(
    reference: &PackageArtifactRef,
    artifact: &'a PackageArtifact,
    module_path: &str,
    type_index: u32,
) -> Result<&'a skiff_artifact_model::TypeExport, DeploymentBytecodeHydrationError> {
    let mut matches = artifact
        .implementation_links
        .types
        .values()
        .filter(|export| export.file.module_path == module_path && export.type_index == type_index);
    let selected = matches.next().ok_or_else(|| {
        manifest_mismatch(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "publication type {module_path}#{type_index} has no path-free implementation link"
            ),
        )
    })?;
    if matches.any(|candidate| candidate != selected) {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "publication type {module_path}#{type_index} has conflicting implementation links"
            ),
        );
    }
    if selected.symbol.is_empty() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!("publication type {module_path}#{type_index} has no canonical source symbol"),
        );
    }
    Ok(selected)
}

fn validate_service_symbol_type(
    caller: &HydratedBytecodePackage,
    symbol: &skiff_artifact_model::ServiceSymbolRef,
) -> Result<(), DeploymentBytecodeHydrationError> {
    exact_service_symbol_type_export(caller.reference(), caller.artifact(), symbol).map(|_| ())
}

fn exact_service_symbol_type_export<'a>(
    reference: &PackageArtifactRef,
    artifact: &'a PackageArtifact,
    symbol: &skiff_artifact_model::ServiceSymbolRef,
) -> Result<&'a skiff_artifact_model::TypeExport, DeploymentBytecodeHydrationError> {
    let mut matches = artifact
        .implementation_links
        .types
        .values()
        .filter(|export| {
            export.file.module_path == symbol.module_path && export.symbol == symbol.symbol
        });
    let selected = matches.next().ok_or_else(|| {
        manifest_mismatch(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "service type {} has no path-free implementation link",
                symbol.symbol_path()
            ),
        )
    })?;
    if matches.any(|candidate| candidate != selected) {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "service type {} has conflicting implementation links",
                symbol.symbol_path()
            ),
        );
    }
    if selected.symbol.is_empty() {
        return manifest_error(
            reference,
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "service type {} has no canonical source symbol",
                symbol.symbol_path()
            ),
        );
    }
    Ok(selected)
}

fn validate_package_schema_type(
    caller: &HydratedBytecodePackage,
    package_id: &str,
    stable_schema_key: &str,
    type_id: &skiff_artifact_model::PackageSchemaTypeId,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let mut matches = packages
        .values()
        .filter(|package| package.reference().package_id.as_str() == package_id);
    let owner = matches.next().ok_or_else(|| {
        manifest_mismatch(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!("package schema type {package_id}:{type_id} has no exact package owner"),
        )
    })?;
    if matches.next().is_some()
        || owner
            .artifact()
            .bytecode_schema_records
            .get(type_id)
            .is_none_or(|record| {
                record.package_id.as_str() != package_id
                    || record.stable_schema_key.as_str() != stable_schema_key
                    || &record.package_schema_type_id != type_id
            })
    {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "package schema type {package_id}:{stable_schema_key}:{type_id} has ambiguous or mismatched bytecode descriptor authority"
            ),
        );
    }
    Ok(())
}

fn resolve_package_ref<'a>(
    caller: &HydratedBytecodePackage,
    package_ref: &PackageRefIr,
    deployment: &ServiceDeployment,
    packages: &'a BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<&'a HydratedBytecodePackage, DeploymentBytecodeHydrationError> {
    match package_ref {
        PackageRefIr::PackageId { package_id } => {
            let mut matches = packages
                .values()
                .filter(|package| package.reference().package_id == *package_id);
            let selected = matches.next().ok_or_else(|| {
                manifest_mismatch(
                    caller.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!("package id {package_id:?} is absent from the exact closure"),
                )
            })?;
            if matches.next().is_some() {
                return manifest_error(
                    caller.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!("package id {package_id:?} is ambiguous in the exact closure"),
                );
            }
            Ok(selected)
        }
        PackageRefIr::Dependency { dependency_ref } => {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: caller.reference().package_build_id.clone(),
                package_requirement_alias: dependency_ref.clone(),
            };
            let mut bindings = deployment
                .package_bindings
                .iter()
                .filter(|binding| binding.key == key);
            let binding = bindings.next().ok_or_else(|| {
                manifest_mismatch(
                    caller.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!("dependency alias {dependency_ref:?} has no exact deployment binding"),
                )
            })?;
            if bindings.next().is_some() {
                return manifest_error(
                    caller.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!("dependency alias {dependency_ref:?} has duplicate bindings"),
                );
            }
            let requirement = exact_package_requirement(caller, dependency_ref)?;
            if binding.package.package_id != requirement.package_id
                || binding.package.package_version != requirement.exact_version
                || binding.package.package_local_abi_identity != requirement.expected_local_abi
                || requirement
                    .expected_package_build
                    .as_ref()
                    .is_some_and(|expected| expected != &binding.package.package_build_id)
            {
                return manifest_error(
                    caller.reference(),
                    DeploymentBytecodeManifestKind::PackageReference,
                    format!("dependency alias {dependency_ref:?} binding violates its exact ABI/build requirement"),
                );
            }
            packages
                .get(&binding.package.package_build_id)
                .filter(|target| target.reference() == &binding.package)
                .ok_or_else(|| {
                    manifest_mismatch(
                        caller.reference(),
                        DeploymentBytecodeManifestKind::PackageReference,
                        format!(
                            "dependency alias {dependency_ref:?} target is absent from the hydrated closure"
                        ),
                    )
                })
        }
    }
}

fn exact_package_requirement<'a>(
    caller: &'a HydratedBytecodePackage,
    dependency_ref: &str,
) -> Result<&'a PackageRequirement, DeploymentBytecodeHydrationError> {
    let mut requirements = caller
        .artifact()
        .package_requirements
        .iter()
        .filter(|requirement| requirement.alias == dependency_ref);
    let requirement = requirements.next().ok_or_else(|| {
        manifest_mismatch(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "dependency alias {dependency_ref:?} is absent from the caller package manifest"
            ),
        )
    })?;
    if requirements.next().is_some() {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "dependency alias {dependency_ref:?} has duplicate caller package requirements"
            ),
        );
    }
    Ok(requirement)
}

fn validate_abi_expectation(
    caller: &HydratedBytecodePackage,
    symbol: &PackageSymbolRef,
    target: &HydratedBytecodePackage,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if symbol
        .abi_expectation
        .as_deref()
        .is_some_and(|expected| expected != target.reference().package_local_abi_identity.as_str())
    {
        return manifest_error(
            caller.reference(),
            DeploymentBytecodeManifestKind::PackageReference,
            format!(
                "package symbol {:?} ABI expectation {:?} disagrees with exact target {}",
                symbol.symbol_path,
                symbol.abi_expectation,
                target.reference().package_local_abi_identity
            ),
        );
    }
    Ok(())
}

fn target_coordinate(
    target: &skiff_artifact_model::OperationTargetRef,
) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: target.file_ref.file_ir_identity.clone(),
        module_path: target.file_ref.module_path.clone(),
        executable_index: target.executable_index,
    }
}

fn manifest_error<T>(
    package: &PackageArtifactRef,
    kind: DeploymentBytecodeManifestKind,
    detail: String,
) -> Result<T, DeploymentBytecodeHydrationError> {
    Err(manifest_mismatch(package, kind, detail))
}

fn manifest_mismatch(
    package: &PackageArtifactRef,
    kind: DeploymentBytecodeManifestKind,
    detail: String,
) -> DeploymentBytecodeHydrationError {
    DeploymentBytecodeHydrationError::ManifestMismatch {
        package: Box::new(package.clone()),
        kind,
        detail,
    }
}
