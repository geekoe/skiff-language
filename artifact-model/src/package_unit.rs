use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    abi_identity::AbiIdentityFacts,
    executable::ExecutableSignatureIr,
    metadata::MetadataValue,
    publication_abi::{OperationAbiRef, PublicationAbiUnit},
    recoverable::RecoverableArtifactMetadata,
    refs::FileIrRef,
    resources::PublicationResourceRef,
    schema::PACKAGE_UNIT_SCHEMA_VERSION,
    service_unit::{
        LocalReceiverExecutableRef, OperationCallableKind, OperationTargetRef, PublicInstanceExport,
    },
    types::{FunctionTypeParamIr, TypeDescriptorIr, TypeRefIr},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageUnit {
    pub schema_version: String,
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub abi_identity: String,
    #[serde(default, skip_serializing_if = "AbiIdentityFacts::is_empty")]
    pub abi_identity_projection: AbiIdentityFacts,
    pub publication_abi: PublicationAbiUnit,
    pub files: Vec<FileIrRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<PublicationResourceRef>,
    #[serde(default, skip_serializing_if = "PackageImplementationLinks::is_empty")]
    pub implementation_links: PackageImplementationLinks,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<PackageDependencyConstraint>,
    #[serde(default, skip_serializing_if = "RecoverableArtifactMetadata::is_empty")]
    pub recoverable_metadata: RecoverableArtifactMetadata,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
}

impl PackageUnit {
    pub fn empty(
        package_id: impl Into<String>,
        version: impl Into<String>,
        build_identity: impl Into<String>,
        abi_identity: impl Into<String>,
    ) -> Self {
        let package_id = package_id.into();
        let version = version.into();
        let abi_identity = abi_identity.into();
        Self {
            schema_version: PACKAGE_UNIT_SCHEMA_VERSION.to_string(),
            package_id: package_id.clone(),
            version: version.clone(),
            build_identity: build_identity.into(),
            abi_identity: abi_identity.clone(),
            abi_identity_projection: AbiIdentityFacts::default(),
            publication_abi: PublicationAbiUnit::empty(package_id, version, abi_identity),
            files: Vec::new(),
            resources: Vec::new(),
            implementation_links: PackageImplementationLinks::default(),
            dependencies: Vec::new(),
            recoverable_metadata: RecoverableArtifactMetadata::default(),
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageExportIndex {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub functions: BTreeMap<String, ExecutableExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub impl_methods: BTreeMap<String, ExecutableExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_instances: Vec<PublicInstanceExport>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageImplementationLinks {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, TypeExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, ConstExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub functions: BTreeMap<String, ExecutableExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub impl_methods: BTreeMap<String, ExecutableExport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operation_targets: BTreeMap<String, PackageOperationTarget>,
}

impl PackageImplementationLinks {
    pub fn from_exports(exports: &PackageExportIndex) -> Self {
        Self {
            types: exports.types.clone(),
            constants: exports.constants.clone(),
            functions: exports.functions.clone(),
            impl_methods: exports.impl_methods.clone(),
            operation_targets: BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.constants.is_empty()
            && self.functions.is_empty()
            && self.impl_methods.is_empty()
            && self.operation_targets.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum PackageOperationTarget {
    LocalExecutable {
        operation: OperationAbiRef,
        target: OperationTargetRef,
    },
    LocalConstReceiverExecutable {
        operation: OperationAbiRef,
        target: LocalReceiverExecutableRef,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeExport {
    pub file: FileIrRef,
    pub type_index: u32,
    #[serde(default)]
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<TypeDescriptorIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface_methods: Vec<InterfaceMethodSignature>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableExport {
    pub file: FileIrRef,
    pub executable_index: u32,
    #[serde(default)]
    pub symbol: String,
    pub signature: ExecutableSignatureIr,
}

impl ExecutableExport {
    pub fn operation_target_ref(
        &self,
        callable_abi_id: impl Into<String>,
        callable_kind: OperationCallableKind,
    ) -> OperationTargetRef {
        OperationTargetRef {
            file_ref: self.file.clone(),
            executable_index: self.executable_index,
            callable_abi_id: callable_abi_id.into(),
            callable_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstExport {
    pub file: FileIrRef,
    pub const_index: u32,
    #[serde(default)]
    pub symbol: String,
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyConstraint {
    pub id: String,
    pub version: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "dependency_config_is_empty")]
    pub config: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceMethodSignature {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub is_native: bool,
    #[serde(default)]
    pub is_provider: bool,
    pub is_static: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implicit_self: Option<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageAbiExpectation {
    pub id: String,
    pub version: String,
    pub abi_identity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_symbols: Vec<PackageUsedSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageUsedSymbol {
    pub symbol_path: String,
    pub kind: PackageUsedSymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PackageUsedSymbolKind {
    Type,
    Function,
    ImplMethod,
    Const,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigAndEffectMetadata {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, MetadataValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub effects: BTreeMap<String, EffectMetadata>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, MetadataValue>,
}

fn dependency_config_is_empty(value: &Value) -> bool {
    matches!(value, Value::Null)
}
