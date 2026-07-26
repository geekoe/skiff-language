mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;
use skiff_artifact_identity::validate_service_contract_identities;
use skiff_artifact_model::{
    file_ir_service_call_sites, validate_file_ir_service_calls, BoundaryCallableProjection,
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, ContractRequirement, ContractTypeRef, PackageLocalAbiSymbol,
    PackageSchemaTypeId, PackageTypeRef, PackageTypeRequirement, ServiceCallRef,
    ServiceRequirement, ValueEscapeLane,
};
use skiff_compiler::{
    definition_contract_operation_id, ContractDefinitionError, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

use common::{
    artifacts::module_artifact,
    contracts::{compile_service_contract, package_contract_dependency},
    package_project::{
        compile_package_project, compile_package_project_with_contract_dependencies,
        compile_package_project_with_contract_dependencies_and_schemas,
        compile_service_package_project,
    },
    package_schemas::{public_contract_type, resolved_package_schema},
    TestDir,
};

const SERVICE_ID: &str = "example.echo";
const CONTRACT_VERSION: &str = "1.0.0";
const SCHEMA_PACKAGE_ID: &str = "example.com/echo-schema";

#[test]
fn package_stream_call_keeps_exact_item_type_through_binding_and_iteration() {
    let consumer = TestDir::new("skiff-compiler", "package-stream-expression-consumer");
    consumer.write(
        "package.yml",
        r#"id: example.com/package-stream-expression-consumer
version: 1.0.0
packages:
  - id: example.com/package-stream-expression-provider
    version: 1.0.0
    alias: feed
"#,
    );
    consumer.write("api.yml", "run: main.run\n");
    consumer.write(
        "main.skiff",
        r#"import feed

function run() -> Stream<string> {
  for event in feed/events() {
    emit(event)
  }
  const inferred = feed/events()
  for event in inferred {
    emit(event)
  }
  const events: Stream<string> = feed/events()
  for event in events {
    emit(event)
  }
  return null
}
"#,
    );
    write_stream_dependency(&consumer);

    compile_package_project(consumer.path()).expect(
        "package stream result must retain exact Event identity through expression, binding and iteration",
    );
}

#[test]
fn package_non_stream_call_remains_non_iterable() {
    let consumer = TestDir::new("skiff-compiler", "package-non-stream-iteration-consumer");
    consumer.write(
        "package.yml",
        r#"id: example.com/package-non-stream-iteration-consumer
version: 1.0.0
packages:
  - id: example.com/package-stream-expression-provider
    version: 1.0.0
    alias: feed
"#,
    );
    consumer.write("api.yml", "run: main.run\n");
    consumer.write(
        "main.skiff",
        r#"import feed

function run() -> null {
  for event in feed/one() {
    return null
  }
  return null
}
"#,
    );
    write_stream_dependency(&consumer);

    let error = compile_package_project(consumer.path())
        .expect_err("a scalar package call must not become iterable")
        .to_string();
    assert!(
        error.contains("for iterable must be Array, Stream, or Map"),
        "unexpected scalar iteration error: {error}"
    );
}

#[test]
fn stream_producer_rejects_a_non_null_completion_value() {
    let package = TestDir::new("skiff-compiler", "stream-producer-completion-value");
    write_package(
        &package,
        "example.com/stream-producer-completion-value",
        "run: main.run\n",
        r#"function run() -> Stream<string> {
  emit("event")
  return "not-a-completion"
}
"#,
    );

    let error = compile_package_project(package.path())
        .expect_err("stream producer completion must remain distinct from its Stream result")
        .to_string();
    assert!(
        error.contains("stream producer completion type mismatch")
            && error.contains("expected null, found"),
        "unexpected stream completion error: {error}"
    );
}

