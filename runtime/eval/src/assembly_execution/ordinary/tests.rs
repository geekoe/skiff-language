mod representation_combined_probe;
pub(crate) mod service_error_consumer;
mod source_generic_json_encode_red;
mod source_inline_effect_e2e;
#[path = "test_runtime.rs"]
pub(crate) mod test_runtime;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::*;
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
    RequestActivationContext, RuntimeActivation,
};
use skiff_runtime_linked_program::LinkedCallTarget;
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapHandle, HeapNode, RuntimeValue},
    service_error::{
        CatchIdentity, ExceptionStackFrame, LocalExecutionTypeIdentity, NominalTypeIdentity,
    },
};

use crate::{
    assembly_execution::service_error_channel::{
        start_restricted_service_diagnostic_probe_for_test,
        take_restricted_service_diagnostics_for_test,
    },
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

const DEPENDENCY_ALIAS: &str = "mutableDependency";
const TYPED_THROW_REQUEST_TRACE_ID: &str = "test-trace:inline-effect-typed-throw";

fn test_instruction_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

/// Executes a real linked caller and package callee through the canonical dispatcher. The
/// callee's index assignment can only be observed here when package-direct keeps the exact
/// request heap and handle; a manual handle clone would not exercise this path.
#[tokio::test]
async fn package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation() {
    let fixture = package_direct_fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let caller_handle = heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller mutable array should allocate");

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(caller_handle)],
        )
        .await
        .expect("canonical caller and package-direct callee should execute");

    let RuntimeValue::Heap(returned_handle) = result else {
        panic!("package callee should return the caller heap handle")
    };
    assert_eq!(
        returned_handle, caller_handle,
        "package-direct must preserve handle identity"
    );
    assert_array_item(&heap, caller_handle, "package-callee");
}

#[tokio::test]
async fn package_constant_load_resolves_exact_dependency_implementation_address() {
    let fixture = package_constant_fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();

    let result = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("linked package constant should execute through its exact ConstAddr");

    assert_eq!(result, RuntimeValue::String("private-value".to_string()));
}

#[tokio::test]
async fn inline_effect_setup_dispatch_reports_request_subset_mismatch() {
    let fixture = package_direct_fixture_with_caller(CallerFixtureKind::EffectMismatch);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("actual".to_string())])
        .expect("request array should allocate");

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect_err("registered effect must reject a mismatching request subset");

    assert!(error
        .to_string()
        .contains("test effect expectation failed: expected request subset"));
    interpreter
        .finalize_test_case()
        .expect("a dispatched mismatching outcome is still consumed");
}

#[tokio::test]
async fn inline_effect_request_finalization_reports_and_clears_unused_setup() {
    let fixture = package_direct_fixture_with_caller(CallerFixtureKind::EffectUnused);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("actual".to_string())])
        .expect("request array should allocate");

    interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect("body that does not dispatch the registered effect should finish");

    let error = interpreter
        .finalize_test_case()
        .expect_err("request finalization must reject unused setup outcomes");
    assert!(error.to_string().contains("unused test effects"));
    interpreter
        .finalize_test_case()
        .expect("request finalization must clear the registry after reporting");
}

