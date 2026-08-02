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

use super::super::{
    ActorRoutingActorInput, ActorRoutingMethod, ActorRoutingPackageInput,
    ActorRoutingProducerInput, ActorRoutingProjection, ActorRoutingProjectionError,
    ActorRoutingRef, ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use super::framed;

fn deployment(seed: u8) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/docs".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("rev-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(framed(
            DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
            seed,
        )),
    }
}

fn package(seed: u8) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: format!("example.com/pkg-{}", seed as char),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(framed(PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, seed)),
        package_local_abi_identity: PackageLocalAbiIdentity::new(framed(
            PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
            seed,
        )),
    }
}

fn actor(abi_seed: u8, implementation_seed: u8, method_seeds: &[u8]) -> ActorRoutingActorInput {
    ActorRoutingActorInput {
        actor_abi_identity: ActorAbiIdentity::new(framed(ACTOR_ABI_IDENTITY_PREFIX, abi_seed)),
        actor_implementation_identity: ActorImplementationIdentity::new(framed(
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            implementation_seed,
        )),
        methods: method_seeds
            .iter()
            .map(|seed| ActorMethodIdentity::new(framed(ACTOR_METHOD_IDENTITY_PREFIX, *seed)))
            .collect(),
    }
}

fn input(
    deployment: ServiceDeploymentRef,
    packages: Vec<ActorRoutingPackageInput>,
) -> ActorRoutingProducerInput {
    ActorRoutingProducerInput {
        schema_version: ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION.to_string(),
        deployment,
        packages,
    }
}

fn entry(
    deployment: &ServiceDeploymentRef,
    package: &PackageArtifactRef,
    actor_abi_seed: u8,
    implementation_seed: u8,
    method_seed: u8,
) -> ActorRoutingMethod {
    ActorRoutingMethod {
        actor: ActorRoutingRef {
            service_id: deployment.service_id.clone(),
            actor_abi_identity: ActorAbiIdentity::new(framed(
                ACTOR_ABI_IDENTITY_PREFIX,
                actor_abi_seed,
            )),
        },
        actor_implementation_identity: ActorImplementationIdentity::new(framed(
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            implementation_seed,
        )),
        method_identity: ActorMethodIdentity::new(framed(
            ACTOR_METHOD_IDENTITY_PREFIX,
            method_seed,
        )),
        deployment: deployment.clone(),
        package: package.clone(),
    }
}

fn expected_projection(entries: Vec<ActorRoutingMethod>) -> ActorRoutingProjection {
    ActorRoutingProjection::new(ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(), entries)
        .expect("expected projection is valid")
}

#[test]
fn producer_expands_each_public_method_into_a_bound_entry() {
    let deployment = deployment(b'a');
    let first = package(b'b');
    let second = package(b'c');
    let input = input(
        deployment.clone(),
        vec![
            ActorRoutingPackageInput {
                package: first.clone(),
                actors: vec![actor(b'd', b'e', b"12")],
            },
            ActorRoutingPackageInput {
                package: second.clone(),
                actors: vec![actor(b'f', b'0', b"3")],
            },
        ],
    );

    let projection = project(input);
    let expected = expected_projection(vec![
        entry(&deployment, &first, b'd', b'e', b'1'),
        entry(&deployment, &first, b'd', b'e', b'2'),
        entry(&deployment, &second, b'f', b'0', b'3'),
    ]);
    assert_eq!(projection, expected);
    for method in &projection.methods {
        assert_eq!(method.actor.service_id, deployment.service_id);
        assert_eq!(method.deployment, deployment);
    }
}

#[test]
fn producer_keeps_cross_package_identical_triples_distinct() {
    let deployment = deployment(b'a');
    let first = package(b'b');
    let second = package(b'c');
    let input = input(
        deployment.clone(),
        vec![
            ActorRoutingPackageInput {
                package: first.clone(),
                actors: vec![actor(b'd', b'e', b"1")],
            },
            ActorRoutingPackageInput {
                package: second.clone(),
                actors: vec![actor(b'd', b'e', b"1")],
            },
        ],
    );

    let projection = project(input);
    let expected = expected_projection(vec![
        entry(&deployment, &first, b'd', b'e', b'1'),
        entry(&deployment, &second, b'd', b'e', b'1'),
    ]);
    assert_eq!(projection, expected);
}

#[test]
fn producer_is_input_order_independent() {
    let deployment = deployment(b'a');
    let first = package(b'b');
    let second = package(b'c');
    let input = input(
        deployment.clone(),
        vec![
            ActorRoutingPackageInput {
                package: second.clone(),
                actors: vec![actor(b'f', b'0', b"3")],
            },
            ActorRoutingPackageInput {
                package: first.clone(),
                actors: vec![actor(b'd', b'e', b"21")],
            },
        ],
    );

    let projection = project(input);
    let expected = expected_projection(vec![
        entry(&deployment, &first, b'd', b'e', b'1'),
        entry(&deployment, &first, b'd', b'e', b'2'),
        entry(&deployment, &second, b'f', b'0', b'3'),
    ]);
    assert_eq!(projection, expected);
}

