use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref};
use skiff_artifact_model::*;
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    compile_contract, CompilerPlatformSources, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ServiceContractPointer};
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_capability_context::RestrictedServiceDiagnostic;
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, HydratedPackageCode, PublicationResourceTable, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
    },
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InstantiatedTypeArgumentIdentity,
        LocalExecutionTypeIdentity, NominalTypeIdentity, OpaqueServiceError,
        PlatformBuiltinErrorIdentity, RequestException, ServiceErrorEnvelope,
    },
};
use skiff_test_runner::{
    canonical_fixture::discover_package_test_cases,
    canonical_package::compile_package_project,
    canonical_std_seed::seed_canonical_std,
    test_overlay::{compile_package_test_overlay, PublishedPackageTestOverlay},
};

use crate::{
    assembly_execution::service_error_channel::{
        start_restricted_service_diagnostic_probe_for_test,
        take_restricted_service_diagnostics_for_test,
    },
    error::{RuntimeError, UserException},
    test_effect_registry::{
        RegisteredTestEffect, RegisteredTestEffectFailure, RegisteredTestEffectOutcome,
        RegisteredTestEffectThrow, TestEffectTarget,
    },
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

use super::{activation_context, execution_context_with_trace, test_runtime, TestResolver};

const ERROR_PACKAGE_ID: &str = "example.com/typed-effect-errors";
const ERROR_PACKAGE_VERSION: &str = "1.0.0";
const ERROR_STABLE_SCHEMA_KEY: &str = "Failure";
const SERVICE_ID: &str = "example.com/typed-effect-payments";
const SERVICE_VERSION: &str = "1.0.0";
const LINKED_SERVICE_EFFECT_PACKAGE_ID: &str = "skiff.run/std";
const LINKED_ERROR_DEPENDENCY_ALIAS: &str = "errors";
const LINKED_INTERNAL_ERROR_KEY: &str = "std.service.InternalError";
const LINKED_SERVICE_EFFECT_TRACE_ID: &str = "trace:linked-service-effect";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct LinkedEffectExecution {
    interpreter: Interpreter,
    result: crate::error::Result<RuntimeValue>,
    heap: RequestHeap,
    diagnostics: Vec<RestrictedServiceDiagnostic>,
}

#[tokio::test]
async fn service_error_channel_contract_operation_restricted_service_diagnostic_effect_throw() {
    let run = execute_linked_effect(0, None).await;
    let result = run.result.as_ref().expect("public throw must be caught");
    let exception = caught_exception(result, &run.heap);
    let expected_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: skiff_runtime_linked_program::TypeAddr {
                unit: UnitAddr::Package(1),
                file: FileAddr::LoadedFileIndex(0),
                type_index: 0,
            },
            type_arguments: Vec::new(),
        },
    ));
    assert_eq!(exception.local_catch_identity(), Some(&expected_identity));
    assert!(matches!(
        exception.fixed_service_error().unwrap().envelope(),
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            ..
        } if package_id == ERROR_PACKAGE_ID
            && stable_schema_key == ERROR_STABLE_SCHEMA_KEY
    ));
    assert_boundary_stack(exception);
    let RuntimeValue::Heap(payload) = exception.local_value().unwrap().value() else {
        panic!("public payload must be caller-local");
    };
    let HeapNode::Object(payload) = run.heap.get(*payload).unwrap() else {
        panic!("public payload must be a caller-local object");
    };
    assert_eq!(
        payload.fields().get("message"),
        Some(&RuntimeValue::String("denied".to_string()))
    );
    let RuntimeValue::Heap(result) = result else {
        panic!("linked result must be an object");
    };
    let HeapNode::Object(result) = run.heap.get(*result).unwrap() else {
        panic!("linked result must remain an object");
    };
    assert_eq!(
        result.fields().get("response"),
        Some(&RuntimeValue::String("accepted".to_string()))
    );
    assert_service_diagnostic(&run, exception);
    run.interpreter.finalize_test_case().unwrap();
}

#[tokio::test]
async fn linked_service_effect_internalization_matrix_is_fixed_once() {
    let private = execute_linked_effect(1, None).await;
    assert_internal(
        caught_exception(private.result.as_ref().unwrap(), &private.heap),
        "private-detail",
    );
    assert_service_diagnostic(
        &private,
        caught_exception(private.result.as_ref().unwrap(), &private.heap),
    );
    private.interpreter.finalize_test_case().unwrap();

    for (case, unit_index, type_index, args, malformed) in [
        ("encode", 1, 0, Vec::new(), true),
        (
            "nonclosed",
            0,
            3,
            vec![InstantiatedTypeArgumentIdentity::new("builtin:string").unwrap()],
            false,
        ),
    ] {
        let mut setup_heap = RequestHeap::default();
        let leaked = format!("{case}-private-detail");
        let value = if malformed {
            RuntimeValue::Heap(
                setup_heap
                    .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                        "leak".to_string(),
                        RuntimeValue::String(leaked.clone()),
                    )])))
                    .unwrap(),
            )
        } else {
            RuntimeValue::String(leaked.clone())
        };
        let correlation = ErrorCorrelation {
            trace_id: format!("trace:{case}"),
            error_id: format!("trace:{case}:error:1"),
        };
        let failure = local_provider_failure(
            RuntimeValueCarrier::identified(value, local_identity_at(unit_index, type_index, args)),
            correlation.clone(),
        );
        let run = execute_linked_effect(
            2,
            Some((
                RegisteredTestEffectFailure::ProviderFailure(failure),
                setup_heap,
            )),
        )
        .await;
        let exception = caught_exception(run.result.as_ref().unwrap(), &run.heap);
        assert_eq!(exception.correlation(), &correlation);
        assert_internal(exception, &leaked);
        assert_service_diagnostic(&run, exception);
        run.interpreter.finalize_test_case().unwrap();
    }
}

