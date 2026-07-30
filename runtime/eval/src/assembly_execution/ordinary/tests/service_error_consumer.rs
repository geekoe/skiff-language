use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::Value;
use skiff_artifact_model::*;
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
    ActivationServiceBinding, RequestActivationContext,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, HydratedPackageCode, PublicationResourceTable, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ExceptionStackFrame, NominalTypeIdentity, OpaqueServiceError,
        PlatformBuiltinErrorIdentity, ServiceErrorEnvelope,
    },
};

use super::{execution_context_with_trace, test_runtime};
use crate::{
    assembly_execution::{
        service_error_channel::{
            start_restricted_service_diagnostic_probe_for_test,
            take_restricted_service_diagnostics_for_test, CanonicalServiceErrorChannel,
            ServiceErrorExportContext, ServiceErrorImportContext,
        },
        start_in_process_boundary_failure_import_probe_for_test,
        take_in_process_boundary_failure_import_records_for_test,
        InProcessBoundaryFailureImportRecord,
    },
    error::{RuntimeError, UserException},
    exceptions::user_exception_for_catch,
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

pub(crate) const ERROR_TRACE_ID: &str = "trace:ordinary-service-error";
const SERVICE_ID: &str = "example.error-service";
const CONTRACT_VERSION: &str = "1.0.0";
const PROVIDER_PACKAGE: &str = "example.error-provider";
const CALLER_PACKAGE: &str = "example.error-caller";
const STD_PACKAGE: &str = "skiff.run/std";
const PROVIDER_FAULT: &str = "api.ProviderFault";
const CODE_FAULT: &str = "api.CodeFault";
const INTERNAL_ERROR: &str = "std.service.InternalError";
const FILE_ERROR: &str = "std.file.FileError";
const RESOURCE_ERROR: &str = "std.resource.ResourceError";
const STD_ALIAS: &str = "std";
const PROVIDER_ALIAS: &str = "providerErrors";

const RELAY_EXECUTABLE: usize = 0;
const PUBLIC_RECORD_EXECUTABLE: usize = 1;
const REPRESENTATION_EXECUTABLE: usize = 2;
const PRIVATE_EXECUTABLE: usize = 3;
const PLATFORM_FILE_EXECUTABLE: usize = 4;
const RESOURCE_EXECUTABLE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderFailureKind {
    PublicRecord,
    Representation,
    Private,
    PlatformFile,
    Resource,
}

impl ProviderFailureKind {
    fn executable(self) -> usize {
        match self {
            Self::PublicRecord => PUBLIC_RECORD_EXECUTABLE,
            Self::Representation => REPRESENTATION_EXECUTABLE,
            Self::Private => PRIVATE_EXECUTABLE,
            Self::PlatformFile => PLATFORM_FILE_EXECUTABLE,
            Self::Resource => RESOURCE_EXECUTABLE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsumerTopology {
    OneHop,
    ThreeHop,
}

pub(crate) struct ServiceErrorConsumerFixture {
    image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
    resolver: Arc<FixtureResolver>,
    contract: Arc<ServiceContract>,
    contract_ref: ServiceContractRef,
    operation_id: ContractOperationId,
    provider_target: OperationTargetRef,
    caller_activation: Arc<ActivationContext>,
    terminal_activation: Arc<ActivationContext>,
    caller_addr: ExecutableAddr,
    caller_build: PackageBuildId,
    caller_site: InstructionSourceSite,
    relay_site: InstructionSourceSite,
}

impl ServiceErrorConsumerFixture {
    pub(crate) fn new(
        kind: ProviderFailureKind,
        topology: ConsumerTopology,
        linked_caller: bool,
        catch_in_caller: bool,
    ) -> Self {
        Self::build(kind, topology, linked_caller, catch_in_caller)
    }

    fn build(
        kind: ProviderFailureKind,
        topology: ConsumerTopology,
        linked_caller: bool,
        catch_in_caller: bool,
    ) -> Self {
        let operation_id = ContractOperationId::new("operation:ordinary-errors");
        let contract = Arc::new(service_contract(operation_id.clone()));
        let contract_ref = service_contract_ref(&contract);
        let service_call = ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: operation_id.clone(),
            expected_protocol_identity: contract_ref.service_protocol_identity.clone(),
        };

        let std = std_package();
        let provider = provider_package(&std.reference, &contract_ref, &service_call);
        let caller = caller_package(
            &std.reference,
            &provider.reference,
            &contract_ref,
            &service_call,
            kind,
            linked_caller,
            catch_in_caller,
        );
        let assembly_identity = AssemblyIdentity::new("assembly:ordinary-service-error-consumer");
        let assembly = runtime_assembly(
            assembly_identity.clone(),
            &caller,
            &provider,
            &std,
            linked_caller,
        );
        let image = skiff_runtime_linker::link_package_fixture_from_runtime_assembly(
            &assembly,
            [caller.hydrated(), provider.hydrated(), std.hydrated()],
        )
        .expect("ordinary service-error execution image must link");

        let caller_site = caller_call_site();
        let relay_site = relay_call_site();
        let terminal_target = operation_target(
            &provider.files[0],
            kind.executable(),
            &format!("provider-terminal-{}", kind.executable()),
        );
        let relay_target = operation_target(&provider.files[0], RELAY_EXECUTABLE, "provider-relay");

        let terminal_identity =
            activation_identity(assembly_identity.clone(), "terminal", "terminal-r1");
        let terminal_activation = ActivationContext::new(
            terminal_identity,
            provider.reference.package_build_id.clone(),
            activation_owned_bindings(),
            Vec::new(),
        )
        .expect("terminal activation");
        let relay_identity = activation_identity(assembly_identity.clone(), "relay", "relay-r1");
        let relay_binding = ActivationServiceBinding::new(
            ServiceRequirementKey {
                caller_package_build_id: provider.reference.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            terminal_activation.activation_id().clone(),
            contract_ref.clone(),
            vec![operation_id.clone()],
        )
        .expect("relay-to-terminal service binding");
        let relay_activation = ActivationContext::new(
            relay_identity,
            provider.reference.package_build_id.clone(),
            activation_owned_bindings(),
            vec![relay_binding],
        )
        .expect("relay activation");

        let selected_provider = match topology {
            ConsumerTopology::OneHop => Arc::clone(&terminal_activation),
            ConsumerTopology::ThreeHop => Arc::clone(&relay_activation),
        };
        let caller_identity = activation_identity(assembly_identity, "caller", "caller-r1");
        let caller_binding = ActivationServiceBinding::new(
            ServiceRequirementKey {
                caller_package_build_id: caller.reference.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            selected_provider.activation_id().clone(),
            contract_ref.clone(),
            vec![operation_id.clone()],
        )
        .expect("caller-to-provider service binding");
        let caller_activation = ActivationContext::new(
            caller_identity,
            caller.reference.package_build_id.clone(),
            activation_owned_bindings(),
            vec![caller_binding],
        )
        .expect("caller activation");

        let activations = [
            Arc::clone(&caller_activation),
            Arc::clone(&relay_activation),
            Arc::clone(&terminal_activation),
        ]
        .into_iter()
        .map(|activation| (activation.activation_id().clone(), activation))
        .collect();
        let operation_targets = BTreeMap::from([
            (relay_activation.activation_id().clone(), relay_target),
            (
                terminal_activation.activation_id().clone(),
                terminal_target.clone(),
            ),
        ]);
        let resolver = Arc::new(FixtureResolver {
            activations,
            contract: Arc::clone(&contract),
            contract_ref: contract_ref.clone(),
            schema_records: Arc::new(BTreeMap::new()),
            operation_id: operation_id.clone(),
            operation_targets,
        });
        let caller_addr = ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            executable: 0,
        };

        Self {
            image,
            resolver,
            contract,
            contract_ref,
            operation_id,
            provider_target: terminal_target,
            caller_activation,
            terminal_activation,
            caller_addr,
            caller_build: caller.reference.package_build_id.clone(),
            caller_site,
            relay_site,
        }
    }

    pub(crate) fn caller_eval_target(&self) -> RuntimeAssemblyEvalTarget {
        self.eval_target(Arc::clone(&self.caller_activation))
    }

    pub(crate) fn terminal_eval_target(&self) -> RuntimeAssemblyEvalTarget {
        self.eval_target(Arc::clone(&self.terminal_activation))
    }

    pub(crate) fn execution_context<'a>(
        &self,
        interpreter: &Interpreter,
        target: RuntimeAssemblyEvalTarget,
    ) -> crate::program_execution::ProgramExecutionContext<'a> {
        execution_context_with_trace(interpreter, target, ERROR_TRACE_ID)
    }

    fn eval_target(&self, activation: Arc<ActivationContext>) -> RuntimeAssemblyEvalTarget {
        let request =
            RequestActivationContext::begin(activation).expect("fixture request must begin");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = self.resolver.clone();
        RuntimeAssemblyEvalTarget::new(Arc::clone(&self.image), request, resolver)
            .expect("fixture eval target")
    }

    pub(crate) async fn execute_internal(
        &self,
        interpreter: &Interpreter,
    ) -> (crate::error::Result<RuntimeValue>, RequestHeap, u64) {
        let target = self.caller_eval_target();
        let generation = target.request_activation().generation();
        let context = execution_context_with_trace(interpreter, target, ERROR_TRACE_ID);
        let mut heap = RequestHeap::default();
        let result = interpreter
            .execute_runtime_assembly_addr(context, &mut heap, &self.caller_addr, Vec::new())
            .await;
        (result, heap, generation)
    }

    pub(crate) fn ingress_target(
        &self,
        eval_target: &RuntimeAssemblyEvalTarget,
    ) -> crate::RuntimeAssemblyServiceCallTarget {
        eval_target
            .resolve_ingress_target(
                &self.contract_ref,
                &self.operation_id,
                Arc::clone(&self.contract),
                &self.provider_target,
            )
            .expect("terminal ingress target")
    }

    pub(crate) fn contract(&self) -> &Arc<ServiceContract> {
        &self.contract
    }

    pub(crate) fn operation_id(&self) -> &ContractOperationId {
        &self.operation_id
    }

    pub(crate) fn provider_target(&self) -> &OperationTargetRef {
        &self.provider_target
    }

    pub(crate) fn caller_build(&self) -> &PackageBuildId {
        &self.caller_build
    }

    pub(crate) fn caller_addr(&self) -> &ExecutableAddr {
        &self.caller_addr
    }

    pub(crate) fn caller_site(&self) -> &InstructionSourceSite {
        &self.caller_site
    }

    pub(crate) fn relay_site(&self) -> &InstructionSourceSite {
        &self.relay_site
    }
}

#[tokio::test]
async fn restricted_service_diagnostic_ordinary_exports_before_provider_heap_drop() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let target = fixture.caller_eval_target();
    let generation = target.request_activation().generation();
    start_restricted_service_diagnostic_probe_for_test(generation);
    let context = fixture.execution_context(&interpreter, target);
    let mut heap = RequestHeap::default();
    let result = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, fixture.caller_addr(), Vec::new())
        .await;
    let error = result.expect_err("provider throw must cross the ordinary boundary");
    let exception = user_exception(&error);
    let request = exception.request();
    let fixed = request
        .fixed_service_error()
        .expect("ordinary provider failure must already be fixed");
    assert!(matches!(
        fixed.envelope(),
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            trace_id,
            ..
        } if package_id == PROVIDER_PACKAGE
            && stable_schema_key == PROVIDER_FAULT
            && trace_id == ERROR_TRACE_ID
    ));
    let local = request
        .local_value()
        .expect("linked caller must receive a caller-local carrier");
    let RuntimeValue::Heap(handle) = local.value() else {
        panic!("public record import must allocate in the caller heap");
    };
    let HeapNode::Object(object) = heap
        .get(*handle)
        .expect("provider heap is gone but caller handle must remain valid")
    else {
        panic!("imported public error must be an object");
    };
    assert_eq!(
        object.fields().get("message"),
        Some(&RuntimeValue::String("provider denied".to_string()))
    );
    assert_eq!(request.source(), fixture.caller_site());
    assert_eq!(request.stack().len(), 2);
    assert_eq!(
        request.stack().first(),
        Some(&ExceptionStackFrame::Local {
            site: fixture.caller_site().clone(),
        })
    );
    assert!(matches!(
        request.stack().last(),
        Some(ExceptionStackFrame::RemoteBoundary {
            service_id,
            operation_id,
            error_id,
        }) if service_id == SERVICE_ID
            && operation_id == fixture.operation_id().as_str()
            && error_id == fixed.envelope().error_id()
    ));
    let diagnostics = take_restricted_service_diagnostics_for_test(generation);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].source, provider_throw_site());
    assert_eq!(
        diagnostics[0].stack,
        vec![ExceptionStackFrame::Local {
            site: provider_throw_site(),
        }]
    );
    assert_eq!(
        diagnostics[0].correlation.trace_id,
        fixed.envelope().trace_id()
    );
    assert_eq!(
        diagnostics[0].correlation.error_id,
        fixed.envelope().error_id()
    );
}

