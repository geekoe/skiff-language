use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ContractTypeDescriptor, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FileIrRef, PackageArtifact, PackageArtifactRef, PackageBinding,
    PackageBuildId, PackageLocalAbiSymbol, PackageRefIr, PackageRequirement, PackageRequirementKey,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSymbolRef, ServiceDeployment, TypeDescriptorIr, TypeExport, TypeRefIr,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use super::{bytecode, contract, package, ExactResolver};

pub(in crate::tests) const OWNER_CALLER_PACKAGE_ID: &str = "example.verifier-owner";
pub(in crate::tests) const OWNER_TARGET_PACKAGE_ID: &str = "example.verifier-type-provider";
pub(in crate::tests) const OWNER_DEPENDENCY_ALIAS: &str = "types";
pub(in crate::tests) const OWNER_TYPE_PATH: &str = "model.Value";
const OWNER_INNER_TYPE_PATH: &str = "model.InnerValue";
pub(in crate::tests) const OWNER_SELF_TYPE_PATH: &str = "model.SelfValue";
pub(in crate::tests) const OWNER_SCHEMA_KEY: &str = "model.Schema";

#[derive(Debug, Clone, Copy)]
pub(in crate::tests) enum OwnerRequirementMode {
    Unpinned,
    Exact,
    DuplicateExact,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::tests) enum OwnerTypeSurface {
    Public,
    Private,
    Conflicting,
}

pub(in crate::tests) struct OwnerAuthorityFixture {
    pub(in crate::tests) hydrated: HydratedDeploymentBytecode,
    pub(in crate::tests) caller_build_id: PackageBuildId,
    pub(in crate::tests) target: PackageArtifactRef,
    pub(in crate::tests) schema_type_id: PackageSchemaTypeId,
}

pub(in crate::tests) fn owner_authority_fixture(
    requirement_mode: OwnerRequirementMode,
    surface: OwnerTypeSurface,
) -> OwnerAuthorityFixture {
    let schema_record = owner_schema_record();
    let schema_type_id = schema_record.package_schema_type_id.clone();
    let target_bytecode = bytecode(
        vec![
            TypeRefIr::PackageSchema {
                package_id: schema_record.package_id.clone(),
                stable_schema_key: schema_record.stable_schema_key.clone(),
                package_schema_type_id: schema_type_id.clone(),
            },
            TypeRefIr::PackageSchema {
                package_id: schema_record.package_id.clone(),
                stable_schema_key: schema_record.stable_schema_key.clone(),
                package_schema_type_id: schema_type_id.clone(),
            },
        ],
        Vec::new(),
    );
    let mut target_package = package(target_bytecode.reference().clone());
    retag_package(&mut target_package, OWNER_TARGET_PACKAGE_ID);
    let descriptor = empty_record_descriptor();
    let inner = exact_package_symbol(OWNER_TARGET_PACKAGE_ID, OWNER_INNER_TYPE_PATH, None);
    match surface {
        OwnerTypeSurface::Public => {
            install_implementation_type(
                &mut target_package,
                OWNER_INNER_TYPE_PATH,
                0,
                descriptor.clone(),
            );
            target_package.package_local_abi.public_symbols.insert(
                OWNER_TYPE_PATH.to_string(),
                public_type_symbol(OWNER_TYPE_PATH, TypeDescriptorIr::Alias { target: inner }),
            );
        }
        OwnerTypeSurface::Private => {
            install_implementation_type(
                &mut target_package,
                OWNER_INNER_TYPE_PATH,
                0,
                descriptor.clone(),
            );
            install_implementation_type(
                &mut target_package,
                OWNER_TYPE_PATH,
                1,
                TypeDescriptorIr::Alias { target: inner },
            )
        }
        OwnerTypeSurface::Conflicting => {
            install_implementation_type(
                &mut target_package,
                OWNER_TYPE_PATH,
                0,
                descriptor.clone(),
            );
            target_package.package_local_abi.public_symbols.insert(
                OWNER_TYPE_PATH.to_string(),
                public_type_symbol(
                    OWNER_TYPE_PATH,
                    TypeDescriptorIr::Alias {
                        target: TypeRefIr::builtin("string"),
                    },
                ),
            );
        }
    }
    target_package
        .bytecode_schema_records
        .insert(schema_type_id.clone(), schema_record);
    reassign_package_identities(&mut target_package);
    let target_reference = skiff_artifact_identity::package_artifact_ref(&target_package).unwrap();

    let mut caller_types = vec![TypeRefIr::PublicationType {
        module_path: "model".to_string(),
        type_index: 0,
    }];
    if (matches!(surface, OwnerTypeSurface::Private)
        && matches!(
            requirement_mode,
            OwnerRequirementMode::Exact | OwnerRequirementMode::DuplicateExact
        ))
        || matches!(surface, OwnerTypeSurface::Public)
    {
        caller_types.push(TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: OWNER_DEPENDENCY_ALIAS.to_string(),
                },
                symbol_path: OWNER_TYPE_PATH.to_string(),
                abi_expectation: None,
            },
        });
    }
    if matches!(
        (requirement_mode, surface),
        (
            OwnerRequirementMode::DuplicateExact,
            OwnerTypeSurface::Public
        )
    ) {
        caller_types.push(exact_package_symbol(
            OWNER_TARGET_PACKAGE_ID,
            OWNER_TYPE_PATH,
            None,
        ));
    }
    let caller_bytecode = bytecode(caller_types, Vec::new());
    let mut caller_package = package(caller_bytecode.reference().clone());
    retag_package(&mut caller_package, OWNER_CALLER_PACKAGE_ID);
    install_implementation_type(&mut caller_package, OWNER_SELF_TYPE_PATH, 0, descriptor);
    let requirement = |alias: &str, exact_build: bool| PackageRequirement {
        alias: alias.to_string(),
        package_id: target_reference.package_id.clone(),
        exact_version: target_reference.package_version.clone(),
        expected_local_abi: target_reference.package_local_abi_identity.clone(),
        expected_package_build: exact_build.then(|| target_reference.package_build_id.clone()),
    };
    caller_package.package_requirements = match requirement_mode {
        OwnerRequirementMode::Unpinned => vec![requirement(OWNER_DEPENDENCY_ALIAS, false)],
        OwnerRequirementMode::Exact => vec![requirement(OWNER_DEPENDENCY_ALIAS, true)],
        OwnerRequirementMode::DuplicateExact => vec![
            requirement(OWNER_DEPENDENCY_ALIAS, true),
            requirement("other-types", true),
        ],
    };
    reassign_package_identities(&mut caller_package);
    let caller_reference = skiff_artifact_identity::package_artifact_ref(&caller_package).unwrap();

    let contract = contract();
    let mut bindings = vec![PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: caller_reference.package_build_id.clone(),
            package_requirement_alias: OWNER_DEPENDENCY_ALIAS.to_string(),
        },
        package: target_reference.clone(),
    }];
    if matches!(requirement_mode, OwnerRequirementMode::DuplicateExact) {
        bindings.push(PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: caller_reference.package_build_id.clone(),
                package_requirement_alias: "other-types".to_string(),
            },
            package: target_reference.clone(),
        });
    }
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:verifier-owner-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: caller_reference.clone(),
        operation_bindings: Vec::new(),
        package_bindings: bindings,
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "verifier owner test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let deployment_reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        packages: BTreeMap::from([
            (caller_reference.clone(), Arc::new(caller_package)),
            (target_reference.clone(), Arc::new(target_package)),
        ]),
        bytecodes: BTreeMap::from([
            (caller_reference.clone(), caller_bytecode),
            (target_reference.clone(), target_bytecode),
        ]),
    };
    let hydrated = DeploymentBytecodeLoader::new(&resolver)
        .load(&deployment_reference)
        .unwrap();
    OwnerAuthorityFixture {
        hydrated,
        caller_build_id: caller_reference.package_build_id,
        target: target_reference,
        schema_type_id,
    }
}