#[tokio::test]
async fn linked_service_effect_opaque_failure_forwards_exact_bytes_and_new_stack() {
    let envelope = ServiceErrorEnvelope::PublicTypedError {
        package_id: "unknown.example/errors".to_string(),
        stable_schema_key: "Opaque".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:opaque"),
        encoded_payload: vec![1],
        trace_id: "trace:opaque".to_string(),
        error_id: "trace:opaque:error:1".to_string(),
    };
    let encoded = skiff_canonical_json::canonical_json_bytes(&envelope).unwrap();
    let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
    let run = execute_linked_effect(
        2,
        Some((
            RegisteredTestEffectFailure::FixedService(opaque),
            RequestHeap::default(),
        )),
    )
    .await;
    let error = run.result.as_ref().expect_err("opaque catch must miss");
    let exception = crate::exceptions::user_exception_for_catch(error)
        .unwrap()
        .request();
    assert_eq!(
        exception.fixed_service_error().unwrap().encoded_bytes(),
        encoded
    );
    assert!(exception.local_value().is_none());
    assert_boundary_stack(exception);
    assert_service_diagnostic(&run, exception);
    run.interpreter.finalize_test_case().unwrap();
}

#[tokio::test]
async fn linked_service_effect_platform_failure_uses_the_same_r0_channel() {
    let run = execute_linked_effect(
        2,
        Some((
            RegisteredTestEffectFailure::ProviderFailure(RuntimeError::FileError {
                message: "denied".to_string(),
            }),
            RequestHeap::default(),
        )),
    )
    .await;
    let error = run.result.as_ref().expect_err("platform catch must miss");
    let exception = crate::exceptions::user_exception_for_catch(error)
        .unwrap()
        .request();
    assert!(matches!(
        exception.fixed_service_error().unwrap().envelope(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::File,
            ..
        }
    ));
    assert_eq!(
        exception.local_catch_identity(),
        Some(&PlatformBuiltinErrorIdentity::File.catch_identity())
    );
    assert_boundary_stack(exception);
    assert_service_diagnostic(&run, exception);
    run.interpreter.finalize_test_case().unwrap();
}

async fn execute_linked_effect(
    executable_index: usize,
    injected: Option<(RegisteredTestEffectFailure, RequestHeap)>,
) -> LinkedEffectExecution {
    let (target, addr) = linked_service_effect_fixture(executable_index);
    let generation = target.request_activation().generation();
    start_restricted_service_diagnostic_probe_for_test(generation);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    if let Some((failure, setup_heap)) = injected {
        interpreter.runtime_test_effects.register(
            service_effect_target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: RegisteredTestEffectOutcome::Throw(RegisteredTestEffectThrow {
                    failure,
                    setup_heap,
                    setup_package_build_id: PackageBuildId::new("build:linked-service-effect"),
                }),
            },
        );
    }
    let context =
        execution_context_with_trace(&interpreter, target, LINKED_SERVICE_EFFECT_TRACE_ID);
    let mut heap = RequestHeap::default();
    let result = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &addr, Vec::new())
        .await;
    let diagnostics = take_restricted_service_diagnostics_for_test(generation);
    LinkedEffectExecution {
        interpreter,
        result,
        heap,
        diagnostics,
    }
}

fn assert_service_diagnostic(run: &LinkedEffectExecution, exception: &RequestException) {
    let fixed = exception
        .fixed_service_error()
        .expect("service effect keeps fixed carrier");
    assert_eq!(
        run.diagnostics.len(),
        1,
        "ContractOperation effect submits exactly one restricted diagnostic"
    );
    assert_eq!(
        run.diagnostics[0].correlation.trace_id,
        fixed.envelope().trace_id()
    );
    assert_eq!(
        run.diagnostics[0].correlation.error_id,
        fixed.envelope().error_id()
    );
    assert!(
        !format!("{:?}", run.diagnostics[0]).contains("private-detail"),
        "diagnostic safe fields do not contain the provider payload"
    );
}

fn caught_exception<'a>(caught: &RuntimeValue, heap: &'a RequestHeap) -> &'a RequestException {
    let RuntimeValue::Heap(caught) = caught else {
        panic!("catch result must be an object");
    };
    let HeapNode::Object(caught) = heap.get(*caught).unwrap() else {
        panic!("catch result must remain an object");
    };
    let RuntimeValue::Heap(exception) = caught.fields().get("exception").unwrap() else {
        panic!("catch result must contain an exception");
    };
    let HeapNode::Exception(exception) = heap.get(*exception).unwrap() else {
        panic!("catch result must retain RequestException");
    };
    exception
}