#[tokio::test]
async fn restricted_service_diagnostic_package_callable_typed_throw_submits_zero() {
    let fixture = package_direct_fixture_with_caller(CallerFixtureKind::EffectThrowCatch);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let generation = fixture.eval_target.request_activation().generation();
    start_restricted_service_diagnostic_probe_for_test(generation);
    let context = execution_context_with_trace(
        &interpreter,
        fixture.eval_target,
        TYPED_THROW_REQUEST_TRACE_ID,
    );
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("request".to_string())])
        .expect("request array should allocate");

    let caught = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect("the exact nominal catch must handle the registered typed throw");

    let RuntimeValue::Heap(caught_handle) = caught else {
        panic!("catch result should be a request-heap object");
    };
    let HeapNode::Object(caught_object) = heap.get(caught_handle).expect("caught result") else {
        panic!("catch result should be an object");
    };
    assert_eq!(
        caught_object.fields().get("tag"),
        Some(&RuntimeValue::String("err".to_string()))
    );
    let RuntimeValue::Heap(exception_handle) = caught_object
        .fields()
        .get("exception")
        .expect("caught result should retain the request-local exception")
    else {
        panic!("request-local exception should be a heap node");
    };
    let HeapNode::Exception(exception) = heap
        .get(*exception_handle)
        .expect("request-local exception")
    else {
        panic!("caught value must retain RequestException");
    };
    let expected_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: skiff_runtime_linked_program::TypeAddr {
                unit: skiff_runtime_linked_program::UnitAddr::Package(0),
                file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
                type_index: 0,
            },
            type_arguments: Vec::new(),
        },
    ));
    let expected_site = test_instruction_site();
    assert_eq!(
        exception.local_catch_identity(),
        Some(&expected_identity),
        "catch must retain the exact linked nominal identity",
    );
    assert_eq!(exception.source(), &expected_site);
    assert!(matches!(
        exception.stack().last(),
        Some(ExceptionStackFrame::Local { site }) if site == &expected_site
    ));
    assert_eq!(
        exception.correlation().trace_id,
        TYPED_THROW_REQUEST_TRACE_ID,
        "local exception correlation must retain the exact request trace",
    );
    assert!(
        exception
            .correlation()
            .error_id
            .starts_with(&format!("{TYPED_THROW_REQUEST_TRACE_ID}:local-error:")),
        "local exception error id must derive from the exact request trace",
    );
    let RuntimeValue::Heap(payload_handle) = exception
        .local_value()
        .expect("request-local exception cause")
        .value()
    else {
        panic!("typed payload should be an object");
    };
    let HeapNode::Object(payload) = heap.get(*payload_handle).expect("typed local payload") else {
        panic!("typed payload should be an object");
    };
    assert_eq!(
        payload.fields().get("message"),
        Some(&RuntimeValue::String("denied".to_string()))
    );
    assert!(
        take_restricted_service_diagnostics_for_test(generation).is_empty(),
        "PackageCallable test effects must not submit service diagnostics"
    );
    interpreter
        .finalize_test_case()
        .expect("caught throw outcome should be consumed");
}

#[tokio::test]
async fn inline_effect_stream_is_consumed_in_buffered_event_order() {
    let fixture = package_direct_fixture_with_caller(CallerFixtureKind::EffectStream);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("request".to_string())])
        .expect("request array should allocate");

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect("body should consume the registered buffered stream");

    assert_eq!(result, RuntimeValue::String("second".to_string()));
    interpreter
        .finalize_test_case()
        .expect("stream outcome should be consumed");
}

#[tokio::test]
async fn inline_effect_response_is_materialized_in_spawned_stream_producer_heap() {
    let fixture = package_direct_fixture_with_caller(CallerFixtureKind::EffectProducerHeap);
    let interpreter = Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
        Default::default(),
        test_runtime::runtime_factory(),
    );
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("request".to_string())])
        .expect("request array should allocate");

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect("spawned producer should materialize and emit the registered response");

    let RuntimeValue::Heap(response) = result else {
        panic!("stream consumer should return the response array");
    };
    assert_array_item(&heap, response, "response");
    interpreter
        .finalize_test_case()
        .expect("producer-dispatched response should be consumed");
}

#[tokio::test]
async fn object_materialization_interpreter_heap_shape_distinguishes_construct_and_map_literal() {
    let (object, object_heap) = execute_materialization_expression(ExprIr::Construct {
        type_ref: TypeRefIr::Record {
            fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("string"))]),
        },
        fields: BTreeMap::from([("value".to_string(), ExprRefIr { expression: 0 })]),
    })
    .await;
    let RuntimeValue::Heap(object_handle) = object else {
        panic!("Construct should return a heap value")
    };
    assert!(matches!(
        object_heap
            .get(object_handle)
            .expect("Construct heap handle should resolve"),
        HeapNode::Object(_)
    ));

    let (map, map_heap) = execute_materialization_expression(ExprIr::MapLiteral {
        entries: BTreeMap::from([("value".to_string(), ExprRefIr { expression: 0 })]),
    })
    .await;
    let RuntimeValue::Heap(map_handle) = map else {
        panic!("MapLiteral should return a heap value")
    };
    assert!(matches!(
        map_heap
            .get(map_handle)
            .expect("MapLiteral heap handle should resolve"),
        HeapNode::Map(_)
    ));
}

struct PackageDirectFixture {
    eval_target: RuntimeAssemblyEvalTarget,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
}