fn retag_package(package: &mut PackageArtifact, package_id: &str) {
    package.package_id = package_id.to_string();
    package.package_version = "1.0.0".to_string();
    package.package_schema_index.package_id = package_id.to_string();
    reassign_package_identities(package);
}

fn reassign_package_identities(package: &mut PackageArtifact) {
    package.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &package.package_id,
            &BTreeMap::new(),
        )
        .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(package).unwrap();
}

fn install_implementation_type(
    package: &mut PackageArtifact,
    symbol_path: &str,
    type_index: u32,
    descriptor: TypeDescriptorIr,
) {
    let (module_path, symbol) = symbol_path
        .rsplit_once('.')
        .expect("test implementation type path is qualified");
    let file = FileIrRef::new(format!("file-ir:{}", package.package_id), module_path);
    if !package.files.contains(&file) {
        package.files.push(file.clone());
    }
    package.package_local_abi.implementation_symbols.insert(
        symbol_path.to_string(),
        implementation_type_symbol(&package.package_id, symbol_path, descriptor.clone()),
    );
    package.implementation_links.types.insert(
        symbol_path.to_string(),
        TypeExport {
            file,
            type_index,
            symbol: symbol.to_string(),
            is_interface: false,
            descriptor: Some(descriptor),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
}

fn implementation_type_symbol(
    package_id: &str,
    symbol_path: &str,
    descriptor: TypeDescriptorIr,
) -> PackageLocalAbiSymbol {
    type_symbol(
        format!("type:{package_id}:top-level:{symbol_path}"),
        descriptor,
    )
}

fn public_type_symbol(symbol_path: &str, descriptor: TypeDescriptorIr) -> PackageLocalAbiSymbol {
    type_symbol(format!("type:{symbol_path}"), descriptor)
}

fn type_symbol(local_type_id: String, descriptor: TypeDescriptorIr) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Type {
        local_type_id,
        descriptor,
        is_alias: false,
        is_interface: false,
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: None,
    }
}

fn exact_package_symbol(
    package_id: &str,
    symbol_path: &str,
    abi_expectation: Option<&str>,
) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: abi_expectation.map(str::to_string),
        },
    }
}

fn empty_record_descriptor() -> TypeDescriptorIr {
    TypeDescriptorIr::Record {
        fields: BTreeMap::new(),
    }
}

fn owner_schema_record() -> PackageSchemaTypeRecord {
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Enumeration {
            variants: vec!["ready".to_string()],
        },
    };
    let package_schema_type_id = skiff_artifact_model::derive_package_schema_type_id(
        OWNER_TARGET_PACKAGE_ID,
        OWNER_SCHEMA_KEY,
        &canonical_descriptor,
    )
    .unwrap();
    PackageSchemaTypeRecord {
        package_id: OWNER_TARGET_PACKAGE_ID.to_string(),
        stable_schema_key: OWNER_SCHEMA_KEY.to_string(),
        package_schema_type_id,
        canonical_descriptor,
    }
}