fn assert_internal(exception: &RequestException, forbidden: &str) {
    assert_eq!(
        exception.local_catch_identity(),
        Some(&local_identity(2, Vec::new()))
    );
    let fixed = exception.fixed_service_error().unwrap();
    assert!(matches!(
        fixed.envelope(),
        ServiceErrorEnvelope::InternalError { .. }
    ));
    let bytes = String::from_utf8_lossy(fixed.encoded_bytes());
    assert_eq!(bytes.matches("Internal service error").count(), 1);
    assert!(!bytes.contains(forbidden));
    assert_boundary_stack(exception);
}

fn assert_boundary_stack(exception: &RequestException) {
    assert_eq!(exception.source(), &linked_service_effect_call_site());
    assert_eq!(exception.stack().len(), 2);
    assert!(matches!(
        exception.stack().last(),
        Some(ExceptionStackFrame::RemoteBoundary {
            service_id,
            operation_id,
            error_id,
        }) if service_id == "test-effect:protocol:linked-service-effect"
            && operation_id == "operation:echo"
            && error_id == &exception.correlation().error_id
    ));
    assert!(!exception.stack().iter().any(|frame| matches!(
        frame,
        ExceptionStackFrame::Local {
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        }
    )));
}

fn local_identity(
    type_index: usize,
    type_arguments: Vec<InstantiatedTypeArgumentIdentity>,
) -> CatchIdentity {
    local_identity_at(0, type_index, type_arguments)
}

fn local_identity_at(
    unit_index: usize,
    type_index: usize,
    type_arguments: Vec<InstantiatedTypeArgumentIdentity>,
) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: skiff_runtime_linked_program::TypeAddr {
                unit: UnitAddr::Package(unit_index),
                file: FileAddr::LoadedFileIndex(0),
                type_index,
            },
            type_arguments,
        },
    ))
}

fn local_provider_failure(
    payload: RuntimeValueCarrier,
    correlation: ErrorCorrelation,
) -> RuntimeError {
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    RuntimeError::UserException(UserException::new(
        RequestException::local(
            payload,
            site.clone(),
            vec![ExceptionStackFrame::Local { site }],
            correlation,
        )
        .unwrap(),
    ))
}

fn service_effect_target() -> TestEffectTarget {
    TestEffectTarget::contract_operation(
        ContractOperationId::new("operation:echo"),
        ServiceProtocolIdentity::new("protocol:linked-service-effect"),
    )
}