async fn execute_materialization_expression(expression: ExprIr) -> (RuntimeValue, RequestHeap) {
    let fixture = materialization_fixture(expression);
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("materialization expression should execute");
    (value, heap)
}

fn materialization_fixture(expression: ExprIr) -> PackageDirectFixture {
    let mut file = FileIrUnit::empty("object_materialization.heap_shape", "source:heap-shape");
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "heapShape".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "materialized".to_string(),
                    },
                },
                expression,
            ],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("materialization File IR should receive a canonical identity");
    let mut package = private_package("example.object-materialization", &file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("materialization package should receive canonical identities");
    let package_ref = package_ref(&package);
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:object-materialization-heap-shape"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image =
        crate::test_support::link_package_fixture(assembly.clone(), vec![(package, vec![file])]);
    let caller_addr = skiff_runtime_linked_program::ExecutableAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let activation = activation_context(assembly.assembly_identity, package_ref.package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("materialization request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("materialization image and activation should form an eval target");
    PackageDirectFixture {
        eval_target,
        caller_addr,
    }
}

fn package_direct_fixture() -> PackageDirectFixture {
    package_direct_fixture_with_caller(CallerFixtureKind::Mutation)
}

fn package_constant_fixture() -> PackageDirectFixture {
    const CONSTANT_PATH: &str = "internal.values.PRIVATE_VALUE";
    let string_type = TypeRefIr::builtin("string");

    let mut dependency_file =
        FileIrUnit::empty("internal.values", "source:package-constant-dependency");
    dependency_file.declarations.constants.insert(
        "PRIVATE_VALUE".to_string(),
        ConstDeclarationIr {
            const_index: 0,
            symbol: "PRIVATE_VALUE".to_string(),
            ty: string_type.clone(),
            source_span: None,
        },
    );
    dependency_file.constants.push(ConstIr {
        name: "PRIVATE_VALUE".to_string(),
        ty: string_type.clone(),
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions: vec![ExprIr::Literal {
                value: LiteralIr::String {
                    value: "private-value".to_string(),
                },
            }],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut dependency_file)
        .expect("dependency File IR should receive a canonical identity");

    let mut dependency_package =
        private_package("example.package-constant-dependency", &dependency_file);
    dependency_package
        .package_local_abi
        .implementation_symbols
        .insert(
            CONSTANT_PATH.to_string(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!(
                    "pkg-const:{}:top-level:{CONSTANT_PATH}",
                    dependency_package.package_id
                ),
                ty: PackageTypeRef::Local {
                    local_type: string_type.clone(),
                },
            },
        );
    dependency_package.implementation_links.constants.insert(
        CONSTANT_PATH.to_string(),
        ConstExport {
            file: file_ref(&dependency_file),
            const_index: 0,
            symbol: "PRIVATE_VALUE".to_string(),
            ty: string_type,
        },
    );
    skiff_artifact_identity::assign_package_artifact_identities(&mut dependency_package)
        .expect("dependency package should receive canonical identities");
    let dependency_ref = package_ref(&dependency_package);

    let package_symbol = PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: DEPENDENCY_ALIAS.to_string(),
        },
        symbol_path: CONSTANT_PATH.to_string(),
        abi_expectation: Some(dependency_ref.package_local_abi_identity.to_string()),
    };
    let mut caller_file = FileIrUnit::empty("main", "source:package-constant-caller");
    caller_file
        .external_refs
        .package_symbols
        .push(package_symbol.clone());
    caller_file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "loadPrivateValue".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("string"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions: vec![ExprIr::LoadPackageConst {
                symbol: package_symbol,
            }],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut caller_file)
        .expect("caller File IR should receive a canonical identity");
    let mut caller_package = private_package("example.package-constant-caller", &caller_file);
    caller_package
        .package_requirements
        .push(PackageRequirement {
            alias: DEPENDENCY_ALIAS.to_string(),
            package_id: dependency_ref.package_id.clone(),
            exact_version: dependency_ref.package_version.clone(),
            expected_local_abi: dependency_ref.package_local_abi_identity.clone(),
            collection_name_mapping: BTreeMap::new(),
            expected_package_build: Some(dependency_ref.package_build_id.clone()),
        });
    skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
        .expect("caller package should receive canonical identities");
    let caller_ref = package_ref(&caller_package);

    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:package-constant"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![caller_ref.clone(), dependency_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: caller_ref.clone(),
                },
                PackageCodeSlot {
                    package: dependency_ref.clone(),
                },
            ],
            package_links: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller_ref.package_build_id.clone(),
                    package_requirement_alias: DEPENDENCY_ALIAS.to_string(),
                },
                package: dependency_ref,
                collection_name_mapping: BTreeMap::new(),
            }],
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = crate::test_support::link_package_fixture(
        assembly.clone(),
        vec![
            (caller_package, vec![caller_file]),
            (dependency_package, vec![dependency_file]),
        ],
    );
    let caller_addr = skiff_runtime_linked_program::ExecutableAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let activation = activation_context(assembly.assembly_identity, caller_ref.package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("package constant request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("package constant image and activation should form an eval target");
    PackageDirectFixture {
        eval_target,
        caller_addr,
    }
}

#[derive(Clone, Copy)]
enum CallerFixtureKind {
    Mutation,
    EffectMismatch,
    EffectUnused,
    EffectThrowCatch,
    EffectStream,
    EffectProducerHeap,
}

fn package_direct_fixture_with_caller(caller_kind: CallerFixtureKind) -> PackageDirectFixture {
    let array_type = array_type();
    let callable_id = PackageCallableId::new("pkg-callable:example.package-direct-callee:mutate");
    let real_callee_is_stream_producer = matches!(caller_kind, CallerFixtureKind::EffectStream);

    let mut callee_file = FileIrUnit::empty("package_direct.callee", "source:callee");
    callee_file
        .executables
        .push(if real_callee_is_stream_producer {
            stream_callee_executable(array_type.clone())
        } else {
            callee_executable(array_type.clone())
        });
    skiff_artifact_identity::assign_file_ir_identity(&mut callee_file)
        .expect("callee File IR should receive a canonical identity");
    let mut callee_package = if real_callee_is_stream_producer {
        stream_callable_package(
            "example.package-direct-callee",
            &callee_file,
            callable_id.clone(),
            array_type.clone(),
        )
    } else {
        callable_package(
            "example.package-direct-callee",
            &callee_file,
            callable_id.clone(),
            array_type.clone(),
        )
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut callee_package)
        .expect("callee package should receive canonical identities");
    let callee_ref = package_ref(&callee_package);

    let dependency_ref = PackageRefIr::Dependency {
        dependency_ref: DEPENDENCY_ALIAS.to_string(),
    };
    let mut caller_file = FileIrUnit::empty("package_direct.caller", "source:caller");
    caller_file
        .external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: dependency_ref.clone(),
            package_callable_id: callable_id.clone(),
        });
    if matches!(caller_kind, CallerFixtureKind::EffectThrowCatch) {
        caller_file.type_table.push(TypeDeclIr {
            name: "DeniedError".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
    }
    let producer_array_type = array_type.clone();
    caller_file.executables.push(match caller_kind {
        CallerFixtureKind::Mutation => {
            caller_executable(array_type, dependency_ref.clone(), callable_id.clone())
        }
        CallerFixtureKind::EffectMismatch => inline_effect_caller_executable(
            array_type,
            dependency_ref.clone(),
            callable_id.clone(),
            true,
        ),
        CallerFixtureKind::EffectUnused => inline_effect_caller_executable(
            array_type,
            dependency_ref.clone(),
            callable_id.clone(),
            false,
        ),
        CallerFixtureKind::EffectThrowCatch => inline_effect_throw_catch_executable(
            array_type,
            dependency_ref.clone(),
            callable_id.clone(),
        ),
        CallerFixtureKind::EffectStream => {
            inline_effect_stream_executable(array_type, dependency_ref.clone(), callable_id.clone())
        }
        CallerFixtureKind::EffectProducerHeap => inline_effect_producer_heap_caller_executable(
            array_type.clone(),
            dependency_ref.clone(),
            callable_id.clone(),
        ),
    });
    if matches!(caller_kind, CallerFixtureKind::EffectProducerHeap) {
        caller_file
            .executables
            .push(inline_effect_dispatching_stream_producer(
                producer_array_type,
                dependency_ref,
                callable_id,
            ));
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut caller_file)
        .expect("caller File IR should receive a canonical identity");
    let mut caller_package = private_package("example.package-direct-caller", &caller_file);
    caller_package
        .package_requirements
        .push(PackageRequirement {
            alias: DEPENDENCY_ALIAS.to_string(),
            package_id: callee_ref.package_id.clone(),
            exact_version: callee_ref.package_version.clone(),
            expected_local_abi: callee_ref.package_local_abi_identity.clone(),
            collection_name_mapping: BTreeMap::new(),
            expected_package_build: None,
        });
    skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
        .expect("caller package should receive canonical identities");
    let caller_ref = package_ref(&caller_package);

    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:package-direct-same-heap"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![caller_ref.clone(), callee_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: caller_ref.clone(),
                },
                PackageCodeSlot {
                    package: callee_ref.clone(),
                },
            ],
            package_links: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller_ref.package_build_id.clone(),
                    package_requirement_alias: DEPENDENCY_ALIAS.to_string(),
                },
                package: callee_ref,
                collection_name_mapping: BTreeMap::new(),
            }],
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = crate::test_support::link_package_fixture(
        assembly.clone(),
        vec![
            (caller_package, vec![caller_file]),
            (callee_package, vec![callee_file]),
        ],
    );
    let caller_addr = skiff_runtime_linked_program::ExecutableAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        executable: 0,
    };

    let activation = activation_context(assembly.assembly_identity, caller_ref.package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("package-direct request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("canonical image and activation should form an eval target");
    PackageDirectFixture {
        eval_target,
        caller_addr,
    }
}

fn inline_effect_producer_heap_caller_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "inlineEffectProducerHeapCase".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type.clone(),
        self_type: None,
        slots: SlotLayout {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "value".to_string(),
                    kind: SlotKind::Param,
                },
                SlotIr {
                    index: 1,
                    name: "item".to_string(),
                    kind: SlotKind::Temp,
                },
                SlotIr {
                    index: 2,
                    name: "last".to_string(),
                    kind: SlotKind::Temp,
                },
            ],
            frame_size: 3,
        },
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![
                BlockIr {
                    label: "entry".to_string(),
                    statements: vec![
                        StmtRefIr { statement: 0 },
                        StmtRefIr { statement: 1 },
                        StmtRefIr { statement: 2 },
                        StmtRefIr { statement: 4 },
                    ],
                },
                BlockIr {
                    label: "consume".to_string(),
                    statements: vec![StmtRefIr { statement: 3 }],
                },
            ],
            statements: vec![
                StmtIr::TestEffectRegister {
                    target: TestEffectRegisterTargetIr::PackageCallable {
                        package_ref,
                        callable_id: package_callable_id,
                    },
                    expect: None,
                    step_expect: None,
                    outcome: TestEffectOutcomeIr::Respond {
                        value: ExprRefIr { expression: 2 },
                        value_type: array_type.clone(),
                    },
                },
                StmtIr::Let {
                    slot: 2,
                    value: ExprRefIr { expression: 0 },
                },
                StmtIr::ForIn {
                    item_slot: 1,
                    item_type: Some(array_type.clone()),
                    value_slot: None,
                    iterable: ExprRefIr { expression: 3 },
                    body: "consume".to_string(),
                },
                StmtIr::Assign {
                    target: AssignTargetIr::Slot { slot: 2 },
                    value: ExprRefIr { expression: 4 },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 5 }),
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "response".to_string(),
                    },
                },
                ExprIr::ArrayLiteral {
                    items: vec![ExprRefIr { expression: 1 }],
                },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::LocalExecutable {
                            executable_index: 1,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::LoadSlot { slot: 1 },
                ExprIr::LoadSlot { slot: 2 },
            ],
        },
        source_span: None,
    }
}