#[test]
fn generated_service_stream_contract_compiles_through_consumer_file_ir() {
    let (contract, schema) = compile_generated_stream_contract(
        "generated-service-stream-provider",
        "example.com/generated-service-stream-provider",
        "example.com/generated-stream",
    );
    let operation_id =
        definition_contract_operation_id("example.com/generated-stream", "1.0.0", "events")
            .unwrap();
    let operation = &contract.operations[&operation_id];
    let request_type = operation.contract.parameters[0].ty.clone();
    let BoundaryStreamContract::ServerStream {
        item_type: event_type,
        ..
    } = &operation.contract.stream
    else {
        panic!("generated operation must stream")
    };

    assert_eq!(operation.contract.parameters[0].ty, request_type);
    assert_eq!(
        operation.contract.return_value.ty,
        ContractTypeRef::builtin("void")
    );
    assert!(matches!(
        &operation.contract.stream,
        BoundaryStreamContract::ServerStream {
            item_type,
            item_value_plan: BoundaryValuePlan::Linkable {
                owner: BoundaryValueOwner::Provider,
                ..
            },
        } if item_type == event_type
    ));
    assert_eq!(contract.package_type_requirements.len(), 1);

    let consumer = TestDir::new("skiff-compiler", "generated-service-stream-consumer");
    write_package(
        &consumer,
        "example.com/generated-service-stream-consumer",
        "run: main.run\n",
        r#"function run(input: feed.Request) -> string {
  for event in feed/events(input) {
    return event.message
  }
  return ""
}
"#,
    );
    let dependencies = BTreeMap::from([(
        (
            "example.com/generated-service-stream-consumer".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("feed", contract.clone())],
    )]);
    let schemas = schemas_for("example.com/generated-service-stream-consumer", schema);
    let consumer_project = compile_package_project_with_contract_dependencies_and_schemas(
        consumer.path(),
        &dependencies,
        &schemas,
    )
    .expect("consumer should type and lower a generated service stream contract call");
    let expected_call_ref = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: operation_id.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    assert_eq!(
        consumer_project.package.artifact.service_call_refs,
        vec![expected_call_ref.clone()]
    );
    assert_eq!(
        consumer_project.package.artifact.service_requirements,
        vec![ServiceRequirement {
            contract_requirement: ContractRequirement {
                alias: "feed".to_string(),
                service_id: contract.service_id.clone(),
                contract_version: contract.contract_version.clone(),
                expected_protocol_identity: contract.service_protocol_identity.clone(),
            },
            service_binding_slot: 0,
            used_operations: BTreeSet::from([operation_id]),
        }]
    );

    let main = module_artifact(&consumer_project.package, "main");
    validate_file_ir_service_calls(&main.unit)
        .expect("stream consumer File IR service-call refs must be internally valid");
    assert_eq!(
        main.unit.external_refs.service_call_refs,
        vec![expected_call_ref.clone()]
    );
    let call_sites = file_ir_service_call_sites(&main.unit).collect::<Vec<_>>();
    assert_eq!(call_sites.len(), 1);
    assert_eq!(
        main.unit
            .external_refs
            .service_call_ref(call_sites[0].service_call_ref_index),
        Some(&expected_call_ref)
    );
    assert_no_provider_binding_wire(&consumer_project.package.artifact);
}

#[test]
fn generated_service_stream_consumer_rejects_an_undeclared_alias() {
    let (contract, schema) = compile_generated_stream_contract(
        "generated-service-stream-wrong-alias-provider",
        "example.com/generated-service-stream-wrong-alias-provider",
        "example.com/generated-stream-wrong-alias",
    );

    let consumer = TestDir::new(
        "skiff-compiler",
        "generated-service-stream-wrong-alias-consumer",
    );
    write_package(
        &consumer,
        "example.com/generated-service-stream-wrong-alias-consumer",
        "run: main.run\n",
        r#"function run(input: feed.Request) -> void {
  for event in wrong/events(input) {
    return
  }
}
"#,
    );
    let dependencies = BTreeMap::from([(
        (
            "example.com/generated-service-stream-wrong-alias-consumer".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("feed", contract)],
    )]);
    let schemas = schemas_for(
        "example.com/generated-service-stream-wrong-alias-consumer",
        schema,
    );
    let error = compile_package_project_with_contract_dependencies_and_schemas(
        consumer.path(),
        &dependencies,
        &schemas,
    )
    .expect_err("an undeclared service alias must fail the package compile trust boundary")
    .to_string();
    assert!(
        error.contains("for iterable must be Array, Stream, or Map"),
        "wrong alias must not produce a typed stream iterable: {error}"
    );
}

