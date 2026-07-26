use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, PackageArtifact, PackageArtifactRef,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexRef, PackageSchemaTypeId,
    PackageSchemaTypeRecordRef, PackageServiceCallRoot, ServiceCallRef,
};

use crate::{
    package::projection::{
        implementation_links::{
            OperationTargetIdentityProjection, PackageImplementationLinksIdentityProjection,
        },
        FileIrOwnerIdentityProjection,
    },
    ArtifactIdentityError, Result,
};

mod projection;
mod validation;

/// Complete canonical preimage of a package-local public ABI identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageArtifactLocalAbiIdentityProjection {
    schema: &'static str,
    package_id: String,
    public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
}

/// Complete canonical preimage of a package artifact build identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageArtifactBuildIdentityProjection {
    schema: &'static str,
    package_id: String,
    local_abi_identity: PackageLocalAbiIdentity,
    implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    package_schema_index: PackageSchemaIndexRef,
    package_schema_type_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
    files: Vec<FileIrOwnerIdentityProjection>,
    static_resources: Vec<ResourceIdentityProjection>,
    implementation_links: PackageImplementationLinksIdentityProjection,
    callable_links: BTreeMap<PackageCallableId, CallableLinkIdentityProjection>,
    package_requirements: Value,
    contract_requirements: Value,
    service_requirements: Value,
    runtime_requirements: PackageRuntimeRequirements,
    callable_semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
    service_call_roots: Vec<PackageServiceCallRoot>,
    service_call_refs: Vec<ServiceCallRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceIdentityProjection {
    path: String,
    sha256: String,
    byte_len: u64,
    content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CallableLinkIdentityProjection {
    callable_id: PackageCallableId,
    target: OperationTargetIdentityProjection,
}

pub fn package_artifact_local_abi_identity_projection(
    artifact: &PackageArtifact,
) -> Result<PackageArtifactLocalAbiIdentityProjection> {
    validation::validate_package_artifact_surface(artifact)?;
    Ok(projection::local_abi_projection(artifact))
}

pub fn package_artifact_local_abi_identity(
    artifact: &PackageArtifact,
) -> Result<PackageLocalAbiIdentity> {
    let projection = package_artifact_local_abi_identity_projection(artifact)?;
    projection::local_abi_identity_from_projection(&projection)
}

pub fn package_artifact_build_identity_projection(
    artifact: &PackageArtifact,
) -> Result<PackageArtifactBuildIdentityProjection> {
    validation::validate_package_artifact_surface(artifact)?;
    let local_abi_identity = projection::local_abi_identity_from_validated(artifact)?;
    projection::build_projection_from_validated(artifact, local_abi_identity)
}

pub fn package_artifact_build_identity(artifact: &PackageArtifact) -> Result<PackageBuildId> {
    let projection = package_artifact_build_identity_projection(artifact)?;
    projection::build_identity_from_projection(&projection)
}

pub fn assign_package_artifact_identities(
    artifact: &mut PackageArtifact,
) -> Result<(PackageBuildId, PackageLocalAbiIdentity)> {
    validation::validate_package_artifact_surface(artifact)?;
    let local_abi_identity = projection::local_abi_identity_from_validated(artifact)?;
    artifact.package_local_abi.local_abi_identity = local_abi_identity.clone();
    let build_projection =
        projection::build_projection_from_validated(artifact, local_abi_identity.clone())?;
    let build_identity = projection::build_identity_from_projection(&build_projection)?;
    artifact.package_build_id = build_identity.clone();
    validate_package_artifact_identities(artifact)?;
    Ok((build_identity, local_abi_identity))
}

pub fn validate_package_artifact_identities(artifact: &PackageArtifact) -> Result<()> {
    validation::validate_package_artifact_surface(artifact)?;
    let computed_local = projection::local_abi_identity_from_validated(artifact)?;
    if artifact.package_local_abi.local_abi_identity != computed_local {
        return Err(
            ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch {
                declared: artifact.package_local_abi.local_abi_identity.to_string(),
                computed: computed_local.to_string(),
            },
        );
    }
    let build_projection =
        projection::build_projection_from_validated(artifact, computed_local.clone())?;
    let computed_build = projection::build_identity_from_projection(&build_projection)?;
    if artifact.package_build_id != computed_build {
        return Err(
            ArtifactIdentityError::PackageArtifactBuildIdentityMismatch {
                declared: artifact.package_build_id.to_string(),
                computed: computed_build.to_string(),
            },
        );
    }
    Ok(())
}

pub fn package_artifact_ref(artifact: &PackageArtifact) -> Result<PackageArtifactRef> {
    validate_package_artifact_identities(artifact)?;
    Ok(PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    })
}