fn linked_service_effect_fixture(
    executable_index: usize,
) -> (RuntimeAssemblyEvalTarget, ExecutableAddr) {
    let error_dependency = linked_error_dependency();
    let operation_id = ContractOperationId::new("operation:echo");
    let protocol_identity = ServiceProtocolIdentity::new("protocol:linked-service-effect");
    let service_call = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: operation_id.clone(),
        expected_protocol_identity: protocol_identity.clone(),
    };
    let service_call_index = ServiceCallRefIndex::try_from(0_usize).expect("service call index");
    let public_error_symbol = PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: LINKED_ERROR_DEPENDENCY_ALIAS.to_string(),
        },
        symbol_path: ERROR_STABLE_SCHEMA_KEY.to_string(),
        abi_expectation: Some(
            error_dependency
                .artifact_ref
                .package_local_abi_identity
                .to_string(),
        ),
    };
    let public_error_type = TypeRefIr::PackageSymbol {
        symbol: public_error_symbol.clone(),
    };
    let private_error_type = TypeRefIr::PublicationType {
        module_path: "linked.effect".to_string(),
        type_index: 1,
    };
    let internal_error_type = TypeRefIr::PublicationType {
        module_path: "linked.effect".to_string(),
        type_index: 2,
    };
    let mut file = FileIrUnit::empty("linked.effect", "source:linked-service-effect");
    file.type_table.extend([
        TypeDeclIr {
            name: ERROR_STABLE_SCHEMA_KEY.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "PrivateFailure".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("secret".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: LINKED_INTERNAL_ERROR_KEY.to_string(),
            descriptor: internal_error_source_descriptor(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "GenericFailure".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        },
    ]);
    file.external_refs
        .service_call_refs
        .push(service_call.clone());
    file.external_refs.package_symbols.push(public_error_symbol);
    file.executables.extend([
        linked_service_effect_executable(
            service_call_index,
            public_error_type.clone(),
            public_error_type,
            "message",
            "denied",
        ),
        linked_service_effect_executable(
            service_call_index,
            private_error_type,
            internal_error_type.clone(),
            "secret",
            "private-detail",
        ),
        linked_service_effect_consumer_executable(service_call_index, internal_error_type),
    ]);
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("linked service-effect File IR identity");

    let internal_canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: internal_error_contract_descriptor(),
    };
    let internal_type_id = skiff_artifact_identity::package_schema_type_id(
        LINKED_SERVICE_EFFECT_PACKAGE_ID,
        LINKED_INTERNAL_ERROR_KEY,
        &internal_canonical_descriptor,
    )
    .expect("linked Internal error identity");
    let index_types = BTreeMap::from([(
        LINKED_INTERNAL_ERROR_KEY.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: internal_type_id.clone(),
            public_path: Some(LINKED_INTERNAL_ERROR_KEY.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let schema_index = PackageSchemaIndex {
        package_id: LINKED_SERVICE_EFFECT_PACKAGE_ID.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            LINKED_SERVICE_EFFECT_PACKAGE_ID,
            &index_types,
        )
        .expect("linked service-effect schema index identity"),
        types: index_types,
    };
    let internal_record = Arc::new(PackageSchemaTypeRecord {
        package_id: LINKED_SERVICE_EFFECT_PACKAGE_ID.to_string(),
        stable_schema_key: LINKED_INTERNAL_ERROR_KEY.to_string(),
        package_schema_type_id: internal_type_id.clone(),
        canonical_descriptor: internal_canonical_descriptor,
    });
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let package_build_id = PackageBuildId::new("build:linked-service-effect");
    let package_local_abi_identity = PackageLocalAbiIdentity::new("abi:linked-service-effect");
    let mut package = super::private_package(LINKED_SERVICE_EFFECT_PACKAGE_ID, &file);
    package.package_build_id = package_build_id.clone();
    package.package_local_abi.local_abi_identity = package_local_abi_identity.clone();
    package.package_schema_index.package_schema_index_identity =
        schema_index.package_schema_index_identity.clone();
    package.package_schema_type_records = BTreeMap::from([(
        internal_type_id.clone(),
        PackageSchemaTypeRecordRef {
            package_id: LINKED_SERVICE_EFFECT_PACKAGE_ID.to_string(),
            package_schema_type_id: internal_type_id.clone(),
        },
    )]);
    package.implementation_links.types = BTreeMap::from([(
        LINKED_INTERNAL_ERROR_KEY.to_string(),
        TypeExport {
            file: file_ref,
            type_index: 2,
            symbol: LINKED_INTERNAL_ERROR_KEY.to_string(),
            is_interface: false,
            descriptor: Some(internal_error_source_descriptor()),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    )]);
    package.package_requirements = vec![PackageRequirement {
        alias: LINKED_ERROR_DEPENDENCY_ALIAS.to_string(),
        package_id: error_dependency.artifact_ref.package_id.clone(),
        exact_version: error_dependency.artifact_ref.package_version.clone(),
        expected_local_abi: error_dependency
            .artifact_ref
            .package_local_abi_identity
            .clone(),
        collection_name_mapping: BTreeMap::new(),
        expected_package_build: Some(error_dependency.artifact_ref.package_build_id.clone()),
    }];
    package.service_requirements = vec![ServiceRequirement {
        contract_requirement: ContractRequirement {
            alias: "payments".to_string(),
            service_id: "example.com/linked-effect-provider".to_string(),
            contract_version: "1.0.0".to_string(),
            expected_protocol_identity: protocol_identity,
        },
        service_binding_slot: 0,
        used_operations: BTreeSet::from([operation_id]),
    }];
    package.service_call_refs = vec![service_call];
    let package_ref = super::package_ref(&package);
    let dependency_ref = error_dependency.artifact_ref.clone();
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:linked-service-effect"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone(), dependency_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: package_ref.clone(),
                },
                PackageCodeSlot {
                    package: dependency_ref.clone(),
                },
            ],
            package_links: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: package_ref.package_build_id.clone(),
                    package_requirement_alias: LINKED_ERROR_DEPENDENCY_ALIAS.to_string(),
                },
                package: dependency_ref,
                collection_name_mapping: BTreeMap::new(),
            }],
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let records = BTreeMap::from([(internal_type_id, internal_record)]);
    let hydrated = HydratedPackageCode::new(
        Arc::new(package),
        vec![Arc::new(file)],
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(schema_index))
    .with_schema_records(records);
    let dependency_hydrated = HydratedPackageCode::new(
        Arc::new(error_dependency.artifact),
        vec![Arc::new(error_dependency.file)],
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(error_dependency.schema_index))
    .with_schema_records(error_dependency.records);
    let image = skiff_runtime_linker::link_package_fixture_from_runtime_assembly(
        &assembly,
        vec![hydrated, dependency_hydrated],
    )
    .expect("link narrow service-effect fixture");
    let activation = activation_context(assembly.assembly_identity, package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("linked service-effect request generation");
    let target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("linked service-effect eval target");
    (
        target,
        ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            executable: executable_index,
        },
    )
}