#[test]
fn explicit_definition_compiles_to_a_code_free_service_contract() {
    let definition = contract_definition();
    let expected_operation = definition.operations["echo"].clone();
    let (_, type_id) = request_type();

    let contract = compile_service_contract(definition).expect("explicit contract should compile");
    validate_service_contract_identities(&contract).expect("contract identities should be valid");

    let operation_id =
        definition_contract_operation_id(SERVICE_ID, CONTRACT_VERSION, "echo").unwrap();
    let operation = contract
        .operations
        .get(&operation_id)
        .expect("stable operation key should derive a contract-owned identity");
    assert_eq!(contract.service_id, SERVICE_ID);
    assert_eq!(contract.contract_version, CONTRACT_VERSION);
    assert_eq!(operation.operation_id, operation_id);
    assert_eq!(operation.stable_key, "echo");
    assert_eq!(operation.contract, expected_operation);
    assert_eq!(
        contract.package_type_requirements[0].required_type_ids,
        vec![type_id.clone()]
    );
    assert_eq!(contract.diagnostic_text.operations[&operation_id], "Echo");
    assert_eq!(contract.diagnostic_text.types[&type_id], "Echo request");
}

#[test]
fn definition_wire_rejects_provider_and_deployment_state() {
    let value = serde_json::to_value(contract_definition()).unwrap();
    let definition: ServiceContractDefinition = serde_json::from_value(value.clone()).unwrap();
    compile_service_contract(definition).expect("strict code-free definition should compile");

    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "deploymentRevision",
        "operationBindings",
        "ingress",
        "configBindings",
        "runtimeReplica",
    ] {
        let mut invalid = value.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!("forbidden"));
        assert!(
            serde_json::from_value::<ServiceContractDefinition>(invalid).is_err(),
            "{forbidden} must not enter the contract definition"
        );
    }
}

#[test]
fn missing_contract_schema_reference_fails_closed() {
    let mut definition = contract_definition();
    definition.operations.get_mut("echo").unwrap().parameters[0].ty =
        ContractTypeRef::package_schema(
            SCHEMA_PACKAGE_ID,
            "Missing",
            PackageSchemaTypeId::new("missing"),
        );

    assert!(matches!(
        compile_service_contract(definition),
        Err(ContractDefinitionError::Identity(_))
    ));
}

#[test]
fn protocol_identity_tracks_semantics_but_not_diagnostic_text() {
    let baseline = compile_service_contract(contract_definition()).unwrap();

    let mut renamed = contract_definition();
    renamed.diagnostic_text.service = "Renamed service".to_string();
    renamed
        .diagnostic_text
        .operations
        .insert("echo".to_string(), "Renamed operation".to_string());
    renamed
        .diagnostic_text
        .types
        .insert(request_type().1, "Renamed type".to_string());
    let renamed = compile_service_contract(renamed).unwrap();
    assert_eq!(
        baseline.service_protocol_identity,
        renamed.service_protocol_identity
    );

    let mut changed = contract_definition();
    changed.operations.get_mut("echo").unwrap().may_suspend = true;
    let changed = compile_service_contract(changed).unwrap();
    assert_ne!(
        baseline.service_protocol_identity,
        changed.service_protocol_identity
    );
}