fn invalid_artifact<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidPackageArtifact {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        BoundaryUnavailableReason, CallableEffectSummary, CallableMayEffects,
        CallableProvenanceSummary, CallableSemanticFacts, FileIrRef, NominalTypeRefBaseIr,
        OperationCallableKind, OperationTargetRef, PackageCallableLinkFact,
        PackageCallableParameter, PackageCallableSignature, PackageImplementationLinks,
        PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr, ValueProvenance,
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn current_package_artifact_generation_assigns_and_rejects_stale_domains() {
        let artifact = fixture();
        assert!(artifact
            .package_build_id
            .as_str()
            .starts_with(crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX));
        assert!(artifact
            .package_local_abi
            .local_abi_identity
            .as_str()
            .starts_with(crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX));
        assert_eq!(
            serde_json::to_value(package_artifact_build_identity_projection(&artifact).unwrap())
                .unwrap()["schema"],
            crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER
        );
        assert_eq!(
            serde_json::to_value(
                package_artifact_local_abi_identity_projection(&artifact).unwrap()
            )
            .unwrap()["schema"],
            crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER
        );
        validate_package_artifact_identities(&artifact).unwrap();

        let mut stale_schema = artifact.clone();
        stale_schema.schema_version = "skiff-package-artifact-v5".to_string();
        assert!(matches!(
            validate_package_artifact_identities(&stale_schema),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut stale_local = artifact.clone();
        stale_local.package_local_abi.local_abi_identity = PackageLocalAbiIdentity::new(
            stale_local
                .package_local_abi
                .local_abi_identity
                .as_str()
                .replacen(
                    crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
                    "skiff-package-local-abi-v4:sha256",
                    1,
                ),
        );
        assert!(matches!(
            validate_package_artifact_identities(&stale_local),
            Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
        ));

        let mut stale_build = artifact;
        stale_build.package_build_id =
            PackageBuildId::new(stale_build.package_build_id.as_str().replacen(
                crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
                "skiff-package-build-v6:sha256",
                1,
            ));
        assert!(matches!(
            validate_package_artifact_identities(&stale_build),
            Err(ArtifactIdentityError::PackageArtifactBuildIdentityMismatch { .. })
        ));
    }

    #[test]
    fn package_artifact_human_version_label_is_not_an_identity_input() {
        let base = fixture();
        let mut relabeled = base.clone();
        relabeled.package_version = "99.0.0".to_string();

        assert_eq!(
            package_artifact_local_abi_identity(&base).unwrap(),
            package_artifact_local_abi_identity(&relabeled).unwrap()
        );
        assert_eq!(
            package_artifact_build_identity(&base).unwrap(),
            package_artifact_build_identity(&relabeled).unwrap()
        );
    }

    #[test]
    fn implementation_throw_facts_change_build_but_not_open_local_abi() {
        let base = callable_fixture();
        let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
        let baseline_build = package_artifact_build_identity(&base).unwrap();
        let callable_id = base.callable_semantic_facts.keys().next().unwrap().clone();

        let mut changed = base;
        let facts = changed
            .callable_semantic_facts
            .get_mut(&callable_id)
            .unwrap();
        let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
            panic!("fixture effects must be analyzed")
        };
        effects.throws_caller_alias = true;
        let CallableProvenanceSummary::Analyzed { throw_origins, .. } = &mut facts.provenance
        else {
            panic!("fixture provenance must be analyzed")
        };
        *throw_origins = vec![ValueProvenance::CallerParameter { index: 0 }];

        assert_eq!(
            package_artifact_local_abi_identity(&changed).unwrap(),
            baseline_local
        );
        assert_ne!(
            package_artifact_build_identity(&changed).unwrap(),
            baseline_build
        );
    }

    #[test]
    fn service_call_roots_change_only_build_identity_and_are_order_stable() {
        let base = two_callable_fixture();
        let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
        let baseline_build = package_artifact_build_identity(&base).unwrap();
        let run_id = callable_id_for_path(&base, "run");
        let echo_id = callable_id_for_path(&base, "echo");

        let mut selected = base.clone();
        selected.service_call_roots = vec![
            PackageServiceCallRoot::Function {
                public_path: "run".to_string(),
                callable_id: run_id.clone(),
            },
            PackageServiceCallRoot::Function {
                public_path: "echo".to_string(),
                callable_id: echo_id.clone(),
            },
        ];
        let mut reordered = selected.clone();
        reordered.service_call_roots.reverse();
        let mut one_root = base.clone();
        one_root.service_call_roots = vec![PackageServiceCallRoot::Function {
            public_path: "run".to_string(),
            callable_id: run_id,
        }];

        assert_eq!(
            package_artifact_local_abi_identity(&selected).unwrap(),
            baseline_local
        );
        assert_eq!(
            package_artifact_local_abi_identity(&one_root).unwrap(),
            baseline_local
        );
        assert_ne!(
            package_artifact_build_identity(&selected).unwrap(),
            baseline_build
        );
        assert_ne!(
            package_artifact_build_identity(&selected).unwrap(),
            package_artifact_build_identity(&one_root).unwrap()
        );
        assert_eq!(
            package_artifact_build_identity(&selected).unwrap(),
            package_artifact_build_identity(&reordered).unwrap()
        );

        let build =
            serde_json::to_value(package_artifact_build_identity_projection(&selected).unwrap())
                .unwrap();
        let local = serde_json::to_value(
            package_artifact_local_abi_identity_projection(&selected).unwrap(),
        )
        .unwrap();
        assert!(build.get("serviceCallRoots").is_some());
        assert!(local.get("serviceCallRoots").is_none());

        let mut wrong_id = selected.clone();
        let PackageServiceCallRoot::Function { callable_id, .. } =
            &mut wrong_id.service_call_roots[0]
        else {
            unreachable!()
        };
        *callable_id = PackageCallableId::new("pkg-callable:example.identity:wrong");
        assert!(matches!(
            package_artifact_build_identity(&wrong_id),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut duplicate_path = selected;
        duplicate_path
            .service_call_roots
            .push(PackageServiceCallRoot::Function {
                public_path: "echo".to_string(),
                callable_id: echo_id,
            });
        assert!(matches!(
            package_artifact_build_identity(&duplicate_path),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));
    }

    #[test]
    fn service_call_public_instance_root_requires_exact_methods_ids_and_link_kinds() {
        let selected = public_instance_fixture();
        package_artifact_build_identity(&selected).unwrap();

        let mut missing_method = selected.clone();
        let PackageServiceCallRoot::PublicInstance { methods, .. } =
            &mut missing_method.service_call_roots[0]
        else {
            unreachable!()
        };
        methods.remove("stop");
        assert!(matches!(
            package_artifact_build_identity(&missing_method),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut wrong_id = selected.clone();
        let PackageServiceCallRoot::PublicInstance { methods, .. } =
            &mut wrong_id.service_call_roots[0]
        else {
            unreachable!()
        };
        methods.insert(
            "run".to_string(),
            PackageCallableId::new("pkg-callable:example.identity:wrong"),
        );
        assert!(matches!(
            package_artifact_build_identity(&wrong_id),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut wrong_kind = selected.clone();
        let run_id = callable_id_for_path(&wrong_kind, "worker.run");
        wrong_kind
            .callable_links
            .get_mut(&run_id)
            .unwrap()
            .target
            .callable_kind = OperationCallableKind::PublicFunction;
        assert!(matches!(
            package_artifact_build_identity(&wrong_kind),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut no_interfaces = selected;
        let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = no_interfaces
            .package_local_abi
            .public_symbols
            .get_mut("worker")
            .unwrap()
        else {
            unreachable!()
        };
        interfaces.clear();
        assert!(matches!(
            package_artifact_build_identity(&no_interfaces),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));
    }

    #[test]
    fn callable_parameter_return_and_suspend_mutations_change_local_abi_without_throw_set() {
        let base = callable_fixture();
        let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
        let baseline_build = package_artifact_build_identity(&base).unwrap();

        let mut parameter = base.clone();
        callable_signature_mut(&mut parameter).parameters[0].ty = PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("integer"),
        };
        let mut returned = base.clone();
        callable_signature_mut(&mut returned).return_type = PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("bool"),
        };
        let mut suspended = base.clone();
        callable_signature_mut(&mut suspended).may_suspend = true;

        for changed in [&parameter, &returned, &suspended] {
            assert_ne!(
                package_artifact_local_abi_identity(changed).unwrap(),
                baseline_local
            );
            assert_ne!(
                package_artifact_build_identity(changed).unwrap(),
                baseline_build
            );
        }

        let PackageLocalAbiSymbol::Callable { signature, .. } =
            &base.package_local_abi.public_symbols["run"]
        else {
            panic!("fixture run must be callable")
        };
        let wire = serde_json::to_value(signature).unwrap();
        assert!(wire.get("throwTypes").is_none());
        assert_eq!(
            wire.as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["parameters", "returnType", "maySuspend"]
        );

        let mut legacy = wire;
        legacy["throwTypes"] = serde_json::json!([]);
        assert!(serde_json::from_value::<PackageCallableSignature>(legacy).is_err());
    }

    #[test]
    fn applied_nominal_argument_matrix_changes_local_abi_and_build_and_rejects_tampering() {
        let mut string_box = callable_fixture();
        set_parameter_local_type(
            &mut string_box,
            applied_package_nominal("Box", vec![TypeRefIr::builtin("string")]),
        );
        let mut number_box = callable_fixture();
        set_parameter_local_type(
            &mut number_box,
            applied_package_nominal("Box", vec![TypeRefIr::builtin("number")]),
        );
        assert_ne!(
            package_artifact_local_abi_identity(&string_box).unwrap(),
            package_artifact_local_abi_identity(&number_box).unwrap()
        );
        assert_ne!(
            package_artifact_build_identity(&string_box).unwrap(),
            package_artifact_build_identity(&number_box).unwrap()
        );

        let mut ordered = callable_fixture();
        set_parameter_local_type(
            &mut ordered,
            applied_package_nominal(
                "Box",
                vec![applied_package_nominal(
                    "Pair",
                    vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
                )],
            ),
        );
        let mut reordered = callable_fixture();
        set_parameter_local_type(
            &mut reordered,
            applied_package_nominal(
                "Box",
                vec![applied_package_nominal(
                    "Pair",
                    vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
                )],
            ),
        );
        assert_ne!(
            package_artifact_local_abi_identity(&ordered).unwrap(),
            package_artifact_local_abi_identity(&reordered).unwrap()
        );
        assert_ne!(
            package_artifact_build_identity(&ordered).unwrap(),
            package_artifact_build_identity(&reordered).unwrap()
        );

        assign_package_artifact_identities(&mut string_box).unwrap();
        let PackageTypeRef::Local { local_type } =
            &mut callable_signature_mut(&mut string_box).parameters[0].ty
        else {
            panic!("fixture parameter must be local")
        };
        let TypeRefIr::AppliedNominal { arguments, .. } = local_type else {
            panic!("fixture parameter must be applied")
        };
        arguments[0] = TypeRefIr::builtin("number");
        assert!(matches!(
            validate_package_artifact_identities(&string_box),
            Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
        ));

        assign_package_artifact_identities(&mut ordered).unwrap();
        let PackageTypeRef::Local { local_type } =
            &mut callable_signature_mut(&mut ordered).parameters[0].ty
        else {
            panic!("fixture parameter must be local")
        };
        let TypeRefIr::AppliedNominal { base, .. } = local_type else {
            panic!("fixture parameter must be applied")
        };
        let NominalTypeRefBaseIr::PackageSymbol { symbol } = base else {
            panic!("fixture base must be a package symbol")
        };
        symbol.package = PackageRefIr::PackageId {
            package_id: "example.other-model".to_string(),
        };
        assert!(matches!(
            validate_package_artifact_identities(&ordered),
            Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
        ));
    }

    #[test]
    fn package_artifact_admission_rejects_empty_and_applied_package_schema() {
        let mut empty = callable_fixture();
        set_parameter_local_type(
            &mut empty,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: package_symbol("Box"),
                },
                arguments: Vec::new(),
            },
        );
        assert!(matches!(
            package_artifact_local_abi_identity(&empty),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));

        let mut package_schema = callable_fixture();
        set_parameter_local_type(
            &mut package_schema,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSchema {
                    package_id: "example.model".to_string(),
                    stable_schema_key: "Box".to_string(),
                    package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new(
                        "schema:box",
                    ),
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        );
        assert!(matches!(
            package_artifact_local_abi_identity(&package_schema),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));
    }

    fn fixture() -> PackageArtifact {
        let mut artifact = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.identity".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: skiff_artifact_model::PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: "example.identity".to_string(),
                package_schema_index_identity: crate::package_schema_index_identity(
                    "example.identity",
                    &BTreeMap::new(),
                )
                .unwrap(),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_roots: Vec::new(),
            service_call_refs: Vec::new(),
        };
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn callable_fixture() -> PackageArtifact {
        let mut artifact = fixture();
        let callable_id = PackageCallableId::new("pkg-callable:example.identity:run");
        let file = FileIrRef::new(
            "skiff-file-ir-v7:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "api",
        );
        artifact.files.push(file.clone());
        artifact.package_local_abi.public_symbols.insert(
            "run".to_string(),
            PackageLocalAbiSymbol::Callable {
                callable_id: callable_id.clone(),
                signature: PackageCallableSignature {
                    parameters: vec![PackageCallableParameter {
                        name: "value".to_string(),
                        ty: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("string"),
                        },
                    }],
                    return_type: PackageTypeRef::Local {
                        local_type: TypeRefIr::builtin("string"),
                    },
                    may_suspend: false,
                },
            },
        );
        artifact.callable_links.insert(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target: OperationTargetRef {
                    file_ref: file,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        );
        artifact.callable_semantic_facts.insert(
            callable_id.clone(),
            CallableSemanticFacts {
                effects: CallableEffectSummary::Analyzed {
                    effects: CallableMayEffects {
                        writes_caller_reachable: false,
                        returns_caller_alias: false,
                        throws_caller_alias: false,
                        escapes_caller_value: false,
                        requires_same_heap_identity: false,
                        invokes_unknown_target: false,
                        may_suspend: false,
                    },
                },
                provenance: CallableProvenanceSummary::Analyzed {
                    return_origins: vec![ValueProvenance::Fresh],
                    direct_return_origins: vec![ValueProvenance::Fresh],
                    throw_origins: Vec::new(),
                    escape_lanes: Vec::new(),
                },
                resolved_call_targets: BTreeMap::new(),
            },
        );
        artifact.boundary_projections.insert(
            callable_id,
            BoundaryCallableProjection::Unavailable {
                reasons: vec![BoundaryUnavailableReason::AnalysisPending],
            },
        );
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn two_callable_fixture() -> PackageArtifact {
        let mut artifact = callable_fixture();
        let run_id = callable_id_for_path(&artifact, "run");
        let echo_id = PackageCallableId::new("pkg-callable:example.identity:echo");
        let mut echo_symbol = artifact.package_local_abi.public_symbols["run"].clone();
        let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut echo_symbol else {
            unreachable!()
        };
        *callable_id = echo_id.clone();
        artifact
            .package_local_abi
            .public_symbols
            .insert("echo".to_string(), echo_symbol);

        let mut echo_link = artifact.callable_links[&run_id].clone();
        echo_link.callable_id = echo_id.clone();
        echo_link.target.callable_abi_id = echo_id.to_string();
        echo_link.target.executable_index = 1;
        artifact.callable_links.insert(echo_id.clone(), echo_link);
        artifact.callable_semantic_facts.insert(
            echo_id.clone(),
            artifact.callable_semantic_facts[&run_id].clone(),
        );
        artifact
            .boundary_projections
            .insert(echo_id, artifact.boundary_projections[&run_id].clone());
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn public_instance_fixture() -> PackageArtifact {
        let mut artifact = two_callable_fixture();
        let run_id = callable_id_for_path(&artifact, "run");
        let stop_id = callable_id_for_path(&artifact, "echo");
        let run = artifact
            .package_local_abi
            .public_symbols
            .remove("run")
            .unwrap();
        let stop = artifact
            .package_local_abi
            .public_symbols
            .remove("echo")
            .unwrap();
        artifact
            .package_local_abi
            .public_symbols
            .insert("worker.run".to_string(), run);
        artifact
            .package_local_abi
            .public_symbols
            .insert("worker.stop".to_string(), stop);
        artifact.package_local_abi.public_symbols.insert(
            "worker".to_string(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: "worker".to_string(),
                declared_receiver_type: TypeRefIr::builtin("Worker"),
                interfaces: vec![TypeRefIr::builtin("WorkerApi")],
                methods: BTreeMap::from([
                    ("run".to_string(), run_id.clone()),
                    ("stop".to_string(), stop_id.clone()),
                ]),
            },
        );
        artifact
            .callable_links
            .get_mut(&run_id)
            .unwrap()
            .target
            .callable_kind = OperationCallableKind::ImplMethod;
        artifact
            .callable_links
            .get_mut(&stop_id)
            .unwrap()
            .target
            .callable_kind = OperationCallableKind::ImplMethod;
        artifact.service_call_roots = vec![PackageServiceCallRoot::PublicInstance {
            public_path: "worker".to_string(),
            methods: BTreeMap::from([("run".to_string(), run_id), ("stop".to_string(), stop_id)]),
        }];
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn callable_id_for_path(artifact: &PackageArtifact, path: &str) -> PackageCallableId {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } =
            &artifact.package_local_abi.public_symbols[path]
        else {
            panic!("{path} must be callable")
        };
        callable_id.clone()
    }

    fn callable_signature_mut(artifact: &mut PackageArtifact) -> &mut PackageCallableSignature {
        let PackageLocalAbiSymbol::Callable { signature, .. } = artifact
            .package_local_abi
            .public_symbols
            .get_mut("run")
            .unwrap()
        else {
            panic!("fixture run must be callable")
        };
        signature
    }

    fn set_parameter_local_type(artifact: &mut PackageArtifact, local_type: TypeRefIr) {
        callable_signature_mut(artifact).parameters[0].ty = PackageTypeRef::Local { local_type };
    }

    fn applied_package_nominal(symbol_path: &str, arguments: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: package_symbol(symbol_path),
            },
            arguments,
        }
    }

    fn package_symbol(symbol_path: &str) -> PackageSymbolRef {
        PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: "example.model".to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: Some("model-abi".to_string()),
        }
    }
}
