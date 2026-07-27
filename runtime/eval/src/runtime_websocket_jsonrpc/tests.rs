use super::*;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use skiff_artifact_model::{
    GatewayEntryIdentity, OperationTargetRef, PackageArtifact, PackageCallableId,
    PackageCallableSignature, PackageLocalAbiSymbol, PackageSchemaIndex, RuntimeAssembly,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_runtime_activation::{
    ActivationContext, ActivationId, RequestActivationContext, RuntimeActivation,
};
use skiff_runtime_capability_context::{CancellationSource, DbCapabilityContext};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, HydratedPackageCode, PublicationResourceTable,
};
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use skiff_test_runner::canonical_std_seed::seed_canonical_std;

use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    AdmittedPackageSchemaRecords, RuntimeAssemblyEvalResolver,
};

const PACKAGE_ID: &str = "example.com/runtime-websocket-jsonrpc-execution";
const SERVICE_ID: &str = "example.com/runtime-websocket-jsonrpc-execution-service";
const VERSION: &str = "1.0.0";
const PRIVATE_SENTINEL: &str = "private-jsonrpc-handler-secret";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE: OnceLock<Arc<CompiledGatewayFixture>> = OnceLock::new();

#[tokio::test]
async fn runtime_websocket_jsonrpc_record_and_array_params_use_linked_type_plans() {
    let fixture = fixture();

    let record = execute(
        fixture.target("record", 1),
        br#"{"value":"record-value"}"#,
        "connection-1",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        success_json(record),
        json!({"accepted": true, "value": "record-value"})
    );

    let array = execute(
        fixture.target("arrayResult", 1),
        br#"["first","second"]"#,
        "connection-1",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(success_json(array), json!(["first", "second"]));
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_invalid_params_never_enter_handler() {
    let fixture = fixture();
    let target = fixture.target("invalidProbe", 1);
    let oversized = vec![b' '; RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES + 1];
    let invalid = [
        b"{".as_slice(),
        b"42".as_slice(),
        b"null".as_slice(),
        b"\"scalar\"".as_slice(),
        br#"{"value":42}"#.as_slice(),
        [0xff, 0xfe].as_slice(),
        oversized.as_slice(),
    ];

    for params in invalid {
        let terminal = execute(
            target.clone(),
            params,
            "connection-1",
            None,
            CancellationToken::new(),
            None,
        )
        .await;
        assert_eq!(
            terminal,
            RuntimeWebSocketJsonRpcExecutionTerminal::Response(
                RuntimeWebSocketJsonRpcExecutionOutcome::InvalidParams
            ),
            "invalid params must fail before the throwing handler can run"
        );
    }
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_connection_and_business_identity_ignore_peer_spoofing() {
    let fixture = fixture();
    let target = fixture.target("identity", 1);
    let peer = br#"{"connectionId":"peer-connection","businessIdentity":"peer-business"}"#;

    let present = execute(
        target.clone(),
        peer,
        "trusted-connection",
        Some("trusted-business"),
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        success_json(present),
        json!({
            "businessIdentity": "trusted-business",
            "connectionId": "trusted-connection",
            "peerBusinessIdentity": "peer-business",
            "peerConnectionId": "peer-connection"
        })
    );

    let absent = execute(
        target,
        peer,
        "trusted-connection",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        success_json(absent),
        json!({
            "businessIdentity": null,
            "connectionId": "trusted-connection",
            "peerBusinessIdentity": "peer-business",
            "peerConnectionId": "peer-connection"
        })
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_string_record_array_union_and_void_results_are_typed() {
    let fixture = fixture();
    let params = br#"{"value":"value"}"#;

    assert_eq!(
        success_bytes(
            execute(
                fixture.target("stringResult", 1),
                params,
                "connection-1",
                None,
                CancellationToken::new(),
                None,
            )
            .await
        ),
        br#""value""#
    );
    assert_eq!(
        success_json(
            execute(
                fixture.target("record", 1),
                params,
                "connection-1",
                None,
                CancellationToken::new(),
                None,
            )
            .await
        ),
        json!({"accepted": true, "value": "value"})
    );
    assert_eq!(
        success_json(
            execute(
                fixture.target("arrayFromRecord", 1),
                params,
                "connection-1",
                None,
                CancellationToken::new(),
                None,
            )
            .await
        ),
        json!(["value"])
    );
    assert_eq!(
        success_json(
            execute(
                fixture.target("unionResult", 1),
                params,
                "connection-1",
                None,
                CancellationToken::new(),
                None,
            )
            .await
        ),
        json!({"tag": "ok", "value": "value"})
    );

    let expected_failure = execute(
        fixture.target("expectedFailure", 1),
        params,
        "connection-1",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        success_json(expected_failure),
        json!({"reason": "expected", "tag": "expectedFailure"}),
        "a business failure represented by the return union remains success"
    );

    assert_eq!(
        success_bytes(
            execute(
                fixture.target("acknowledge", 1),
                params,
                "connection-1",
                None,
                CancellationToken::new(),
                None,
            )
            .await
        ),
        b"null",
        "void must encode to the exact four JSON null bytes"
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_throw_is_sanitized_internal_error() {
    let fixture = fixture();
    let terminal = execute(
        fixture.target("throws", 1),
        br#"{"value":"value"}"#,
        "connection-1",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::InternalError
        )
    );
    let debug = format!("{terminal:?}");
    assert!(!debug.contains(PRIVATE_SENTINEL));
    assert!(!debug.contains("PrivateFailure"));
    assert!(!debug.contains("stack"));
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_return_encode_failure_and_oversize_are_internal_only() {
    let mut heap = RequestHeap::default();
    let integer_plan =
        RuntimeTypePlan::synthetic_named_builtin("integer", RuntimeTypeNode::Integer, vec![]);
    assert!(encode_result(
        &RuntimeValue::String(PRIVATE_SENTINEL.to_string()),
        &integer_plan,
        &mut heap
    )
    .is_err());

    let string_plan =
        RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, vec![]);
    assert!(encode_result(
        &RuntimeValue::String("x".repeat(RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES + 1)),
        &string_plan,
        &mut heap
    )
    .is_err());

    let terminal = RuntimeWebSocketJsonRpcExecutionTerminal::internal_error();
    assert_eq!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::InternalError
        )
    );
    assert!(!format!("{terminal:?}").contains(PRIVATE_SENTINEL));

    let value = "x".repeat(RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES - 16);
    let params = format!(r#"{{"value":"{value}"}}"#).into_bytes();
    assert!(params.len() <= RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES);
    assert_internal(
        execute(
            fixture().target("record", 1),
            &params,
            "connection-1",
            None,
            CancellationToken::new(),
            None,
        )
        .await,
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_cancel_disconnect_has_no_response_and_drops_late_result() {
    let fixture = fixture();
    let target = fixture.target("slow", 1);
    let cancellation = CancellationSource::new();
    let token = cancellation.token();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel.cancel();
    });

    let terminal = execute(
        target,
        br#"{"value":"late"}"#,
        "connection-1",
        None,
        token,
        None,
    )
    .await;
    assert_eq!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
    );
    tokio::time::sleep(Duration::from_millis(220)).await;
    assert!(
        matches!(
            terminal,
            RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
        ),
        "a late handler value cannot replace an internal cancelled terminal"
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_deadline_and_cancel_race_use_biased_cancel() {
    let fixture = fixture();
    let target = fixture.target("slow", 1);

    let deadline = Instant::now()
        .checked_add(Duration::from_millis(10))
        .expect("test deadline");
    let deadline_terminal = execute(
        target.clone(),
        br#"{"value":"late"}"#,
        "connection-1",
        None,
        CancellationToken::new(),
        Some(deadline),
    )
    .await;
    assert_eq!(
        deadline_terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded
        )
    );
    let RuntimeWebSocketJsonRpcExecutionTerminal::Response(outcome) = &deadline_terminal else {
        panic!("deadline must be response-producing")
    };
    assert_eq!(outcome.payload(), None);

    let cancellation = CancellationSource::new();
    cancellation.cancel();
    let simultaneous = execute(
        target,
        br#"{"value":"late"}"#,
        "connection-1",
        None,
        cancellation.token(),
        Some(Instant::now()),
    )
    .await;
    assert_eq!(
        simultaneous,
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled,
        "ancestor/peer cancellation wins when cancellation and deadline are both ready"
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_revalidates_formals_sources_before_handler() {
    let fixture = fixture();
    let params = br#"{"connectionId":"peer","businessIdentity":"peer"}"#;

    let mut reordered = fixture.target("identity", 1);
    reordered.plan.args.swap(0, 2);
    assert_internal(
        execute(
            reordered,
            params,
            "trusted",
            Some("trusted"),
            CancellationToken::new(),
            None,
        )
        .await,
    );

    let mut duplicate = fixture.target("identity", 1);
    duplicate.plan.args[2].source = GatewayAdapterSource::WebSocketConnectionId;
    assert_internal(
        execute(
            duplicate,
            params,
            "trusted",
            Some("trusted"),
            CancellationToken::new(),
            None,
        )
        .await,
    );

    let mut unknown = fixture.target("identity", 1);
    unknown.plan.args[0].source = GatewayAdapterSource::HttpBody;
    assert_internal(
        execute(
            unknown,
            params,
            "trusted",
            Some("trusted"),
            CancellationToken::new(),
            None,
        )
        .await,
    );

    let mut missing = fixture.target("identity", 1);
    missing.plan.args.pop();
    assert_internal(
        execute(
            missing,
            params,
            "trusted",
            Some("trusted"),
            CancellationToken::new(),
            None,
        )
        .await,
    );

    let mut unknown_formal = fixture.target("identity", 1);
    unknown_formal.plan.args[0].param = "forged".to_string();
    assert_internal(
        execute(
            unknown_formal,
            params,
            "trusted",
            Some("trusted"),
            CancellationToken::new(),
            None,
        )
        .await,
    );
}

#[tokio::test]
async fn runtime_websocket_jsonrpc_pinned_old_a_executes_after_replacement_b() {
    let mut active_fixture = compile_version_fixture("old");
    let old_target = active_fixture.target("versionResult", 1);
    active_fixture = compile_version_fixture("new");
    let new_target = active_fixture.target("versionResult", 2);

    let old = execute(
        old_target,
        br#"{}"#,
        "connection-old",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    let new = execute(
        new_target,
        br#"{}"#,
        "connection-new",
        None,
        CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(success_bytes(old), br#""old""#);
    assert_eq!(success_bytes(new), br#""new""#);
}

#[derive(Clone)]
struct TestCallable {
    id: PackageCallableId,
    signature: PackageCallableSignature,
    addr: ExecutableAddr,
}

#[derive(Clone)]
struct TestGatewayTarget {
    eval: RuntimeAssemblyEvalTarget,
    key: GatewayEntryKey,
    identity: GatewayEntryIdentity,
    surface: GatewayEntryProtocolSurface,
    plan: GatewayAdapterPlan,
    handler: TestCallable,
}

impl RuntimeWebSocketJsonRpcExecutionTarget for TestGatewayTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.key
    }

    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        &self.surface
    }

    fn adapter_plan(&self) -> &GatewayAdapterPlan {
        &self.plan
    }

    fn handler(&self) -> RuntimeWebSocketJsonRpcCallable<'_> {
        RuntimeWebSocketJsonRpcCallable {
            callable_id: &self.handler.id,
            signature: &self.handler.signature,
            addr: &self.handler.addr,
        }
    }
}

struct CompiledGatewayFixture {
    assembly: Arc<RuntimeAssembly>,
    deployment: Arc<ServiceDeployment>,
    implementation: Arc<PackageArtifact>,
    image: Arc<AssemblyExecutionImage>,
}

impl CompiledGatewayFixture {
    fn target(&self, key: &str, generation: u64) -> TestGatewayTarget {
        let key = GatewayEntryKey::parse(key).expect("fixture gateway key");
        let entry = self
            .deployment
            .gateway_entries
            .get(&key)
            .unwrap_or_else(|| panic!("missing fixture gateway entry {key}"));
        TestGatewayTarget {
            eval: self.eval_target(generation),
            key,
            identity: entry.gateway_entry_identity.clone(),
            surface: entry.protocol_surface.clone(),
            plan: entry.adapter_plan.clone(),
            handler: self.callable(
                entry
                    .handler
                    .as_ref()
                    .expect("JSON-RPC fixture entry requires a handler")
                    .as_str(),
            ),
        }
    }

    fn callable(&self, selector_or_id: &str) -> TestCallable {
        let (id, signature) = self
            .implementation
            .package_local_abi
            .implementation_symbols
            .iter()
            .find_map(|(selector, symbol)| match symbol {
                PackageLocalAbiSymbol::Callable {
                    callable_id,
                    signature,
                } if selector == selector_or_id || callable_id.as_str() == selector_or_id => {
                    Some((callable_id.clone(), signature.clone()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing fixture callable {selector_or_id}"));
        let target = self
            .implementation
            .callable_links
            .get(&id)
            .map(|fact| &fact.target)
            .expect("fixture callable target");
        let addr = self
            .image
            .entry_executable(&self.implementation.package_build_id, target)
            .expect("fixture executable address")
            .addr()
            .clone();
        TestCallable {
            id,
            signature,
            addr,
        }
    }

    fn eval_target(&self, generation: u64) -> RuntimeAssemblyEvalTarget {
        let deployment_ref = service_deployment_ref(&self.deployment);
        let activation_template = self
            .assembly
            .activation_templates
            .iter()
            .find(|template| template.deployment == deployment_ref)
            .expect("fixture activation template");
        let binding_template = self
            .assembly
            .service_binding_templates
            .iter()
            .find(|template| template.activation == activation_template.deployment)
            .expect("fixture service binding template");
        let activation = ActivationContext::from_assembly_templates(
            self.assembly.assembly_identity.clone(),
            generation,
            format!("runtime-websocket-jsonrpc-test-{generation}"),
            activation_template,
            binding_template,
        )
        .expect("fixture activation context");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(FixtureEvalResolver {
            activation: Arc::clone(&activation),
        });
        let request =
            RequestActivationContext::begin(activation).expect("fixture request activation");
        RuntimeAssemblyEvalTarget::new(Arc::clone(&self.image), request, resolver)
            .expect("fixture eval target")
    }
}

struct FixtureEvalResolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for FixtureEvalResolver {
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
    ) -> Option<AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &skiff_artifact_model::ContractOperationId,
    ) -> Option<OperationTargetRef> {
        None
    }
}

async fn execute(
    target: TestGatewayTarget,
    params: &[u8],
    connection_id: &str,
    business_identity: Option<&str>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
) -> RuntimeWebSocketJsonRpcExecutionTerminal {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    interpreter
        .execute_runtime_websocket_jsonrpc(
            execution_context(&interpreter, target.eval.clone()),
            RuntimeWebSocketJsonRpcRequest {
                params,
                connection_id,
                business_identity,
            },
            &target,
            cancellation,
            deadline,
        )
        .await
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: DbCapabilityContext::unavailable(),
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
        runtime_activation: Arc::new(RuntimeActivation {
            service: skiff_runtime_linked_program::ServiceMeta {
                id: SERVICE_ID.to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: VERSION.to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: Default::default(),
        }),
        actor: actor.clone(),
        spawn: actor,
        outbound: test_runtime::outbound_context(),
        request_heap_limits: RequestHeapLimits::default(),
    })
    .with_websocket_capability_rebinder(test_runtime::websocket_rebinder())
    .with_runtime_assembly_target(target)
}

fn success_bytes(terminal: RuntimeWebSocketJsonRpcExecutionTerminal) -> Vec<u8> {
    let RuntimeWebSocketJsonRpcExecutionTerminal::Response(
        RuntimeWebSocketJsonRpcExecutionOutcome::Success { payload },
    ) = terminal
    else {
        panic!("expected JSON-RPC success, got {terminal:?}")
    };
    assert!(std::str::from_utf8(&payload).is_ok());
    payload
}

fn success_json(terminal: RuntimeWebSocketJsonRpcExecutionTerminal) -> Value {
    serde_json::from_slice(&success_bytes(terminal)).expect("success payload JSON")
}

fn assert_internal(terminal: RuntimeWebSocketJsonRpcExecutionTerminal) {
    assert_eq!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::InternalError
        )
    );
}

fn fixture() -> Arc<CompiledGatewayFixture> {
    Arc::clone(FIXTURE.get_or_init(|| {
        Arc::new(compile_fixture(
            "typed-kernel",
            typed_websocket_source(),
            typed_handler_source(),
        ))
    }))
}

fn compile_version_fixture(value: &str) -> CompiledGatewayFixture {
    compile_fixture(
        &format!("version-{value}"),
        r#"path: /socket
jsonRpc:
  versionResult:
    method: version.get
    handler: main.versionResult
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
"#,
        &format!(
            r#"type VersionParams {{}}

function versionResult(params: VersionParams) -> string {{
  return "{value}"
}}
"#
        ),
    )
}

fn compile_fixture(
    name: &str,
    websocket_source: &str,
    handler_source: &str,
) -> CompiledGatewayFixture {
    let temp = TempFixture::new(name);
    let service_root = temp.child("service");
    let artifact_root = temp.child("artifacts");
    write_service_fixture(&service_root, websocket_source, handler_source);
    let platform = repository_platform_sources();
    seed_canonical_std(&platform, &artifact_root).expect("canonical std seed");
    let output = build_authoring_object(
        &platform,
        AuthoringObject::Package,
        &service_root,
        &artifact_root,
        "dev",
        true,
    )
    .expect("WebSocket JSON-RPC service authoring");
    let root_package_ref =
        serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
            .expect("JSON-RPC package ref");
    let deployment_ref: ServiceDeploymentRef =
        serde_json::from_value(output["serviceDeploymentReceipt"]["deployment"].clone())
            .expect("JSON-RPC deployment ref");
    let contract_ref: ServiceContractRef =
        serde_json::from_value(output["serviceContractReceipt"]["contract"].clone())
            .expect("JSON-RPC contract ref");
    let store = CanonicalArtifactStore::open(&artifact_root).expect("JSON-RPC artifact store");
    let deployment = store
        .read_service_deployment(&deployment_ref)
        .expect("JSON-RPC deployment");
    let contract = store
        .read_service_contract(&contract_ref)
        .expect("JSON-RPC contract");
    let implementation = store
        .read_package_artifact(&root_package_ref)
        .expect("JSON-RPC implementation");
    let mut package_refs =
        BTreeMap::from([(implementation.package_build_id.clone(), root_package_ref)]);
    for binding in &deployment.package_bindings {
        package_refs.insert(
            binding.package.package_build_id.clone(),
            binding.package.clone(),
        );
    }
    let packages = package_refs
        .values()
        .map(|reference| store.read_package_artifact(reference))
        .collect::<Result<Vec<_>, _>>()
        .expect("JSON-RPC package closure");
    let package_values = packages
        .iter()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let root = service_deployment_ref(&deployment);
    let assembly = Arc::new(
        resolve_runtime_assembly(
            std::slice::from_ref(&root),
            std::slice::from_ref(deployment.as_ref()),
            std::slice::from_ref(contract.as_ref()),
            &package_values,
        )
        .expect("JSON-RPC runtime assembly"),
    );
    let hydrated = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| hydrate_package(&store, &slot.package))
        .collect::<Vec<_>>();
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
            .expect("JSON-RPC execution image");
    CompiledGatewayFixture {
        assembly,
        deployment,
        implementation,
        image,
    }
}

fn hydrate_package(
    store: &CanonicalArtifactStore,
    reference: &skiff_artifact_model::PackageArtifactRef,
) -> HydratedPackageCode {
    let artifact = store
        .read_package_artifact(reference)
        .expect("fixture package artifact");
    let files = artifact
        .files
        .iter()
        .map(|file| store.read_file_ir(reference, file))
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture File IR closure");
    let schema_index = Arc::new(PackageSchemaIndex {
        package_id: artifact.package_schema_index.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    });
    HydratedPackageCode::new(artifact, files, PublicationResourceTable::default())
        .with_schema_index(schema_index)
}

fn service_deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn write_service_fixture(root: &Path, websocket_source: &str, handler_source: &str) {
    fs::create_dir_all(root).expect("JSON-RPC fixture directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n"),
    )
    .expect("JSON-RPC package manifest");
    fs::write(root.join("api.yml"), "{}\n").expect("JSON-RPC API");
    fs::write(root.join("service.yml"), format!("id: {SERVICE_ID}\n"))
        .expect("JSON-RPC service manifest");
    fs::write(root.join("websocket.yml"), websocket_source).expect("JSON-RPC WebSocket manifest");
    fs::write(
            root.join("config.dev.yml"),
            "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:runtime-websocket-jsonrpc\nlifecycle: { maxConcurrency: 1 }\n",
        )
        .expect("JSON-RPC config");
    fs::write(root.join("main.skiff"), handler_source).expect("JSON-RPC source");
}

fn typed_websocket_source() -> &'static str {
    r#"path: /socket
jsonRpc:
  record:
    method: result.record
    handler: main.record
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  arrayResult:
    method: params.array
    handler: main.arrayResult
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  identity:
    method: identity.read
    handler: main.identity
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
  stringResult:
    method: result.string
    handler: main.stringResult
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  arrayFromRecord:
    method: result.array
    handler: main.arrayFromRecord
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  unionResult:
    method: result.union
    handler: main.unionResult
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  expectedFailure:
    method: result.expectedFailure
    handler: main.expectedFailure
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  acknowledge:
    method: result.void
    handler: main.acknowledge
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  throws:
    method: result.throw
    handler: main.throws
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  invalidProbe:
    method: params.invalid
    handler: main.invalidProbe
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  slow:
    method: result.slow
    handler: main.slow
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
"#
}

fn typed_handler_source() -> &'static str {
    r#"import std

type EchoParams { value: string }
type SpoofParams { connectionId: string, businessIdentity: string }
type RecordResult { value: string, accepted: bool }
type IdentityResult {
  connectionId: string,
  businessIdentity: string?,
  peerConnectionId: string,
  peerBusinessIdentity: string,
}
type ResultUnion discriminator "tag" =
  { tag: "ok", value: string }
  | { tag: "expectedFailure", reason: string }
type PrivateFailure { message: string }

function record(params: EchoParams) -> RecordResult {
  return { value: params.value, accepted: true }
}

function arrayResult(params: Array<string>) -> Array<string> {
  return params
}

function identity(
  params: SpoofParams,
  connectionId: string,
  businessIdentity: string?
) -> IdentityResult {
  return {
    connectionId: connectionId,
    businessIdentity: businessIdentity,
    peerConnectionId: params.connectionId,
    peerBusinessIdentity: params.businessIdentity,
  }
}

function stringResult(params: EchoParams) -> string {
  return params.value
}

function arrayFromRecord(params: EchoParams) -> Array<string> {
  const items = Array.empty<string>()
  items.push(params.value)
  return items
}

function unionResult(params: EchoParams) -> ResultUnion {
  return { tag: "ok", value: params.value }
}

function expectedFailure(params: EchoParams) -> ResultUnion {
  return { tag: "expectedFailure", reason: "expected" }
}

function acknowledge(params: EchoParams) -> void {}

function throws(params: EchoParams) -> string {
  throw PrivateFailure { message: "private-jsonrpc-handler-secret" }
}

function invalidProbe(params: EchoParams) -> string {
  throw PrivateFailure { message: "invalid params reached handler" }
}

function slow(params: EchoParams) -> string {
  std.time.sleep(Duration.milliseconds(200))
  return params.value
}
"#
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval must live below the Skiff root")
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
            "skiff-runtime-eval-jsonrpc-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("JSON-RPC temp fixture root");
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
