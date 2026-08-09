use std::collections::BTreeMap;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::{
    actor_declaration::{
        ActorAbiIdentity, ActorAbiInput, ActorImplementationIdentity, ActorMethodIdentity,
    },
    boundary::{BoundaryCallableProjection, CallableSemanticFacts},
    compile_identity::{
        PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageSchemaTypeId,
    },
    compile_requirements::{
        ContractRequirement, PackageRequirement, PackageRuntimeRequirements, ServiceCallRef,
        ServiceRequirement,
    },
    contract_types::{
        PackageSchemaIndexRef, PackageSchemaTypeRecord, PackageSchemaTypeRecordRef, PackageTypeRef,
    },
    executable::ParamModeIr,
    executable_target::OperationTargetRef,
    package_unit::{InterfaceMethodSignature, PackageImplementationLinks},
    publication_abi::InterfaceInstantiationRef,
    refs::{BytecodeArtifactRef, FileIrRef},
    resources::PublicationResourceRef,
    symbols::ServiceSymbolRef,
    types::{TypeDescriptorIr, TypeRefIr},
};

mod authority;
mod schema_records;

pub use authority::{
    derive_synthetic_callback_callable_id, validate_package_build_authority,
    PackageBuildAuthorityValidationError, PackageSyntheticCallbackOwner,
    MAX_PACKAGE_SYNTHETIC_CALLBACK_OWNERS, PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX,
    PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_SCHEMA_MARKER,
};
pub use schema_records::{
    derive_package_schema_type_id, validate_bytecode_schema_records,
    MAX_BYTECODE_SCHEMA_CANONICAL_BYTES, MAX_BYTECODE_SCHEMA_DEPTH, MAX_BYTECODE_SCHEMA_RECORDS,
    MAX_BYTECODE_SCHEMA_STRING_BYTES, MAX_BYTECODE_SCHEMA_TYPE_NODES,
    PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX, PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
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

/// Exact actor create implementation selected by the owning package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageActorCreateBinding {
    pub method_identity: ActorMethodIdentity,
    pub package_callable_id: PackageCallableId,
}

/// Build-owned actor implementation authority. ABI shape is joined through
/// `PackageActorAbi`; callable targets are joined through `callable_links`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageActorImplementation {
    pub actor: ServiceSymbolRef,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub methods: BTreeMap<ActorMethodIdentity, PackageCallableId>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub create: Option<PackageActorCreateBinding>,
}

/// Exact package-local conformance table. Method vector index is the
/// interface method slot; rows carry no executable or build identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageLocalInterfaceConformance {
    pub type_parameters: Vec<String>,
    pub receiver: TypeRefIr,
    pub interface: InterfaceInstantiationRef,
    pub methods: Vec<PackageCallableId>,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn deserialize_canonical_actor_implementations<'de, D>(
    deserializer: D,
) -> Result<Vec<PackageActorImplementation>, D::Error>
where
    D: Deserializer<'de>,
{
    let rows = Vec::<PackageActorImplementation>::deserialize(deserializer)?;
    for adjacent in rows.windows(2) {
        let left = &adjacent[0];
        let right = &adjacent[1];
        let left_key = (left.actor.module_path.as_str(), left.actor.symbol.as_str());
        let right_key = (
            right.actor.module_path.as_str(),
            right.actor.symbol.as_str(),
        );
        if left_key >= right_key {
            return Err(D::Error::custom(
                "actorImplementations must be strictly ordered and unique by actor",
            ));
        }
    }
    Ok(rows)
}

fn deserialize_canonical_local_interface_conformances<'de, D>(
    deserializer: D,
) -> Result<Vec<PackageLocalInterfaceConformance>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SortKey<'a> {
        type_parameters: &'a [String],
        receiver: &'a TypeRefIr,
        interface: &'a InterfaceInstantiationRef,
    }

    let rows = Vec::<PackageLocalInterfaceConformance>::deserialize(deserializer)?;
    let mut previous: Option<Vec<u8>> = None;
    for row in &rows {
        let key = skiff_canonical_json::canonical_json_bytes(&SortKey {
            type_parameters: &row.type_parameters,
            receiver: &row.receiver,
            interface: &row.interface,
        })
        .map_err(D::Error::custom)?;
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(D::Error::custom(
                "localInterfaceConformances must be strictly ordered by template, receiver, and interface",
            ));
        }
        previous = Some(key);
    }
    Ok(rows)
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
    /// Canonical synthetic callback owners. The owning ordinary executable is
    /// path-free; the canonical callable id is derived from its exact
    /// implementation callable id plus `siteOrdinal`.
    #[serde(deserialize_with = "authority::deserialize_synthetic_callback_owners")]
    pub synthetic_callback_owners: Vec<PackageSyntheticCallbackOwner>,
    /// Owner-local PackageSchema descriptor closure used only by linked
    /// bytecode. Cross-package children remain references and are hydrated
    /// from the target owner's PackageArtifact. Required even when empty and
    /// included in PackageBuild identity, never PackageLocalAbi identity.
    #[serde(deserialize_with = "schema_records::deserialize_bytecode_schema_records")]
    pub bytecode_schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    #[serde(deserialize_with = "deserialize_canonical_actor_implementations")]
    pub actor_implementations: Vec<PackageActorImplementation>,
    #[serde(deserialize_with = "deserialize_canonical_local_interface_conformances")]
    pub local_interface_conformances: Vec<PackageLocalInterfaceConformance>,
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