#[test]
fn provider_and_consumer_compile_against_the_same_contract_without_provider_binding() {
    let contract = compile_service_contract(contract_definition()).unwrap();
    let schema = echo_schema();
    let operation_id =
        definition_contract_operation_id(SERVICE_ID, CONTRACT_VERSION, "echo").unwrap();
    let request_type_id = request_type().1;
    let operation_body = contract.operations[&operation_id].contract.clone();
    let expected_requirement = ContractRequirement {
        alias: "payments".to_string(),
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };

    let provider = TestDir::new("skiff-compiler", "service-contract-provider-wrapper");
    write_package(
        &provider,
        "example.com/payments-provider",
        "handle: main.handle\n",
        r#"function handle(request: payments.Request) -> string {
  return "accepted"
}
"#,
    );
    let provider_dependencies = BTreeMap::from([(
        (
            "example.com/payments-provider".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("payments", contract.clone())],
    )]);
    let provider_schemas = schemas_for("example.com/payments-provider", schema.clone());
    let provider_project = compile_package_project_with_contract_dependencies_and_schemas(
        provider.path(),
        &provider_dependencies,
        &provider_schemas,
    )
    .expect("provider wrapper should compile from package source and contract only");
    assert_eq!(
        provider_project.package.artifact.contract_requirements,
        vec![expected_requirement.clone()]
    );
    assert!(provider_project
        .package
        .artifact
        .package_requirements
        .is_empty());
    assert!(provider_project
        .package
        .artifact
        .service_requirements
        .is_empty());
    assert!(provider_project
        .package
        .artifact
        .service_call_refs
        .is_empty());

    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = &provider_project
        .package
        .artifact
        .package_local_abi
        .public_symbols["handle"]
    else {
        panic!("provider wrapper must be a Local ABI callable");
    };
    assert_eq!(signature.parameters.len(), 1);
    assert_eq!(signature.parameters[0].name, "request");
    assert_eq!(
        signature.parameters[0].ty,
        PackageTypeRef::PackageSchema {
            package_id: SCHEMA_PACKAGE_ID.to_string(),
            stable_schema_key: "Request".to_string(),
            package_schema_type_id: request_type_id.clone(),
        }
    );
    assert_eq!(
        signature.return_type,
        PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }
    );
    let BoundaryCallableProjection::Available {
        operation_contract,
        implementation_requirements: _,
    } = &provider_project.package.artifact.boundary_projections[callable_id]
    else {
        panic!("provider wrapper must have an Available boundary projection");
    };
    assert_eq!(operation_contract, &operation_body);
    assert_no_provider_binding_wire(&provider_project.package.artifact);

    let consumer = TestDir::new("skiff-compiler", "service-contract-consumer");
    write_package(
        &consumer,
        "example.com/payments-consumer",
        "run: main.run\n",
        r#"function run(input: payments.Request) -> string {
  return payments/echo(input)
}
"#,
    );
    let consumer_dependencies = BTreeMap::from([(
        (
            "example.com/payments-consumer".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("payments", contract.clone())],
    )]);
    let consumer_schemas = schemas_for("example.com/payments-consumer", schema);
    let consumer_project = compile_package_project_with_contract_dependencies_and_schemas(
        consumer.path(),
        &consumer_dependencies,
        &consumer_schemas,
    )
    .expect("consumer should compile from package source and ServiceContract only");
    let expected_call_ref = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: operation_id.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    assert!(consumer_project
        .package
        .artifact
        .package_requirements
        .is_empty());
    assert_eq!(
        consumer_project.package.artifact.contract_requirements,
        vec![expected_requirement.clone()]
    );
    assert_eq!(
        consumer_project.package.artifact.service_requirements,
        vec![ServiceRequirement {
            contract_requirement: expected_requirement,
            service_binding_slot: 0,
            used_operations: BTreeSet::from([operation_id]),
        }]
    );
    assert_eq!(
        consumer_project.package.artifact.service_call_refs,
        vec![expected_call_ref.clone()]
    );

    let PackageLocalAbiSymbol::Callable { signature, .. } = &consumer_project
        .package
        .artifact
        .package_local_abi
        .public_symbols["run"]
    else {
        panic!("consumer entry must be a Local ABI callable");
    };
    assert_eq!(
        signature.parameters[0].ty,
        PackageTypeRef::PackageSchema {
            package_id: SCHEMA_PACKAGE_ID.to_string(),
            stable_schema_key: "Request".to_string(),
            package_schema_type_id: request_type_id,
        }
    );
    assert_eq!(
        signature.return_type,
        PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }
    );

    let main = module_artifact(&consumer_project.package, "main");
    validate_file_ir_service_calls(&main.unit)
        .expect("consumer File IR service-call refs must be internally valid");
    assert_eq!(
        main.unit.external_refs.service_call_refs,
        vec![expected_call_ref.clone()]
    );
    let call_sites = file_ir_service_call_sites(&main.unit).collect::<Vec<_>>();
    assert_eq!(call_sites.len(), 1);
    assert_eq!(
        main.unit
            .external_refs
            .service_call_ref(call_sites[0].service_call_ref_index),
        Some(&expected_call_ref)
    );
    assert_no_provider_binding_wire(&consumer_project.package.artifact);
}