fn inline_effect_dispatching_stream_producer(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "dispatchEffectInProducerHeap".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![array_type],
        },
        self_type: None,
        slots: SlotLayout {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "value".to_string(),
                    kind: SlotKind::Param,
                },
                SlotIr {
                    index: 1,
                    name: "response".to_string(),
                    kind: SlotKind::Temp,
                },
            ],
            frame_size: 2,
        },
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Let {
                    slot: 1,
                    value: ExprRefIr { expression: 1 },
                },
                StmtIr::Emit {
                    operation: "provide".to_string(),
                    value: ExprRefIr { expression: 2 },
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::LoadSlot { slot: 1 },
            ],
        },
        source_span: None,
    }
}

fn inline_effect_stream_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "inlineEffectStreamCase".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type,
        }],
        return_type: TypeRefIr::builtin("string"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "value".to_string(),
                    kind: SlotKind::Param,
                },
                SlotIr {
                    index: 1,
                    name: "item".to_string(),
                    kind: SlotKind::Temp,
                },
                SlotIr {
                    index: 2,
                    name: "last".to_string(),
                    kind: SlotKind::Temp,
                },
            ],
            frame_size: 3,
        },
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![
                BlockIr {
                    label: "entry".to_string(),
                    statements: vec![
                        StmtRefIr { statement: 0 },
                        StmtRefIr { statement: 1 },
                        StmtRefIr { statement: 2 },
                        StmtRefIr { statement: 4 },
                    ],
                },
                BlockIr {
                    label: "consume".to_string(),
                    statements: vec![StmtRefIr { statement: 3 }],
                },
            ],
            statements: vec![
                StmtIr::TestEffectRegister {
                    target: TestEffectRegisterTargetIr::PackageCallable {
                        package_ref: package_ref.clone(),
                        callable_id: package_callable_id.clone(),
                    },
                    expect: None,
                    step_expect: None,
                    outcome: TestEffectOutcomeIr::Stream {
                        values: vec![ExprRefIr { expression: 1 }, ExprRefIr { expression: 2 }],
                        item_type: TypeRefIr::builtin("string"),
                    },
                },
                StmtIr::Let {
                    slot: 2,
                    value: ExprRefIr { expression: 1 },
                },
                StmtIr::ForIn {
                    item_slot: 1,
                    item_type: Some(TypeRefIr::builtin("string")),
                    value_slot: None,
                    iterable: ExprRefIr { expression: 3 },
                    body: "consume".to_string(),
                },
                StmtIr::Assign {
                    target: AssignTargetIr::Slot { slot: 2 },
                    value: ExprRefIr { expression: 4 },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 5 }),
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "first".to_string(),
                    },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "second".to_string(),
                    },
                },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::LoadSlot { slot: 1 },
                ExprIr::LoadSlot { slot: 2 },
            ],
        },
        source_span: None,
    }
}

