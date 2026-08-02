//! Source-free actor routing projection (authoritative design §2.4).
//!
//! This module freezes the minimal actor routing projection consumed by the
//! stateless `ActorMethodCatalogView` inside an immutable `RoutingEpoch`
//! (plan §3.2 / §3.3). It carries exactly:
//!
//! - a stable actor ref (`service_id` + `actor_abi_identity`);
//! - method admission / implementation identity
//!   (`actor_abi_identity` + `actor_implementation_identity` + `method_identity`);
//! - exact deployment binding (`ServiceDeploymentRef` + owning `PackageArtifactRef`).
//!
//! It deliberately never carries source, File IR coordinates, symbol paths, or
//! executable payloads. Identity generation itself stays in
//! `skiff-artifact-identity` (`actor_abi_identity`, `actor_method_identity`,
//! `actor_implementation_identity`); this module only carries the generated
//! framed identities and validates their shape.
//!
//! A1 additionally owns the deployment-side producer: it consumes a source-free
//! typed input that carries only generated framed identities and emits the
//! frozen projection. The producer never reads File IR records, source text or
//! executable payloads; the compiler side is responsible for supplying the
//! typed facts (see the A1 leaf task).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use skiff_artifact_identity::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, ACTOR_METHOD_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, PackageArtifactRef,
    ServiceDeploymentRef,
};

/// Frozen schema version of the actor routing projection.
pub const ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION: &str = "skiff-actor-routing-projection-v1";

/// Canonical relative record path of the current actor routing projection.
///
/// A1 owns the producer output surface (A3 leaf D2): the compiler publish
/// path writes this mutable "current" record and the A2 TS loader / A3 Rust
/// strict reader consume the same relative path inside the artifact root.
pub const ACTOR_ROUTING_PROJECTION_RECORD_PATH: &str = "records/actor-routing/current.json";

/// Stable, source-free actor declaration reference.
///
/// `actor_abi_identity` canonically covers the actor type, key field type and
/// key canonical encoding (actor-model.md "任期与 Version"); `service_id` is
/// the actor's home service, which is part of the actor identity. Service
/// version / build id never enter the ref.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRoutingRef {
    pub service_id: String,
    pub actor_abi_identity: ActorAbiIdentity,
}

/// One actor method routing entry in the frozen projection.
///
/// `actor` is the stable actor ref, `actor_implementation_identity` and
/// `method_identity` are the implementation / admission identities, and
/// `deployment` + `package` are the exact immutable deployment binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRoutingMethod {
    pub actor: ActorRoutingRef,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
    pub deployment: ServiceDeploymentRef,
    pub package: PackageArtifactRef,
}

/// Immutable actor routing projection.
///
/// Entries are sorted by their full typed key and must be unique. The
/// projection never contains source, File IR or executable payload; the
/// serde shape uses `deny_unknown_fields` so File-IR-style coordinates are
/// rejected at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "ActorRoutingProjectionWire"
)]
pub struct ActorRoutingProjection {
    pub schema_version: String,
    pub methods: Vec<ActorRoutingMethod>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorRoutingProjectionWire {
    schema_version: String,
    methods: Vec<ActorRoutingMethod>,
}

impl TryFrom<ActorRoutingProjectionWire> for ActorRoutingProjection {
    type Error = ActorRoutingProjectionError;

    fn try_from(wire: ActorRoutingProjectionWire) -> Result<Self, Self::Error> {
        Self::new(wire.schema_version, wire.methods)
    }
}

impl ActorRoutingProjection {
    /// Builds and validates the projection.
    ///
    /// Validation is order-independent: entries are sorted by their full typed
    /// key, duplicates are rejected, and every identity / binding field is
    /// checked before the projection is accepted.
    pub fn new(
        schema_version: String,
        mut methods: Vec<ActorRoutingMethod>,
    ) -> Result<Self, ActorRoutingProjectionError> {
        if schema_version != ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION {
            return Err(ActorRoutingProjectionError::UnsupportedSchemaVersion(
                schema_version,
            ));
        }
        for method in &methods {
            validate_method(method)?;
        }
        methods.sort();
        let mut seen = BTreeSet::new();
        for method in &methods {
            if !seen.insert(method.clone()) {
                return Err(ActorRoutingProjectionError::DuplicateMethod);
            }
        }
        Ok(Self {
            schema_version,
            methods,
        })
    }
}

/// Frozen schema version of the source-free producer input.
pub const ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION: &str =
    "skiff-actor-routing-producer-input-v1";

/// Complete typed, source-free input for the actor routing projection producer.
///
/// Carries only generated framed identity strings: no module path, actor name,
/// method name, executable coordinates or source facts are admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRoutingProducerInput {
    pub schema_version: String,
    pub deployment: ServiceDeploymentRef,
    pub packages: Vec<ActorRoutingPackageInput>,
}

/// One owning package's actor routing facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRoutingPackageInput {
    pub package: PackageArtifactRef,
    pub actors: Vec<ActorRoutingActorInput>,
}

/// Source-free actor facts for one package actor declaration.
///
/// `methods` are the public method admission identities; the create
/// implementation is not a method catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRoutingActorInput {
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub methods: Vec<ActorMethodIdentity>,
}