#[tokio::test]
async fn ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses() {
    let linked = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        true,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (result, heap, _) = linked.execute_internal(&interpreter).await;
    let caught = result.expect("exact linked public catch must succeed");
    let caught_carrier = caught_exception_carrier(&caught, &heap);
    let linked_caught_request = caught_request(&caught, &heap).clone();
    let rethrown = crate::exceptions::request_exception_for_rethrow(&caught_carrier, &heap)
        .expect("same-service rethrow must reuse the imported exception");
    assert_eq!(
        rethrown, linked_caught_request,
        "local rethrow must preserve source, stack, correlation, and raw bytes"
    );
    let linked_bytes = linked_caught_request
        .fixed_service_error()
        .expect("caught imported exception must retain raw fixed bytes")
        .encoded_bytes()
        .to_vec();
    assert!(linked_caught_request.local_value().is_some());

    let unlinked = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        false,
        true,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (result, _, _) = unlinked.execute_internal(&interpreter).await;
    let error = result.expect_err("unlinked caller catch must miss");
    let exception = user_exception(&error);
    assert!(
        exception.request().local_value().is_none(),
        "unlinked public identity must remain opaque"
    );
    assert_eq!(
        exception
            .request()
            .fixed_service_error()
            .expect("opaque import retains fixed bytes")
            .encoded_bytes(),
        linked_bytes,
        "linked materialization must not rewrite provider bytes"
    );

    let internal = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::Private,
        ConsumerTopology::OneHop,
        true,
        true,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (result, heap, _) = internal.execute_internal(&interpreter).await;
    let caught = result.expect("exact linked std.service.InternalError catch must succeed");
    let request = caught_request(&caught, &heap);
    assert!(matches!(
        request
            .fixed_service_error()
            .expect("caught InternalError keeps the fixed cause")
            .envelope(),
        ServiceErrorEnvelope::InternalError { .. }
    ));
    assert_internal_local_value(request, &heap);
}