fn inline_effect_throw_catch_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    let error_type = TypeRefIr::PublicationType {
        module_path: "package_direct.caller".to_string(),
        type_index: 0,
    };
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "inlineEffectThrowCatchCase".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type,
        }],
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "value".to_string(),
                    kind: SlotKind::Param,
                },
                SlotIr {
                    index: 1,
                    name: "$catch".to_string(),
                    kind: SlotKind::Temp,
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::TestEffectRegister {
                    target: TestEffectRegisterTargetIr::PackageCallable {
                        package_ref: package_ref.clone(),
                        callable_id: package_callable_id.clone(),
                    },
                    expect: None,
                    step_expect: None,
                    outcome: TestEffectOutcomeIr::Throw {
                        value: ExprRefIr { expression: 2 },
                        payload_type: error_type.clone(),
                    },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 5 }),
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "denied".to_string(),
                    },
                },
                ExprIr::Construct {
                    type_ref: TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "message".to_string(),
                            TypeRefIr::builtin("string"),
                        )]),
                    },
                    fields: BTreeMap::from([("message".to_string(), ExprRefIr { expression: 1 })]),
                },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                ExprIr::LoadSlot { slot: 1 },
                ExprIr::Catch {
                    try_expression: ExprRefIr { expression: 3 },
                    catch_slot: 1,
                    catch_type: error_type,
                    body: ExprRefIr { expression: 4 },
                },
            ],
        },
        source_span: None,
    }
}

