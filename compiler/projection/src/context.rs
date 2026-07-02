#![allow(dead_code)]

use std::{collections::BTreeMap, path::Path, path::PathBuf};

use crate::{
    package_unit_artifacts::ProjectedPackageIrArtifacts, ServiceAccessProjectionConfig,
    TimeoutProjectionConfig, WebSocketGatewayProjectionConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use skiff_artifact_model::{InterfaceInstantiationRef, OperationAbiRef};
use skiff_compiler_projection_input::PackageProjectionInput;

use crate::prelude::PreludeProjection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionPolicy {
    Service,
    Package,
}

#[derive(Debug, Clone)]
pub enum ProjectionContext<'a> {
    Service(ServiceProjectionContext<'a>),
    Package(PackageProjectionContext<'a>),
}

impl<'a> ProjectionContext<'a> {
    pub fn for_service(input: ServiceProjectionContextInput<'a>) -> Self {
        Self::Service(ServiceProjectionContext::new(input))
    }

    pub fn for_package(
        package_id: &'a str,
        version: &'a str,
        dependencies: Vec<ProjectedPackageDependency>,
        api_entries: Vec<PackageApiEntryProjection>,
        api_source: Option<PackageApiSourceProjection>,
        package_root: &'a Path,
        prelude: &'a PreludeProjection,
        _dependency_artifacts: &'a BTreeMap<String, impl Sized>,
    ) -> Self {
        Self::Package(PackageProjectionContext::new(
            PackageProjectionContextInput {
                package_id,
                version,
                dependencies,
                api_entries,
                api_source,
                package_root,
                prelude,
            },
        ))
    }

    pub fn policy(&self) -> ProjectionPolicy {
        match self {
            Self::Service(_) => ProjectionPolicy::Service,
            Self::Package(_) => ProjectionPolicy::Package,
        }
    }

    pub fn as_service(&self) -> Option<&ServiceProjectionContext<'a>> {
        match self {
            Self::Service(context) => Some(context),
            Self::Package(_) => None,
        }
    }

    pub fn as_package(&self) -> Option<&PackageProjectionContext<'a>> {
        match self {
            Self::Service(_) => None,
            Self::Package(context) => Some(context),
        }
    }

    pub fn prelude(&self) -> &PreludeProjection {
        match self {
            Self::Service(context) => context.prelude(),
            Self::Package(context) => context.prelude(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServiceProjectionContext<'a> {
    service_id: &'a str,
    service_target_component: &'a str,
    access: &'a ServiceAccessProjectionConfig,
    timeout: &'a TimeoutProjectionConfig,
    publication_api_has_entries: bool,
    websocket_gateway: Option<&'a WebSocketGatewayProjectionConfig>,
    service_version: &'a str,
    service_http_response_max_bytes: Option<u64>,
    package_publications: &'a [PackageProjectionInput],
    package_artifacts: &'a [ProjectedPackageIrArtifacts],
    package_dependencies: Vec<ProjectedPackageDependency>,
    api_source: Option<PackageApiSourceProjection>,
    prelude: &'a PreludeProjection,
}

impl<'a> ServiceProjectionContext<'a> {
    pub fn new(input: ServiceProjectionContextInput<'a>) -> Self {
        Self {
            service_id: input.service_id,
            service_target_component: input.service_target_component,
            access: input.access,
            timeout: input.timeout,
            publication_api_has_entries: input.publication_api_has_entries,
            websocket_gateway: input.websocket_gateway,
            service_version: input.service_version,
            service_http_response_max_bytes: input.service_http_response_max_bytes,
            package_publications: input.package_publications,
            package_artifacts: input.package_artifacts,
            package_dependencies: input.package_dependencies,
            api_source: input.api_source,
            prelude: input.prelude,
        }
    }

    pub fn service_id(&self) -> &'a str {
        self.service_id
    }

    pub fn service_target_component(&self) -> &'a str {
        self.service_target_component
    }

    pub fn access(&self) -> &'a ServiceAccessProjectionConfig {
        self.access
    }

    pub fn timeout(&self) -> &'a TimeoutProjectionConfig {
        self.timeout
    }

    pub fn publication_api_has_entries(&self) -> bool {
        self.publication_api_has_entries
    }

    pub fn websocket_gateway(&self) -> Option<&'a WebSocketGatewayProjectionConfig> {
        self.websocket_gateway
    }

    pub fn service_version(&self) -> &'a str {
        self.service_version
    }

    pub fn service_http_response_max_bytes(&self) -> Option<u64> {
        self.service_http_response_max_bytes
    }

    pub fn package_publications(&self) -> &'a [PackageProjectionInput] {
        self.package_publications
    }

    pub fn package_artifacts(&self) -> &'a [ProjectedPackageIrArtifacts] {
        self.package_artifacts
    }

    pub fn package_dependencies(&self) -> &[ProjectedPackageDependency] {
        &self.package_dependencies
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceProjection> {
        self.api_source.as_ref()
    }

    pub fn prelude(&self) -> &'a PreludeProjection {
        self.prelude
    }
}