#[tokio::test]
async fn ordinary_representation_private_platform_and_resource_share_one_channel() {
    for kind in [
        ProviderFailureKind::Representation,
        ProviderFailureKind::Private,
        ProviderFailureKind::PlatformFile,
        ProviderFailureKind::Resource,
    ] {
        let fixture = ServiceErrorConsumerFixture::new(kind, ConsumerTopology::OneHop, true, false);
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let (result, heap, _) = fixture.execute_internal(&interpreter).await;
        let error = result.expect_err("fixture provider must fail");
        let exception = user_exception(&error);
        let request = exception.request();
        let fixed = request
            .fixed_service_error()
            .expect("every ordinary provider failure must use the fixed carrier");
        match kind {
            ProviderFailureKind::Representation => {
                assert!(matches!(
                    fixed.envelope(),
                    ServiceErrorEnvelope::PublicTypedError {
                        package_id,
                        stable_schema_key,
                        ..
                    } if package_id == PROVIDER_PACKAGE && stable_schema_key == CODE_FAULT
                ));
                assert_eq!(
                    request.local_value().map(|value| value.value()),
                    Some(&RuntimeValue::String("E_DENIED".to_string()))
                );
            }
            ProviderFailureKind::Private => {
                assert!(matches!(
                    fixed.envelope(),
                    ServiceErrorEnvelope::InternalError { payload }
                        if payload.message == "Internal service error"
                ));
                let bytes = String::from_utf8_lossy(fixed.encoded_bytes());
                assert!(!bytes.contains("provider-private-secret"));
                assert!(!bytes.contains("PrivateFault"));
                assert_internal_local_value(request, &heap);
            }
            ProviderFailureKind::PlatformFile => {
                assert!(matches!(
                    fixed.envelope(),
                    ServiceErrorEnvelope::PlatformError {
                        builtin_error_identity: PlatformBuiltinErrorIdentity::File,
                        ..
                    }
                ));
                assert_eq!(
                    request.local_catch_identity(),
                    Some(&PlatformBuiltinErrorIdentity::File.catch_identity())
                );
            }
            ProviderFailureKind::Resource => {
                assert!(matches!(
                    fixed.envelope(),
                    ServiceErrorEnvelope::PublicTypedError {
                        package_id,
                        stable_schema_key,
                        ..
                    } if package_id == STD_PACKAGE && stable_schema_key == RESOURCE_ERROR
                ));
                assert!(matches!(
                    request.local_catch_identity(),
                    Some(CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
                        _
                    )))
                ));
                assert_resource_local_value(request, &heap);
            }
            ProviderFailureKind::PublicRecord => unreachable!(),
        }
    }
}

#[tokio::test]
async fn native_resource_failure_exports_public_std_type_and_caller_catches_it() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::Resource,
        ConsumerTopology::OneHop,
        true,
        true,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (result, heap, _) = fixture.execute_internal(&interpreter).await;
    let caught = result.expect("caller must catch the exact public std ResourceError");
    let request = caught_request(&caught, &heap);
    assert!(matches!(
        request
            .fixed_service_error()
            .expect("caught service ResourceError retains fixed bytes")
            .envelope(),
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            ..
        } if package_id == STD_PACKAGE && stable_schema_key == RESOURCE_ERROR
    ));
    assert_resource_local_value(request, &heap);
}