fn inline_effect_caller_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
    dispatch: bool,
) -> ExecutableIr {
    let mut statements = vec![StmtIr::TestEffectRegister {
        target: TestEffectRegisterTargetIr::PackageCallable {
            package_ref: package_ref.clone(),
            callable_id: package_callable_id.clone(),
        },
        expect: Some(TestEffectExpectedIr {
            value: ExprRefIr { expression: 1 },
            request_type: array_type.clone(),
        }),
        step_expect: None,
        outcome: TestEffectOutcomeIr::Respond {
            value: ExprRefIr { expression: 0 },
            value_type: array_type.clone(),
        },
    }];
    let return_expression = if dispatch { 2 } else { 0 };
    statements.push(StmtIr::Return {
        value: Some(ExprRefIr {
            expression: return_expression,
        }),
    });
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "inlineEffectCase".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type,
        self_type: None,
        slots: parameter_slots(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements,
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "expected".to_string(),
                    },
                },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        },
        source_span: None,
    }
}

fn caller_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "callPackageMutator".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type,
        self_type: None,
        slots: parameter_slots(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        site: test_instruction_site(),
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        },
        source_span: None,
    }
}

fn callee_executable(array_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "mutate".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type,
        self_type: None,
        slots: parameter_slots(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Assign {
                    target: AssignTargetIr::Index {
                        object: ExprRefIr { expression: 0 },
                        index: ExprRefIr { expression: 1 },
                    },
                    value: ExprRefIr { expression: 2 },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 3 }),
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(0),
                    },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "package-callee".to_string(),
                    },
                },
                ExprIr::LoadSlot { slot: 0 },
            ],
        },
        source_span: None,
    }
}

