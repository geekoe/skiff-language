use serde_json::json;
use skiff_artifact_identity::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, ACTOR_METHOD_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, DeploymentArtifactIdentity,
    DeploymentRevision, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    ServiceDeploymentRef,
};

use super::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingProjectionError, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

mod producer;

fn hex64(byte: u8) -> String {
    assert!(byte.is_ascii_hexdigit(), "fixture byte must be a hex digit");
    String::from_utf8(vec![byte; 64]).expect("fixture hex digit")
}

fn framed(prefix: &str, byte: u8) -> String {
    format!("{prefix}:{}", hex64(byte))
}

fn method(seed: u8) -> ActorRoutingMethod {
    ActorRoutingMethod {
        actor: ActorRoutingRef {
            service_id: "example.com/svc".to_string(),
            actor_abi_identity: ActorAbiIdentity::new(framed(
                ACTOR_ABI_IDENTITY_PREFIX,
                b'0' + seed,
            )),
        },
        actor_implementation_identity: ActorImplementationIdentity::new(framed(
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            b'0' + seed,
        )),
        method_identity: ActorMethodIdentity::new(framed(
            ACTOR_METHOD_IDENTITY_PREFIX,
            b'0' + seed,
        )),
        deployment: ServiceDeploymentRef {
            service_id: "example.com/svc".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("rev-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(framed(
                DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
                b'0' + seed,
            )),
        },
        package: PackageArtifactRef {
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new(framed(
                PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
                b'0' + seed,
            )),
            package_local_abi_identity: PackageLocalAbiIdentity::new(framed(
                PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
                b'0' + seed,
            )),
        },
    }
}

#[test]
fn build_sorts_entries_deterministically() {
    let first = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![method(2), method(0), method(1)],
    )
    .expect("valid projection");
    let second = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![method(1), method(2), method(0)],
    )
    .expect("valid projection");
    let expected = vec![method(0), method(1), method(2)];
    assert_eq!(first.methods, expected);
    assert_eq!(second.methods, expected);
    assert_eq!(first, second);
}

#[test]
fn build_rejects_duplicate_entries() {
    let error = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![method(1), method(1)],
    )
    .expect_err("duplicate entries must fail");
    assert_eq!(error, ActorRoutingProjectionError::DuplicateMethod);
}

#[test]
fn build_rejects_unsupported_schema_version() {
    let error = ActorRoutingProjection::new(
        "skiff-actor-routing-projection-v0".to_string(),
        vec![method(0)],
    )
    .expect_err("unsupported schema version must fail");
    assert!(matches!(
        error,
        ActorRoutingProjectionError::UnsupportedSchemaVersion(_)
    ));
}

#[test]
fn build_rejects_invalid_abi_identity() {
    let mut entry = method(0);
    entry.actor.actor_abi_identity = ActorAbiIdentity::new("not-a-framed-identity".to_string());
    let error = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![entry],
    )
    .expect_err("invalid ABI identity must fail");
    assert!(matches!(
        error,
        ActorRoutingProjectionError::InvalidIdentity { field, .. } if field == "actor.actorAbiIdentity"
    ));
}

#[test]
fn build_rejects_invalid_implementation_identity() {
    let mut entry = method(0);
    entry.actor_implementation_identity =
        ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:zz".to_string());
    let error = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![entry],
    )
    .expect_err("invalid implementation identity must fail");
    assert!(matches!(
        error,
        ActorRoutingProjectionError::InvalidIdentity { field, .. }
            if field == "actorImplementationIdentity"
    ));
}

#[test]
fn build_rejects_invalid_method_identity() {
    let mut entry = method(0);
    entry.method_identity =
        ActorMethodIdentity::new("skiff-actor-method-v1:sha256:not-hex".to_string());
    let error = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![entry],
    )
    .expect_err("invalid method identity must fail");
    assert!(matches!(
        error,
        ActorRoutingProjectionError::InvalidIdentity { field, .. }
            if field == "methodIdentity"
    ));
}

#[test]
fn build_rejects_service_id_mismatch() {
    let mut entry = method(0);
    entry.actor.service_id = "example.com/other".to_string();
    let error = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![entry],
    )
    .expect_err("service id mismatch must fail");
    assert_eq!(error, ActorRoutingProjectionError::ServiceIdMismatch);
}

#[test]
fn build_accepts_empty_methods() {
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection is valid");
    assert!(projection.methods.is_empty());
}

#[test]
fn serde_surface_is_exactly_frozen_schema() {
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![method(0)],
    )
    .expect("valid projection");
    let value = serde_json::to_value(&projection).expect("projection serializes");
    let method_value = &value["methods"][0];
    assert_eq!(method_keys(&value), ["methods", "schemaVersion"].as_slice());
    assert_eq!(
        object_keys(method_value),
        [
            "actor",
            "actorImplementationIdentity",
            "deployment",
            "methodIdentity",
            "package"
        ]
        .as_slice()
    );
    assert_eq!(
        object_keys(&method_value["actor"]),
        ["actorAbiIdentity", "serviceId"].as_slice()
    );
    assert_eq!(
        object_keys(&method_value["deployment"]),
        [
            "contractVersion",
            "deploymentArtifactIdentity",
            "deploymentRevision",
            "serviceId"
        ]
        .as_slice()
    );
    assert_eq!(
        object_keys(&method_value["package"]),
        [
            "packageBuildId",
            "packageId",
            "packageLocalAbiIdentity",
            "packageVersion"
        ]
        .as_slice()
    );
    let decoded: ActorRoutingProjection = serde_json::from_value(value).expect("roundtrip decodes");
    assert_eq!(decoded, projection);
}

#[test]
fn serde_rejects_file_ir_and_payload_coordinates() {
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![method(0)],
    )
    .expect("valid projection");
    let mut value = serde_json::to_value(&projection).expect("projection serializes");
    for forbidden in [
        "declarationOwner",
        "unit",
        "file",
        "actorSymbol",
        "fileIrIdentity",
        "loadedFileIndex",
        "sourceSpan",
        "source",
        "executable",
        "payload",
        "modulePath",
    ] {
        let mut polluted = value.clone();
        polluted["methods"][0][forbidden] = json!({"kind": "fileIrIdentity", "value": 0});
        let error = serde_json::from_value::<ActorRoutingProjection>(polluted)
            .expect_err("forbidden coordinate must be rejected");
        assert!(
            error.to_string().contains("unknown field"),
            "forbidden field {forbidden}: {error}"
        );
    }
    value["source"] = json!("file source must never enter the projection");
    let error = serde_json::from_value::<ActorRoutingProjection>(value)
        .expect_err("top-level source field must be rejected");
    assert!(error.to_string().contains("unknown field"));
}

fn method_keys(value: &serde_json::Value) -> Vec<String> {
    object_keys(value)
}

fn object_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("JSON object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}