#[derive(Debug, Clone)]
pub struct PackageProjectionContext<'a> {
    manifest: PackageManifest,
    api_source: Option<PackageApiSourceProjection>,
    package_root: &'a Path,
    prelude: &'a PreludeProjection,
}

impl<'a> PackageProjectionContext<'a> {
    pub fn new(input: PackageProjectionContextInput<'a>) -> Self {
        let manifest = PackageManifest {
            id: input.package_id.to_string(),
            version: input.version.to_string(),
            dependencies: input.dependencies,
            api: PackageApiProjection {
                entries: input.api_entries,
                public_instances: Vec::new(),
            },
        };
        Self {
            manifest,
            api_source: input.api_source,
            package_root: input.package_root,
            prelude: input.prelude,
        }
    }

    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    pub fn package_id(&self) -> &str {
        &self.manifest.id
    }

    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    pub fn dependencies(&self) -> &[ProjectedPackageDependency] {
        &self.manifest.dependencies
    }

    pub fn api_entries(&self) -> &[PackageApiEntryProjection] {
        &self.manifest.api.entries
    }

    pub fn api_source(&self) -> Option<&PackageApiSourceProjection> {
        self.api_source.as_ref()
    }

    pub fn package_root(&self) -> &'a Path {
        self.package_root
    }

    pub fn prelude(&self) -> &'a PreludeProjection {
        self.prelude
    }
}

pub struct ServiceProjectionContextInput<'a> {
    pub service_id: &'a str,
    pub service_target_component: &'a str,
    pub access: &'a ServiceAccessProjectionConfig,
    pub timeout: &'a TimeoutProjectionConfig,
    pub publication_api_has_entries: bool,
    pub websocket_gateway: Option<&'a WebSocketGatewayProjectionConfig>,
    pub service_version: &'a str,
    pub service_http_response_max_bytes: Option<u64>,
    pub package_publications: &'a [PackageProjectionInput],
    pub package_artifacts: &'a [ProjectedPackageIrArtifacts],
    pub package_dependencies: Vec<ProjectedPackageDependency>,
    pub api_source: Option<PackageApiSourceProjection>,
    pub prelude: &'a PreludeProjection,
}

pub struct PackageProjectionContextInput<'a> {
    pub package_id: &'a str,
    pub version: &'a str,
    pub dependencies: Vec<ProjectedPackageDependency>,
    pub api_entries: Vec<PackageApiEntryProjection>,
    pub api_source: Option<PackageApiSourceProjection>,
    pub package_root: &'a Path,
    pub prelude: &'a PreludeProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedPackageDependency {
    pub id: String,
    pub version: String,
    pub alias: Option<String>,
    pub config: Value,
    pub collection_name_mapping: BTreeMap<String, String>,
}

impl ProjectedPackageDependency {
    pub fn effective_alias(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| {
            if self.id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID {
                "std"
            } else {
                &self.id
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageApiEntryProjection {
    pub path: String,
    pub module: String,
}

impl PackageApiEntryProjection {
    pub fn public_path_string(&self) -> String {
        self.path.clone()
    }

    pub fn source_module_hint(&self) -> &str {
        &self.module
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageApiProjection {
    entries: Vec<PackageApiEntryProjection>,
    public_instances: Vec<PackageApiEntryProjection>,
}

impl PackageApiProjection {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            public_instances: Vec::new(),
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &PackageApiEntryProjection> {
        self.entries.iter()
    }

    pub fn public_instances(&self) -> impl Iterator<Item = &PackageApiEntryProjection> {
        self.public_instances.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    pub id: String,
    pub version: String,
    pub dependencies: Vec<ProjectedPackageDependency>,
    pub api: PackageApiProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageApiSourceProjection {
    pub relative_path: PathBuf,
    pub content_hash: String,
}

pub fn empty_dependency_config() -> Value {
    Value::Object(Map::new())
}

pub fn dependency_config_is_empty(value: &Value) -> bool {
    matches!(value, Value::Object(object) if object.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedServiceDependencyLockEntry {
    kind: String,
    id: String,
    version: String,
    alias: String,
    declared_alias: String,
    build_id: String,
    service_protocol_identity: String,
    operations: Vec<String>,
    targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    remote_box_provenance: Vec<ProjectedServiceDependencyRemoteBoxProvenance>,
}

impl ProjectedServiceDependencyLockEntry {
    pub fn from_serializable(source: &impl Serialize) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(source)?)
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn add_remote_box_provenance(
        &mut self,
        provenance: ProjectedServiceDependencyRemoteBoxProvenance,
    ) {
        if !self.remote_box_provenance.contains(&provenance) {
            self.remote_box_provenance.push(provenance);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedServiceDependencyRemoteBoxProvenance {
    pub interface: InterfaceInstantiationRef,
    pub interface_display: String,
    pub public_instance: String,
    pub method_abi_id: String,
    pub operation: OperationAbiRef,
}
