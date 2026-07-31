use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    runtime_value::RuntimeObject,
    service_error::{
        CatchIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, WebSocketRequestErrorKind,
    },
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

use super::*;

fn local_owner() -> NamedUnionOwnerIdentity {
    NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::loaded_file(0),
            type_index: 7,
        },
        type_arguments: Vec::new(),
    })
}

fn invocation(owner: Option<NamedUnionOwnerIdentity>) -> RuntimeNativeInvocation {
    invocation_with_payload_plan(owner, RuntimeTypePlan::synthetic_request_record(Vec::new()))
}

fn invocation_with_payload_plan(
    owner: Option<NamedUnionOwnerIdentity>,
    payload_plan: RuntimeTypePlan,
) -> RuntimeNativeInvocation {
    let mut plan = NativeCallPlan::new(
        NativeBindingKey::from_static("std.websocket.requestJsonToConnection"),
        vec![
            RuntimeTypePlan::new("string", None, RuntimeTypeNode::String),
            RuntimeTypePlan::new("string", None, RuntimeTypeNode::String),
            payload_plan,
        ],
        RuntimeTypePlan::json_value_plan(),
        NativeRequiredContext::Websocket,
    );
    if let Some(owner) = owner {
        plan = plan
            .with_named_union_error_owner(owner)
            .expect("request binding admits exact owner");
    }
    RuntimeNativeInvocation::new(
        "std.websocket.requestJsonToConnection".to_string(),
        "std.websocket.requestJsonToConnection",
        Some(plan),
        None,
        None,
    )
}

struct TerminalWebsocketContext {
    terminal: ConnectionRequestTerminal,
    calls: Arc<AtomicUsize>,
}

impl NativeWebsocketCapability for TerminalWebsocketContext {
    fn request_json_to_connection<'a>(
        &'a self,
        _connection_id: String,
        _method: String,
        _payload: Vec<u8>,
    ) -> crate::capability::NativeCapabilityFuture<'a, ConnectionRequestTerminal> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let terminal = self.terminal.clone();
        Box::pin(async move { Ok(terminal) })
    }

    fn send_connection_text_to_business_identity(&self, _: String, _: String) -> Result<()> {
        unreachable!()
    }

    fn send_connection_binary_to_business_identity(&self, _: String, _: Vec<u8>) -> Result<()> {
        unreachable!()
    }

    fn send_connection_text_to_connection(&self, _: String, _: String) -> Result<()> {
        unreachable!()
    }

    fn send_connection_binary_to_connection(&self, _: String, _: Vec<u8>) -> Result<()> {
        unreachable!()
    }
}

fn request_args(payload: RuntimeValue) -> Vec<RuntimeValue> {
    vec![
        RuntimeValue::String("connection-1".to_string()),
        RuntimeValue::String("status.get".to_string()),
        payload,
    ]
}

fn empty_object(heap: &mut RequestHeap) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(Default::default()))
            .expect("test object"),
    )
}

#[test]
fn websocket_request_error_materialization_requires_linked_union_owner() {
    let error = WebsocketNativeDispatch::request_error(
        &invocation(None),
        WebSocketRequestErrorKind::ProtocolError,
        "protocol",
        None,
        None,
    )
    .expect_err("missing exact owner must fail closed");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error.to_string().contains("linked named-union error owner"));
}

#[test]
fn websocket_request_error_materialization_keeps_all_five_exact_branches() {
    let owner = local_owner();
    let invocation = invocation(Some(owner.clone()));
    for kind in WebSocketRequestErrorKind::ALL {
        let remote = kind == WebSocketRequestErrorKind::Remote;
        let error = WebsocketNativeDispatch::request_error(
            &invocation,
            kind,
            "sanitized",
            remote.then_some(-32603),
            remote.then(|| serde_json::json!({"peer": true})),
        )
        .expect_err("request branch is an ordinary error");
        let RuntimeError::Opaque(error) = error else {
            panic!("expected exact opaque error");
        };
        assert_eq!(
            error.catch_projection().unwrap().0,
            CatchIdentity::NamedUnionBranch {
                union: owner.clone(),
                branch: NamedUnionBranchIdentity::SyntheticDiscriminator {
                    discriminator_field: "kind".to_string(),
                    discriminator_value: kind.discriminator().to_string(),
                },
            }
        );
    }
}

#[tokio::test]
async fn websocket_request_missing_owner_fails_before_capability_dispatch() {
    let calls = Arc::new(AtomicUsize::new(0));
    let context = TerminalWebsocketContext {
        terminal: ConnectionRequestTerminal::ProtocolError,
        calls: Arc::clone(&calls),
    };
    let mut heap = RequestHeap::default();
    let payload = empty_object(&mut heap);
    let error = WebsocketNativeDispatch::dispatch(
        context,
        invocation(None),
        "std.websocket.requestJsonToConnection".to_string(),
        request_args(payload),
        &mut heap,
    )
    .await
    .expect_err("missing owner must fail before the Host future");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn websocket_request_local_codec_failure_never_dispatches_capability() {
    let calls = Arc::new(AtomicUsize::new(0));
    let context = TerminalWebsocketContext {
        terminal: ConnectionRequestTerminal::Success(b"null".to_vec()),
        calls: Arc::clone(&calls),
    };
    let mut heap = RequestHeap::default();
    let error = WebsocketNativeDispatch::dispatch(
        context,
        invocation(Some(local_owner())),
        "std.websocket.requestJsonToConnection".to_string(),
        request_args(RuntimeValue::String("not-a-record".to_string())),
        &mut heap,
    )
    .await
    .expect_err("local payload mismatch must remain a JSON codec error");

    assert!(matches!(
        error,
        RuntimeError::DecodeTarget { ref target, .. } if target == "std.json.encode"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn websocket_request_deadline_and_ancestor_cancel_keep_distinct_terminals() {
    for (terminal, expected) in [
        (
            ConnectionRequestTerminal::DeadlineExceeded,
            "deadlineExceeded",
        ),
        (
            ConnectionRequestTerminal::AncestorCancelled,
            "ancestorCancelled",
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let context = TerminalWebsocketContext {
            terminal,
            calls: Arc::clone(&calls),
        };
        let mut heap = RequestHeap::default();
        let payload = empty_object(&mut heap);
        let error = WebsocketNativeDispatch::dispatch(
            context,
            invocation(Some(local_owner())),
            "std.websocket.requestJsonToConnection".to_string(),
            request_args(payload),
            &mut heap,
        )
        .await
        .expect_err("terminal must not become a success value");
        match expected {
            "deadlineExceeded" => assert!(matches!(
                error,
                RuntimeError::ExecutionBudgetExceeded {
                    reason: crate::error::BudgetReason::DeadlineExceeded,
                    ..
                }
            )),
            "ancestorCancelled" => assert!(matches!(error, RuntimeError::Cancelled)),
            _ => unreachable!(),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