fn assert_resource_local_value(
    request: &skiff_runtime_model::service_error::RequestException,
    heap: &RequestHeap,
) {
    let local = request
        .local_value()
        .expect("linked caller must materialize ResourceError locally");
    let RuntimeValue::Heap(handle) = local.value() else {
        panic!("ResourceError local value must be a record");
    };
    let HeapNode::Object(object) = heap.get(*handle).expect("ResourceError heap record") else {
        panic!("ResourceError local value must be an object");
    };
    assert_eq!(
        object.fields().get("path"),
        Some(&RuntimeValue::String("missing-resource.txt".to_string()))
    );
    assert!(matches!(
        object.fields().get("message"),
        Some(RuntimeValue::String(message)) if message.contains("is not declared")
    ));
}

#[test]
fn restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks() {
    std::thread::Builder::new()
        .name("ordinary-three-hop".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("three-hop test runtime")
                .block_on(async {
                    for kind in [
                        ProviderFailureKind::PublicRecord,
                        ProviderFailureKind::Private,
                    ] {
                        let fixture = ServiceErrorConsumerFixture::new(
                            kind,
                            ConsumerTopology::ThreeHop,
                            true,
                            false,
                        );
                        let interpreter =
                            Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
                        let target = fixture.caller_eval_target();
                        let generation = target.request_activation().generation();
                        start_in_process_boundary_failure_import_probe_for_test(generation);
                        start_restricted_service_diagnostic_probe_for_test(generation);
                        let context =
                            execution_context_with_trace(&interpreter, target, ERROR_TRACE_ID);
                        let mut heap = RequestHeap::default();
                        let result = interpreter
                            .execute_runtime_assembly_addr(
                                context,
                                &mut heap,
                                fixture.caller_addr(),
                                Vec::new(),
                            )
                            .await;
                        let error = result
                            .expect_err("terminal failure must cross both service boundaries");
                        let final_exception = user_exception(&error);
                        let records =
                            take_in_process_boundary_failure_import_records_for_test(generation);
                        let provider_records =
                            take_restricted_service_diagnostics_for_test(generation);
                        assert_eq!(
                            provider_records.len(),
                            2,
                            "terminal C and relay B must each reach provider export once"
                        );
                        assert_ne!(
                            provider_records[0].owner.provider_activation_id,
                            provider_records[1].owner.provider_activation_id,
                            "B and C provider failures must belong to distinct activations"
                        );
                        assert_eq!(
                            provider_records[0].source,
                            provider_throw_site()
                        );
                        assert_eq!(
                            provider_records[0].stack,
                            vec![ExceptionStackFrame::Local {
                                site: provider_throw_site(),
                            }],
                            "C provider scope must not inherit A or B frames"
                        );
                        assert_eq!(provider_records[1].source, fixture.relay_site().clone());
                        assert_eq!(
                            provider_records[1].stack.len(),
                            2,
                            "B keeps only its local call site and C remote boundary"
                        );
                        assert_eq!(records.len(), 2, "B and A must each import exactly once");
                        let original =
                            OpaqueServiceError::decode(records[0].encoded_error.clone())
                                .expect("recorded fixed bytes remain strict");
                        for diagnostic in &provider_records {
                            assert_eq!(
                                diagnostic.correlation.trace_id,
                                original.envelope().trace_id()
                            );
                            assert_eq!(
                                diagnostic.correlation.error_id,
                                original.envelope().error_id()
                            );
                            assert!(
                                !format!("{diagnostic:?}").contains("provider-private-secret"),
                                "restricted safe fields must not contain the provider payload"
                            );
                        }
                        assert_ne!(
                            records[0].caller_activation_id,
                            records[1].caller_activation_id,
                            "each hop must import in its own activation"
                        );
                        assert_hop_record(&records[0], fixture.relay_site());
                        assert_hop_record(&records[1], fixture.caller_site());
                        assert_eq!(records[0].encoded_error, records[1].encoded_error);
                        assert_eq!(
                            final_exception
                                .request()
                                .fixed_service_error()
                                .expect("final A exception keeps the original carrier")
                                .encoded_bytes(),
                            records[0].encoded_error
                        );
                        let [ExceptionStackFrame::Local { .. }, ExceptionStackFrame::RemoteBoundary { .. }] =
                            records[0].stack.as_slice()
                        else {
                            panic!("B import stack must contain only its local call site and one remote frame");
                        };
                        let [ExceptionStackFrame::Local { .. }, ExceptionStackFrame::RemoteBoundary { .. }] =
                            records[1].stack.as_slice()
                        else {
                            panic!("A import stack must contain only its local call site and one remote frame");
                        };
                    }
                });
        })
        .expect("spawn three-hop test")
        .join()
        .expect("three-hop test thread");
}

#[test]
fn known_public_identity_owner_key_and_id_mutations_fail_closed() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let target = fixture.caller_eval_target();
    let projection = target.execution_projection();
    let mut heap = RequestHeap::default();
    let source = fixture.caller_site().clone();
    let stack = [ExceptionStackFrame::Local {
        site: source.clone(),
    }];
    let valid = fixed_public_error_for_mutation(&fixture);

    for (field, value) in [
        ("packageId", Value::String(PROVIDER_PACKAGE.to_string())),
        (
            "stableSchemaKey",
            Value::String("api.WrongProviderFault".to_string()),
        ),
        (
            "packageSchemaTypeId",
            Value::String("schema:wrong-provider-fault".to_string()),
        ),
    ] {
        let mut wire: Value =
            serde_json::from_slice(valid.encoded_bytes()).expect("fixed envelope JSON");
        if field == "packageId" {
            wire[field] = Value::String(CALLER_PACKAGE.to_string());
        } else {
            wire[field] = value;
        }
        let mutated = OpaqueServiceError::decode(
            skiff_canonical_json::canonical_json_bytes(&wire).expect("canonical mutation"),
        )
        .expect("mutation remains syntactically valid");
        let error = CanonicalServiceErrorChannel::import_caller_failure(
            mutated,
            ServiceErrorImportContext {
                execution_image: target.execution_image().as_ref(),
                type_view: projection.type_view(),
                caller_heap: &mut heap,
                caller_package_build_id: fixture.caller_build(),
                caller_executable_addr: fixture.caller_addr(),
                call_site: &source,
                caller_stack_at_site: &stack,
                remote_service_id: SERVICE_ID,
                remote_operation_id: fixture.operation_id().as_str(),
            },
        )
        .expect_err("known admitted identity mutation must fail closed");
        assert!(matches!(error, RuntimeError::Protocol { .. }));
    }
}