#[test]
fn package_direct_mutation_then_detached_contract_projects_available() {
    let mut definition = contract_definition();
    definition.operations.get_mut("echo").unwrap().parameters[0].ty =
        ContractTypeRef::builtin("string");
    definition.package_type_requirements.clear();
    definition.diagnostic_text.types.clear();
    let contract = compile_service_contract(definition).unwrap();

    let provider = TestDir::new("skiff-compiler", "callable-effects-provider");
    write_package(
        &provider,
        "example.com/callable-effects-provider",
        "handle: main.handle\n",
        r#"function handle(input: string) -> string {
  if input == "helper-mutated" { return "accepted" }
  return "rejected"
}
"#,
    );
    let provider_project =
        compile_package_project_with_contract_dependencies(provider.path(), &BTreeMap::new())
            .expect("branching provider should compile");
    assert!(matches!(
        public_callable_projection(&provider_project.package.artifact, "handle"),
        BoundaryCallableProjection::Available { .. }
    ));

    let consumer = TestDir::new("skiff-compiler", "callable-effects-consumer");
    consumer.write(
        "package.yml",
        r#"id: example.com/callable-effects-consumer
version: 1.0.0
packages:
  - id: example.com/callable-effects-helper
    version: 1.0.0
    alias: helper
"#,
    );
    consumer.write("api.yml", "run: main.run\n");
    consumer.write(
        "main.skiff",
        r#"import helper

type Box { value: string }

function run() -> string {
  const box = Box { value: "consumer" }
  helper/tools.mutate(box)
  return payments/echo(box.value)
}
"#,
    );
    consumer.write(
        ".skiff-packages/example~com~~callable-effects-helper/1.0.0/package.yml",
        "id: example.com/callable-effects-helper\nversion: 1.0.0\n",
    );
    consumer.write(
        ".skiff-packages/example~com~~callable-effects-helper/1.0.0/api.yml",
        "Box: helper.Box\ntools:\n  mutate: helper.mutate\n",
    );
    consumer.write(
        ".skiff-packages/example~com~~callable-effects-helper/1.0.0/helper.skiff",
        r#"type Box { value: string }

function mutate(input: Box) -> void {
  input.value = "helper-mutated"
}
"#,
    );
    let dependencies = BTreeMap::from([(
        (
            "example.com/callable-effects-consumer".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("payments", contract)],
    )]);
    let schemas = schemas_for("example.com/callable-effects-consumer", echo_schema());
    let project = compile_package_project_with_contract_dependencies_and_schemas(
        consumer.path(),
        &dependencies,
        &schemas,
    )
    .expect("fresh helper value followed by detached contract call should compile");
    let helper = project
        .dependency("example.com/callable-effects-helper", "1.0.0")
        .expect("helper artifact must be in the canonical dependency closure");
    let PackageLocalAbiSymbol::Callable {
        callable_id: mutate_id,
        ..
    } = &helper.artifact.package_local_abi.public_symbols["tools.mutate"]
    else {
        panic!("mutate must resolve to a public callable");
    };
    let mutate_facts = &helper.artifact.callable_semantic_facts[mutate_id];
    assert_eq!(
        mutate_facts.effects,
        CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                writes_caller_reachable: true,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            }
        }
    );
    assert!(matches!(
        mutate_facts.provenance,
        CallableProvenanceSummary::Analyzed { .. }
    ));
    let BoundaryCallableProjection::Unavailable { reasons } =
        &helper.artifact.boundary_projections[mutate_id]
    else {
        panic!("mutating helper must remain boundary unavailable");
    };
    assert!(reasons.contains(&BoundaryUnavailableReason::WritesCallerReachable));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownEffect));
    assert!(!reasons.contains(&BoundaryUnavailableReason::UnknownCallTarget));
    assert!(matches!(
        public_callable_projection(&project.package.artifact, "run"),
        BoundaryCallableProjection::Available { .. }
    ));
}

#[test]
fn database_reads_and_detached_writes_project_available_but_owned_writes_do_not() {
    let package = TestDir::new("skiff-compiler", "database-boundary-provenance");
    write_package(
        &package,
        "example.com/database-boundary-provenance",
        "read: main.read\nput: main.put\nputOwned: main.putOwned\n",
        r#"type Stored { id: string, value: string, tags: Array<string> }

db object Stored {
  primary key(id)
}

function read(id: string) -> string {
  const stored = db require Stored(id)
  return stored.value
}

function put(id: string, value: string) -> void {
  db insert Stored {
    id = id
    value = value
    tags = Array.empty<string>()
  }
}

function putOwned(id: string, tags: Array<string>) -> void {
  db insert Stored { id = id value = "owned" tags = tags }
}
"#,
    );
    let project =
        compile_package_project_with_contract_dependencies(package.path(), &BTreeMap::new())
            .expect("database provenance fixture should compile");

    for callable in ["read", "put"] {
        let projection = public_callable_projection(&project.package.artifact, callable);
        assert!(
            matches!(projection, BoundaryCallableProjection::Available { .. }),
            "{callable} must remain boundary available: {projection:?}"
        );
    }
    let BoundaryCallableProjection::Unavailable { reasons } =
        public_callable_projection(&project.package.artifact, "putOwned")
    else {
        panic!("persisting a caller-owned payload must remain boundary unavailable");
    };
    assert_eq!(
        reasons,
        &vec![BoundaryUnavailableReason::EscapesCallerValue {
            lane: ValueEscapeLane::Database,
        }]
    );
}