fn stream_callee_executable(array_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "mutate".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type,
        }],
        return_type: stream_string_type(),
        self_type: None,
        slots: parameter_slots(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Emit {
                operation: "mutate".to_string(),
                value: ExprRefIr { expression: 0 },
            }],
            expressions: vec![ExprIr::Literal {
                value: LiteralIr::String {
                    value: "real-package-stream".to_string(),
                },
            }],
        },
        source_span: None,
    }
}

fn parameter_slots() -> SlotLayout {
    SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "value".to_string(),
            kind: SlotKind::Param,
        }],
        frame_size: 1,
    }
}

fn callable_package(
    package_id: &str,
    file: &FileIrUnit,
    callable_id: PackageCallableId,
    array_type: TypeRefIr,
) -> PackageArtifact {
    let implementation = executable_export(file, 0);
    let target = implementation.operation_target_ref(
        callable_id.to_string(),
        OperationCallableKind::PublicFunction,
    );
    let contract = ordinary_array_contract();
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        direct_return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut package = private_package(package_id, file);
    package.package_local_abi.public_symbols.insert(
        "mutate".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: callable_id.clone(),
            signature: PackageCallableSignature {
                type_params: Vec::new(),
                parameters: vec![PackageCallableParameter {
                    name: "value".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: array_type.clone(),
                    },
                }],
                return_type: PackageTypeRef::Local {
                    local_type: array_type,
                },
                may_suspend: false,
            },
        },
    );
    package
        .implementation_links
        .functions
        .insert("mutate".to_string(), implementation);
    package.callable_links.insert(
        callable_id.clone(),
        PackageCallableLinkFact {
            callable_id: callable_id.clone(),
            target,
        },
    );
    package.callable_semantic_facts.insert(
        callable_id.clone(),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: effects.clone(),
            },
            provenance: provenance.clone(),
            resolved_call_targets: BTreeMap::new(),
        },
    );
    package.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Available {
            operation_contract: contract,
            implementation_requirements: BoundaryImplementationRequirements {
                config: Vec::new(),
                state: Vec::new(),
                native_capabilities: Vec::new(),
                runtime_capabilities: Vec::new(),
                complete_may_effects: effects,
                provenance,
            },
        },
    );
    package
}

fn stream_callable_package(
    package_id: &str,
    file: &FileIrUnit,
    callable_id: PackageCallableId,
    array_type: TypeRefIr,
) -> PackageArtifact {
    let implementation = executable_export(file, 0);
    let target = implementation.operation_target_ref(
        callable_id.to_string(),
        OperationCallableKind::PublicFunction,
    );
    let mut effects = no_effects();
    effects.may_suspend = true;
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        direct_return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let item_value_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Stream,
    };
    let operation_contract = BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "value".to_string(),
            ty: ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![ContractTypeRef::builtin("string")],
            },
            value_plan: detached_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: detached_plan(BoundaryValueOwner::Provider),
        },
        stream: BoundaryStreamContract::ServerStream {
            item_type: ContractTypeRef::builtin("string"),
            item_value_plan,
        },
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    };
    let mut package = private_package(package_id, file);
    package.package_local_abi.public_symbols.insert(
        "mutate".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: callable_id.clone(),
            signature: PackageCallableSignature {
                type_params: Vec::new(),
                parameters: vec![PackageCallableParameter {
                    name: "value".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: array_type,
                    },
                }],
                return_type: PackageTypeRef::Local {
                    local_type: stream_string_type(),
                },
                may_suspend: true,
            },
        },
    );
    package
        .implementation_links
        .functions
        .insert("mutate".to_string(), implementation);
    package.callable_links.insert(
        callable_id.clone(),
        PackageCallableLinkFact {
            callable_id: callable_id.clone(),
            target,
        },
    );
    package.callable_semantic_facts.insert(
        callable_id.clone(),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: effects.clone(),
            },
            provenance: provenance.clone(),
            resolved_call_targets: BTreeMap::new(),
        },
    );
    package.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Available {
            operation_contract,
            implementation_requirements: BoundaryImplementationRequirements {
                config: Vec::new(),
                state: Vec::new(),
                native_capabilities: Vec::new(),
                runtime_capabilities: Vec::new(),
                complete_may_effects: effects,
                provenance,
            },
        },
    );
    package
}