fn fixed_public_error_for_mutation(fixture: &ServiceErrorConsumerFixture) -> OpaqueServiceError {
    let target = fixture.terminal_eval_target();
    let projection = target.execution_projection();
    let provider_addr = ExecutableAddr {
        unit: UnitAddr::Package(1),
        file: FileAddr::LoadedFileIndex(0),
        executable: PUBLIC_RECORD_EXECUTABLE,
    };
    let provider_type = skiff_runtime_linked_program::TypeAddr {
        unit: UnitAddr::Package(1),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_object_carriers(BTreeMap::from([(
            "message".to_string(),
            skiff_runtime_model::runtime_value::RuntimeValueCarrier::unidentified(
                RuntimeValue::String("provider denied".to_string()),
            ),
        )]))
        .expect("mutation fixture payload");
    let site = provider_throw_site();
    let request = skiff_runtime_model::service_error::RequestException::local(
        skiff_runtime_model::runtime_value::RuntimeValueCarrier::identified(
            RuntimeValue::Heap(handle),
            CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
                skiff_runtime_model::service_error::LocalExecutionTypeIdentity {
                    addr: provider_type,
                    type_arguments: Vec::new(),
                },
            )),
        ),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        skiff_runtime_model::service_error::ErrorCorrelation {
            trace_id: ERROR_TRACE_ID.to_string(),
            error_id: format!("{ERROR_TRACE_ID}:local-error:1"),
        },
    )
    .expect("mutation fixture local error");
    CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::UserException(UserException::new(request)),
        ServiceErrorExportContext {
            execution_image: target.execution_image().as_ref(),
            type_view: projection.type_view(),
            provider_heap: &heap,
            provider_package_build_id: fixture
                .terminal_activation
                .implementation_package_build_id(),
            caller_package_build_id: Some(fixture.caller_build()),
            provider_service_id: SERVICE_ID,
            operation_id: fixture.operation_id().as_str(),
        },
        || panic!("local cause already has correlation"),
    )
    .unwrap_or_else(|error| panic!("mutation fixture export at {provider_addr} failed: {error}"))
}

fn assert_hop_record(record: &InProcessBoundaryFailureImportRecord, site: &InstructionSourceSite) {
    assert_eq!(&record.source, site);
    assert_eq!(
        record.stack.first(),
        Some(&ExceptionStackFrame::Local { site: site.clone() })
    );
    assert!(matches!(
        record.stack.last(),
        Some(ExceptionStackFrame::RemoteBoundary {
            service_id,
            operation_id,
            ..
        }) if service_id == SERVICE_ID && operation_id == "operation:ordinary-errors"
    ));
}

fn assert_internal_local_value(
    request: &skiff_runtime_model::service_error::RequestException,
    heap: &RequestHeap,
) {
    let RuntimeValue::Heap(handle) = request
        .local_value()
        .expect("InternalError must be catchable")
        .value()
    else {
        panic!("InternalError must materialize as a nominal record");
    };
    let HeapNode::Object(object) = heap.get(*handle).expect("caller InternalError object") else {
        panic!("caller InternalError must be an object");
    };
    assert_eq!(
        object.fields().get("message"),
        Some(&RuntimeValue::String("Internal service error".to_string()))
    );
}

fn caught_request<'a>(
    value: &RuntimeValue,
    heap: &'a RequestHeap,
) -> &'a skiff_runtime_model::service_error::RequestException {
    let RuntimeValue::Heap(result) = value else {
        panic!("catch result must be an object");
    };
    let HeapNode::Object(result) = heap.get(*result).expect("catch result") else {
        panic!("catch result must be an object");
    };
    assert_eq!(
        result.fields().get("tag"),
        Some(&RuntimeValue::String("err".to_string()))
    );
    let RuntimeValue::Heap(exception) = result
        .fields()
        .get("exception")
        .expect("catch result exception")
    else {
        panic!("catch exception must be a heap node");
    };
    let HeapNode::Exception(exception) = heap.get(*exception).expect("caught exception") else {
        panic!("caught error must retain RequestException");
    };
    exception
}

fn caught_exception_carrier(value: &RuntimeValue, heap: &RequestHeap) -> RuntimeValueCarrier {
    let RuntimeValue::Heap(result) = value else {
        panic!("catch result must be an object");
    };
    let HeapNode::Object(result) = heap.get(*result).expect("catch result") else {
        panic!("catch result must be an object");
    };
    RuntimeValueCarrier::unidentified(
        result
            .fields()
            .get("exception")
            .expect("catch result exception")
            .clone(),
    )
}

fn user_exception(error: &RuntimeError) -> &UserException {
    user_exception_for_catch(error).expect("error must contain a caller-local UserException")
}

#[derive(Clone)]
struct FixtureResolver {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    contract: Arc<ServiceContract>,
    contract_ref: ServiceContractRef,
    schema_records: crate::AdmittedPackageSchemaRecords,
    operation_id: ContractOperationId,
    operation_targets: BTreeMap<ActivationId, OperationTargetRef>,
}

impl RuntimeAssemblyEvalResolver for FixtureResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        self.activations.get(activation_id).cloned()
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        self.activations
            .values()
            .find(|activation| activation.activation_id().as_str() == activation_id)
            .cloned()
    }

    fn contract(&self, contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        (contract == &self.contract_ref).then(|| Arc::clone(&self.contract))
    }

    fn admitted_schema_records(
        &self,
        contract: &ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        (contract == &self.contract_ref).then(|| Arc::clone(&self.schema_records))
    }

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        (operation == &self.operation_id)
            .then(|| self.operation_targets.get(activation_id).cloned())
            .flatten()
    }
}

struct PackageFixture {
    reference: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    files: Vec<Arc<FileIrUnit>>,
    schema_index: Arc<PackageSchemaIndex>,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl PackageFixture {
    fn hydrated(&self) -> HydratedPackageCode {
        HydratedPackageCode::new(
            Arc::clone(&self.artifact),
            self.files.clone(),
            PublicationResourceTable::default(),
        )
        .with_schema_index(Arc::clone(&self.schema_index))
        .with_schema_records(self.schema_records.clone())
    }
}

fn service_contract(operation_id: ContractOperationId) -> ServiceContract {
    ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("protocol:ordinary-errors"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: "throw".to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("string"),
                        value_plan: detached_value_plan(BoundaryValueOwner::Provider),
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
            },
        )]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "ordinary error fixture".to_string(),
            operations: BTreeMap::from([(operation_id, "throw".to_string())]),
            types: BTreeMap::new(),
        },
    }
}

