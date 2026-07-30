use skiff_artifact_model::{
    runtime_assembly_identity_hash, FileIrRef, PackageArtifactRef, PackageSchemaIndexRef,
    PackageSchemaTypeRecordRef, PublicationResourceRef, RuntimeAssemblyRef, ServiceContractRef,
    ServiceDeploymentRef,
};

use crate::{
    ArtifactIdentityError, ArtifactRelativePath, Result, DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
    FILE_IR_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX, PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX, SERVICE_PROTOCOL_IDENTITY_PREFIX,
};

macro_rules! typed_path {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(ArtifactRelativePath);

        impl $name {
            pub fn as_relative_path(&self) -> &ArtifactRelativePath {
                &self.0
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

typed_path!(PackageArtifactRecordPath);
typed_path!(PackageSchemaIndexRecordPath);
typed_path!(PackageSchemaTypeRecordPath);
typed_path!(ServiceContractRecordPath);
typed_path!(ServiceDeploymentRecordPath);
typed_path!(RuntimeAssemblyRecordPath);
typed_path!(PackageFileIrRecordPath);
typed_path!(PackageResourceRecordPath);
typed_path!(PackageArtifactPointerPath);
typed_path!(ServiceContractPointerPath);
typed_path!(ServiceDeploymentPointerPath);
typed_path!(RuntimeAssemblyPointerPath);
typed_path!(EnvironmentActivationStatePath);

impl PackageArtifactRecordPath {
    pub fn new(reference: &PackageArtifactRef) -> Result<Self> {
        let coordinate = package_coordinate(reference)?;
        let build = identity_hash(
            reference.package_build_id.as_str(),
            PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
            "packageBuildId",
        )?;
        identity_hash(
            reference.package_local_abi_identity.as_str(),
            PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
            "packageLocalAbiIdentity",
        )?;
        relative(format!(
            "records/package-artifacts/{coordinate}/{build}/package.json"
        ))
        .map(Self)
    }
}

impl PackageSchemaIndexRecordPath {
    pub fn new(reference: &PackageSchemaIndexRef) -> Result<Self> {
        let package = coordinate_segment(&reference.package_id, "packageId")?;
        let identity = identity_hash(
            reference.package_schema_index_identity.as_str(),
            PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
            "packageSchemaIndexIdentity",
        )?;
        relative(format!(
            "records/package-schema-indexes/{package}/{identity}.json"
        ))
        .map(Self)
    }
}

impl PackageSchemaTypeRecordPath {
    pub fn new(reference: &PackageSchemaTypeRecordRef) -> Result<Self> {
        let package = coordinate_segment(&reference.package_id, "packageId")?;
        let identity = identity_hash(
            reference.package_schema_type_id.as_str(),
            PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
            "packageSchemaTypeId",
        )?;
        relative(format!(
            "records/package-schema-types/{package}/{identity}.json"
        ))
        .map(Self)
    }
}

impl PackageFileIrRecordPath {
    pub fn new(package: &PackageArtifactRef, file: &FileIrRef) -> Result<Self> {
        let package_path = PackageArtifactRecordPath::new(package)?;
        let file_hash = identity_hash(
            &file.file_ir_identity,
            FILE_IR_IDENTITY_PREFIX,
            "fileIrIdentity",
        )?;
        let path = package_path
            .as_str()
            .strip_suffix("package.json")
            .expect("package record path suffix is fixed");
        let canonical = format!("{path}file-ir/{file_hash}.json");
        validate_declared_path(file.artifact_path.as_deref(), &canonical, "FileIrRef")?;
        relative(canonical).map(Self)
    }
}

impl PackageResourceRecordPath {
    pub fn new(package: &PackageArtifactRef, resource: &PublicationResourceRef) -> Result<Self> {
        let package_path = PackageArtifactRecordPath::new(package)?;
        validate_sha256(&resource.sha256, "static resource sha256")?;
        let path = package_path
            .as_str()
            .strip_suffix("package.json")
            .expect("package record path suffix is fixed");
        let canonical = format!("{path}resources/{}.blob", resource.sha256);
        validate_declared_path(
            resource.artifact_path.as_deref(),
            &canonical,
            "PublicationResourceRef",
        )?;
        relative(canonical).map(Self)
    }
}

impl ServiceContractRecordPath {
    pub fn new(reference: &ServiceContractRef) -> Result<Self> {
        let service = coordinate_segment(&reference.service_id, "serviceId")?;
        let version = safe_segment(&reference.contract_version, "contractVersion")?;
        let protocol = identity_hash(
            reference.service_protocol_identity.as_str(),
            SERVICE_PROTOCOL_IDENTITY_PREFIX,
            "serviceProtocolIdentity",
        )?;
        relative(format!(
            "records/service-contracts/{service}/{version}/{protocol}.json"
        ))
        .map(Self)
    }
}

impl ServiceDeploymentRecordPath {
    pub fn new(reference: &ServiceDeploymentRef) -> Result<Self> {
        let service = coordinate_segment(&reference.service_id, "serviceId")?;
        let version = safe_segment(&reference.contract_version, "contractVersion")?;
        let revision = safe_segment(reference.deployment_revision.as_str(), "deploymentRevision")?;
        let identity = identity_hash(
            reference.deployment_artifact_identity.as_str(),
            DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
            "deploymentArtifactIdentity",
        )?;
        relative(format!(
            "records/service-deployments/{service}/{version}/{revision}/{identity}.json"
        ))
        .map(Self)
    }
}

impl RuntimeAssemblyRecordPath {
    pub fn new(reference: &RuntimeAssemblyRef) -> Result<Self> {
        let identity = runtime_assembly_identity_hash(reference.assembly_identity.as_str())
            .map_err(|_| ArtifactIdentityError::InvalidArtifactSegment {
                label: "assemblyIdentity".to_string(),
                value: reference.assembly_identity.to_string(),
            })?;
        relative(format!("records/runtime-assemblies/{identity}.json")).map(Self)
    }
}

impl PackageArtifactPointerPath {
    pub fn new(package_id: &str, package_version: &str) -> Result<Self> {
        let package = coordinate_segment(package_id, "packageId")?;
        let version = safe_segment(package_version, "packageVersion")?;
        relative(format!(
            "pointers/package-artifacts/{package}/{version}.json"
        ))
        .map(Self)
    }
}

impl ServiceContractPointerPath {
    pub fn new(service_id: &str, contract_version: &str) -> Result<Self> {
        let service = coordinate_segment(service_id, "serviceId")?;
        let version = safe_segment(contract_version, "contractVersion")?;
        relative(format!(
            "pointers/service-contracts/{service}/{version}.json"
        ))
        .map(Self)
    }
}

impl ServiceDeploymentPointerPath {
    pub fn new(service_id: &str, contract_version: &str) -> Result<Self> {
        let service = coordinate_segment(service_id, "serviceId")?;
        let version = safe_segment(contract_version, "contractVersion")?;
        relative(format!(
            "pointers/service-deployments/{service}/{version}.json"
        ))
        .map(Self)
    }
}

impl RuntimeAssemblyPointerPath {
    pub fn new(release: &str) -> Result<Self> {
        let release = safe_segment(release, "assembly release")?;
        relative(format!("pointers/runtime-assemblies/{release}.json")).map(Self)
    }
}

impl EnvironmentActivationStatePath {
    pub fn new(environment: &str) -> Result<Self> {
        let environment = safe_segment(environment, "environment")?;
        relative(format!("environments/{environment}/activation.json")).map(Self)
    }
}

fn package_coordinate(reference: &PackageArtifactRef) -> Result<String> {
    Ok(format!(
        "{}/{}",
        coordinate_segment(&reference.package_id, "packageId")?,
        safe_segment(&reference.package_version, "packageVersion")?
    ))
}

fn coordinate_segment(value: &str, label: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 200
        || value != value.trim()
        || value.contains('~')
        || value.contains("//")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.bytes().any(|byte| {
            !matches!(
                byte,
                b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'
            )
        })
    {
        return invalid_segment(label, value);
    }
    Ok(value.replace('.', "~d").replace('/', "~s"))
}

fn safe_segment(value: &str, label: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 200
        || value != value.trim()
        || value == "."
        || value == ".."
        || value.bytes().any(|byte| {
            !matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'
            )
        })
    {
        return invalid_segment(label, value);
    }
    Ok(value.to_string())
}

fn identity_hash<'a>(value: &'a str, prefix: &str, label: &str) -> Result<&'a str> {
    let expected = format!("{prefix}:");
    let Some(hash) = value.strip_prefix(&expected) else {
        return invalid_segment(label, value);
    };
    validate_sha256(hash, label)?;
    Ok(hash)
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return invalid_segment(label, value);
    }
    Ok(())
}

fn validate_declared_path(declared: Option<&str>, expected: &str, label: &str) -> Result<()> {
    if let Some(declared) = declared {
        let declared = ArtifactRelativePath::parse(declared, format!("{label}.artifactPath"))?;
        if declared.as_str() != expected {
            return Err(ArtifactIdentityError::NonCanonicalArtifactPath {
                label: label.to_string(),
                path: declared.to_string(),
                expected: expected.to_string(),
            });
        }
    }
    Ok(())
}

fn relative(value: String) -> Result<ArtifactRelativePath> {
    ArtifactRelativePath::parse(&value, "canonical ecosystem path")
}

fn invalid_segment<T>(label: &str, value: &str) -> Result<T> {
    Err(ArtifactIdentityError::InvalidArtifactSegment {
        label: label.to_string(),
        value: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
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
        assert!(EnvironmentActivationStatePath::new("../prod").is_err());
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

        let protocol = ServiceProtocolIdentity::new(format!(
            "{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}",
            hash('c')
        ));
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
            ServiceContractPointerPath::new(
                &adjacent_dots.service_id,
                &adjacent_dots.contract_version
            )
            .unwrap()
        );
    }
}