struct LinkedErrorDependency {
    artifact_ref: PackageArtifactRef,
    artifact: PackageArtifact,
    file: FileIrUnit,
    schema_index: PackageSchemaIndex,
    records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

fn linked_error_dependency() -> LinkedErrorDependency {
    let source_descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
    };
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("message".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let type_id = skiff_artifact_identity::package_schema_type_id(
        ERROR_PACKAGE_ID,
        ERROR_STABLE_SCHEMA_KEY,
        &canonical_descriptor,
    )
    .expect("linked dependency error identity");
    let index_types = BTreeMap::from([(
        ERROR_STABLE_SCHEMA_KEY.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id.clone(),
            public_path: Some(ERROR_STABLE_SCHEMA_KEY.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let schema_index = PackageSchemaIndex {
        package_id: ERROR_PACKAGE_ID.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            ERROR_PACKAGE_ID,
            &index_types,
        )
        .expect("linked dependency schema index identity"),
        types: index_types,
    };
    let record = Arc::new(PackageSchemaTypeRecord {
        package_id: ERROR_PACKAGE_ID.to_string(),
        stable_schema_key: ERROR_STABLE_SCHEMA_KEY.to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor,
    });
    let mut file = FileIrUnit::empty("errors", "source:linked-service-effect-errors");
    file.type_table.push(TypeDeclIr {
        name: ERROR_STABLE_SCHEMA_KEY.to_string(),
        descriptor: source_descriptor.clone(),
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("linked dependency File IR identity");
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let mut artifact = super::private_package(ERROR_PACKAGE_ID, &file);
    artifact.package_build_id = PackageBuildId::new("build:linked-service-effect-errors");
    artifact.package_local_abi.local_abi_identity =
        PackageLocalAbiIdentity::new("abi:linked-service-effect-errors");
    artifact.package_schema_index.package_schema_index_identity =
        schema_index.package_schema_index_identity.clone();
    artifact.package_schema_type_records = BTreeMap::from([(
        type_id.clone(),
        PackageSchemaTypeRecordRef {
            package_id: ERROR_PACKAGE_ID.to_string(),
            package_schema_type_id: type_id.clone(),
        },
    )]);
    artifact.implementation_links.types = BTreeMap::from([(
        ERROR_STABLE_SCHEMA_KEY.to_string(),
        TypeExport {
            file: file_ref,
            type_index: 0,
            symbol: ERROR_STABLE_SCHEMA_KEY.to_string(),
            is_interface: false,
            descriptor: Some(source_descriptor),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    )]);
    let artifact_ref = super::package_ref(&artifact);
    LinkedErrorDependency {
        artifact_ref,
        artifact,
        file,
        schema_index,
        records: BTreeMap::from([(type_id, record)]),
    }
}

fn linked_service_effect_executable(
    service_call_ref_index: ServiceCallRefIndex,
    payload_type: TypeRefIr,
    catch_type: TypeRefIr,
    payload_field: &str,
    payload_value: &str,
) -> ExecutableIr {
    let service_call = |site| ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index,
            },
            site,
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    };
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "linkedServiceEffectCase".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "$exception".to_string(),
                    kind: SlotKind::Temp,
                },
                SlotIr {
                    index: 1,
                    name: "caught".to_string(),
                    kind: SlotKind::Temp,
                },
                SlotIr {
                    index: 2,
                    name: "response".to_string(),
                    kind: SlotKind::Temp,
                },
            ],
            frame_size: 3,
        },
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: (0_u32..5)
                    .map(|statement| StmtRefIr { statement })
                    .collect(),
            }],
            statements: vec![
                StmtIr::TestEffectRegister {
                    target: TestEffectRegisterTargetIr::ContractOperation {
                        service_call_ref_index,
                    },
                    expect: None,
                    step_expect: None,
                    outcome: TestEffectOutcomeIr::Throw {
                        value: ExprRefIr { expression: 1 },
                        payload_type: payload_type.clone(),
                    },
                },
                StmtIr::TestEffectRegister {
                    target: TestEffectRegisterTargetIr::ContractOperation {
                        service_call_ref_index,
                    },
                    expect: None,
                    step_expect: None,
                    outcome: TestEffectOutcomeIr::Respond {
                        value: ExprRefIr { expression: 5 },
                        value_type: TypeRefIr::builtin("string"),
                    },
                },
                StmtIr::Let {
                    slot: 1,
                    value: ExprRefIr { expression: 4 },
                },
                StmtIr::Let {
                    slot: 2,
                    value: ExprRefIr { expression: 6 },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 10 }),
                },
            ],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: payload_value.to_string(),
                    },
                },
                ExprIr::Construct {
                    type_ref: TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            payload_field.to_string(),
                            TypeRefIr::builtin("string"),
                        )]),
                    },
                    fields: BTreeMap::from([(
                        payload_field.to_string(),
                        ExprRefIr { expression: 0 },
                    )]),
                },
                service_call(linked_service_effect_call_site()),
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Catch {
                    try_expression: ExprRefIr { expression: 2 },
                    catch_slot: 0,
                    catch_type,
                    body: ExprRefIr { expression: 3 },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "accepted".to_string(),
                    },
                },
                service_call(InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
                }),
                ExprIr::LoadSlot { slot: 1 },
                ExprIr::Field {
                    object: ExprRefIr { expression: 7 },
                    field: "exception".to_string(),
                },
                ExprIr::LoadSlot { slot: 2 },
                ExprIr::Construct {
                    type_ref: TypeRefIr::Record {
                        fields: BTreeMap::from([
                            ("exception".to_string(), TypeRefIr::builtin("Json")),
                            ("response".to_string(), TypeRefIr::builtin("string")),
                        ]),
                    },
                    fields: BTreeMap::from([
                        ("exception".to_string(), ExprRefIr { expression: 8 }),
                        ("response".to_string(), ExprRefIr { expression: 9 }),
                    ]),
                },
            ],
        },
        source_span: None,
    }
}