fn provider_package(
    std_ref: &PackageArtifactRef,
    contract: &ServiceContractRef,
    service_call: &ServiceCallRef,
) -> PackageFixture {
    let std_file = package_symbol(std_ref, "std.file.FileError");
    let mut file = FileIrUnit::empty("provider.main", "source:provider-private-path");
    file.type_table = vec![
        TypeDeclIr {
            name: PROVIDER_FAULT.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: CODE_FAULT.to_string(),
            descriptor: TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("string"),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "PrivateFault".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("secret".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ];
    file.external_refs
        .service_call_refs
        .push(service_call.clone());
    file.external_refs.package_symbols = vec![std_file.clone()];
    file.executables = vec![
        service_call_executable("relay", relay_call_site(), None),
        record_throw_executable(
            "throwPublic",
            TypeRefIr::LocalType { type_index: 0 },
            "message",
            "provider denied",
        ),
        representation_throw_executable(),
        record_throw_executable(
            "throwPrivate",
            TypeRefIr::LocalType { type_index: 2 },
            "secret",
            "provider-private-secret",
        ),
        record_throw_executable(
            "throwFile",
            TypeRefIr::PackageSymbol { symbol: std_file },
            "message",
            "provider file failed",
        ),
        native_resource_error_executable(),
    ];
    skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("provider file identity");
    let file_ref = file_reference(&file);
    let build = PackageBuildId::new("build:ordinary-error-provider");
    let abi = PackageLocalAbiIdentity::new("abi:ordinary-error-provider");
    let public_types = [
        (
            PROVIDER_FAULT,
            0_u32,
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "message".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        ),
        (
            CODE_FAULT,
            1_u32,
            ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin("string"),
            },
        ),
    ];
    let (schema_index, schema_records, schema_refs) = schema_facts(PROVIDER_PACKAGE, &public_types);
    let implementation_types = public_types
        .iter()
        .map(|(name, index, _)| {
            (
                (*name).to_string(),
                TypeExport {
                    file: file_ref.clone(),
                    type_index: *index,
                    symbol: (*name).to_string(),
                    is_interface: false,
                    descriptor: Some(file.type_table[*index as usize].descriptor.clone()),
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                },
            )
        })
        .collect();
    let contract_requirement = contract_requirement(contract);
    let artifact = Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: PROVIDER_PACKAGE.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build.clone(),
        files: vec![file_ref],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: abi.clone(),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: PROVIDER_PACKAGE.to_string(),
            package_schema_index_identity: schema_index.package_schema_index_identity.clone(),
        },
        package_schema_type_records: schema_refs,
        implementation_links: PackageImplementationLinks {
            types: implementation_types,
            ..PackageImplementationLinks::default()
        },
        callable_links: (0..file.executables.len())
            .map(|executable| {
                let abi = if executable == RELAY_EXECUTABLE {
                    "provider-relay".to_string()
                } else {
                    format!("provider-terminal-{executable}")
                };
                let callable_id = PackageCallableId::new(abi.clone());
                (
                    callable_id.clone(),
                    PackageCallableLinkFact {
                        callable_id,
                        target: operation_target(&file, executable, &abi),
                    },
                )
            })
            .collect(),
        package_requirements: vec![package_requirement(STD_ALIAS, std_ref)],
        contract_requirements: vec![contract_requirement.clone()],
        service_requirements: vec![ServiceRequirement {
            contract_requirement,
            service_binding_slot: 0,
            used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
        }],
        runtime_requirements: empty_runtime_requirements(),
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: vec![service_call.clone()],
    });
    PackageFixture {
        reference: PackageArtifactRef {
            package_id: PROVIDER_PACKAGE.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: build,
            package_local_abi_identity: abi,
        },
        artifact,
        files: vec![Arc::new(file)],
        schema_index: Arc::new(schema_index),
        schema_records,
    }
}