#[test]
fn invalid_contract_type_operation_and_call_uses_fail_package_compilation() {
    let contract = compile_service_contract(contract_definition()).unwrap();
    for (name, source, expected) in [
        (
            "unknown-type",
            "function run(input: payments.Missing) -> string { return \"no\" }",
            "no contract type stable key `Missing`",
        ),
        (
            "unknown-operation",
            r#"function run(input: payments.Request) -> string {
  return payments/missing(input)
}"#,
            "no operation stable key `missing`",
        ),
        (
            "wrong-argument",
            r#"function run() -> string {
  return payments/echo("not a request")
}"#,
            "argument 1 type mismatch",
        ),
        (
            "wrong-return-use",
            r#"function run(input: payments.Request) -> bool {
  return payments/echo(input)
}"#,
            "return type mismatch",
        ),
    ] {
        let temp = TestDir::new("skiff-compiler", &format!("service-contract-{name}"));
        let package_id = format!("example.com/service-contract-{name}");
        write_package(&temp, &package_id, "run: main.run\n", source);
        let dependencies = BTreeMap::from([(
            (package_id.clone(), "1.0.0".to_string()),
            vec![package_contract_dependency("payments", contract.clone())],
        )]);
        let schemas = schemas_for(&package_id, echo_schema());
        let error = compile_package_project_with_contract_dependencies_and_schemas(
            temp.path(),
            &dependencies,
            &schemas,
        )
        .expect_err("invalid contract source must fail the package compile trust boundary")
        .to_string();
        assert!(error.contains(expected), "unexpected {name} error: {error}");
    }
}

#[test]
fn package_and_contract_alias_conflict_fails_the_package_compile_trust_boundary() {
    let temp = TestDir::new("skiff-compiler", "service-contract-alias-conflict");
    write_package(
        &temp,
        "example.com/service-contract-alias-conflict",
        "run: main.run\n",
        "function run(input: payments.Request) -> string { return \"no\" }",
    );
    temp.write(
        "package.yml",
        r#"id: example.com/service-contract-alias-conflict
version: 1.0.0
packages:
  - id: example.com/package-payments
    version: 1.0.0
    alias: payments
"#,
    );
    temp.write(
        ".skiff-packages/example~com~~package-payments/1.0.0/package.yml",
        "id: example.com/package-payments\nversion: 1.0.0\n",
    );
    temp.write(
        ".skiff-packages/example~com~~package-payments/1.0.0/api.yml",
        "Request: dependency.Request\n",
    );
    temp.write(
        ".skiff-packages/example~com~~package-payments/1.0.0/dependency.skiff",
        "type Request { message: string }\n",
    );

    let contract = compile_service_contract(contract_definition()).unwrap();
    let dependencies = BTreeMap::from([(
        (
            "example.com/service-contract-alias-conflict".to_string(),
            "1.0.0".to_string(),
        ),
        vec![package_contract_dependency("payments", contract)],
    )]);
    let schemas = schemas_for("example.com/service-contract-alias-conflict", echo_schema());
    let error = compile_package_project_with_contract_dependencies_and_schemas(
        temp.path(),
        &dependencies,
        &schemas,
    )
    .expect_err("package/contract alias conflict must fail before source resolution")
    .to_string();
    assert!(
        error.contains("dependency alias `payments` is declared by both a package and a contract"),
        "unexpected alias conflict error: {error}"
    );
}

fn contract_definition() -> ServiceContractDefinition {
    let (request, request_id) = request_type();
    ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "request".to_string(),
                    ty: request,
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                stream: BoundaryStreamContract::Unary,
                cancellation: BoundaryCancellationContract::NotCancellable,
                callbacks: BoundaryCallbackContract::None,
                may_suspend: false,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        )]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: SCHEMA_PACKAGE_ID.to_string(),
            required_type_ids: vec![request_id.clone()],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Echo service".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::from([(request_id, "Echo request".to_string())]),
        },
    }
}

fn request_type() -> (ContractTypeRef, PackageSchemaTypeId) {
    let seed = echo_schema_seed();
    public_contract_type(&seed.package, "Request")
}