#[test]
fn producer_accepts_empty_assembly() {
    let projection = project(input(deployment(b'a'), Vec::new()));
    assert!(projection.methods.is_empty());
}

#[test]
fn producer_rejects_unsupported_input_schema_version() {
    let mut input = input(deployment(b'a'), Vec::new());
    input.schema_version = "skiff-actor-routing-producer-input-v0".to_string();
    let error = project_err(input);
    assert!(matches!(
        error,
        ActorRoutingProjectionError::ProducerUnsupportedSchemaVersion(_)
    ));
}

#[test]
fn producer_rejects_actor_without_methods() {
    let input = input(
        deployment(b'a'),
        vec![ActorRoutingPackageInput {
            package: package(b'b'),
            actors: vec![actor(b'c', b'd', &[])],
        }],
    );
    assert_eq!(
        project_err(input),
        ActorRoutingProjectionError::ProducerActorWithoutMethods
    );
}

#[test]
fn producer_rejects_duplicate_actor_method() {
    let input = input(
        deployment(b'a'),
        vec![ActorRoutingPackageInput {
            package: package(b'b'),
            actors: vec![actor(b'c', b'd', b"11")],
        }],
    );
    assert_eq!(
        project_err(input),
        ActorRoutingProjectionError::ProducerDuplicateActorMethod
    );
}

#[test]
fn producer_rejects_duplicate_actor_in_package() {
    let input = input(
        deployment(b'a'),
        vec![ActorRoutingPackageInput {
            package: package(b'b'),
            actors: vec![actor(b'c', b'd', b"1"), actor(b'c', b'd', b"2")],
        }],
    );
    assert_eq!(
        project_err(input),
        ActorRoutingProjectionError::ProducerDuplicateActor
    );
}

#[test]
fn producer_rejects_duplicate_entries_for_identical_package_ref() {
    let input = input(
        deployment(b'a'),
        vec![
            ActorRoutingPackageInput {
                package: package(b'b'),
                actors: vec![actor(b'c', b'd', b"1")],
            },
            ActorRoutingPackageInput {
                package: package(b'b'),
                actors: vec![actor(b'c', b'd', b"1")],
            },
        ],
    );
    assert_eq!(
        project_err(input),
        ActorRoutingProjectionError::DuplicateMethod
    );
}

#[test]
fn producer_rejects_invalid_framed_identity() {
    let mut input = input(
        deployment(b'a'),
        vec![ActorRoutingPackageInput {
            package: package(b'b'),
            actors: vec![actor(b'c', b'd', b"1")],
        }],
    );
    input.packages[0].actors[0].actor_abi_identity =
        ActorAbiIdentity::new(format!("{ACTOR_ABI_IDENTITY_PREFIX}:{}", "z".repeat(64)));
    let error = project_err(input);
    assert!(matches!(
        error,
        ActorRoutingProjectionError::InvalidIdentity { field, .. } if field == "actor.actorAbiIdentity"
    ));
}

#[test]
fn producer_input_serde_is_camel_case_and_rejects_unknown_fields() {
    let deployment = deployment(b'a');
    let input = input(
        deployment,
        vec![ActorRoutingPackageInput {
            package: package(b'b'),
            actors: vec![actor(b'c', b'd', b"1")],
        }],
    );
    let value = serde_json::to_value(&input).expect("producer input serializes");
    let actor_value = &value["packages"][0]["actors"][0];
    assert_eq!(
        object_keys(actor_value),
        ["actorAbiIdentity", "actorImplementationIdentity", "methods"].as_slice()
    );
    let decoded: ActorRoutingProducerInput =
        serde_json::from_value(value).expect("producer input roundtrips");
    assert_eq!(decoded, input);

    let mut polluted = serde_json::to_value(&input).expect("producer input serializes");
    polluted["packages"][0]["actors"][0]["unexpectedField"] =
        serde_json::json!("unexpected producer input field");
    let error = serde_json::from_value::<ActorRoutingProducerInput>(polluted)
        .expect_err("unknown producer input field must be rejected");
    assert!(error.to_string().contains("unknown field"));
}

fn project(input: ActorRoutingProducerInput) -> ActorRoutingProjection {
    super::super::project_actor_routing(input).expect("producer input is valid")
}

fn project_err(input: ActorRoutingProducerInput) -> ActorRoutingProjectionError {
    super::super::project_actor_routing(input).expect_err("producer input must fail")
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