fn internal_error_source_descriptor() -> TypeDescriptorIr {
    TypeDescriptorIr::Record {
        fields: BTreeMap::from([
            ("errorId".to_string(), TypeRefIr::builtin("string")),
            ("message".to_string(), TypeRefIr::builtin("string")),
            ("traceId".to_string(), TypeRefIr::builtin("string")),
        ]),
    }
}

fn linked_service_effect_consumer_executable(
    service_call_ref_index: ServiceCallRefIndex,
    catch_type: TypeRefIr,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "linkedOpaqueServiceEffectCase".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![SlotIr {
                index: 0,
                name: "$exception".to_string(),
                kind: SlotKind::Temp,
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 2 }),
            }],
            expressions: vec![
                ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::ServiceCall {
                            service_call_ref_index,
                        },
                        site: linked_service_effect_call_site(),
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Catch {
                    try_expression: ExprRefIr { expression: 0 },
                    catch_slot: 0,
                    catch_type,
                    body: ExprRefIr { expression: 1 },
                },
            ],
        },
        source_span: None,
    }
}

fn internal_error_contract_descriptor() -> ContractTypeDescriptor {
    ContractTypeDescriptor::Record {
        fields: BTreeMap::from([
            ("errorId".to_string(), ContractTypeRef::builtin("string")),
            ("message".to_string(), ContractTypeRef::builtin("string")),
            ("traceId".to_string(), ContractTypeRef::builtin("string")),
        ]),
    }
}

fn linked_service_effect_call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
    }
}

#[tokio::test]
async fn source_inline_service_effect_sequence_typed_throw_is_caught_then_responds() {
    let fixture = TempFixture::new("source-inline-service-typed-throw");
    let platform_sources = repository_platform_sources();
    let artifacts = fixture.child("artifacts");
    seed_canonical_std(&platform_sources, &artifacts).expect("canonical std seed");

    let error_package = fixture.child("errors");
    write_error_package(&error_package);
    build_authoring_object(
        &platform_sources,
        AuthoringObject::Package,
        &error_package,
        &artifacts,
        "dev",
        true,
    )
    .expect("error package publication");

    let store = CanonicalArtifactStore::open(&artifacts).expect("canonical store");
    let error_pointer = store
        .read_package_artifact_pointer(ERROR_PACKAGE_ID, ERROR_PACKAGE_VERSION)
        .expect("error package pointer read")
        .expect("error package pointer");
    let error_artifact = store
        .read_package_artifact(&error_pointer.artifact)
        .expect("error package artifact");
    let error_schema = store
        .resolve_package_artifact_schema(&error_artifact)
        .expect("error package schema");
    let _failure_entry = error_schema
        .index
        .types
        .get(ERROR_STABLE_SCHEMA_KEY)
        .expect("public Failure package schema");

    publish_open_error_service_contract(&store);

    let consumer = fixture.child("consumer");
    write_consumer_package(&consumer);
    let project = compile_package_project(&platform_sources, &consumer, &artifacts)
        .expect("consumer source package compile");
    let cases = discover_package_test_cases(&consumer, &consumer, false).expect("test discovery");
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources, &consumer, &artifacts, &project, &cases)
            .expect("source test overlay compile and lower");
    assert_throw_lowered_to_exact_package_symbol(&overlay);
    execute_overlay_case(&store, &overlay, &project.dependency_packages);
}

#[tokio::test]
async fn source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable() {
    let fixture = TempFixture::new("source-inline-compiler-owned-std");
    let platform_sources = repository_platform_sources();
    let artifacts = fixture.child("artifacts");
    seed_canonical_std(&platform_sources, &artifacts).expect("canonical std seed");

    let consumer = fixture.child("consumer");
    write_std_effect_consumer_package(&consumer);
    let project = compile_package_project(&platform_sources, &consumer, &artifacts)
        .expect("std effect consumer source package compile");
    let cases = discover_package_test_cases(&consumer, &consumer, false).expect("test discovery");
    assert_eq!(cases.len(), 1);
    let overlay =
        compile_package_test_overlay(&platform_sources, &consumer, &artifacts, &project, &cases)
            .expect("compiler-owned std effect overlay compile and lower");

    let request_callable = PackageCallableId::new("pkg-callable:skiff.run/std:std.http.request");
    let std_calls = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.expressions)
        .filter(|expression| {
            matches!(
                expression,
                skiff_artifact_model::ExprIr::Call {
                    call:
                        skiff_artifact_model::CallIr {
                            target:
                                skiff_artifact_model::CallTargetIr::PackageCallable {
                                    package_ref:
                                        PackageRefIr::Dependency { dependency_ref },
                                    package_callable_id,
                                },
                            ..
                        },
                } if dependency_ref == "std" && package_callable_id == &request_callable
            )
        })
        .count();
    assert_eq!(
        std_calls, 1,
        "the production call must use the exact std package callable"
    );
    let registrations = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.statements)
        .filter(|statement| {
            matches!(
                statement,
                skiff_artifact_model::StmtIr::TestEffectRegister {
                    target:
                        skiff_artifact_model::TestEffectRegisterTargetIr::PackageCallable {
                            package_ref: PackageRefIr::Dependency { dependency_ref },
                            callable_id,
                        },
                    ..
                } if dependency_ref == "std" && callable_id == &request_callable
            )
        })
        .count();
    assert_eq!(
        registrations, 1,
        "the setup must register the same exact std package callable"
    );

    let store = CanonicalArtifactStore::open(&artifacts).expect("canonical store");
    execute_overlay_case(&store, &overlay, &project.dependency_packages);
}

