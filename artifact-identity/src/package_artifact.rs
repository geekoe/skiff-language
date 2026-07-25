use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, PackageArtifact, PackageArtifactRef,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexRef, PackageSchemaTypeId,
    PackageSchemaTypeRecordRef, ServiceCallRef,
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
        CallableProvenanceSummary, CallableSemanticFacts, FileIrRef, OperationCallableKind,
        OperationTargetRef, PackageCallableLinkFact, PackageCallableParameter,
        PackageCallableSignature, PackageImplementationLinks, PackageTypeRef, TypeRefIr,
        ValueProvenance, PACKAGE_ARTIFACT_SCHEMA_VERSION,
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
        stale_schema.schema_version = "skiff-package-artifact-v3".to_string();
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
                    "skiff-package-local-abi-v3:sha256",
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
                "skiff-package-build-v4:sha256",
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
            service_call_refs: Vec::new(),
        };
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn callable_fixture() -> PackageArtifact {
        let mut artifact = fixture();
        let callable_id = PackageCallableId::new("pkg-callable:example.identity:run");
        let file = FileIrRef::new(
            "skiff-file-ir-v6:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
}
