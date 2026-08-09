use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    actor_declaration::{ActorAbiIdentity, ActorAbiInput},
    boundary::{BoundaryCallableProjection, CallableSemanticFacts},
    compile_identity::{
        PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageSchemaTypeId,
    },
    compile_requirements::{
        ContractRequirement, PackageRequirement, PackageRuntimeRequirements, ServiceCallRef,
        ServiceRequirement,
    },
    contract_types::{PackageSchemaIndexRef, PackageSchemaTypeRecordRef, PackageTypeRef},
    executable::ParamModeIr,
    executable_target::OperationTargetRef,
    package_unit::{InterfaceMethodSignature, PackageImplementationLinks},
    refs::{BytecodeArtifactRef, FileIrRef},
    resources::PublicationResourceRef,
    types::{TypeDescriptorIr, TypeRefIr},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableParameter {
    pub name: String,
    pub ty: PackageTypeRef,
    /// Calling convention is part of the package-local ABI and is required
    /// on the wire even for ordinary value parameters.
    pub mode: ParamModeIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableSignature {
    pub type_params: Vec<String>,
    pub parameters: Vec<PackageCallableParameter>,
    pub return_type: PackageTypeRef,
    pub may_suspend: bool,
}

/// Actor facts projected from a `FileIrUnit.actor_declarations` entry onto the
/// attached nominal type. The ABI identity is carried verbatim from the
/// lowered declaration; the `abi` shape is normalized to the owning package's
/// artifact view so dependents can resolve actor key/create/method surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageActorAbi {
    pub actor_abi_identity: ActorAbiIdentity,
    pub abi: ActorAbiInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
// This is a cold-path typed artifact DTO; boxing a variant would change its
// public construction API solely to optimize a non-hot representation.
#[allow(clippy::large_enum_variant)]
pub enum PackageLocalAbiSymbol {
    Type {
        local_type_id: String,
        descriptor: TypeDescriptorIr,
        /// True only for a transparent source `alias` declaration. Nominal
        /// representation declarations also use an alias-shaped descriptor,
        /// but remain real package types.
        is_alias: bool,
        #[serde(default)]
        is_interface: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        type_params: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        interface_methods: Vec<InterfaceMethodSignature>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<PackageActorAbi>,
    },
    Callable {
        callable_id: PackageCallableId,
        signature: PackageCallableSignature,
    },
    Constant {
        const_id: String,
        ty: PackageTypeRef,
    },
    PublicInstance {
        instance_id: String,
        declared_receiver_type: TypeRefIr,
        interfaces: Vec<TypeRefIr>,
        methods: BTreeMap<String, PackageCallableId>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageLocalAbi {
    pub local_abi_identity: PackageLocalAbiIdentity,
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    /// Exact implementation source addresses available only through a
    /// test-service dependency's `topLevelAlias`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableLinkFact {
    pub callable_id: PackageCallableId,
    pub target: OperationTargetRef,
}

/// Canonical user-code artifact. No publication or service aggregate is
/// embedded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifact {
    pub schema_version: String,
    pub package_id: String,
    pub package_version: String,
    pub package_build_id: PackageBuildId,
    pub files: Vec<FileIrRef>,
    pub static_resources: Vec<PublicationResourceRef>,
    /// Bytecode image record reference (D11: one image per package; `None`
    /// during the migration period before a package gains bytecode). The
    /// identity enters the build preimage only when present (D18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<BytecodeArtifactRef>,
    pub package_local_abi: PackageLocalAbi,
    pub package_schema_index: PackageSchemaIndexRef,
    pub package_schema_type_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
    pub implementation_links: PackageImplementationLinks,
    pub callable_links: BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    pub package_requirements: Vec<PackageRequirement>,
    pub contract_requirements: Vec<ContractRequirement>,
    pub service_requirements: Vec<ServiceRequirement>,
    pub runtime_requirements: PackageRuntimeRequirements,
    pub callable_semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    pub boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
    pub service_call_refs: Vec<ServiceCallRef>,
}

#[cfg(test)]
mod tests;