fn caller_package(
    std_ref: &PackageArtifactRef,
    provider_ref: &PackageArtifactRef,
    contract: &ServiceContractRef,
    service_call: &ServiceCallRef,
    failure_kind: ProviderFailureKind,
    linked_provider: bool,
    catch_in_caller: bool,
) -> PackageFixture {
    let provider_fault = package_symbol(provider_ref, PROVIDER_FAULT);
    let mut file = FileIrUnit::empty("caller.main", "source:caller");
    file.type_table.push(TypeDeclIr {
        name: "UnrelatedCatch".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.external_refs
        .service_call_refs
        .push(service_call.clone());
    let catch_type = match failure_kind {
        ProviderFailureKind::Private => {
            let internal_error = package_symbol(std_ref, INTERNAL_ERROR);
            file.external_refs
                .package_symbols
                .push(internal_error.clone());
            TypeRefIr::PackageSymbol {
                symbol: internal_error,
            }
        }
        ProviderFailureKind::Resource => {
            let resource_error = package_symbol(std_ref, RESOURCE_ERROR);
            file.external_refs
                .package_symbols
                .push(resource_error.clone());
            TypeRefIr::PackageSymbol {
                symbol: resource_error,
            }
        }
        _ if linked_provider => {
            file.external_refs
                .package_symbols
                .push(provider_fault.clone());
            TypeRefIr::PackageSymbol {
                symbol: provider_fault,
            }
        }
        _ => TypeRefIr::LocalType { type_index: 0 },
    };
    file.executables.push(service_call_executable(
        "caller",
        caller_call_site(),
        catch_in_caller.then_some(catch_type),
    ));
    skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("caller file identity");
    let build = PackageBuildId::new(if linked_provider {
        "build:ordinary-error-caller-linked"
    } else {
        "build:ordinary-error-caller-unlinked"
    });
    let abi = PackageLocalAbiIdentity::new(if linked_provider {
        "abi:ordinary-error-caller-linked"
    } else {
        "abi:ordinary-error-caller-unlinked"
    });
    let empty_types = BTreeMap::new();
    let schema_identity =
        skiff_artifact_identity::package_schema_index_identity(CALLER_PACKAGE, &empty_types)
            .expect("empty caller schema identity");
    let contract_requirement = contract_requirement(contract);
    let mut package_requirements = vec![package_requirement(STD_ALIAS, std_ref)];
    if linked_provider {
        package_requirements.push(package_requirement(PROVIDER_ALIAS, provider_ref));
    }
    let artifact = Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: CALLER_PACKAGE.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build.clone(),
        files: vec![file_reference(&file)],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: abi.clone(),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: CALLER_PACKAGE.to_string(),
            package_schema_index_identity: schema_identity.clone(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements,
        contract_requirements: vec![contract_requirement.clone()],
        service_requirements: vec![ServiceRequirement {
            contract_requirement,
            service_binding_slot: 0,
            used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
        }],
        runtime_requirements: empty_runtime_requirements(),
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: vec![service_call.clone()],
    });
    PackageFixture {
        reference: PackageArtifactRef {
            package_id: CALLER_PACKAGE.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: build,
            package_local_abi_identity: abi,
        },
        artifact,
        files: vec![Arc::new(file)],
        schema_index: Arc::new(PackageSchemaIndex {
            package_id: CALLER_PACKAGE.to_string(),
            package_schema_index_identity: schema_identity,
            types: BTreeMap::new(),
        }),
        schema_records: BTreeMap::new(),
    }
}

fn std_package() -> PackageFixture {
    let specs = [
        (
            "std.service",
            "InternalError",
            INTERNAL_ERROR,
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("errorId".to_string(), TypeRefIr::builtin("string")),
                    ("message".to_string(), TypeRefIr::builtin("string")),
                    ("traceId".to_string(), TypeRefIr::builtin("string")),
                ]),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("errorId".to_string(), ContractTypeRef::builtin("string")),
                    ("message".to_string(), ContractTypeRef::builtin("string")),
                    ("traceId".to_string(), ContractTypeRef::builtin("string")),
                ]),
            },
        ),
        (
            "std.file",
            "FileError",
            FILE_ERROR,
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "message".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        ),
        (
            "std.resource",
            "ResourceError",
            RESOURCE_ERROR,
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("message".to_string(), TypeRefIr::builtin("string")),
                    ("path".to_string(), TypeRefIr::builtin("string")),
                ]),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("message".to_string(), ContractTypeRef::builtin("string")),
                    ("path".to_string(), ContractTypeRef::builtin("string")),
                ]),
            },
        ),
    ];
    let mut files = Vec::new();
    for (module, name, _, descriptor, _) in &specs {
        let mut file = FileIrUnit::empty(*module, format!("source:{module}"));
        file.type_table.push(TypeDeclIr {
            name: (*name).to_string(),
            descriptor: descriptor.clone(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("std file identity");
        files.push(file);
    }
    let public_types = specs
        .iter()
        .enumerate()
        .map(|(file_index, (_, _, stable_key, _, schema))| {
            (*stable_key, file_index as u32, schema.clone())
        })
        .collect::<Vec<_>>();
    let schema_inputs = public_types
        .iter()
        .map(|(name, index, schema)| (*name, *index, schema.clone()))
        .collect::<Vec<_>>();
    let (schema_index, schema_records, schema_refs) = schema_facts(STD_PACKAGE, &schema_inputs);
    let implementation_types = specs
        .iter()
        .enumerate()
        .map(|(file_index, (_, _, stable_key, descriptor, _))| {
            (
                (*stable_key).to_string(),
                TypeExport {
                    file: file_reference(&files[file_index]),
                    type_index: 0,
                    symbol: (*stable_key).to_string(),
                    is_interface: false,
                    descriptor: Some(descriptor.clone()),
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let public_symbols = specs
        .iter()
        .map(|(_, _, stable_key, descriptor, _)| {
            (
                (*stable_key).to_string(),
                PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{stable_key}"),
                    descriptor: descriptor.clone(),
                    is_alias: false,
                    is_interface: false,
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                },
            )
        })
        .collect();
    let build = PackageBuildId::new("build:ordinary-error-std");
    let abi = PackageLocalAbiIdentity::new("abi:ordinary-error-std");
    let artifact = Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: STD_PACKAGE.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build.clone(),
        files: files.iter().map(file_reference).collect(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: abi.clone(),
            public_symbols,
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: STD_PACKAGE.to_string(),
            package_schema_index_identity: schema_index.package_schema_index_identity.clone(),
        },
        package_schema_type_records: schema_refs,
        implementation_links: PackageImplementationLinks {
            types: implementation_types,
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: empty_runtime_requirements(),
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    });
    PackageFixture {
        reference: PackageArtifactRef {
            package_id: STD_PACKAGE.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: build,
            package_local_abi_identity: abi,
        },
        artifact,
        files: files.into_iter().map(Arc::new).collect(),
        schema_index: Arc::new(schema_index),
        schema_records,
    }
}

fn schema_facts(
    package_id: &str,
    public_types: &[(&str, u32, ContractTypeDescriptor)],
) -> (
    PackageSchemaIndex,
    BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
) {
    let mut index_types = BTreeMap::new();
    let mut records = BTreeMap::new();
    let mut refs = BTreeMap::new();
    for (stable_key, _, descriptor) in public_types {
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: descriptor.clone(),
        };
        let type_id = skiff_artifact_identity::package_schema_type_id(
            package_id,
            stable_key,
            &canonical_descriptor,
        )
        .expect("fixture Package schema type id");
        index_types.insert(
            (*stable_key).to_string(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id.clone(),
                public_path: Some((*stable_key).to_string()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        );
        records.insert(
            type_id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: package_id.to_string(),
                stable_schema_key: (*stable_key).to_string(),
                package_schema_type_id: type_id.clone(),
                canonical_descriptor,
            }),
        );
        refs.insert(
            type_id.clone(),
            PackageSchemaTypeRecordRef {
                package_id: package_id.to_string(),
                package_schema_type_id: type_id,
            },
        );
    }
    let identity = skiff_artifact_identity::package_schema_index_identity(package_id, &index_types)
        .expect("fixture Package schema index identity");
    (
        PackageSchemaIndex {
            package_id: package_id.to_string(),
            package_schema_index_identity: identity,
            types: index_types,
        },
        records,
        refs,
    )
}

fn runtime_assembly(
    identity: AssemblyIdentity,
    caller: &PackageFixture,
    provider: &PackageFixture,
    std: &PackageFixture,
    linked_provider: bool,
) -> RuntimeAssembly {
    let mut package_links = vec![
        PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: caller.reference.package_build_id.clone(),
                package_requirement_alias: STD_ALIAS.to_string(),
            },
            package: std.reference.clone(),
            collection_name_mapping: BTreeMap::new(),
        },
        PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: provider.reference.package_build_id.clone(),
                package_requirement_alias: STD_ALIAS.to_string(),
            },
            package: std.reference.clone(),
            collection_name_mapping: BTreeMap::new(),
        },
    ];
    if linked_provider {
        package_links.push(PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: caller.reference.package_build_id.clone(),
                package_requirement_alias: PROVIDER_ALIAS.to_string(),
            },
            package: provider.reference.clone(),
            collection_name_mapping: BTreeMap::new(),
        });
    }
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: identity,
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![
            caller.reference.clone(),
            provider.reference.clone(),
            std.reference.clone(),
        ],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: caller.reference.clone(),
                },
                PackageCodeSlot {
                    package: provider.reference.clone(),
                },
                PackageCodeSlot {
                    package: std.reference.clone(),
                },
            ],
            package_links,
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}