/// Projects typed actor routing facts into the frozen projection.
///
/// Each public method identity expands to one `ActorRoutingMethod` entry bound
/// to the exact deployment and owning package. The frozen
/// `ActorRoutingProjection::new` performs the shared ordering, uniqueness and
/// identity validation; producer input checks fail closed before expansion.
pub fn project_actor_routing(
    input: ActorRoutingProducerInput,
) -> Result<ActorRoutingProjection, ActorRoutingProjectionError> {
    if input.schema_version != ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION {
        return Err(
            ActorRoutingProjectionError::ProducerUnsupportedSchemaVersion(input.schema_version),
        );
    }
    validate_deployment_ref(&input.deployment)?;

    let mut methods = Vec::new();
    for package in &input.packages {
        validate_package_ref(&package.package)?;
        let mut actor_keys = BTreeSet::new();
        for actor in &package.actors {
            let key = (
                actor.actor_abi_identity.clone(),
                actor.actor_implementation_identity.clone(),
            );
            if !actor_keys.insert(key) {
                return Err(ActorRoutingProjectionError::ProducerDuplicateActor);
            }
            if actor.methods.is_empty() {
                return Err(ActorRoutingProjectionError::ProducerActorWithoutMethods);
            }
            let mut seen_methods = BTreeSet::new();
            for method_identity in &actor.methods {
                if !seen_methods.insert(method_identity.clone()) {
                    return Err(ActorRoutingProjectionError::ProducerDuplicateActorMethod);
                }
                methods.push(ActorRoutingMethod {
                    actor: ActorRoutingRef {
                        service_id: input.deployment.service_id.clone(),
                        actor_abi_identity: actor.actor_abi_identity.clone(),
                    },
                    actor_implementation_identity: actor.actor_implementation_identity.clone(),
                    method_identity: method_identity.clone(),
                    deployment: input.deployment.clone(),
                    package: package.package.clone(),
                });
            }
        }
    }
    ActorRoutingProjection::new(ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(), methods)
}

fn validate_deployment_ref(
    deployment: &ServiceDeploymentRef,
) -> Result<(), ActorRoutingProjectionError> {
    validate_nonempty(&deployment.service_id, "deployment.serviceId")?;
    validate_nonempty(
        deployment.contract_version.as_str(),
        "deployment.contractVersion",
    )?;
    validate_nonempty(
        deployment.deployment_revision.as_str(),
        "deployment.deploymentRevision",
    )?;
    validate_framed(
        deployment.deployment_artifact_identity.as_str(),
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        "deployment.deploymentArtifactIdentity",
    )
}

fn validate_package_ref(package: &PackageArtifactRef) -> Result<(), ActorRoutingProjectionError> {
    validate_nonempty(&package.package_id, "package.packageId")?;
    validate_nonempty(&package.package_version, "package.packageVersion")?;
    validate_framed(
        package.package_build_id.as_str(),
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "package.packageBuildId",
    )?;
    validate_framed(
        package.package_local_abi_identity.as_str(),
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
        "package.packageLocalAbiIdentity",
    )
}

fn validate_method(method: &ActorRoutingMethod) -> Result<(), ActorRoutingProjectionError> {
    validate_nonempty(&method.actor.service_id, "actor.serviceId")?;
    if method.actor.service_id != method.deployment.service_id {
        return Err(ActorRoutingProjectionError::ServiceIdMismatch);
    }
    validate_framed(
        method.actor.actor_abi_identity.as_str(),
        ACTOR_ABI_IDENTITY_PREFIX,
        "actor.actorAbiIdentity",
    )?;
    validate_framed(
        method.actor_implementation_identity.as_str(),
        ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
        "actorImplementationIdentity",
    )?;
    validate_framed(
        method.method_identity.as_str(),
        ACTOR_METHOD_IDENTITY_PREFIX,
        "methodIdentity",
    )?;
    validate_nonempty(
        method.deployment.contract_version.as_str(),
        "deployment.contractVersion",
    )?;
    validate_nonempty(
        method.deployment.deployment_revision.as_str(),
        "deployment.deploymentRevision",
    )?;
    validate_framed(
        method.deployment.deployment_artifact_identity.as_str(),
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        "deployment.deploymentArtifactIdentity",
    )?;
    validate_nonempty(&method.package.package_id, "package.packageId")?;
    validate_nonempty(&method.package.package_version, "package.packageVersion")?;
    validate_framed(
        method.package.package_build_id.as_str(),
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "package.packageBuildId",
    )?;
    validate_framed(
        method.package.package_local_abi_identity.as_str(),
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
        "package.packageLocalAbiIdentity",
    )?;
    Ok(())
}

fn validate_framed(
    value: &str,
    prefix: &str,
    field: &'static str,
) -> Result<(), ActorRoutingProjectionError> {
    let Some(rest) = value.strip_prefix(prefix) else {
        return Err(ActorRoutingProjectionError::InvalidIdentity {
            field,
            value: value.to_string(),
        });
    };
    let Some(hex) = rest.strip_prefix(':') else {
        return Err(ActorRoutingProjectionError::InvalidIdentity {
            field,
            value: value.to_string(),
        });
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ActorRoutingProjectionError::InvalidIdentity {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), ActorRoutingProjectionError> {
    if value.trim().is_empty() {
        return Err(ActorRoutingProjectionError::InvalidIdentity {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

/// Construction / validation failure of the actor routing projection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActorRoutingProjectionError {
    #[error("unsupported actor routing projection schemaVersion {0:?}")]
    UnsupportedSchemaVersion(String),
    #[error("unsupported actor routing producer input schemaVersion {0:?}")]
    ProducerUnsupportedSchemaVersion(String),
    #[error("invalid actor routing projection field {field}: {value:?}")]
    InvalidIdentity { field: &'static str, value: String },
    #[error("actor routing ref serviceId must match its deployment serviceId")]
    ServiceIdMismatch,
    #[error("actor routing projection contains duplicate method entries")]
    DuplicateMethod,
    #[error("actor routing producer input package declares duplicate actor facts")]
    ProducerDuplicateActor,
    #[error("actor routing producer input actor declares no methods")]
    ProducerActorWithoutMethods,
    #[error("actor routing producer input actor declares duplicate method identities")]
    ProducerDuplicateActorMethod,
}

#[cfg(test)]
mod tests;