fn execute_overlay_case(
    store: &CanonicalArtifactStore,
    overlay: &PublishedPackageTestOverlay,
    dependencies: &[PackageArtifact],
) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("source-inline-overlay".to_string())
            .stack_size(16 * 1024 * 1024)
            .spawn_scoped(scope, || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("source inline overlay test runtime")
                    .block_on(execute_hydrated_overlay_case(store, overlay, dependencies));
            })
            .expect("source inline overlay test thread")
            .join()
            .expect("source inline overlay test thread should finish");
    });
}

async fn execute_hydrated_overlay_case(
    store: &CanonicalArtifactStore,
    overlay: &PublishedPackageTestOverlay,
    dependencies: &[PackageArtifact],
) {
    let mut packages = vec![overlay.overlay.artifact.clone()];
    packages.extend(dependencies.iter().cloned());
    let assembly = package_link_fixture(&packages);
    let overlay_ref =
        package_artifact_ref(&overlay.overlay.artifact).expect("overlay package reference");
    let binding = overlay.bindings.first().expect("one test binding");
    let callable = overlay
        .overlay
        .artifact
        .callable_links
        .get(&binding.callable_id)
        .expect("test callable link");
    let hydrated = hydrate_packages(store, overlay, dependencies);
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
            .expect("fully hydrated source overlay packages should link");
    let caller_addr = image
        .shared_packages()
        .code_by_build(&overlay_ref.package_build_id)
        .expect("overlay code slot")
        .executable_addr(&callable.target)
        .expect("test callable executable address");

    let activation = activation_context(
        assembly.assembly_identity,
        overlay_ref.package_build_id.clone(),
    );
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request =
        RequestActivationContext::begin(activation).expect("test request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("linked source overlay should form an eval target");
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context =
        execution_context_with_trace(&interpreter, eval_target, LINKED_SERVICE_EFFECT_TRACE_ID);
    let mut heap = RequestHeap::default();

    interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &caller_addr, Vec::new())
        .await
        .expect("the first typed throw must be caught and the second response must be returned");
    interpreter
        .finalize_test_case()
        .expect("both ordered service effect outcomes must be consumed");
}