fn service_call_executable(
    symbol: &str,
    site: InstructionSourceSite,
    catch_type: Option<TypeRefIr>,
) -> ExecutableIr {
    let mut expressions = vec![ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: ServiceCallRefIndex::new(0),
            },
            site,
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    }];
    let (return_type, slots, returned) = if let Some(catch_type) = catch_type {
        expressions.push(ExprIr::LoadSlot { slot: 0 });
        expressions.push(ExprIr::Catch {
            try_expression: ExprRefIr { expression: 0 },
            catch_slot: 0,
            catch_type,
            body: ExprRefIr { expression: 1 },
        });
        (
            TypeRefIr::builtin("Json"),
            SlotLayout {
                slots: vec![SlotIr {
                    index: 0,
                    name: "$caught".to_string(),
                    kind: SlotKind::Temp,
                }],
                frame_size: 1,
            },
            2,
        )
    } else {
        (TypeRefIr::builtin("string"), SlotLayout::default(), 0)
    };
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type,
        self_type: None,
        slots,
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr {
                    expression: returned,
                }),
            }],
            expressions,
        },
        source_span: None,
    }
}

fn record_throw_executable(
    symbol: &str,
    payload_type: TypeRefIr,
    field: &str,
    value: &str,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
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
                value: Some(ExprRefIr { expression: 2 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: value.to_string(),
                    },
                },
                ExprIr::Construct {
                    type_ref: payload_type.clone(),
                    fields: BTreeMap::from([(field.to_string(), ExprRefIr { expression: 0 })]),
                },
                ExprIr::Throw {
                    value: ExprRefIr { expression: 1 },
                    payload_type,
                    site: provider_throw_site(),
                },
            ],
        },
        source_span: None,
    }
}

fn native_resource_error_executable() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "throwResource".to_string(),
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
                value: Some(ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "missing-resource.txt".to_string(),
                    },
                },
                ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::Native {
                            target: NativeTarget {
                                namespace: "std.resource".to_string(),
                                symbol: "text".to_string(),
                                binding_key: Some("std.resource.text".to_string()),
                                metadata: BTreeMap::new(),
                            },
                        },
                        site: provider_throw_site(),
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

fn representation_throw_executable() -> ExecutableIr {
    let payload_type = TypeRefIr::LocalType { type_index: 1 };
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "throwRepresentation".to_string(),
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
                value: Some(ExprRefIr { expression: 2 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "E_DENIED".to_string(),
                    },
                },
                ExprIr::RepresentationWrap {
                    value: ExprRefIr { expression: 0 },
                    type_ref: payload_type.clone(),
                },
                ExprIr::Throw {
                    value: ExprRefIr { expression: 1 },
                    payload_type,
                    site: provider_throw_site(),
                },
            ],
        },
        source_span: None,
    }
}

fn operation_target(file: &FileIrUnit, executable: usize, abi: &str) -> OperationTargetRef {
    OperationTargetRef {
        file_ref: file_reference(file),
        executable_index: u32::try_from(executable).expect("fixture executable index"),
        callable_abi_id: abi.to_string(),
        callable_kind: OperationCallableKind::PublicFunction,
    }
}

fn activation_identity(
    assembly_identity: AssemblyIdentity,
    label: &str,
    revision: &str,
) -> ActivationIdentity {
    ActivationIdentity {
        assembly_identity,
        assembly_generation: 1,
        runtime_replica_id: "replica:ordinary-error".to_string(),
        deployment: ServiceDeploymentRef {
            service_id: SERVICE_ID.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
            deployment_revision: DeploymentRevision::new(revision),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "deployment:ordinary-error:{label}"
            )),
        },
    }
}

fn activation_owned_bindings() -> ActivationOwnedBindings {
    ActivationOwnedBindings {
        resource_bindings: Vec::new(),
    }
}

fn package_symbol(package: &PackageArtifactRef, symbol_path: &str) -> PackageSymbolRef {
    PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: if package.package_id == STD_PACKAGE {
                STD_ALIAS.to_string()
            } else {
                PROVIDER_ALIAS.to_string()
            },
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: Some(package.package_local_abi_identity.to_string()),
    }
}

fn package_requirement(alias: &str, package: &PackageArtifactRef) -> PackageRequirement {
    PackageRequirement {
        alias: alias.to_string(),
        package_id: package.package_id.clone(),
        exact_version: package.package_version.clone(),
        expected_local_abi: package.package_local_abi_identity.clone(),
        collection_name_mapping: BTreeMap::new(),
        expected_package_build: Some(package.package_build_id.clone()),
    }
}

fn contract_requirement(contract: &ServiceContractRef) -> ContractRequirement {
    ContractRequirement {
        alias: "errors".to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn service_contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn file_reference(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn empty_runtime_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        config: Vec::new(),
        resources: Vec::new(),
        runtime_capabilities: Vec::new(),
    }
}

fn detached_value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn caller_call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn relay_call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
    }
}

fn provider_throw_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}