fn echo_schema_seed() -> common::package_project::PublishedPackageProject {
    let seed = TestDir::new("skiff-compiler", "echo-contract-schema-seed");
    write_package(
        &seed,
        SCHEMA_PACKAGE_ID,
        "Request: main.Request\n",
        "type Request { message: string }\n",
    );
    compile_package_project(seed.path()).expect("echo schema seed should compile")
}

fn echo_schema() -> skiff_compiler::ResolvedPackageSchema {
    resolved_package_schema("contract-schema", &echo_schema_seed().package)
        .expect("echo schema seed should resolve")
}

fn schemas_for(
    package_id: &str,
    schema: skiff_compiler::ResolvedPackageSchema,
) -> BTreeMap<
    skiff_compiler_input::package_config::PackageManifestKey,
    Vec<skiff_compiler::ResolvedPackageSchema>,
> {
    BTreeMap::from([((package_id.to_string(), "1.0.0".to_string()), vec![schema])])
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn write_package(temp: &TestDir, package_id: &str, api: &str, source: &str) {
    temp.write(
        "package.yml",
        &format!("id: {package_id}\nversion: 1.0.0\n"),
    );
    temp.write("api.yml", api);
    temp.write("main.skiff", source);
}

fn write_stream_dependency(consumer: &TestDir) {
    let root = ".skiff-packages/example~com~~package-stream-expression-provider/1.0.0";
    consumer.write(
        &format!("{root}/package.yml"),
        "id: example.com/package-stream-expression-provider\nversion: 1.0.0\n",
    );
    consumer.write(
        &format!("{root}/api.yml"),
        "events: feed.events\none: feed.one\n",
    );
    consumer.write(
        &format!("{root}/feed.skiff"),
        r#"function events() -> Stream<string> {
  emit("event")
  return null
}

function one() -> string {
  return "one"
}
"#,
    );
}

fn compile_generated_stream_contract(
    fixture_name: &str,
    package_id: &str,
    service_id: &str,
) -> (
    skiff_artifact_model::ServiceContract,
    skiff_compiler::ResolvedPackageSchema,
) {
    let provider = TestDir::new("skiff-compiler", fixture_name);
    write_package(
        &provider,
        package_id,
        "Event: model.Event\nRequest: model.Request\nevents:\n  source: main.events\n  serviceCall: true\n",
        r#"function events(input: root.model.Request) -> Stream<root.model.Event> {
  emit(root.model.Event { message: "event" })
  return
}
"#,
    );
    provider.write("service.yml", format!("id: {service_id}\n"));
    provider.write(
        "model.skiff",
        "type Event { message: string }\ntype Request { topic: string }\n",
    );
    let (provider_project, projected_service_api) =
        compile_service_package_project(provider.path())
            .expect("stream provider package should compile through the real service pipeline");
    assert!(
        matches!(
            public_callable_projection(&provider_project.package.artifact, "events"),
            BoundaryCallableProjection::Available { .. }
        ),
        "stream projection: {:?}",
        public_callable_projection(&provider_project.package.artifact, "events")
    );
    assert_no_provider_binding_wire(&provider_project.package.artifact);
    let contract = projected_service_api.contract;
    let schema = resolved_package_schema("contract-schema", &provider_project.package)
        .expect("stream provider schema should resolve");
    (contract, schema)
}

fn public_callable_projection<'a>(
    artifact: &'a skiff_artifact_model::PackageArtifact,
    public_path: &str,
) -> &'a BoundaryCallableProjection {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &artifact.package_local_abi.public_symbols[public_path]
    else {
        panic!("{public_path} must resolve to a public callable");
    };
    &artifact.boundary_projections[callable_id]
}

fn assert_no_provider_binding_wire(artifact: &skiff_artifact_model::PackageArtifact) {
    let value = serde_json::to_value(artifact).unwrap();
    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "providerDeploymentId",
        "deploymentId",
        "deploymentRevision",
        "route",
        "executableTarget",
    ] {
        assert!(
            !contains_object_key(&value, forbidden),
            "PackageArtifact must not contain provider/deployment field `{forbidden}`"
        );
    }
}

fn contains_object_key(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Array(values) => {
            values.iter().any(|value| contains_object_key(value, key))
        }
        serde_json::Value::Object(object) => {
            object.contains_key(key) || object.values().any(|value| contains_object_key(value, key))
        }
        _ => false,
    }
}