fn publish_open_error_service_contract(store: &CanonicalArtifactStore) {
    let contract = compile_contract(ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: SERVICE_VERSION.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "value".to_string(),
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                stream: BoundaryStreamContract::Unary,
                callbacks: BoundaryCallbackContract::None,
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
        package_type_requirements: Vec::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "open error effect payments".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("open service error channel contract compile");
    let reference =
        service_contract_ref(&contract).expect("open service error channel contract reference");
    store
        .write_service_contract(&contract)
        .expect("open service error channel contract record");
    let pointer = ServiceContractPointer::new(reference)
        .expect("open service error channel contract pointer");
    store
        .compare_and_swap_service_contract_pointer(None, &pointer)
        .expect("open service error channel contract pointer publication");
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn assert_throw_lowered_to_exact_package_symbol(overlay: &PublishedPackageTestOverlay) {
    let error_dependency = overlay
        .dependency_packages
        .iter()
        .find(|package| package.package_id == ERROR_PACKAGE_ID)
        .expect("error dependency package");
    let payload_types = overlay
        .overlay
        .file_ir_units
        .iter()
        .flat_map(|file| &file.unit.executables)
        .flat_map(|executable| &executable.body.statements)
        .filter_map(|statement| {
            let skiff_artifact_model::StmtIr::TestEffectRegister {
                outcome: TestEffectOutcomeIr::Throw { payload_type, .. },
                ..
            } = statement
            else {
                return None;
            };
            Some(payload_type)
        })
        .collect::<Vec<_>>();
    assert_eq!(payload_types.len(), 1);
    assert!(matches!(
        payload_types[0],
        TypeRefIr::PackageSymbol { symbol }
            if symbol.package
                == (PackageRefIr::Dependency {
                    dependency_ref: LINKED_ERROR_DEPENDENCY_ALIAS.to_string(),
                })
                && symbol.symbol_path == ERROR_STABLE_SCHEMA_KEY
                && symbol.abi_expectation.as_deref()
                    == Some(error_dependency.package_local_abi.local_abi_identity.as_str())
    ));
}

fn package_link_fixture(packages: &[PackageArtifact]) -> RuntimeAssembly {
    let references = packages
        .iter()
        .map(|package| package_artifact_ref(package).expect("package reference"))
        .collect::<Vec<_>>();
    let by_coordinate = packages
        .iter()
        .map(|package| {
            (
                (
                    package.package_id.as_str(),
                    package.package_version.as_str(),
                ),
                package,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let package_links = packages
        .iter()
        .flat_map(|caller| {
            caller
                .package_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| {
            let dependency = by_coordinate
                .get(&(
                    requirement.package_id.as_str(),
                    requirement.exact_version.as_str(),
                ))
                .expect("exact package dependency in link closure");
            assert_eq!(
                dependency.package_local_abi.local_abi_identity,
                requirement.expected_local_abi
            );
            PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: package_artifact_ref(dependency).expect("dependency package reference"),
                collection_name_mapping: requirement.collection_name_mapping.clone(),
            }
        })
        .collect();
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("test-fixture:source-inline-service-typed-throw"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: references.clone(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: references
                .into_iter()
                .map(|package| PackageCodeSlot { package })
                .collect(),
            package_links,
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}

fn hydrate_packages(
    store: &CanonicalArtifactStore,
    overlay: &PublishedPackageTestOverlay,
    dependencies: &[PackageArtifact],
) -> Vec<HydratedPackageCode> {
    let overlay_schema_records = overlay
        .overlay
        .package_schema_type_records
        .iter()
        .map(|(type_id, record)| (type_id.clone(), Arc::new(record.clone())))
        .collect();
    let mut hydrated = vec![HydratedPackageCode::new(
        Arc::new(overlay.overlay.artifact.clone()),
        overlay
            .overlay
            .file_ir_units
            .iter()
            .map(|file| Arc::new(file.unit.clone()))
            .collect(),
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(overlay.overlay.package_schema_index.clone()))
    .with_schema_records(overlay_schema_records)];
    hydrated.extend(dependencies.iter().map(|package| {
        let reference = package_artifact_ref(package).expect("dependency package reference");
        let files = package
            .files
            .iter()
            .map(|file| {
                store
                    .read_file_ir(&reference, file)
                    .expect("dependency File IR")
            })
            .collect();
        let schema = store
            .resolve_package_artifact_schema(package)
            .expect("dependency package schema");
        HydratedPackageCode::new(
            Arc::new(package.clone()),
            files,
            PublicationResourceTable::default(),
        )
        .with_schema_index(schema.index)
        .with_schema_records(schema.records)
    }));
    hydrated
}

fn write_error_package(root: &Path) {
    fs::create_dir_all(root).expect("error package directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {ERROR_PACKAGE_ID}\nversion: {ERROR_PACKAGE_VERSION}\n"),
    )
    .expect("error package manifest");
    fs::write(root.join("api.yml"), "Failure: main.Failure\n").expect("error package API");
    fs::write(
        root.join("main.skiff"),
        r#"type Failure {
  message: string,
}
"#,
    )
    .expect("error package source");
}

fn write_consumer_package(root: &Path) {
    fs::create_dir_all(root).expect("consumer directory");
    fs::write(
        root.join("package.yml"),
        format!(
            r#"id: example.com/typed-effect-consumer
version: 1.0.0
packages:
  - id: {ERROR_PACKAGE_ID}
    version: {ERROR_PACKAGE_VERSION}
    alias: errors
services:
  - id: {SERVICE_ID}
    version: {SERVICE_VERSION}
    alias: payments
"#
        ),
    )
    .expect("consumer manifest");
    fs::write(root.join("api.yml"), "{}\n").expect("consumer API");
    fs::write(
        root.join("main.skiff"),
        r#"import errors

function exercise() -> string {
  const first = catch<errors.Failure>(payments/echo("first"))
  if first.tag == "ok" {
    return "typed-throw-was-not-caught"
  }
  return first.exception.error.message + ":" + payments/echo("second")
}
"#,
    )
    .expect("consumer source");
    fs::write(
        root.join("main.test.skiff"),
        r#"import errors

test "typed service throw is caught before sequence response" effects {
  payments/echo {
    sequence: [
      {
        expect: "first",
        throw: errors.Failure { message: "denied" },
      },
      {
        expect: "second",
        respond: "accepted",
      },
    ],
  }
} {
  assert root.main.exercise() == "denied:accepted"
}
"#,
    )
    .expect("consumer test source");
}

fn write_std_effect_consumer_package(root: &Path) {
    fs::create_dir_all(root).expect("std effect consumer directory");
    fs::write(
        root.join("package.yml"),
        "id: example.com/std-effect-consumer\nversion: 1.0.0\n",
    )
    .expect("std effect consumer manifest");
    fs::write(root.join("api.yml"), "{}\n").expect("std effect consumer API");
    fs::write(
        root.join("main.skiff"),
        r#"import std

function fetchStatus() -> integer {
  const response = std.http.request(std.http.HttpClientRequest {
    method: "GET",
    url: "https://must-not-run.invalid/resource",
    headers: Array.empty<std.http.HttpHeader>(),
    body: null,
    timeoutMs: null,
  })
  return response.status
}
"#,
    )
    .expect("std effect consumer source");
    fs::write(
        root.join("main.test.skiff"),
        r#"import std

test "compiler-owned std request is replaced by exact package identity" effects {
  std/http.request {
    expect: {
      method: "GET",
      url: "https://must-not-run.invalid/resource",
    },
    respond: std.http.HttpClientResponse {
      status: 204,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8(""),
    },
  }
} {
  assert root.main.fetchStatus() == 204
}
"#,
    )
    .expect("std effect consumer test source");
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval must live two levels below the Skiff root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary fixture root");
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
