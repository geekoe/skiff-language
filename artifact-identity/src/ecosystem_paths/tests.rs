use super::*;
use crate::ASSEMBLY_IDENTITY_PREFIX;
use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId,
    PackageLocalAbiIdentity, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    PackageSchemaTypeId, PackageSchemaTypeRecordRef, ServiceProtocolIdentity,
};

fn hash(char: char) -> String {
    std::iter::repeat_n(char, 64).collect()
}

#[test]
fn typed_paths_are_canonical_and_identity_addressed() {
    let package = PackageArtifactRef {
        package_id: "example.com/echo".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!(
            "{PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX}:{}",
            hash('a')
        )),
        package_local_abi_identity: PackageLocalAbiIdentity::new(format!(
            "{PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX}:{}",
            hash('b')
        )),
    };
    assert_eq!(
        PackageArtifactRecordPath::new(&package).unwrap().as_str(),
        format!(
            "records/package-artifacts/example~dcom~secho/1.0.0/{}/package.json",
            hash('a')
        )
    );
    let contract = ServiceContractRef {
        service_id: "example.com/echo".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new(format!(
            "{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}",
            hash('c')
        )),
    };
    assert!(ServiceContractRecordPath::new(&contract)
        .unwrap()
        .as_str()
        .ends_with(&format!("/{}.json", hash('c'))));
    let deployment = ServiceDeploymentRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "{DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX}:{}",
            hash('d')
        )),
    };
    assert!(ServiceDeploymentRecordPath::new(&deployment)
        .unwrap()
        .as_str()
        .contains("/revision-1/"));
    let assembly = RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "{ASSEMBLY_IDENTITY_PREFIX}:{}",
            hash('e')
        )),
    };
    assert_eq!(
        RuntimeAssemblyRecordPath::new(&assembly).unwrap().as_str(),
        format!("records/runtime-assemblies/{}.json", hash('e'))
    );
}

#[test]
fn package_schema_records_have_independent_content_addressed_paths() {
    let index = PackageSchemaIndexRef {
        package_id: "example.com/shared".to_string(),
        package_schema_index_identity: PackageSchemaIndexIdentity::new(format!(
            "{PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX}:{}",
            hash('a')
        )),
    };
    let record = PackageSchemaTypeRecordRef {
        package_id: "example.com/shared".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(format!(
            "{PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX}:{}",
            hash('b')
        )),
    };
    assert_eq!(
        PackageSchemaIndexRecordPath::new(&index).unwrap().as_str(),
        format!(
            "records/package-schema-indexes/example~dcom~sshared/{}.json",
            hash('a')
        )
    );
    assert_eq!(
        PackageSchemaTypeRecordPath::new(&record).unwrap().as_str(),
        format!(
            "records/package-schema-types/example~dcom~sshared/{}.json",
            hash('b')
        )
    );
}

#[test]
fn release_pointer_path_is_profile_service_version_addressed() {
    let path = ReleasePointerPath::new("dev", "example.com/echo", "1.0.0").unwrap();
    assert_eq!(
        path.as_str(),
        "pointers/releases/dev/example~dcom~secho/1.0.0.json"
    );
    assert_eq!(
        ReleasePointerPath::new("a.b", "echo", "1.0.0")
            .unwrap()
            .as_str(),
        "pointers/releases/a.b/echo/1.0.0.json"
    );
    assert!(ReleasePointerPath::new("dev", "a.b/c/d", "1.0.0")
        .unwrap()
        .as_str()
        .contains("/a~db~sc~sd/"));
    assert!(ReleasePointerPath::new("../prod", "echo", "1.0.0").is_err());
    assert!(ReleasePointerPath::new("dev", "echo", "1.0/0").is_err());
    assert!(ReleasePointerPath::new("dev", "echo", "").is_err());
    assert!(ReleasePointerPath::new("", "echo", "1.0.0").is_err());
    assert!(ReleasePointerPath::new("a/b", "echo", "1.0.0").is_err());
    assert_ne!(
        ReleasePointerPath::new("dev", "a.b/c/d", "1.0.0").unwrap(),
        ReleasePointerPath::new("dev", "a.b/c..d", "1.0.0").unwrap()
    );
}

#[test]
fn wrong_identity_domains_and_noncanonical_declared_paths_fail() {
    let package = PackageArtifactRef {
        package_id: "example.com/echo".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!(
            "{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}",
            hash('a')
        )),
        package_local_abi_identity: PackageLocalAbiIdentity::new(format!(
            "{PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX}:{}",
            hash('b')
        )),
    };
    assert!(PackageArtifactRecordPath::new(&package).is_err());
}

#[test]
fn runtime_assembly_path_consumes_model_identity_leaf() {
    let valid = RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "{ASSEMBLY_IDENTITY_PREFIX}:{}",
            hash('a')
        )),
    };
    assert!(RuntimeAssemblyRecordPath::new(&valid).is_ok());

    for invalid in [
        format!("{ASSEMBLY_IDENTITY_PREFIX}:{}", hash('A')),
        format!("{ASSEMBLY_IDENTITY_PREFIX}:short"),
        format!("skiff-service-protocol-v2:sha256:{}", hash('a')),
    ] {
        let reference = RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(invalid),
        };
        assert!(RuntimeAssemblyRecordPath::new(&reference).is_err());
    }
}

#[test]
fn coordinate_codec_is_injective_for_slashes_and_adjacent_dots() {
    assert_eq!(coordinate_segment("a.b", "fixture").unwrap(), "a~db");
    assert_eq!(coordinate_segment("a/b", "fixture").unwrap(), "a~sb");
    assert_eq!(coordinate_segment("a..b", "fixture").unwrap(), "a~d~db");
    assert_eq!(
        coordinate_segment("a.b/c/d", "fixture").unwrap(),
        "a~db~sc~sd"
    );
    assert_eq!(
        coordinate_segment("a.b/c..d", "fixture").unwrap(),
        "a~db~sc~d~dd"
    );

    let protocol =
        ServiceProtocolIdentity::new(format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", hash('c')));
    let slash = ServiceContractRef {
        service_id: "a.b/c/d".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: protocol.clone(),
    };
    let adjacent_dots = ServiceContractRef {
        service_id: "a.b/c..d".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: protocol,
    };

    assert_ne!(
        ServiceContractRecordPath::new(&slash).unwrap(),
        ServiceContractRecordPath::new(&adjacent_dots).unwrap()
    );
    assert_ne!(
        ServiceContractPointerPath::new(&slash.service_id, &slash.contract_version).unwrap(),
        ServiceContractPointerPath::new(&adjacent_dots.service_id, &adjacent_dots.contract_version)
            .unwrap()
    );
}