fn executable_export(file: &FileIrUnit, executable_index: u32) -> ExecutableExport {
    let executable = file
        .executables
        .get(executable_index as usize)
        .expect("implementation export executable");
    ExecutableExport {
        file: file_ref(file),
        executable_index,
        symbol: executable.symbol.clone(),
        signature: ExecutableSignatureIr {
            params: executable.params.clone(),
            return_type: executable.return_type.clone(),
            self_type: executable.self_type.clone(),
            may_suspend: executable.may_suspend,
        },
    }
}

fn private_package(package_id: &str, file: &FileIrUnit) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref(file)],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

fn ordinary_array_contract() -> BoundaryOperationContract {
    let array = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "value".to_string(),
            ty: array.clone(),
            value_plan: detached_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: array,
            value_plan: detached_plan(BoundaryValueOwner::Provider),
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
    }
}

fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

fn array_type() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    }
}

fn stream_string_type() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    }
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn activation_context(
    assembly_identity: AssemblyIdentity,
    package_build_id: PackageBuildId,
) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 1,
            runtime_replica_id: "replica:package-direct-test".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: "example.package-direct-caller".to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("package-direct-test-r1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    "deployment:package-direct-test",
                ),
            },
        },
        package_build_id,
        ActivationOwnedBindings {
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            policy: DeploymentPolicy {
                timeout_ms: Some(1_000),
                resources: ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_048_576,
                },
                activation: ActivationPolicy {
                    max_concurrency: 1,
                    idle_timeout_ms: None,
                },
                principal: "test".to_string(),
            },
        },
        Vec::new(),
    )
    .expect("test activation should build")
}

struct TestResolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for TestResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == activation_id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == activation_id)
            .then(|| Arc::clone(&self.activation))
    }

    fn contract(&self, _contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        None
    }

    fn admitted_schema_records(
        &self,
        _contract: &ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        None
    }
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
) -> ProgramExecutionContext<'a> {
    execution_context_with_actor(interpreter, target, test_runtime::actor_context())
}

fn execution_context_with_trace<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
    trace_id: &'static str,
) -> ProgramExecutionContext<'a> {
    execution_context_with_actor(
        interpreter,
        target,
        test_runtime::actor_context_with_trace(trace_id),
    )
}

fn execution_context_with_actor<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
    actor: skiff_runtime_capability_context::ActorCapabilityContext<'static>,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let runtime_activation = Arc::new(RuntimeActivation {
        service: skiff_runtime_linked_program::ServiceMeta {
            id: "example.package-direct-caller".to_string(),
            display_name: None,
            metadata: BTreeMap::new(),
        },
        version: "1.0.0".to_string(),
        package_configs: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        db: Vec::new(),
        actors: Vec::new(),
        gateway: Default::default(),
    });
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(
            interpreter.stream_runtime.clone(),
        ),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation,
        actor: actor.clone(),
        spawn: actor,
        outbound: test_runtime::outbound_context(),
        request_heap_limits: RequestHeapLimits::default(),
    })
    .with_websocket_capability_rebinder(test_runtime::websocket_rebinder())
    .with_runtime_assembly_target(target)
}

fn assert_array_item(heap: &RequestHeap, handle: HeapHandle, expected: &str) {
    let HeapNode::Array(items) = heap.get(handle).expect("array handle should resolve") else {
        panic!("heap value should remain an array")
    };
    assert_eq!(items, &[RuntimeValue::String(expected.to_string())]);
}
